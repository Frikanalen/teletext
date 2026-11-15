//! Packet 8/30 format 1, the broadcast service data packet (ETS 300 706 §9.8.1).
//!
//! Sent about once a second. It tells decoders the initial page, the network
//! identification code, the date and UTC time with the local offset (some decoders set
//! their clock from it), and a 20-character status display that many decoders show as
//! the programme or channel name.

use crate::charset::encode_char;
use crate::ham::{ham84, parity};
use crate::page::{Packet, PageNo};
use chrono::{DateTime, NaiveDate, Timelike, Utc};

pub struct ServiceData {
    pub initial_page: PageNo,
    /// Network identification code. 0 when the channel has none registered.
    pub network_id: u16,
    /// Up to 20 characters.
    pub status: String,
}

/// BCD digits of `n`, most significant first, each plus one, packed into nibbles.
fn digits_plus_one(n: u32, count: u32) -> u32 {
    (0..count).fold(0, |acc, i| {
        let d = (n / 10u32.pow(count - 1 - i)) % 10;
        acc << 4 | (d + 1)
    })
}

/// Build the packet. `offset_secs` is local time minus UTC.
pub fn packet(sd: &ServiceData, now: DateTime<Utc>, offset_secs: i32) -> Packet {
    let mut p = [0u8; 42];
    // Magazine 8 (sent as 0), row 30: MRAG.
    let row: u8 = 30;
    p[0] = ham84((row & 1) << 3);
    p[1] = ham84(row >> 1);
    p[2] = ham84(0); // designation code 0: format 1
    // Initial page, with magazine bits relative to magazine 8 (that is, as is) and
    // subcode 3F7F meaning "any subpage".
    let mag = sd.initial_page.mag & 7;
    let sub: u16 = 0x3F7F;
    p[3] = ham84(sd.initial_page.num & 0x0F);
    p[4] = ham84(sd.initial_page.num >> 4);
    p[5] = ham84(sub as u8 & 0x0F);
    p[6] = ham84((sub >> 4) as u8 & 0x07 | (mag & 1) << 3);
    p[7] = ham84((sub >> 8) as u8 & 0x0F);
    p[8] = ham84((sub >> 12) as u8 & 0x03 | (mag >> 1) << 2);
    // Network identification code, sent bit-reversed per byte.
    p[9] = (sd.network_id as u8).reverse_bits();
    p[10] = ((sd.network_id >> 8) as u8).reverse_bits();
    // Local time offset in half hours: bit 6 = negative, bits 0 and 7 always set.
    let half_hours = (offset_secs.unsigned_abs() / 1800).min(31) as u8;
    p[11] = 0x81 | half_hours << 1 | ((offset_secs < 0) as u8) << 6;
    // Modified Julian Date, 5 digits each plus one.
    let epoch = NaiveDate::from_ymd_opt(1858, 11, 17).unwrap();
    let mjd = (now.date_naive() - epoch).num_days() as u32;
    let m = digits_plus_one(mjd, 5);
    p[12] = (m >> 16) as u8;
    p[13] = (m >> 8) as u8;
    p[14] = m as u8;
    // UTC hours, minutes, seconds, each digit plus one.
    p[15] = digits_plus_one(now.hour(), 2) as u8;
    p[16] = digits_plus_one(now.minute(), 2) as u8;
    p[17] = digits_plus_one(now.second(), 2) as u8;
    // Bytes 18..21 are reserved in format 1.
    for b in &mut p[18..22] {
        *b = ham84(0);
    }
    // Status display.
    let mut status = [b' '; 20];
    for (d, c) in status.iter_mut().zip(sd.status.chars()) {
        *d = encode_char(c).0;
    }
    for (d, &s) in p[22..].iter_mut().zip(&status) {
        *d = parity(s);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::unham84;
    use chrono::TimeZone;

    #[test]
    fn fields_decode() {
        let sd = ServiceData {
            initial_page: "100".parse().unwrap(),
            network_id: 0x1234,
            status: "Frikanalen".into(),
        };
        let now = Utc.with_ymd_and_hms(2026, 11, 15, 16, 45, 9).unwrap();
        let p = packet(&sd, now, 3600);
        // Magazine 8 row 30.
        assert_eq!(unham84(p[0]), Some(0));
        assert_eq!(unham84(p[1]), Some(15));
        assert_eq!(unham84(p[2]), Some(0));
        // Initial page 100, magazine 1.
        assert_eq!(unham84(p[3]), Some(0));
        assert_eq!(unham84(p[4]), Some(0));
        assert_eq!(unham84(p[6]).unwrap() >> 3, 1);
        assert_eq!(unham84(p[8]).unwrap() >> 2, 0);
        // NI bit-reversed per byte.
        assert_eq!(
            (p[9].reverse_bits() as u16) | (p[10].reverse_bits() as u16) << 8,
            0x1234
        );
        // +1 hour = 2 half hours, positive.
        assert_eq!(p[11], 0x81 | 2 << 1);
        // MJD of 2026-11-15 is 61359: digits plus one = 7 2 4 6 10.
        assert_eq!(&p[12..15], &[0x07, 0x24, 0x6A]);
        // 16:45:09 UTC.
        assert_eq!(&p[15..18], &[0x27, 0x56, 0x1A]);
        assert_eq!(p[22] & 0x7F, b'F');
        assert_eq!(p[41] & 0x7F, b' ');
    }
}
