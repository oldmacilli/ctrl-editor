//! The real-time side (docs/MONITOR.md §3.3): a producer pushes raw packets
//! into two lock-free rings without allocating or blocking; the collector
//! drains them.
//!
//! A packet of up to [`INLINE`] bytes travels in its slot; a longer one
//! travels in the byte ring. Either both rings take a packet or neither does:
//! a packet that does not fit is dropped and counted, never cut.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::model::{Dir, PortIdx, Stamp};

/// Bytes that travel inside a slot.
pub const INLINE: usize = 16;

/// Ring sizes of one tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapConfig {
    /// Packets in flight.
    pub slots: usize,
    /// Bytes in flight for packets longer than [`INLINE`].
    pub bytes: usize,
}

impl Default for TapConfig {
    fn default() -> Self {
        TapConfig {
            slots: 4096,
            bytes: 1 << 20,
        }
    }
}

#[derive(Clone, Copy)]
struct Slot {
    at: Stamp,
    port: PortIdx,
    dir: Dir,
    len: u32,
    inline: [u8; INLINE],
}

/// The producer's end. One per producer thread.
pub struct Tap {
    slots: Producer<Slot>,
    bytes: Producer<u8>,
    dropped: Arc<AtomicU64>,
}

/// The collector's end.
pub struct TapReader {
    slots: Consumer<Slot>,
    bytes: Consumer<u8>,
    dropped: Arc<AtomicU64>,
    scratch: Vec<u8>,
}

/// One packet as the producer pushed it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Packet<'a> {
    pub at: Stamp,
    pub port: PortIdx,
    pub dir: Dir,
    pub bytes: &'a [u8],
}

/// A connected tap.
pub fn tap(config: TapConfig) -> (Tap, TapReader) {
    let (slot_tx, slot_rx) = RingBuffer::new(config.slots.max(1));
    let (byte_tx, byte_rx) = RingBuffer::new(config.bytes.max(1));
    let dropped = Arc::new(AtomicU64::new(0));
    (
        Tap {
            slots: slot_tx,
            bytes: byte_tx,
            dropped: dropped.clone(),
        },
        TapReader {
            slots: slot_rx,
            bytes: byte_rx,
            dropped,
            scratch: Vec::with_capacity(INLINE),
        },
    )
}

impl Tap {
    /// Pushes one packet. Returns false, and counts the drop, when a ring is
    /// full. Real-time safe: no allocation, no lock, no system call.
    pub fn push(&mut self, at: Stamp, port: PortIdx, dir: Dir, bytes: &[u8]) -> bool {
        let long = bytes.len() > INLINE;
        if self.slots.slots() == 0
            || (long && self.bytes.slots() < bytes.len())
            || bytes.len() > u32::MAX as usize
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let mut inline = [0u8; INLINE];
        if long {
            match self.bytes.write_chunk_uninit(bytes.len()) {
                Ok(chunk) => {
                    chunk.fill_from_iter(bytes.iter().copied());
                }
                Err(_) => {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
            }
        } else {
            inline[..bytes.len()].copy_from_slice(bytes);
        }
        let slot = Slot {
            at,
            port,
            dir,
            len: bytes.len() as u32,
            inline,
        };
        // Checked above, and only this end pushes.
        let pushed = self.slots.push(slot).is_ok();
        debug_assert!(pushed);
        pushed
    }

    /// Packets dropped so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl TapReader {
    /// Hands every waiting packet to `f`, oldest first, and returns how many.
    pub fn drain(&mut self, mut f: impl FnMut(Packet<'_>)) -> usize {
        let mut n = 0;
        while let Ok(slot) = self.slots.pop() {
            let len = slot.len as usize;
            if len > INLINE {
                self.scratch.clear();
                if let Ok(chunk) = self.bytes.read_chunk(len) {
                    let (a, b) = chunk.as_slices();
                    self.scratch.extend_from_slice(a);
                    self.scratch.extend_from_slice(b);
                    chunk.commit_all();
                }
                f(Packet {
                    at: slot.at,
                    port: slot.port,
                    dir: slot.dir,
                    bytes: &self.scratch,
                });
            } else {
                f(Packet {
                    at: slot.at,
                    port: slot.port,
                    dir: slot.dir,
                    bytes: &slot.inline[..len],
                });
            }
            n += 1;
        }
        n
    }

    /// Packets the producer dropped so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// True when the producer end is gone.
    pub fn is_abandoned(&self) -> bool {
        self.slots.is_abandoned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(ns: u64) -> Stamp {
        Stamp {
            host_ns: ns,
            ..Stamp::default()
        }
    }

    #[test]
    fn short_and_long_packets_come_back_in_order() {
        let (mut tx, mut rx) = tap(TapConfig {
            slots: 8,
            bytes: 64,
        });
        let long: Vec<u8> = (0..40).collect();
        assert!(tx.push(stamp(1), PortIdx(0), Dir::In, &[0x90, 60, 100]));
        assert!(tx.push(stamp(2), PortIdx(1), Dir::Out, &long));
        let mut got = Vec::new();
        rx.drain(|p| got.push((p.at.host_ns, p.port, p.dir, p.bytes.to_vec())));
        assert_eq!(
            got,
            [
                (1, PortIdx(0), Dir::In, vec![0x90, 60, 100]),
                (2, PortIdx(1), Dir::Out, long)
            ]
        );
    }

    #[test]
    fn a_packet_that_does_not_fit_is_dropped_whole_and_counted() {
        let (mut tx, mut rx) = tap(TapConfig {
            slots: 2,
            bytes: 32,
        });
        assert!(!tx.push(stamp(1), PortIdx(0), Dir::In, &[0xF0; 33]));
        assert!(tx.push(stamp(2), PortIdx(0), Dir::In, &[0xF8]));
        assert!(tx.push(stamp(3), PortIdx(0), Dir::In, &[0xF8]));
        assert!(!tx.push(stamp(4), PortIdx(0), Dir::In, &[0xF8]));
        assert_eq!(rx.dropped(), 2);
        assert_eq!(rx.drain(|_| {}), 2);
        assert!(tx.push(stamp(5), PortIdx(0), Dir::In, &[0xF0; 32]));
        let mut len = 0;
        rx.drain(|p| len = p.bytes.len());
        assert_eq!(len, 32);
    }

    #[test]
    fn long_packets_wrap_around_the_byte_ring() {
        let (mut tx, mut rx) = tap(TapConfig {
            slots: 4,
            bytes: 50,
        });
        for round in 0..10u8 {
            let packet = vec![round; 30];
            assert!(tx.push(stamp(round as u64), PortIdx(0), Dir::In, &packet));
            let mut got = Vec::new();
            rx.drain(|p| got = p.bytes.to_vec());
            assert_eq!(got, packet);
        }
    }
}
