//! Teletext error-protection codes (ETS 300 706 §8).

/// Hamming 8/4 code words for nibbles 0..15.
const HAM84: [u8; 16] = [
    0x15, 0x02, 0x49, 0x5E, 0x64, 0x73, 0x38, 0x2F, 0xD0, 0xC7, 0x8C, 0x9B, 0xA1, 0xB6, 0xFD, 0xEA,
];

/// Hamming 8/4 encode the low nibble of `n`.
pub fn ham84(n: u8) -> u8 {
    HAM84[(n & 0x0F) as usize]
}

/// Decode a Hamming 8/4 byte, without error correction (exact match only).
pub fn unham84(b: u8) -> Option<u8> {
    HAM84.iter().position(|&c| c == b).map(|p| p as u8)
}

/// Add odd parity in bit 7 to a 7-bit character.
pub fn parity(c: u8) -> u8 {
    let c = c & 0x7F;
    if c.count_ones().is_multiple_of(2) {
        c | 0x80
    } else {
        c
    }
}

/// Bit positions (1-based) of the 18 data bits in a Hamming 24/18 word.
const DATA_POS: [usize; 18] = [
    3, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23,
];

/// Hamming 24/18 encode 18 data bits (D1 = bit 0), giving 3 bytes, first transmitted first.
pub fn ham2418(d: u32) -> [u8; 3] {
    let mut bits = [0u8; 25];
    for (i, &p) in DATA_POS.iter().enumerate() {
        bits[p] = ((d >> i) & 1) as u8;
    }
    for k in 0..5 {
        let pp = 1usize << k;
        let mut s = 0;
        for (pos, b) in bits.iter().enumerate().take(24).skip(1) {
            if pos & pp != 0 && pos != pp {
                s ^= b;
            }
        }
        bits[pp] = s ^ 1; // odd parity
    }
    let s = bits[1..24].iter().fold(0, |a, b| a ^ b);
    bits[24] = s ^ 1;
    let mut out = [0u8; 3];
    for pos in 1..=24 {
        out[(pos - 1) / 8] |= bits[pos] << ((pos - 1) % 8);
    }
    out
}

/// Decode a Hamming 24/18 word, without correction. Returns None if any parity check fails.
pub fn unham2418(b: [u8; 3]) -> Option<u32> {
    let bit = |pos: usize| (b[(pos - 1) / 8] >> ((pos - 1) % 8)) & 1;
    for k in 0..5 {
        let pp = 1usize << k;
        let s = (1..24).filter(|p| p & pp != 0).fold(0, |a, p| a ^ bit(p));
        if s != 1 {
            return None;
        }
    }
    if (1..=24).fold(0, |a, p| a ^ bit(p)) != 1 {
        return None;
    }
    Some(
        DATA_POS
            .iter()
            .enumerate()
            .fold(0u32, |a, (i, &p)| a | (bit(p) as u32) << i),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ham84_roundtrip() {
        for n in 0..16 {
            assert_eq!(unham84(ham84(n)), Some(n));
        }
    }

    #[test]
    fn ham84_words_differ_in_at_least_4_bits() {
        for a in 0..16u8 {
            for b in (a + 1)..16 {
                assert!((ham84(a) ^ ham84(b)).count_ones() >= 4);
            }
        }
    }

    #[test]
    fn parity_is_odd() {
        for c in 0..128u8 {
            assert_eq!(parity(c).count_ones() % 2, 1);
            assert_eq!(parity(c) & 0x7F, c);
        }
    }

    #[test]
    fn ham2418_roundtrip_and_single_bit_errors_detected() {
        for d in [0u32, 1, 0x3FFFF, 0x2A5A5, 0x12345, 63 | (0x1F << 6)] {
            let w = ham2418(d);
            assert_eq!(unham2418(w), Some(d));
            for bit in 0..24 {
                let mut e = w;
                e[bit / 8] ^= 1 << (bit % 8);
                assert_eq!(unham2418(e), None);
            }
        }
    }
}
