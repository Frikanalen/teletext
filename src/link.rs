//! The link between ttxd and ttx-inject.
//!
//! ttx-inject connects to ttxd over TCP and owns the frame clock: it sends one credit
//! byte per frame it wants, and ttxd answers each credit with one frame. The credit byte
//! is the number of OP-47 packets the frame will go out in (1..=[`MAX_SDPS`]), so the
//! frame holds up to 5 WST packets per credit. A frame is
//!
//!   flags                 bit 0: the carousel started a new cycle in this frame;
//!                         bits 4..6: number of packets n (0..=7), or
//!                         bit 7 set: n is in the next byte (up to [`MAX_PACKETS`])
//!   n x (kind + 42 bytes) kind is a [`Kind`]; the 42 bytes are a WST packet
//!
//! Both sides open with [`HELLO`]. ttxd serves one injector at a time; a new connection
//! replaces the old one.

use crate::page::Packet;
use std::io::{self, Read};

pub const HELLO: &[u8; 4] = b"TTX1";
/// A credit for a frame in one OP-47 packet (5 lines). Credit `n` asks for n packets.
pub const CREDIT: u8 = 0x01;
/// OP-47 packets per frame. 3 x 5 lines every 20 ms is 750 lines/s, inside the
/// 338.4 kbit/s the encoding appliance reserves for teletext (35 lines per 40 ms).
pub const MAX_SDPS: usize = 3;
pub const PACKETS_PER_SDP: usize = 5;
pub const MAX_PACKETS: usize = MAX_SDPS * PACKETS_PER_SDP;

/// Packets a frame may hold for this credit byte, or None if it isn't a credit.
pub fn credit_packets(b: u8) -> Option<usize> {
    (1..=MAX_SDPS as u8)
        .contains(&b)
        .then_some(b as usize * PACKETS_PER_SDP)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Filler = 0,
    Carousel = 1,
    Flash = 2,
    Subtitle = 3,
    ServiceData = 4,
}

impl Kind {
    fn from_u8(b: u8) -> Option<Kind> {
        Some(match b {
            0 => Kind::Filler,
            1 => Kind::Carousel,
            2 => Kind::Flash,
            3 => Kind::Subtitle,
            4 => Kind::ServiceData,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub cycle_start: bool,
    pub packets: Vec<(Kind, Packet)>,
}

impl Frame {
    pub fn packets(&self) -> Vec<Packet> {
        self.packets.iter().map(|(_, p)| *p).collect()
    }

    pub fn encode(&self) -> Vec<u8> {
        assert!(self.packets.len() <= MAX_PACKETS);
        let n = self.packets.len();
        let mut v = Vec::with_capacity(2 + n * 43);
        if n <= 7 {
            v.push(self.cycle_start as u8 | (n as u8) << 4);
        } else {
            v.extend([self.cycle_start as u8 | 0x80, n as u8]);
        }
        for (k, p) in &self.packets {
            v.push(*k as u8);
            v.extend_from_slice(p);
        }
        v
    }

    pub fn read_from(r: &mut impl Read) -> io::Result<Frame> {
        let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
        let mut flags = [0u8];
        r.read_exact(&mut flags)?;
        if flags[0] & 0x0E != 0 {
            return Err(bad("bad frame flags"));
        }
        let n = if flags[0] & 0x80 != 0 {
            if flags[0] & 0x70 != 0 {
                return Err(bad("bad frame flags"));
            }
            let mut b = [0u8];
            r.read_exact(&mut b)?;
            b[0] as usize
        } else {
            (flags[0] >> 4 & 7) as usize
        };
        if n > MAX_PACKETS {
            return Err(bad("too many packets in a frame"));
        }
        let mut packets = Vec::with_capacity(n);
        for _ in 0..n {
            let mut b = [0u8; 43];
            r.read_exact(&mut b)?;
            let kind = Kind::from_u8(b[0]).ok_or_else(|| bad("bad packet kind"))?;
            let mut p = [0u8; 42];
            p.copy_from_slice(&b[1..]);
            packets.push((kind, p));
        }
        Ok(Frame {
            cycle_start: flags[0] & 1 != 0,
            packets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip() {
        let f = Frame {
            cycle_start: true,
            packets: vec![(Kind::Carousel, [0x11; 42]), (Kind::Subtitle, [0x22; 42])],
        };
        let b = f.encode();
        assert_eq!(b.len(), 1 + 2 * 43);
        assert_eq!(Frame::read_from(&mut &b[..]).unwrap(), f);
        let big = Frame {
            cycle_start: false,
            packets: vec![(Kind::Carousel, [0x33; 42]); MAX_PACKETS],
        };
        let b = big.encode();
        assert_eq!(b.len(), 2 + MAX_PACKETS * 43);
        assert_eq!(Frame::read_from(&mut &b[..]).unwrap(), big);
        assert_eq!(credit_packets(CREDIT), Some(5));
        assert_eq!(credit_packets(3), Some(15));
        assert_eq!(credit_packets(0), None);
        assert_eq!(credit_packets(4), None);
        let empty = Frame::default();
        assert_eq!(Frame::read_from(&mut &empty.encode()[..]).unwrap(), empty);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Frame::read_from(&mut &[0x70u8][..]).is_err());
        let mut b = Frame {
            cycle_start: false,
            packets: vec![(Kind::Flash, [0; 42])],
        }
        .encode();
        b[1] = 9;
        assert!(Frame::read_from(&mut &b[..]).is_err());
    }
}
