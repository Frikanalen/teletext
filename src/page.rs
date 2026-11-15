//! Pages, the row markup publishers send, and page encoding into packets.

use crate::charset::{NATIONAL_C12_C14, decode_g0, decode_g2, encode_char};
use crate::ham::{ham84, ham2418, parity};
use serde::{Deserialize, Serialize};
use std::fmt;

/// One teletext line: 42 bytes after clock run-in and framing code (the T42 format).
pub type Packet = [u8; 42];

pub const COLS: usize = 40;
/// Display rows 1..=24 (row 0 is the header).
pub const ROWS: usize = 24;

/// A page number such as 100 or 8FF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PageNo {
    /// Magazine 1..=8.
    pub mag: u8,
    /// Page number within the magazine, 0x00..=0xFF (normally BCD 00..99).
    pub num: u8,
}

impl PageNo {
    pub fn new(mag: u8, num: u8) -> Self {
        PageNo { mag, num }
    }
}

impl std::str::FromStr for PageNo {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let b = s.as_bytes();
        if b.len() != 3 || !(b'1'..=b'8').contains(&b[0]) {
            return Err(format!("bad page number {s:?}: want 100..8FF"));
        }
        let num = u8::from_str_radix(&s[1..], 16).map_err(|_| format!("bad page number {s:?}"))?;
        Ok(PageNo {
            mag: b[0] - b'0',
            num,
        })
    }
}

impl TryFrom<String> for PageNo {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}

impl From<PageNo> for String {
    fn from(p: PageNo) -> String {
        p.to_string()
    }
}

impl fmt::Display for PageNo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{:02X}", self.mag, self.num)
    }
}

/// The content of one (sub)page: 24 rows of 40 codes plus G2 substitutions, and any
/// Level 2.5 enhancement data the publisher supplied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    /// rows[0] is display row 1.
    pub rows: Vec<Vec<u8>>,
    /// (row 1..=24, column, G2 code).
    #[serde(default)]
    pub g2: Vec<(u8, u8, u8)>,
    /// Publisher's X/26 enhancement triplets (address, mode, data), without a
    /// termination marker. The G2 substitutions are merged in when encoding.
    #[serde(default)]
    pub x26: Vec<Triplet>,
    /// Publisher's X/27 and X/28 packets: (packet 27 or 28, designation code, the 39
    /// already coded bytes after the designation).
    #[serde(default)]
    pub extra: Vec<(u8, u8, Vec<u8>)>,
}

/// An enhancement triplet before Hamming 24/18 coding: (address, mode, data).
pub type Triplet = (u8, u8, u8);

/// Most X/26 triplets one page can carry: 16 packets of 13, less the termination marker.
pub const MAX_X26: usize = 16 * 13 - 1;

impl Default for Content {
    fn default() -> Self {
        Content {
            rows: vec![vec![b' '; COLS]; ROWS],
            g2: vec![],
            x26: vec![],
            extra: vec![],
        }
    }
}

impl Content {
    /// Build from markup rows: rows[0] is display row 1. See [`parse_row`].
    pub fn from_markup(rows: &[String]) -> Result<Self, String> {
        if rows.len() > ROWS {
            return Err(format!("at most {ROWS} rows"));
        }
        let mut c = Content::default();
        for (i, r) in rows.iter().enumerate() {
            c.set_row(i as u8 + 1, r)?;
        }
        Ok(c)
    }

    /// Replace display row `row` (1..=24) from markup.
    pub fn set_row(&mut self, row: u8, markup: &str) -> Result<(), String> {
        if !(1..=ROWS as u8).contains(&row) {
            return Err(format!("row {row} out of range 1..=24"));
        }
        let (codes, g2) = parse_row(markup)?;
        self.rows[row as usize - 1] = codes;
        self.g2.retain(|&(r, _, _)| r != row);
        self.g2.extend(g2.into_iter().map(|(col, g)| (row, col, g)));
        self.g2.sort();
        Ok(())
    }

    /// Replace display row `row` (1..=24) with 40 raw 7-bit codes (parity is added on
    /// the way out). For DRCS and object pages, whose rows are data, not text.
    pub fn set_raw_row(&mut self, row: u8, codes: &[u8]) -> Result<(), String> {
        if !(1..=ROWS as u8).contains(&row) {
            return Err(format!("row {row} out of range 1..=24"));
        }
        if codes.len() != COLS || codes.iter().any(|&c| c > 0x7F) {
            return Err(format!("raw row {row} must be {COLS} values 0..=127"));
        }
        self.rows[row as usize - 1] = codes.to_vec();
        self.g2.retain(|&(r, _, _)| r != row);
        Ok(())
    }

    /// Add an X/27 or X/28 packet from 13 triplets (Hamming 24/18 coded here) or from
    /// 39 bytes that are already coded (for X/27/0-3 links, which are Hamming 8/4).
    pub fn add_packet(
        &mut self,
        packet: u8,
        designation: u8,
        triplets: Option<&[Triplet]>,
        raw: Option<&[u8]>,
    ) -> Result<(), String> {
        let bytes = match (triplets, raw) {
            (Some(t), None) if t.len() <= 13 => {
                let mut b = Vec::with_capacity(39);
                for i in 0..13 {
                    let (a, m, d) = t.get(i).copied().unwrap_or((0, 0, 0));
                    b.extend(triplet(a, m, d));
                }
                b
            }
            (None, Some(r)) if r.len() == 39 => r.to_vec(),
            _ => return Err("a packet needs at most 13 triplets or exactly 39 raw bytes".into()),
        };
        if !(packet == 27 || packet == 28) || designation > 15 {
            return Err(format!("packet X/{packet}/{designation} not allowed here"));
        }
        self.extra
            .retain(|(p, d, _)| (*p, *d) != (packet, designation));
        self.extra.push((packet, designation, bytes));
        self.extra.sort_by_key(|(p, d, _)| (*p, *d));
        Ok(())
    }

    /// Set the publisher's X/26 triplets, checking they fit with the G2 substitutions.
    pub fn set_x26(&mut self, triplets: Vec<Triplet>) -> Result<(), String> {
        for &(a, m, d) in &triplets {
            if a > 63 || m > 31 || d > 127 {
                return Err(format!("bad triplet ({a}, {m}, {d})"));
            }
        }
        self.x26 = triplets;
        let n = merge_x26(&self.x26, &self.g2).len();
        if n > MAX_X26 {
            return Err(format!("{n} X/26 triplets, max {MAX_X26}"));
        }
        Ok(())
    }

    /// Plain-text rendering, for previews and tests.
    pub fn preview(&self) -> Vec<String> {
        self.rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                r.iter()
                    .enumerate()
                    .map(|(col, &b)| {
                        match self
                            .g2
                            .iter()
                            .find(|&&(gr, gc, _)| gr as usize == i + 1 && gc as usize == col)
                        {
                            Some(&(_, _, g)) => decode_g2(g),
                            None => decode_g0(b),
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }
}

/// Row codes plus (column, G2 code) substitutions.
pub type Cells = (Vec<u8>, Vec<(u8, u8)>);

/// Spacing attribute names usable in markup as `{name}`.
fn attribute(name: &str) -> Option<u8> {
    Some(match name {
        "black" => 0x00,
        "red" => 0x01,
        "green" => 0x02,
        "yellow" => 0x03,
        "blue" => 0x04,
        "magenta" => 0x05,
        "cyan" => 0x06,
        "white" => 0x07,
        "flash" => 0x08,
        "steady" => 0x09,
        "endbox" => 0x0A,
        "box" => 0x0B,
        "normal" => 0x0C,
        "double" => 0x0D,
        "gred" => 0x11,
        "ggreen" => 0x12,
        "gyellow" => 0x13,
        "gblue" => 0x14,
        "gmagenta" => 0x15,
        "gcyan" => 0x16,
        "gwhite" => 0x17,
        "conceal" => 0x18,
        "contiguous" => 0x19,
        "separated" => 0x1A,
        "blackbg" => 0x1C,
        "newbg" => 0x1D,
        "hold" => 0x1E,
        "release" => 0x1F,
        _ => return None,
    })
}

/// Parse one row of markup into 40 codes plus (column, G2 code) substitutions.
///
/// Text is Unicode. `{red}`, `{double}`, `{newbg}` etc. insert a spacing attribute (one
/// cell, shown as a space). `{x:7f}` inserts a raw code. `{{` and `}}` are literal braces,
/// which teletext cannot show and so become `?`. Rows are padded with spaces; longer than
/// 40 cells is an error.
pub fn parse_row(markup: &str) -> Result<Cells, String> {
    let (mut codes, g2) = parse_cells(markup)?;
    if codes.len() > COLS {
        return Err(format!(
            "row is {} cells, max {COLS}: {markup:?}",
            codes.len()
        ));
    }
    codes.resize(COLS, b' ');
    Ok((codes, g2))
}

/// Like [`parse_row`] but without padding or a length check.
pub fn parse_cells(markup: &str) -> Result<Cells, String> {
    let mut codes = Vec::with_capacity(COLS);
    let mut g2 = vec![];
    let mut chars = markup.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() != Some(&'{') {
            let name: String = chars.by_ref().take_while(|&c| c != '}').collect();
            let code = if let Some(hex) = name.strip_prefix("x:") {
                u8::from_str_radix(hex, 16).ok().filter(|&b| b < 0x80)
            } else {
                attribute(&name)
            };
            codes.push(code.ok_or_else(|| format!("unknown markup {{{name}}}"))?);
            continue;
        }
        if (c == '{' || c == '}') && chars.peek() == Some(&c) {
            chars.next();
        }
        let (code, sub) = encode_char(c);
        if let Some(g) = sub {
            g2.push((codes.len() as u8, g));
        }
        codes.push(code);
    }
    Ok((codes, g2))
}

/// Header control bits (ETS 300 706 §9.3.1).
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub erase: bool,           // C4
    pub newsflash: bool,       // C5
    pub subtitle: bool,        // C6
    pub suppress_header: bool, // C7
    pub update: bool,          // C8
    pub interrupted: bool,     // C9
    pub inhibit: bool,         // C10
}

fn mrag(mag: u8, row: u8) -> [u8; 2] {
    [ham84((mag & 7) | ((row & 1) << 3)), ham84(row >> 1)]
}

/// Build a page header packet. `text` is columns 8..39 (32 G0 codes).
pub fn header_packet(page: PageNo, subcode: u16, f: Flags, text: &[u8; 32]) -> Packet {
    let b = |x: bool| x as u8;
    let (c12, c13, c14) = NATIONAL_C12_C14;
    let mut p = [0u8; 42];
    p[..2].copy_from_slice(&mrag(page.mag, 0));
    p[2] = ham84(page.num & 0x0F);
    p[3] = ham84(page.num >> 4);
    p[4] = ham84(subcode as u8 & 0x0F);
    p[5] = ham84(((subcode >> 4) as u8 & 0x07) | b(f.erase) << 3);
    p[6] = ham84((subcode >> 8) as u8 & 0x0F);
    p[7] = ham84(((subcode >> 12) as u8 & 0x03) | b(f.newsflash) << 2 | b(f.subtitle) << 3);
    p[8] =
        ham84(b(f.suppress_header) | b(f.update) << 1 | b(f.interrupted) << 2 | b(f.inhibit) << 3);
    // C11 = 0: parallel magazine mode, so magazines may interleave.
    p[9] = ham84(b(c12) << 1 | b(c13) << 2 | b(c14) << 3);
    for (d, &s) in p[10..].iter_mut().zip(text) {
        *d = parity(s);
    }
    p
}

/// Build a display row packet (rows 1..=24).
pub fn row_packet(mag: u8, row: u8, codes: &[u8]) -> Packet {
    let mut p = [0u8; 42];
    p[..2].copy_from_slice(&mrag(mag, row));
    for (d, &s) in p[2..].iter_mut().zip(codes) {
        *d = parity(s);
    }
    p
}

fn triplet(address: u8, mode: u8, data: u8) -> [u8; 3] {
    ham2418(address as u32 & 0x3F | (mode as u32 & 0x1F) << 6 | (data as u32 & 0x7F) << 11)
}

/// Row address of a display row in a triplet: 40 means row 24, 41..=63 rows 1..=23.
fn row_address(row: u8) -> u8 {
    if row == 24 { 40 } else { 40 + row }
}

/// Merge the publisher's X/26 triplets with the G2 substitutions (row, col, code), in
/// the ascending active-position order decoders require. The publisher's stream is
/// split into groups that each start where a triplet moves the active position to a
/// row (row address triplets with mode 0x01, 0x04 or 0x07); each G2 character goes into
/// its row's group in column order, and a row with no group gets one.
pub fn merge_x26(publisher: &[Triplet], subs: &[(u8, u8, u8)]) -> Vec<Triplet> {
    // (row, triplets); the first group holds what comes before any row is addressed.
    let mut groups: Vec<(Option<u8>, Vec<Triplet>)> = vec![(None, vec![])];
    for &t in publisher {
        let (a, m, _) = t;
        if a == 63 && m == 0x1F {
            break; // termination marker
        }
        let moves = a >= 40 && matches!(m, 0x01 | 0x04 | 0x07);
        if moves {
            let row = if m == 0x07 {
                0
            } else if a == 40 {
                24
            } else {
                a - 40
            };
            groups.push((Some(row), vec![t]));
        } else {
            groups.last_mut().unwrap().1.push(t);
        }
    }
    for &(row, col, code) in subs {
        let idx = match groups.iter().rposition(|(r, _)| *r == Some(row)) {
            Some(i) => i,
            None => {
                let at = groups
                    .iter()
                    .position(|(r, _)| r.is_some_and(|r| r > row))
                    .unwrap_or(groups.len());
                groups.insert(at, (Some(row), vec![(row_address(row), 0x04, 0)]));
                at
            }
        };
        let g = &mut groups[idx].1;
        // After the row triplets and any column triplet at or before this column.
        let at = g
            .iter()
            .rposition(|&(a, _, _)| a >= 40 || a <= col)
            .map_or(0, |i| i + 1);
        g.insert(at, (col, 0x0F, code)); // mode 0x0F: character from G2
    }
    groups.into_iter().flat_map(|(_, g)| g).collect()
}

/// Build X/26 enhancement packets from triplets (a termination marker is added).
pub fn x26_packets(mag: u8, triplets: &[Triplet]) -> Vec<Packet> {
    let triplets: Vec<[u8; 3]> = triplets
        .iter()
        .take(MAX_X26)
        .map(|&(a, m, d)| triplet(a, m, d))
        .collect();
    triplets
        .chunks(13)
        .enumerate()
        .take(16)
        .map(|(dc, chunk)| {
            let mut p = [0u8; 42];
            p[..2].copy_from_slice(&mrag(mag, 26));
            p[2] = ham84(dc as u8);
            for i in 0..13 {
                // Unused triplets carry a termination marker (address 63, mode 0x1F).
                let t = chunk
                    .get(i)
                    .copied()
                    .unwrap_or_else(|| triplet(63, 0x1F, 0));
                p[3 + i * 3..6 + i * 3].copy_from_slice(&t);
            }
            p
        })
        .collect()
}

/// Encode a whole (sub)page: header, X/26, then rows.
/// With `skip_blank`, rows of only spaces are left out (only safe when `flags.erase`).
pub fn page_packets(
    page: PageNo,
    subcode: u16,
    flags: Flags,
    header: &[u8; 32],
    c: &Content,
    skip_blank: bool,
) -> Vec<Packet> {
    let mut out = vec![header_packet(page, subcode, flags, header)];
    // X/27, then X/28, then X/26, before the display rows.
    for (packet, designation, bytes) in &c.extra {
        let mut p = [0u8; 42];
        p[..2].copy_from_slice(&mrag(page.mag, *packet));
        p[2] = ham84(*designation);
        p[3..].copy_from_slice(&bytes[..39]);
        out.push(p);
    }
    out.extend(x26_packets(page.mag, &merge_x26(&c.x26, &c.g2)));
    for (i, r) in c.rows.iter().enumerate() {
        if skip_blank && r.iter().all(|&b| b == b' ') {
            continue;
        }
        out.push(row_packet(page.mag, i as u8 + 1, r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::{unham84, unham2418};

    #[test]
    fn page_numbers_parse() {
        assert_eq!("100".parse::<PageNo>().unwrap(), PageNo::new(1, 0x00));
        assert_eq!("888".parse::<PageNo>().unwrap(), PageNo::new(8, 0x88));
        assert_eq!("8FF".parse::<PageNo>().unwrap().to_string(), "8FF");
        assert!("900".parse::<PageNo>().is_err());
        assert!("10".parse::<PageNo>().is_err());
    }

    #[test]
    fn markup_and_norwegian_letters() {
        let (codes, g2) = parse_row("{yellow}Blåbærsyltetøy").unwrap();
        assert_eq!(codes[0], 0x03);
        assert_eq!(codes.len(), 40);
        assert_eq!(g2, vec![(5, 0x71), (13, 0x79)]);
        let c = Content::from_markup(&["{yellow}Blåbærsyltetøy".into()]).unwrap();
        assert_eq!(c.preview()[0], " Blåbærsyltetøy");
        assert!(parse_row(&"x".repeat(41)).is_err());
        assert!(parse_row("{nope}").is_err());
    }

    #[test]
    fn header_bits() {
        let f = Flags {
            erase: true,
            subtitle: true,
            ..Default::default()
        };
        let p = header_packet(PageNo::new(8, 0x88), 0, f, &[b' '; 32]);
        // Magazine 8 is sent as 0, row 0.
        assert_eq!(unham84(p[0]), Some(0));
        assert_eq!(unham84(p[1]), Some(0));
        assert_eq!(unham84(p[2]), Some(8));
        assert_eq!(unham84(p[3]), Some(8));
        assert_eq!(unham84(p[5]), Some(0b1000)); // C4 erase
        assert_eq!(unham84(p[7]), Some(0b1000)); // C6 subtitle
        assert_eq!(unham84(p[9]), Some(0b0100)); // C13: Swedish/Finnish/Hungarian
    }

    #[test]
    fn row_address_for_odd_rows() {
        let p = row_packet(1, 23, &[b' '; 40]);
        assert_eq!(unham84(p[0]), Some(1 | 8));
        assert_eq!(unham84(p[1]), Some(11));
    }

    #[test]
    fn x26_triplets_decode() {
        let p = x26_packets(1, &merge_x26(&[], &[(3, 5, 0x79)]));
        assert_eq!(p.len(), 1);
        let t = |i: usize| unham2418([p[0][3 + i * 3], p[0][4 + i * 3], p[0][5 + i * 3]]).unwrap();
        assert_eq!(t(0), 43 | 0x04 << 6);
        assert_eq!(t(1), 5 | 0x0F << 6 | 0x79 << 11);
        assert_eq!(t(2), 63 | 0x1F << 6);
    }

    #[test]
    fn g2_merges_into_publisher_x26_in_position_order() {
        let publisher = [
            (0, 0x00, 0x10), // full screen colour: before any row
            (41, 0x04, 0),   // row 1
            (2, 0x0D, 0x40), // DRCS at column 2
            (9, 0x0D, 0x41), // DRCS at column 9
            (45, 0x04, 0),   // row 5
            (0, 0x00, 0x12), // foreground colour at column 0
            (63, 0x1F, 0),   // terminator, dropped
        ];
        let subs = [(1, 5, 0x61), (3, 1, 0x69), (5, 7, 0x79), (24, 0, 0x71)];
        assert_eq!(
            merge_x26(&publisher, &subs),
            vec![
                (0, 0x00, 0x10),
                (41, 0x04, 0),
                (2, 0x0D, 0x40),
                (5, 0x0F, 0x61),
                (9, 0x0D, 0x41),
                (43, 0x04, 0),
                (1, 0x0F, 0x69),
                (45, 0x04, 0),
                (0, 0x00, 0x12),
                (7, 0x0F, 0x79),
                (40, 0x04, 0),
                (0, 0x0F, 0x71),
            ]
        );
    }

    #[test]
    fn enhancement_packets_come_before_rows() {
        let mut c = Content::from_markup(&["Hei".into()]).unwrap();
        c.set_x26(vec![(41, 0x04, 0), (0, 0x0D, 0x40)]).unwrap();
        c.add_packet(28, 0, Some(&[(1, 2, 3)]), None).unwrap();
        c.add_packet(27, 4, None, Some(&[0x15; 39])).unwrap();
        assert!(c.add_packet(26, 0, Some(&[]), None).is_err());
        let pk = page_packets(
            PageNo::new(1, 0),
            0,
            Flags::default(),
            &[b' '; 32],
            &c,
            true,
        );
        let rows: Vec<u8> = pk
            .iter()
            .map(|p| unham84(p[0]).unwrap() >> 3 | unham84(p[1]).unwrap() << 1)
            .collect();
        assert_eq!(rows, vec![0, 27, 28, 26, 1]);
        assert_eq!(unham84(pk[1][2]), Some(4));
        assert_eq!(
            unham2418([pk[2][3], pk[2][4], pk[2][5]]),
            Some(1 | 2 << 6 | 3 << 11)
        );
    }

    #[test]
    fn raw_rows() {
        let mut c = Content::default();
        c.set_raw_row(1, &[0x7F; 40]).unwrap();
        assert_eq!(c.rows[0][39], 0x7F);
        assert!(c.set_raw_row(1, &[0x80; 40]).is_err());
        assert!(c.set_raw_row(25, &[0; 40]).is_err());
    }
}
