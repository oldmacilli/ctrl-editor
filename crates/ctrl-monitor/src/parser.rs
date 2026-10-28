//! MIDI 1.0 byte streams to messages (docs/MONITOR.md §4.1).
//!
//! One parser per port and direction. It keeps its state across packets, so
//! a message may arrive in pieces; it resolves running status; real-time
//! bytes are messages wherever they appear, inside SysEx too; a SysEx ends
//! with F7, with the next status byte (`Cut`), after a timeout (`Timeout`),
//! or is cut short at a size limit (`Truncated`). Bytes that belong to no
//! message come out as one `Invalid` run.

use crate::model::{Stamp, SysExEnd};

/// What the parser produces.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Voice {
        at: Stamp,
        status: u8,
        d1: u8,
        d2: u8,
    },
    Common {
        at: Stamp,
        status: u8,
        d1: u8,
        d2: u8,
    },
    Realtime {
        at: Stamp,
        status: u8,
    },
    /// From F0, with F7 when complete; stamped with the F0's arrival.
    SysEx {
        at: Stamp,
        bytes: Vec<u8>,
        end: SysExEnd,
    },
    Invalid {
        at: Stamp,
        bytes: Vec<u8>,
    },
    /// A SysEx in progress crossed another [`Parser::PROGRESS_STEP`] bytes.
    Progress {
        at: Stamp,
        len: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParserConfig {
    /// A SysEx without a byte for this long ends as `Timeout`.
    pub sysex_timeout_ns: u64,
    /// Bytes kept of one SysEx; the rest is counted, not kept.
    pub max_sysex: usize,
}

impl Default for ParserConfig {
    fn default() -> Self {
        ParserConfig {
            sysex_timeout_ns: 1_000_000_000,
            max_sysex: 16 << 20,
        }
    }
}

/// Data bytes that follow a status byte, or `None` for an undefined one.
fn data_len(status: u8) -> Option<usize> {
    match status {
        0x80..=0xBF | 0xE0..=0xEF => Some(2),
        0xC0..=0xDF => Some(1),
        0xF1 | 0xF3 => Some(1),
        0xF2 => Some(2),
        0xF6 => Some(0),
        _ => None,
    }
}

#[derive(Debug, Default)]
pub struct Parser {
    config: ParserConfig,
    /// Running status, 80..EF; cleared by system common and SysEx.
    running: Option<u8>,
    /// The message being assembled: status, data so far, its stamp.
    status: Option<u8>,
    data: [u8; 2],
    have: usize,
    msg_at: Stamp,
    sysex: Option<SysExState>,
    invalid: Vec<u8>,
    invalid_at: Stamp,
}

#[derive(Debug)]
struct SysExState {
    at: Stamp,
    bytes: Vec<u8>,
    len: usize,
    last_ns: u64,
}

impl Parser {
    pub const PROGRESS_STEP: usize = 256;

    pub fn new(config: ParserConfig) -> Self {
        Parser {
            config,
            ..Parser::default()
        }
    }

    /// Parses one packet that arrived at `at`.
    pub fn push(&mut self, at: Stamp, bytes: &[u8], out: &mut Vec<Parsed>) {
        for &b in bytes {
            self.byte(at, b, out);
        }
        // A run of stray bytes is reported with its packet, not held back.
        self.flush_invalid(out);
    }

    /// Ends a SysEx that has had no byte for the timeout. Call it now and
    /// then with the current time.
    pub fn poll(&mut self, now_ns: u64, out: &mut Vec<Parsed>) {
        if let Some(s) = &self.sysex {
            if now_ns.saturating_sub(s.last_ns) >= self.config.sysex_timeout_ns {
                self.end_sysex(SysExEnd::Timeout, out);
            }
        }
    }

    /// True while a SysEx is open.
    pub fn in_sysex(&self) -> bool {
        self.sysex.is_some()
    }

    fn byte(&mut self, at: Stamp, b: u8, out: &mut Vec<Parsed>) {
        if b >= 0xF8 {
            if matches!(b, 0xF9 | 0xFD) {
                self.flush_invalid(out);
                out.push(Parsed::Invalid { at, bytes: vec![b] });
            } else {
                self.flush_invalid(out);
                out.push(Parsed::Realtime { at, status: b });
            }
            return;
        }
        if let Some(s) = &mut self.sysex {
            if b < 0x80 {
                s.len += 1;
                s.last_ns = at.host_ns;
                if s.bytes.len() < self.config.max_sysex {
                    s.bytes.push(b);
                }
                if s.len % Parser::PROGRESS_STEP == 0 {
                    out.push(Parsed::Progress { at, len: s.len });
                }
                return;
            }
            if b == 0xF7 {
                if s.bytes.len() < self.config.max_sysex {
                    s.bytes.push(0xF7);
                }
                self.end_sysex(SysExEnd::Complete, out);
                return;
            }
            self.end_sysex(SysExEnd::Cut(b), out);
            // and b starts the next message below
        }
        if b >= 0x80 {
            self.flush_invalid(out);
            self.status_byte(at, b, out);
        } else {
            self.data_byte(at, b, out);
        }
    }

    fn status_byte(&mut self, at: Stamp, b: u8, out: &mut Vec<Parsed>) {
        if let Some(status) = self.status.take() {
            // A message that lost its data to this status byte.
            let mut bytes = vec![status];
            bytes.extend_from_slice(&self.data[..self.have]);
            out.push(Parsed::Invalid {
                at: self.msg_at,
                bytes,
            });
        }
        self.have = 0;
        match b {
            0xF0 => {
                self.running = None;
                self.sysex = Some(SysExState {
                    at,
                    bytes: vec![0xF0],
                    len: 1,
                    last_ns: at.host_ns,
                });
            }
            0x80..=0xEF => {
                self.running = Some(b);
                self.status = Some(b);
                self.msg_at = at;
            }
            _ => match data_len(b) {
                Some(0) => {
                    self.running = None;
                    out.push(Parsed::Common {
                        at,
                        status: b,
                        d1: 0,
                        d2: 0,
                    });
                }
                Some(_) => {
                    self.running = None;
                    self.status = Some(b);
                    self.msg_at = at;
                }
                // F4, F5, and F7 outside a SysEx.
                None => {
                    self.running = None;
                    out.push(Parsed::Invalid { at, bytes: vec![b] });
                }
            },
        }
    }

    fn data_byte(&mut self, at: Stamp, b: u8, out: &mut Vec<Parsed>) {
        let status = match self.status {
            Some(s) => s,
            None => match self.running {
                Some(s) => {
                    self.status = Some(s);
                    self.msg_at = at;
                    s
                }
                None => {
                    if self.invalid.is_empty() {
                        self.invalid_at = at;
                    }
                    self.invalid.push(b);
                    return;
                }
            },
        };
        self.data[self.have] = b;
        self.have += 1;
        let need = data_len(status).unwrap_or(0);
        if self.have < need {
            return;
        }
        let (d1, d2) = (self.data[0], if need == 2 { self.data[1] } else { 0 });
        let at = self.msg_at;
        self.status = None;
        self.have = 0;
        if status < 0xF0 {
            out.push(Parsed::Voice { at, status, d1, d2 });
        } else {
            out.push(Parsed::Common { at, status, d1, d2 });
        }
    }

    fn end_sysex(&mut self, end: SysExEnd, out: &mut Vec<Parsed>) {
        if let Some(s) = self.sysex.take() {
            let end = if s.len > self.config.max_sysex && end == SysExEnd::Complete {
                SysExEnd::Truncated
            } else {
                end
            };
            out.push(Parsed::SysEx {
                at: s.at,
                bytes: s.bytes,
                end,
            });
        }
    }

    fn flush_invalid(&mut self, out: &mut Vec<Parsed>) {
        if !self.invalid.is_empty() {
            out.push(Parsed::Invalid {
                at: self.invalid_at,
                bytes: std::mem::take(&mut self.invalid),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ns: u64) -> Stamp {
        Stamp {
            host_ns: ns,
            ..Stamp::default()
        }
    }

    fn parse(packets: &[&[u8]]) -> Vec<Parsed> {
        let mut p = Parser::new(ParserConfig::default());
        let mut out = Vec::new();
        for (i, packet) in packets.iter().enumerate() {
            p.push(at(i as u64), packet, &mut out);
        }
        out
    }

    fn voice(ns: u64, status: u8, d1: u8, d2: u8) -> Parsed {
        Parsed::Voice {
            at: at(ns),
            status,
            d1,
            d2,
        }
    }

    #[test]
    fn running_status_repeats_the_last_voice_status() {
        assert_eq!(
            parse(&[&[0x90, 60, 100, 62, 90], &[64, 0]]),
            [
                voice(0, 0x90, 60, 100),
                voice(0, 0x90, 62, 90),
                voice(1, 0x90, 64, 0)
            ]
        );
        // Two-byte messages too.
        assert_eq!(
            parse(&[&[0xC1, 5, 6]]),
            [voice(0, 0xC1, 5, 0), voice(0, 0xC1, 6, 0)]
        );
    }

    #[test]
    fn a_message_split_over_packets_keeps_its_first_stamp() {
        assert_eq!(parse(&[&[0xB0], &[7], &[100]]), [voice(0, 0xB0, 7, 100)]);
    }

    #[test]
    fn real_time_bytes_are_messages_anywhere() {
        let out = parse(&[&[0x90, 60, 0xF8, 100, 0xF0, 0x41, 0xFE, 0x10, 0xF7]]);
        assert_eq!(
            out,
            [
                Parsed::Realtime {
                    at: at(0),
                    status: 0xF8
                },
                voice(0, 0x90, 60, 100),
                Parsed::Realtime {
                    at: at(0),
                    status: 0xFE
                },
                Parsed::SysEx {
                    at: at(0),
                    bytes: vec![0xF0, 0x41, 0x10, 0xF7],
                    end: SysExEnd::Complete
                },
            ]
        );
    }

    #[test]
    fn sysex_spans_packets_and_reports_progress() {
        let body = vec![0x11; 600];
        let out = parse(&[&[0xF0], &body[..300], &body[300..], &[0xF7]]);
        let progress: Vec<usize> = out
            .iter()
            .filter_map(|p| match p {
                Parsed::Progress { len, .. } => Some(*len),
                _ => None,
            })
            .collect();
        assert_eq!(progress, [256, 512]);
        match out.last().unwrap() {
            Parsed::SysEx { at: a, bytes, end } => {
                assert_eq!(*a, at(0));
                assert_eq!(bytes.len(), 602);
                assert_eq!(*end, SysExEnd::Complete);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_status_byte_cuts_an_open_sysex_and_starts_the_next_message() {
        assert_eq!(
            parse(&[&[0xF0, 0x7E, 0x01, 0x90, 60, 1]]),
            [
                Parsed::SysEx {
                    at: at(0),
                    bytes: vec![0xF0, 0x7E, 0x01],
                    end: SysExEnd::Cut(0x90)
                },
                voice(0, 0x90, 60, 1),
            ]
        );
    }

    #[test]
    fn sysex_clears_running_status() {
        let out = parse(&[&[0x90, 60, 1, 0xF0, 0xF7, 61, 1]]);
        assert_eq!(
            out[1],
            Parsed::SysEx {
                at: at(0),
                bytes: vec![0xF0, 0xF7],
                end: SysExEnd::Complete
            }
        );
        assert_eq!(
            out[2],
            Parsed::Invalid {
                at: at(0),
                bytes: vec![61, 1]
            }
        );
    }

    #[test]
    fn an_unterminated_sysex_times_out() {
        let mut p = Parser::new(ParserConfig {
            sysex_timeout_ns: 100,
            ..ParserConfig::default()
        });
        let mut out = Vec::new();
        p.push(at(10), &[0xF0, 0x43, 0x10], &mut out);
        p.poll(50, &mut out);
        assert!(out.is_empty() && p.in_sysex());
        p.poll(110, &mut out);
        assert_eq!(
            out,
            [Parsed::SysEx {
                at: at(10),
                bytes: vec![0xF0, 0x43, 0x10],
                end: SysExEnd::Timeout
            }]
        );
    }

    #[test]
    fn a_long_sysex_is_truncated_but_counted() {
        let mut p = Parser::new(ParserConfig {
            max_sysex: 4,
            ..ParserConfig::default()
        });
        let mut out = Vec::new();
        p.push(at(0), &[0xF0, 1, 2, 3, 4, 5, 0xF7], &mut out);
        assert_eq!(
            out,
            [Parsed::SysEx {
                at: at(0),
                bytes: vec![0xF0, 1, 2, 3],
                end: SysExEnd::Truncated
            }]
        );
    }

    #[test]
    fn system_common_and_undefined_bytes() {
        assert_eq!(
            parse(&[&[0xF2, 0x10, 0x02, 0xF6, 0xF1, 0x35, 0xF4, 0xF7, 0xF9]]),
            [
                Parsed::Common {
                    at: at(0),
                    status: 0xF2,
                    d1: 0x10,
                    d2: 0x02
                },
                Parsed::Common {
                    at: at(0),
                    status: 0xF6,
                    d1: 0,
                    d2: 0
                },
                Parsed::Common {
                    at: at(0),
                    status: 0xF1,
                    d1: 0x35,
                    d2: 0
                },
                Parsed::Invalid {
                    at: at(0),
                    bytes: vec![0xF4]
                },
                Parsed::Invalid {
                    at: at(0),
                    bytes: vec![0xF7]
                },
                Parsed::Invalid {
                    at: at(0),
                    bytes: vec![0xF9]
                },
            ]
        );
    }

    #[test]
    fn data_without_status_is_one_invalid_run_and_an_interrupted_message_is_invalid() {
        assert_eq!(
            parse(&[&[1, 2, 3, 0xB0, 7, 0xC0, 3]]),
            [
                Parsed::Invalid {
                    at: at(0),
                    bytes: vec![1, 2, 3]
                },
                Parsed::Invalid {
                    at: at(0),
                    bytes: vec![0xB0, 7]
                },
                voice(0, 0xC0, 3, 0),
            ]
        );
    }
}
