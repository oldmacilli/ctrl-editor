//! Long SysEx (CDL §8.2).
//!
//! Two layers, both driven by the descriptor:
//!
//! 1. The host stack. Some drivers carry at most a few KiB of SysEx per
//!    write or packet (Linux ALSA rawmidi buffers are 4096 bytes by
//!    default). A transport's `sysex-limit` makes the engine write a longer
//!    message in pieces of at most that many bytes, `segment-delay` ms
//!    apart. The pieces are consecutive bytes of one message: the device
//!    sees one SysEx. [`split_sysex`] cuts them; [`SysexJoiner`] puts
//!    pieces back together on input. This works for every device.
//! 2. The device protocol. A device that has its own multi-message form of
//!    a long transfer describes it with `segment` on the message; the
//!    engine then sends that message once per piece ([`Codec::plan`]).

use ctrl_cdl::{Role, SELECTED_TRANSPORT};

use crate::{Bindings, Codec, CodecError, Val};

/// Cuts one message into consecutive pieces of at most `limit` bytes. The
/// pieces joined are the message. A message that fits is one piece.
pub fn split_sysex(msg: &[u8], limit: usize) -> Vec<Vec<u8>> {
    if limit == 0 || msg.len() <= limit {
        return vec![msg.to_vec()];
    }
    msg.chunks(limit).map(<[u8]>::to_vec).collect()
}

/// What the joiner made of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joined {
    /// A complete message: a SysEx from F0 to F7, or any other message as
    /// it came.
    Message(Vec<u8>),
    /// A real-time byte that arrived inside a SysEx (MIDI allows that).
    RealTime(u8),
    /// A SysEx cut off by a status byte before its F7, or longer than the
    /// joiner's maximum; the bytes seen so far.
    Broken(Vec<u8>),
}

/// Joins SysEx that a host stack delivers in pieces (CoreMIDI packets,
/// ALSA events of 256 bytes, a driver's 4 KiB limit). Bytes outside a
/// SysEx pass through unchanged, one push at a time.
#[derive(Debug)]
pub struct SysexJoiner {
    buf: Option<Vec<u8>>,
    max: usize,
}

impl SysexJoiner {
    /// `max` caps one SysEx, so a lost F7 cannot grow without bound.
    pub fn new(max: usize) -> Self {
        Self { buf: None, max }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Joined> {
        let mut out = Vec::new();
        let mut plain = Vec::new();
        for &b in bytes {
            if b >= 0xF8 {
                if self.buf.is_some() {
                    out.push(Joined::RealTime(b));
                } else {
                    plain.push(b);
                }
                continue;
            }
            match (&mut self.buf, b) {
                (None, 0xF0) => {
                    if !plain.is_empty() {
                        out.push(Joined::Message(std::mem::take(&mut plain)));
                    }
                    self.buf = Some(vec![0xF0]);
                }
                (None, _) => plain.push(b),
                (Some(buf), 0xF7) => {
                    buf.push(0xF7);
                    out.push(Joined::Message(self.buf.take().unwrap_or_default()));
                }
                (Some(_), 0x80..=0xF6) => {
                    // a status byte ends the SysEx without its F7
                    out.push(Joined::Broken(self.buf.take().unwrap_or_default()));
                    if b == 0xF0 {
                        self.buf = Some(vec![0xF0]);
                    } else {
                        plain.push(b);
                    }
                }
                (Some(buf), _) => {
                    buf.push(b);
                    if buf.len() > self.max {
                        out.push(Joined::Broken(self.buf.take().unwrap_or_default()));
                    }
                }
            }
        }
        if !plain.is_empty() {
            out.push(Joined::Message(plain));
        }
        out
    }

    /// A SysEx is open and waiting for more pieces.
    pub fn pending(&self) -> bool {
        self.buf.is_some()
    }
}

/// One write to the host: bytes, then a pause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    pub bytes: Vec<u8>,
    pub delay_ms: u32,
}

/// One message on the wire and the writes that carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub message: String,
    pub frame: Vec<u8>,
    pub writes: Vec<Write>,
}

impl Codec<'_> {
    /// The host limit for a message: its transport's `sysex-limit`; for
    /// `selected`, the smallest among the config transports.
    pub fn sysex_limit(&self, name: &str) -> Option<(usize, u32)> {
        let m = self.d.message(name)?;
        let pick = |t: &ctrl_cdl::Transport| {
            t.sysex_limit
                .map(|l| (l as usize, t.segment_delay.unwrap_or(0)))
        };
        match m.transport.as_deref()? {
            SELECTED_TRANSPORT => self
                .d
                .transports
                .iter()
                .filter(|t| t.role == Some(Role::Config))
                .filter_map(pick)
                .min(),
            t => self.d.transport(t).and_then(pick),
        }
    }

    /// Everything the engine writes to send message `name` (CDL §8.2): the
    /// message itself, or its `segment` message once per piece when it is
    /// longer than the threshold; each split into host writes of at most
    /// the transport's `sysex-limit`.
    pub fn plan(&self, name: &str, args: &Bindings) -> Result<Vec<Planned>, CodecError> {
        let m = self.message(name)?;
        let limit = self.sysex_limit(name);
        let full = self.encode(name, args)?;
        let threshold = m
            .segment
            .as_ref()
            .and_then(|s| s.threshold.map(|t| t as usize))
            .or(limit.map(|l| l.0));
        let frames = match (&m.segment, threshold) {
            (Some(seg), Some(threshold)) if full.len() > threshold => {
                let Some(Val::Bytes(data)) = args.get(&seg.over) else {
                    return Err(CodecError::UnknownName(seg.over.clone()));
                };
                let size = seg.size.max(1) as usize;
                let parts = data.len().div_ceil(size).max(1);
                let mut out = Vec::new();
                for (k, piece) in data.chunks(size).enumerate() {
                    let mut a = args.clone();
                    a.insert(seg.over.clone(), Val::Bytes(piece.to_vec()));
                    a.insert("part".into(), Val::Int(k as i64));
                    a.insert("parts".into(), Val::Int(parts as i64));
                    a.insert("offset".into(), Val::Int((k * size) as i64));
                    let frame = self.encode(&seg.message, &a)?;
                    if frame.len() > threshold {
                        return Err(CodecError::Unsupported(format!(
                            "segment `{}` part {k} is {} bytes, over the {threshold}-byte threshold: lower `size`",
                            seg.message,
                            frame.len()
                        )));
                    }
                    out.push((seg.message.clone(), frame));
                }
                out
            }
            _ => {
                if let Some(paced) = self.paced(m, args, &full, limit)? {
                    return Ok(vec![paced]);
                }
                vec![(name.to_string(), full)]
            }
        };
        Ok(frames
            .into_iter()
            .map(|(message, frame)| {
                let writes = match limit {
                    Some((l, delay)) => split_sysex(&frame, l)
                        .into_iter()
                        .map(|bytes| Write {
                            bytes,
                            delay_ms: delay,
                        })
                        .collect(),
                    None => vec![Write {
                        bytes: frame.clone(),
                        delay_ms: 0,
                    }],
                };
                Planned {
                    message,
                    frame,
                    writes,
                }
            })
            .collect())
    }
}

impl<'d> Codec<'d> {
    /// A message whose `records` carry `pace` (CDL §8.3): one host write per
    /// record (the first with the bytes before the records, the last with
    /// those after), the pause after each; a write over the transport's
    /// `sysex-limit` is split further.
    fn paced(
        &self,
        m: &'d ctrl_cdl::Message,
        args: &Bindings,
        full: &[u8],
        limit: Option<(usize, u32)>,
    ) -> Result<Option<Planned>, CodecError> {
        let Some((r, pace)) = m.records.iter().find_map(|r| r.pace.map(|p| (r, p))) else {
            return Ok(None);
        };
        let Some(Val::Bytes(array)) = args.get(&r.name) else {
            return Ok(None);
        };
        let records = self.decode_records(&m.name, &r.name, array)?;
        // where the array sits: encode without it and compare
        let mut empty_args = args.clone();
        empty_args.insert(r.name.clone(), Val::Bytes(Vec::new()));
        let empty = self.encode(&m.name, &empty_args)?;
        let len = array.len();
        let mut at = empty.iter().zip(full).take_while(|(a, b)| a == b).count();
        at = at.min(empty.len());
        while at > 0 && empty[at..] != full[at + len..] {
            at -= 1;
        }
        if full[at..at + len] != array[..] {
            return Err(CodecError::Unsupported(format!(
                "cannot place records `{}` in `{}`",
                r.name, m.name
            )));
        }
        // cut after each record
        let mut cuts = Vec::new();
        let mut pos = at;
        for rec in &records {
            let one = self.encode_records(&m.name, &r.name, std::slice::from_ref(&rec.values))?;
            pos += one.len();
            cuts.push(pos);
        }
        if let Some(last) = cuts.last_mut() {
            *last = full.len();
        }
        let mut writes = Vec::new();
        let mut start = 0;
        for cut in cuts {
            let piece = &full[start..cut];
            let parts = match limit {
                Some((l, _)) => split_sysex(piece, l),
                None => vec![piece.to_vec()],
            };
            let n = parts.len();
            for (i, bytes) in parts.into_iter().enumerate() {
                let delay_ms = if i + 1 == n {
                    pace
                } else {
                    limit.map_or(0, |l| l.1)
                };
                writes.push(Write { bytes, delay_ms });
            }
            start = cut;
        }
        Ok(Some(Planned {
            message: m.name.clone(),
            frame: full.to_vec(),
            writes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_the_bytes_and_order() {
        let msg: Vec<u8> = std::iter::once(0xF0)
            .chain((0..10_000).map(|i| (i % 128) as u8))
            .chain(std::iter::once(0xF7))
            .collect();
        let pieces = split_sysex(&msg, 4096);
        assert_eq!(pieces.len(), 3);
        assert!(pieces.iter().all(|p| p.len() <= 4096));
        assert_eq!(pieces.concat(), msg);
        assert_eq!(split_sysex(&[0xF0, 0xF7], 4096).len(), 1);
    }

    #[test]
    fn joiner_rebuilds_a_split_message() {
        let msg: Vec<u8> = std::iter::once(0xF0)
            .chain((0..9_000).map(|i| (i % 128) as u8))
            .chain(std::iter::once(0xF7))
            .collect();
        let mut j = SysexJoiner::new(65_536);
        let mut got = Vec::new();
        for (k, p) in split_sysex(&msg, 256).iter().enumerate() {
            if k == 3 {
                got.extend(j.push(&[0xFE])); // Active Sensing between packets
            }
            got.extend(j.push(p));
        }
        assert_eq!(got, vec![Joined::RealTime(0xFE), Joined::Message(msg)]);
        assert!(!j.pending());
    }

    #[test]
    fn joiner_reports_a_cut_message() {
        let mut j = SysexJoiner::new(16);
        assert_eq!(
            j.push(&[0xF0, 0x42, 0x01, 0xB0, 0x07, 0x7F]),
            vec![
                Joined::Broken(vec![0xF0, 0x42, 0x01]),
                Joined::Message(vec![0xB0, 0x07, 0x7F])
            ]
        );
        let long: Vec<u8> = std::iter::once(0xF0).chain([0u8; 20]).collect();
        assert!(matches!(j.push(&long)[0], Joined::Broken(_)));
    }
}
