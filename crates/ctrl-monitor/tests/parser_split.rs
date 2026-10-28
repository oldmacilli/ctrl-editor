//! A valid stream parses the same whole or split into packets anywhere,
//! with running status and real-time bytes anywhere (docs/MONITOR.md §4.1).

use ctrl_monitor::{Parsed, Parser, ParserConfig, Stamp};

/// xorshift: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn data(&mut self) -> u8 {
        self.below(128) as u8
    }
}

const REALTIME: [u8; 6] = [0xF8, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF];

fn stream(rng: &mut Rng) -> Vec<u8> {
    let mut out = Vec::new();
    let mut running: Option<u8> = None;
    for _ in 0..200 {
        let mut msg = Vec::new();
        match rng.below(6) {
            0..=2 => {
                let status = [0x80u8, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0][rng.below(7) as usize]
                    | rng.below(16) as u8;
                if running != Some(status) || rng.below(2) == 0 {
                    msg.push(status);
                }
                running = Some(status);
                msg.push(rng.data());
                if !matches!(status & 0xF0, 0xC0 | 0xD0) {
                    msg.push(rng.data());
                }
            }
            3 => {
                let (status, n) =
                    [(0xF1u8, 1), (0xF2, 2), (0xF3, 1), (0xF6, 0)][rng.below(4) as usize];
                msg.push(status);
                for _ in 0..n {
                    msg.push(rng.data());
                }
                running = None;
            }
            4 => {
                msg.push(0xF0);
                for _ in 0..rng.below(700) {
                    msg.push(rng.data());
                }
                msg.push(0xF7);
                running = None;
            }
            _ => msg.push(REALTIME[rng.below(6) as usize]),
        }
        // Real-time bytes may sit between any two bytes of a message.
        for b in msg {
            if rng.below(20) == 0 {
                out.push(REALTIME[rng.below(6) as usize]);
            }
            out.push(b);
        }
    }
    out
}

/// What was parsed, without stamps and progress reports.
fn parse(packets: &[&[u8]]) -> Vec<Parsed> {
    let mut parser = Parser::new(ParserConfig::default());
    let mut out = Vec::new();
    for p in packets {
        parser.push(Stamp::default(), p, &mut out);
    }
    assert!(!parser.in_sysex());
    out.retain(|p| !matches!(p, Parsed::Progress { .. }));
    out
}

#[test]
fn splitting_a_stream_does_not_change_what_is_parsed() {
    for seed in 1..=50u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let bytes = stream(&mut rng);
        let whole = parse(&[&bytes]);
        assert!(
            whole.iter().all(|p| !matches!(p, Parsed::Invalid { .. })),
            "seed {seed}: {whole:?}"
        );
        let mut packets = Vec::new();
        let mut rest = &bytes[..];
        while !rest.is_empty() {
            let n = (1 + rng.below(40) as usize).min(rest.len());
            packets.push(&rest[..n]);
            rest = &rest[n..];
        }
        assert_eq!(parse(&packets), whole, "seed {seed}");
    }
}
