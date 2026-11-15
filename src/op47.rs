//! OP-47 Subtitling Distribution Packet (SMPTE RDD-08), carried in HD-SDI VANC.
//!
//! The SMPTE 291 wrapper (DID 0x43, SDID 0x02, data count, 10-bit parity and checksum)
//! is added by the DeckLink SDK; this module builds the user data words.
//!
//! Layout as we understand RDD-08 (to verify against the spec and the encoding appliance):
//!   0x51 0x15            identifier
//!   len                  total SDP length in bytes, identifier through checksum
//!   0x02                 format code: WST teletext
//!   5 x descriptor       one per packet slot: bit 7 field (1 = first field), bits 4..0 line;
//!                        0x00 means the slot is unused
//!   n x 45 bytes         0x55 0x55 0x27 + 42-byte teletext packet, only for used slots
//!   0x74                 footer identifier
//!   seq_hi seq_lo        16-bit sequence counter
//!   checksum             makes the sum of all SDP bytes 0 mod 256

use crate::page::Packet;

pub const DID: u8 = 0x43;
pub const SDID: u8 = 0x02;
pub const MAX_PACKETS: usize = 5;

/// One frame's packets as `sdps` SDPs (at least one, even with no packets). Packets
/// `5k..5k+5` go in SDP k, with descriptor lines `lines[5k..5k+5]`. Each SDP takes the
/// next sequence number.
pub fn sdps(packets: &[Packet], lines: &[u8], seq: &mut u16) -> Vec<Vec<u8>> {
    let mut chunks: Vec<&[Packet]> = packets.chunks(MAX_PACKETS).collect();
    if chunks.is_empty() {
        chunks.push(&[]);
    }
    assert!(
        lines.len() >= chunks.len() * MAX_PACKETS,
        "not enough OP-47 lines"
    );
    chunks
        .iter()
        .zip(lines.as_chunks::<MAX_PACKETS>().0)
        .map(|(chunk, l)| {
            let v = sdp(chunk, l, *seq);
            *seq = seq.wrapping_add(1);
            v
        })
        .collect()
}

/// Build one SDP from up to 5 packets. `lines[i]` is the VBI line number put in slot i's descriptor.
pub fn sdp(packets: &[Packet], lines: &[u8; MAX_PACKETS], seq: u16) -> Vec<u8> {
    assert!(packets.len() <= MAX_PACKETS);
    let len = 2 + 1 + 1 + MAX_PACKETS + packets.len() * 45 + 4;
    let mut v = Vec::with_capacity(len);
    v.extend_from_slice(&[0x51, 0x15, len as u8, 0x02]);
    for (i, &line) in lines.iter().enumerate() {
        v.push(if i < packets.len() {
            0x80 | (line & 0x1F)
        } else {
            0
        });
    }
    for p in packets {
        v.extend_from_slice(&[0x55, 0x55, 0x27]);
        v.extend_from_slice(p);
    }
    v.push(0x74);
    v.extend_from_slice(&seq.to_be_bytes());
    let sum = v.iter().fold(0u8, |a, &b| a.wrapping_add(b));
    v.push(sum.wrapping_neg());
    debug_assert_eq!(v.len(), len);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdp_layout_and_checksum() {
        let p = [[0xAAu8; 42]; 2];
        let v = sdp(&p, &[7, 8, 9, 10, 11], 0x0102);
        assert_eq!(v.len(), 2 + 1 + 1 + 5 + 90 + 4);
        assert_eq!(v[2] as usize, v.len());
        assert_eq!(&v[4..9], &[0x87, 0x88, 0, 0, 0]);
        assert_eq!(&v[9..12], &[0x55, 0x55, 0x27]);
        assert_eq!(v[v.len() - 4], 0x74);
        assert_eq!(&v[v.len() - 3..v.len() - 1], &[1, 2]);
        assert_eq!(v.iter().fold(0u8, |a, &b| a.wrapping_add(b)), 0);
        // A full SDP fits in one SMPTE 291 packet (max 255 user data words).
        assert!(sdp(&[[0; 42]; 5], &[7, 8, 9, 10, 11], 0).len() <= 255);
    }

    #[test]
    fn frames_split_into_sdps() {
        let lines: Vec<u8> = (7..22).collect();
        let mut seq = 0xFFFF;
        let v = sdps(&[[1; 42]; 7], &lines, &mut seq);
        assert_eq!(v.len(), 2);
        assert_eq!(&v[0][4..9], &[0x87, 0x88, 0x89, 0x8A, 0x8B]);
        assert_eq!(&v[1][4..9], &[0x8C, 0x8D, 0, 0, 0]);
        assert_eq!(&v[1][v[1].len() - 3..v[1].len() - 1], &[0, 0]);
        assert_eq!(seq, 1);
        assert_eq!(sdps(&[], &lines, &mut seq).len(), 1);
    }
}
