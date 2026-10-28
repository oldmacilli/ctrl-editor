//! The bounded record of everything that passed capture (docs/MONITOR.md
//! §2, §3.2): events in a ring bounded by count, payloads in an arena
//! bounded by bytes, so one long dump does not push out the last minutes of
//! notes. What was evicted or dropped is counted and shown, never hidden.

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};

use crate::model::{Annotation, BlobIdx, Dir, Event, Msg, PortIdx, PortKind, PortRef, Stamp};
use crate::parser::Parsed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreConfig {
    /// Events kept.
    pub events: usize,
    /// Payload bytes kept (SysEx, HID, invalid runs).
    pub bytes: usize,
}

impl Default for StoreConfig {
    fn default() -> Self {
        StoreConfig {
            events: 100_000,
            bytes: 64 << 20,
        }
    }
}

/// Counters for the alert strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub events: usize,
    pub bytes: usize,
    /// Events pushed out by newer ones.
    pub evicted_events: u64,
    /// Payloads pushed out while their event is still kept.
    pub evicted_payloads: u64,
    /// Packets the taps dropped because a ring was full.
    pub dropped: u64,
}

/// A SysEx still arriving.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Receiving {
    pub port: PortIdx,
    pub dir: Dir,
    pub since: Stamp,
    pub len: usize,
}

#[derive(Debug, Default)]
pub struct Store {
    config: StoreConfig,
    ports: Vec<PortRef>,
    events: VecDeque<Event>,
    next_seq: u64,
    blobs: VecDeque<(BlobIdx, Vec<u8>)>,
    next_blob: u64,
    blob_bytes: usize,
    notes: BTreeMap<u64, Vec<Annotation>>,
    receiving: BTreeMap<(PortIdx, Dir), Receiving>,
    stats: Stats,
}

impl Store {
    pub fn new(config: StoreConfig) -> Self {
        Store {
            config,
            ..Store::default()
        }
    }

    /// The index of `port`, added when new. The same kind and id give the
    /// same index; a new name replaces the old one.
    pub fn port(&mut self, port: PortRef) -> PortIdx {
        if let Some(i) = self
            .ports
            .iter()
            .position(|p| p.kind == port.kind && p.id == port.id)
        {
            self.ports[i].name = port.name;
            return PortIdx(i as u16);
        }
        assert!(self.ports.len() < u16::MAX as usize, "too many ports");
        self.ports.push(port);
        PortIdx(self.ports.len() as u16 - 1)
    }

    pub fn ports(&self) -> &[PortRef] {
        &self.ports
    }

    pub fn port_ref(&self, idx: PortIdx) -> Option<&PortRef> {
        self.ports.get(idx.0 as usize)
    }

    /// Records one parser result; returns the new event's `seq`.
    pub fn record(&mut self, port: PortIdx, dir: Dir, parsed: Parsed) -> Option<u64> {
        let (at, msg) = match parsed {
            Parsed::Voice { at, status, d1, d2 } => (at, Msg::Voice { status, d1, d2 }),
            Parsed::Common { at, status, d1, d2 } => (at, Msg::Common { status, d1, d2 }),
            Parsed::Realtime { at, status } => (at, Msg::Realtime(status)),
            Parsed::SysEx { at, bytes, end } => {
                self.receiving.remove(&(port, dir));
                let len = bytes.len() as u32;
                (
                    at,
                    Msg::SysEx {
                        blob: self.keep(bytes),
                        len,
                        end,
                    },
                )
            }
            Parsed::Invalid { at, bytes } => {
                let len = bytes.len() as u32;
                (
                    at,
                    Msg::Invalid {
                        blob: self.keep(bytes),
                        len,
                    },
                )
            }
            Parsed::Progress { at, len } => {
                self.receiving
                    .entry((port, dir))
                    .and_modify(|r| r.len = len)
                    .or_insert(Receiving {
                        port,
                        dir,
                        since: at,
                        len,
                    });
                return None;
            }
        };
        Some(self.push(at, port, dir, msg))
    }

    /// Records a HID report.
    pub fn record_hid(&mut self, at: Stamp, port: PortIdx, dir: Dir, report: &[u8]) -> u64 {
        let len = report.len() as u32;
        let blob = self.keep(report.to_vec());
        self.push(at, port, dir, Msg::Hid { blob, len })
    }

    /// Records a message without payload (UMP, or a voice message a
    /// producer already parsed).
    pub fn record_msg(&mut self, at: Stamp, port: PortIdx, dir: Dir, msg: Msg) -> u64 {
        debug_assert!(
            msg.blob().is_none(),
            "payloads go through record or record_hid"
        );
        self.push(at, port, dir, msg)
    }

    pub fn annotate(&mut self, seq: u64, note: Annotation) {
        if let Some(e) = self.get_mut(seq) {
            e.annotated = true;
            self.notes.entry(seq).or_default().push(note);
        }
    }

    pub fn notes(&self, seq: u64) -> &[Annotation] {
        self.notes.get(&seq).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Kept events, oldest first.
    pub fn events(&self) -> impl DoubleEndedIterator<Item = &Event> + ExactSizeIterator {
        self.events.iter()
    }

    /// Events with `seq >= from`, oldest first.
    pub fn since(&self, from: u64) -> impl DoubleEndedIterator<Item = &Event> {
        let skip = from.saturating_sub(self.first_seq()) as usize;
        self.events.iter().skip(skip)
    }

    pub fn get(&self, seq: u64) -> Option<&Event> {
        let i = seq.checked_sub(self.first_seq())?;
        self.events.get(i as usize)
    }

    fn get_mut(&mut self, seq: u64) -> Option<&mut Event> {
        let i = seq.checked_sub(self.first_seq())?;
        self.events.get_mut(i as usize)
    }

    /// The `seq` of the oldest kept event (the next one when empty).
    pub fn first_seq(&self) -> u64 {
        self.events.front().map_or(self.next_seq, |e| e.seq)
    }

    /// The `seq` the next event gets.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// A payload, or `None` once it was evicted.
    pub fn bytes(&self, blob: BlobIdx) -> Option<&[u8]> {
        let front = self.blobs.front()?.0 .0;
        let (idx, bytes) = self.blobs.get(blob.0.checked_sub(front)? as usize)?;
        debug_assert_eq!(*idx, blob);
        Some(bytes)
    }

    /// The message as it went over the wire: rebuilt for short messages,
    /// the kept payload for long ones (`None` once evicted, and for UMP).
    pub fn wire(&self, msg: &Msg) -> Option<Cow<'_, [u8]>> {
        match *msg {
            Msg::Voice { status, d1, d2 } => Some(Cow::Owned(match status & 0xF0 {
                0xC0 | 0xD0 => vec![status, d1],
                _ => vec![status, d1, d2],
            })),
            Msg::Common { status, d1, d2 } => Some(Cow::Owned(match status {
                0xF1 | 0xF3 => vec![status, d1],
                0xF2 => vec![status, d1, d2],
                _ => vec![status],
            })),
            Msg::Realtime(status) => Some(Cow::Owned(vec![status])),
            Msg::Ump(..) => None,
            Msg::SysEx { blob, .. } | Msg::Hid { blob, .. } | Msg::Invalid { blob, .. } => {
                self.bytes(blob).map(Cow::Borrowed)
            }
        }
    }

    /// SysEx messages still arriving.
    pub fn receiving(&self) -> impl Iterator<Item = &Receiving> {
        self.receiving.values()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            events: self.events.len(),
            bytes: self.blob_bytes,
            ..self.stats
        }
    }

    pub(crate) fn add_dropped(&mut self, n: u64) {
        self.stats.dropped += n;
    }

    /// Forgets every event and payload; ports and counters of drops stay.
    pub fn clear(&mut self) {
        self.events.clear();
        self.blobs.clear();
        self.blob_bytes = 0;
        self.notes.clear();
        self.receiving.clear();
    }

    pub(crate) fn is_hid(&self, port: PortIdx) -> bool {
        self.port_ref(port).is_some_and(|p| p.kind == PortKind::Hid)
    }

    fn push(&mut self, at: Stamp, port: PortIdx, dir: Dir, msg: Msg) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push_back(Event {
            seq,
            at,
            port,
            dir,
            msg,
            annotated: false,
        });
        while self.events.len() > self.config.events.max(1) {
            self.evict_event();
        }
        seq
    }

    fn evict_event(&mut self) {
        let Some(e) = self.events.pop_front() else {
            return;
        };
        self.stats.evicted_events += 1;
        if e.annotated {
            self.notes.remove(&e.seq);
        }
        // Payloads are kept in event order: everything up to this one's goes.
        if let Some(blob) = e.msg.blob() {
            while self.blobs.front().is_some_and(|(idx, _)| *idx <= blob) {
                let (_, bytes) = self.blobs.pop_front().unwrap();
                self.blob_bytes -= bytes.len();
            }
        }
    }

    fn keep(&mut self, bytes: Vec<u8>) -> BlobIdx {
        // Room for the new payload; one larger than the budget is still kept.
        while !self.blobs.is_empty() && self.blob_bytes + bytes.len() > self.config.bytes {
            let (_, old) = self.blobs.pop_front().unwrap();
            self.blob_bytes -= old.len();
            self.stats.evicted_payloads += 1;
        }
        let idx = BlobIdx(self.next_blob);
        self.next_blob += 1;
        self.blob_bytes += bytes.len();
        self.blobs.push_back((idx, bytes));
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SysExEnd;

    fn at(ns: u64) -> Stamp {
        Stamp {
            host_ns: ns,
            ..Stamp::default()
        }
    }

    fn note(store: &mut Store, n: u8) -> u64 {
        store
            .record(
                PortIdx(0),
                Dir::In,
                Parsed::Voice {
                    at: at(n as u64),
                    status: 0x90,
                    d1: n,
                    d2: 1,
                },
            )
            .unwrap()
    }

    fn sysex(store: &mut Store, len: usize) -> u64 {
        let bytes = vec![0x7F; len];
        store
            .record(
                PortIdx(0),
                Dir::In,
                Parsed::SysEx {
                    at: at(0),
                    bytes,
                    end: SysExEnd::Complete,
                },
            )
            .unwrap()
    }

    #[test]
    fn events_are_bounded_by_count_and_seq_shows_the_gap() {
        let mut store = Store::new(StoreConfig {
            events: 3,
            bytes: 1024,
        });
        for n in 0..5 {
            note(&mut store, n);
        }
        let seqs: Vec<u64> = store.events().map(|e| e.seq).collect();
        assert_eq!(seqs, [2, 3, 4]);
        assert_eq!(store.first_seq(), 2);
        assert_eq!(store.get(1), None);
        assert_eq!(
            store.get(3).unwrap().msg,
            Msg::Voice {
                status: 0x90,
                d1: 3,
                d2: 1
            }
        );
        assert_eq!(store.stats().evicted_events, 2);
        assert_eq!(store.since(3).count(), 2);
    }

    #[test]
    fn payloads_are_bounded_by_bytes_without_losing_their_events() {
        let mut store = Store::new(StoreConfig {
            events: 100,
            bytes: 100,
        });
        let first = sysex(&mut store, 60);
        let n = note(&mut store, 1);
        let second = sysex(&mut store, 60);
        let blob = |s: &Store, seq| s.get(seq).unwrap().msg.blob().unwrap();
        assert_eq!(
            store.bytes(blob(&store, first)),
            None,
            "the older dump made room"
        );
        assert_eq!(store.bytes(blob(&store, second)).unwrap().len(), 60);
        assert!(store.get(n).is_some(), "the note stays");
        assert_eq!(
            store.stats(),
            Stats {
                events: 3,
                bytes: 60,
                evicted_events: 0,
                evicted_payloads: 1,
                dropped: 0,
            }
        );
    }

    #[test]
    fn evicting_an_event_frees_its_payload() {
        let mut store = Store::new(StoreConfig {
            events: 2,
            bytes: 1000,
        });
        sysex(&mut store, 10);
        sysex(&mut store, 20);
        sysex(&mut store, 30);
        assert_eq!(store.stats().bytes, 50);
        assert_eq!(store.stats().evicted_payloads, 0);
    }

    #[test]
    fn annotations_follow_their_event() {
        let mut store = Store::new(StoreConfig {
            events: 1,
            bytes: 10,
        });
        let a = note(&mut store, 1);
        store.annotate(a, Annotation::State("scene 3".into()));
        assert!(store.get(a).unwrap().annotated);
        assert_eq!(store.notes(a), [Annotation::State("scene 3".into())]);
        note(&mut store, 2);
        assert!(store.notes(a).is_empty());
    }

    #[test]
    fn ports_are_found_again_by_kind_and_id() {
        let mut store = Store::default();
        let a = store.port(PortRef::new(
            PortKind::Alsa,
            "nanoKONTROL Studio:0",
            "nanoKONTROL Studio",
        ));
        let b = store.port(PortRef::new(PortKind::Jack, "nanoKONTROL Studio:0", "x"));
        let c = store.port(PortRef::new(
            PortKind::Alsa,
            "nanoKONTROL Studio:0",
            "renamed",
        ));
        assert_eq!((a, b, c), (PortIdx(0), PortIdx(1), PortIdx(0)));
        assert_eq!(store.port_ref(a).unwrap().name, "renamed");
    }

    #[test]
    fn progress_is_tracked_until_the_sysex_ends() {
        let mut store = Store::default();
        assert_eq!(
            store.record(
                PortIdx(0),
                Dir::In,
                Parsed::Progress {
                    at: at(5),
                    len: 256
                }
            ),
            None
        );
        store.record(
            PortIdx(0),
            Dir::In,
            Parsed::Progress {
                at: at(6),
                len: 512,
            },
        );
        let r: Vec<_> = store.receiving().copied().collect();
        assert_eq!(
            r,
            [Receiving {
                port: PortIdx(0),
                dir: Dir::In,
                since: at(5),
                len: 512
            }]
        );
        sysex(&mut store, 600);
        assert_eq!(store.receiving().count(), 0);
    }
}
