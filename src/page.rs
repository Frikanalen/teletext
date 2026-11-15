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

/// The content of one (sub)page: 24 rows of 40 codes plus G2 substitutions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Content {
    /// rows[0] is display row 1.
    pub rows: Vec<Vec<u8>>,
    /// (row 1..=24, column, G2 code).
    #[serde(default)]
    pub g2: Vec<(u8, u8, u8)>,
}

impl Default for Content {
    fn default() -> Self {
        Content {
            rows: vec![vec![b' '; COLS]; ROWS],
            g2: vec![],
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

/// Build X/26 enhancement packets that place G2 characters at (row, col).
pub fn x26_packets(mag: u8, subs: &[(u8, u8, u8)]) -> Vec<Packet> {
    let mut triplets = vec![];
    let mut cur_row = None;
    for &(row, col, g2) in subs {
        if cur_row != Some(row) {
            // Row address group: 40 means row 24, 41..=63 rows 1..=23. Mode 0x04 sets the active position.
            let addr = if row == 24 { 40 } else { 40 + row };
            triplets.push(triplet(addr, 0x04, 0));
            cur_row = Some(row);
        }
        triplets.push(triplet(col, 0x0F, g2)); // mode 0x0F: character from G2
    }
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
    out.extend(x26_packets(page.mag, &c.g2));
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
        let p = x26_packets(1, &[(3, 5, 0x79)]);
        assert_eq!(p.len(), 1);
        let t = |i: usize| unham2418([p[0][3 + i * 3], p[0][4 + i * 3], p[0][5 + i * 3]]).unwrap();
        assert_eq!(t(0), 43 | 0x04 << 6);
        assert_eq!(t(1), 5 | 0x0F << 6 | 0x79 << 11);
        assert_eq!(t(2), 63 | 0x1F << 6);
    }
}
