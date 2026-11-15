//! The link between ttxd and ttx-inject.
//!
//! ttx-inject connects to ttxd over TCP and owns the frame clock: it sends one credit
//! byte per frame it wants, and ttxd answers each credit with one frame. A frame is
//!
//!   flags                 bit 0: the carousel started a new cycle in this frame;
//!                         bits 4..6: number of packets n (0..=5)
//!   n x (kind + 42 bytes) kind is a [`Kind`]; the 42 bytes are a WST packet
//!
//! Both sides open with [`HELLO`]. ttxd serves one injector at a time; a new connection
//! replaces the old one.

use crate::page::Packet;
use std::io::{self, Read};

pub const HELLO: &[u8; 4] = b"TTX1";
pub const CREDIT: u8 = 0x01;
pub const MAX_PACKETS: usize = 5;

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
        let mut v = Vec::with_capacity(1 + self.packets.len() * 43);
        v.push(self.cycle_start as u8 | (self.packets.len() as u8) << 4);
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
        let n = (flags[0] >> 4) as usize;
        if n > MAX_PACKETS || flags[0] & 0x8E != 0 {
            return Err(bad("bad frame flags"));
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
