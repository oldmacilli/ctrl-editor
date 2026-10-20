//! `packed7` (CDL §8.1): 8-bit data in 7-bit SysEx bytes.
//!
//! Each group of up to seven data bytes is preceded by one byte holding
//! their top bits: bit `i` is the top bit of the group's byte `i`. A partial
//! last group still gets its leading byte. KORG uses this layout
//! (korg-sysex `pack_7in8`).

/// Packs `data`; the result is `len + ceil(len / 7)` bytes.
pub fn pack7(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len().div_ceil(7));
    for group in data.chunks(7) {
        let high = group
            .iter()
            .enumerate()
            .fold(0u8, |h, (i, b)| h | ((b >> 7) << i));
        out.push(high);
        out.extend(group.iter().map(|b| b & 0x7F));
    }
    out
}

/// Unpacks a packed region; its length decides the data length
/// (`len - ceil(len / 8)`).
pub fn unpack7(packed: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packed.len());
    for group in packed.chunks(8) {
        let (high, low) = (group[0], &group[1..]);
        out.extend(
            low.iter()
                .enumerate()
                .map(|(i, b)| b | (((high >> i) & 1) << 7)),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_korg_sysex() {
        // korg-sysex packing_layout_is_msb_first
        assert_eq!(pack7(&[0x80, 0x01]), [0x01, 0x00, 0x01]);
        assert_eq!(pack7(&[0xFF; 7]), [0x7F; 8]);
    }

    #[test]
    fn round_trips() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 0x80) as u8).collect();
            let packed = pack7(&data);
            assert_eq!(packed.len(), len + len.div_ceil(7));
            assert!(packed.iter().all(|b| *b < 0x80));
            assert_eq!(unpack7(&packed), data, "length {len}");
        }
    }
}
