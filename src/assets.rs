//! Shared Level 2.5 assets: the magazine palette, global DRCS glyphs and global objects.
//!
//! A publisher defines them once with `PUT /v1/assets`, and every page can use them
//! without carrying their data itself:
//!
//! - **Palette** goes out as packet M/29/0 in every magazine in use. It has the same
//!   coding as X/28/0 format 1 (colour tables 2 and 3, default screen and row colour,
//!   colour table remapping). Pages that send their own X/28/0 keep their own.
//! - **Glyphs** are 12x10 one-bit DRCS characters on one GDRCS page. A page shows glyph
//!   `n` with the X/26 column triplet `(column, 0x0D, n)` (bit 6 clear = global).
//! - **Objects** are runs of X/26 triplets on one GPOP page. A page places one with two
//!   triplets: set active position, then the invocation that [`Assets::layout`] returns.
//!
//! The Magazine Organisation Table (page mFE of each magazine in use) points decoders at
//! the GPOP and GDRCS pages through its global links (link 0 of rows 19 and 21).
//!
//! Layouts follow ETS 300 706 §10-§13 as libzvbi 0.2.46 implements it (src/packet.c
//! `parse_mot`, `parse_pop`, `convert_drcs`; src/teletext.c `resolve_obj_address`).

use crate::ham::{ham84, parity};
use crate::page::{COLS, Content, PageNo, Triplet, triplet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const GLYPH_W: usize = 12;
pub const GLYPH_H: usize = 10;
/// DRCS characters on one (sub)page.
pub const MAX_GLYPHS: usize = 48;
/// Objects of each type one GPOP subpage can point to through its first pointer packet.
pub const MAX_OBJECTS_PER_TYPE: usize = 8;
/// DRCS pages of their own that the display pages of one magazine can link to.
pub const MOT_DRCS_LINKS: usize = 7;
/// GPOP triplet space: packets 3..=24, 13 triplets each.
pub const GPOP_TRIPLETS: usize = 22 * 13;
/// Charset code sent in M/29/0: Latin G0, Swedish/Finnish/Hungarian national option
/// (group 0, option C12..C14 = 0,1,0), the same as ttxd's page headers.
const CHARSET: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    #[serde(default)]
    pub glyphs: Vec<Glyph>,
    #[serde(default)]
    pub objects: Vec<Object>,
}

/// Colour tables 2 and 3 for every page that doesn't send its own X/28/0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    /// Up to 16 colours `#rrggbb`: table 2 (entries 16-23), then table 3 (24-31).
    /// Missing entries are black. Decoders keep 4 bits per channel.
    pub colours: Vec<String>,
    /// Default screen colour, 0-31 (16 = first colour above).
    #[serde(default)]
    pub screen: u8,
    /// Default row colour, 0-31.
    #[serde(default)]
    pub row: u8,
    /// Colour table remapping, 0-7: which tables the Level 1 colour attributes pick
    /// from (foreground/background). 0 = 0/0 (no remapping), 6 = 2/2.
    #[serde(default)]
    pub remap: u8,
    /// Show a black background as the default row/screen colour instead.
    #[serde(default)]
    pub black_bg_substitution: bool,
}

/// A 12x10 DRCS character: 10 strings of 12, `#` (or `X`, `*`, `1`) for a lit pixel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Glyph {
    pub name: String,
    pub rows: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectType {
    Active = 1,
    Adaptive = 2,
    Passive = 3,
}

/// A global object: X/26 triplets, positioned relative to where a page invokes it.
/// Inside an object, row address 41 means the row below the invocation, 42 two rows
/// below, and so on; column triplets on the invocation row need no row triplet first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: ObjectType,
    pub x26: Vec<Triplet>,
}

/// Where each glyph and object ended up, for publishers to reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Layout {
    /// Glyph name to DRCS character number (the data of a 0x0D triplet).
    pub glyphs: BTreeMap<String, u8>,
    /// Object name to its invocation triplet (address, mode, data).
    pub objects: BTreeMap<String, Triplet>,
}

/// Packs fields into 18-bit triplets, first field in the lowest bits (the order libzvbi
/// reads them).
struct Bits {
    words: Vec<u32>,
    n: usize,
}

impl Bits {
    fn new() -> Self {
        Bits {
            words: vec![],
            n: 0,
        }
    }

    fn put(&mut self, value: u32, bits: usize) -> &mut Self {
        for i in 0..bits {
            if self.n.is_multiple_of(18) {
                self.words.push(0);
            }
            let w = self.words.last_mut().unwrap();
            *w |= ((value >> i) & 1) << (self.n % 18);
            self.n += 1;
        }
        self
    }

    fn triplets(&self) -> Vec<Triplet> {
        let mut out: Vec<Triplet> = self.words.iter().map(|&w| word(w)).collect();
        out.resize(13, (0, 0, 0));
        out
    }
}

fn word(w: u32) -> Triplet {
    (
        (w & 0x3F) as u8,
        (w >> 6 & 0x1F) as u8,
        (w >> 11 & 0x7F) as u8,
    )
}

fn coded_triplets(t: &[Triplet]) -> Vec<u8> {
    t.iter().flat_map(|&(a, m, d)| triplet(a, m, d)).collect()
}

fn rgb12(s: &str) -> Result<u32, String> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("colour {s:?} is not #rrggbb"));
    }
    let ch = |i: usize| u32::from_str_radix(&h[i..i + 2], 16).unwrap();
    // 8 bits to 4, rounded.
    let n = |v: u32| (v * 15 + 127) / 255;
    Ok(n(ch(0)) | n(ch(2)) << 4 | n(ch(4)) << 8)
}

impl Palette {
    /// The 13 triplets of M/29/0 (or X/28/0 format 1).
    pub fn triplets(&self) -> Result<Vec<Triplet>, String> {
        if self.colours.len() > 16 {
            return Err("a palette has at most 16 colours".into());
        }
        if self.screen > 31 || self.row > 31 || self.remap > 7 {
            return Err("screen and row are 0-31, remap is 0-7".into());
        }
        let mut b = Bits::new();
        b.put(0, 4); // page function: basic level one page
        b.put(0, 3); // page coding: 7-bit odd parity
        b.put(CHARSET, 7).put(CHARSET, 7); // default and second G0 set
        b.put(0, 1).put(0, 1).put(0, 1).put(0, 4); // no side panels
        for i in 0..16 {
            let c = match self.colours.get(i) {
                Some(s) => rgb12(s)?,
                None => 0,
            };
            b.put(c, 12);
        }
        b.put(self.screen as u32, 5).put(self.row as u32, 5);
        b.put(self.black_bg_substitution as u32, 1);
        b.put(self.remap as u32, 3);
        Ok(b.triplets())
    }
}

impl Glyph {
    /// 20 bytes of PTU data: per pixel row, 6 pixels then 6 pixels, leftmost in bit 5,
    /// bit 6 always set. Without parity.
    fn ptu(&self) -> Result<[u8; 20], String> {
        if self.rows.len() != GLYPH_H || self.rows.iter().any(|r| r.chars().count() != GLYPH_W) {
            return Err(format!(
                "glyph {:?} must be {GLYPH_H} rows of {GLYPH_W} characters",
                self.name
            ));
        }
        let mut out = [0x40u8; 20];
        for (y, row) in self.rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                if matches!(c, '#' | 'X' | 'x' | '*' | '1' | '@') {
                    out[y * 2 + x / 6] |= 0x20 >> (x % 6);
                }
            }
        }
        Ok(out)
    }
}

/// (pointer slot 0..24 in pointer packet 1, invocation data) for the `n`th object of a
/// type. Triplet `i` (1..=12) of the pointer packet holds two pointers; libzvbi looks
/// one up as triplet `group * 3 + type`, pointer `(data >> 4) & 1`, with the group in
/// data bits 5-6 and the subpage in bits 0-3 (always 0 here).
fn object_slot(kind: ObjectType, n: usize) -> (usize, u8) {
    let group = n / 2;
    let half = n % 2;
    let i = group * 3 + kind as usize;
    (i * 2 + half, (group << 5 | half << 4) as u8)
}

/// Invocation address: source 3 (global), pointer packet 1.
const GLOBAL_SOURCE: u8 = 0x38;

impl Assets {
    pub fn validate(&self) -> Result<Layout, String> {
        if let Some(p) = &self.palette {
            p.triplets()?;
        }
        if self.glyphs.len() > MAX_GLYPHS {
            return Err(format!("at most {MAX_GLYPHS} glyphs"));
        }
        let mut layout = Layout::default();
        for (i, g) in self.glyphs.iter().enumerate() {
            g.ptu()?;
            if layout.glyphs.insert(g.name.clone(), i as u8).is_some() {
                return Err(format!("two glyphs are called {:?}", g.name));
            }
        }
        let mut count = [0usize; 4];
        let mut triplets = 1; // the termination marker
        for o in &self.objects {
            let n = &mut count[o.kind as usize];
            if *n == MAX_OBJECTS_PER_TYPE {
                return Err(format!(
                    "at most {MAX_OBJECTS_PER_TYPE} objects of each type"
                ));
            }
            if let Some(&(a, m, d)) = o.x26.iter().find(|&&(a, m, d)| a > 63 || m > 31 || d > 127) {
                return Err(format!("object {:?}: bad triplet ({a}, {m}, {d})", o.name));
            }
            let (_, data) = object_slot(o.kind, *n);
            *n += 1;
            triplets += 1 + o.x26.len();
            let invoke = (GLOBAL_SOURCE, 0x10 + o.kind as u8, data);
            if layout.objects.insert(o.name.clone(), invoke).is_some() {
                return Err(format!("two objects are called {:?}", o.name));
            }
        }
        if triplets > GPOP_TRIPLETS {
            return Err(format!(
                "objects need {triplets} triplets; the GPOP page holds {GPOP_TRIPLETS}"
            ));
        }
        Ok(layout)
    }

    /// The M/29/0 packet body (designation 0), if there is a palette.
    pub fn m29(&self) -> Option<Vec<u8>> {
        let t = self.palette.as_ref()?.triplets().ok()?;
        Some(coded_triplets(&t))
    }

    /// The GDRCS page: two glyphs per row, and X/28/3 saying every character is 12x10x1.
    pub fn gdrcs(&self) -> Option<Content> {
        if self.glyphs.is_empty() {
            return None;
        }
        let mut c = Content::default();
        let mut b = Bits::new();
        b.put(4, 4); // page function: global DRCS
        b.put(0, 3).put(0, 11);
        for _ in 0..MAX_GLYPHS {
            b.put(0, 4); // mode 0: 12x10, one bit per pixel
        }
        c.extra.push((28, 3, coded_triplets(&b.triplets())));
        for (row, pair) in self.glyphs.chunks(2).enumerate() {
            let mut bytes = [0x40u8; COLS];
            for (k, g) in pair.iter().enumerate() {
                bytes[k * 20..k * 20 + 20].copy_from_slice(&g.ptu().ok()?);
            }
            c.coded
                .insert(row as u8 + 1, bytes.iter().map(|&x| parity(x)).collect());
        }
        Some(c)
    }

    /// The GPOP page: pointer packet 1, then the object definitions from packet 3.
    pub fn gpop(&self) -> Option<Content> {
        if self.objects.is_empty() {
            return None;
        }
        let mut pointers = [0x1FFu32; 26]; // 511: past the end, no object
        let mut stream: Vec<Triplet> = vec![];
        let mut count = [0usize; 4];
        for o in &self.objects {
            let (slot, data) = object_slot(o.kind, count[o.kind as usize]);
            count[o.kind as usize] += 1;
            pointers[slot] = stream.len() as u32;
            // Definition: mode 0x14 + type, address and data matching the invocation.
            stream.push((GLOBAL_SOURCE, 0x14 + o.kind as u8, data));
            stream.extend(&o.x26);
        }
        stream.push((63, 0x1F, 0));
        let mut c = Content::default();
        let mut p1 = vec![ham84(1)];
        p1.extend(triplet(0, 0, 0));
        for i in 1..13 {
            let w = pointers[i * 2] | pointers[i * 2 + 1] << 9;
            p1.extend(coded_triplets(&[word(w)]));
        }
        c.coded.insert(1, p1);
        for (k, chunk) in stream.chunks(13).enumerate() {
            let mut bytes = vec![ham84(0)];
            let mut t = chunk.to_vec();
            t.resize(13, (63, 0x1F, 0));
            bytes.extend(coded_triplets(&t));
            c.coded.insert(k as u8 + 3, bytes);
        }
        Some(c)
    }

    /// The MOT page of a magazine: the global links to the GPOP and GDRCS pages, and
    /// for each display page in `page_drcs` (page number in the magazine, its own DRCS
    /// page) a link to that DRCS page. With a palette, also the magazine's M/29/0.
    ///
    /// Rows 1-14 hold two digits per page, an object link and a DRCS link index;
    /// row 21 holds the DRCS links, 0 the global one and 1-7 the pages' own.
    pub fn mot(&self, gpop: PageNo, gdrcs: PageNo, page_drcs: &[(u8, PageNo, u8)]) -> Content {
        let mut c = Content::default();
        if let Some(m29) = self.m29() {
            c.extra.push((29, 0, m29));
        }
        let mut rows = vec![vec![0u8; COLS]; 15];
        let none = PageNo::new(8, 0xFF);
        let mut drcs: Vec<(PageNo, u8)> = vec![];
        for &(num, target, subpages) in page_drcs {
            let known = drcs.iter().position(|(p, _)| *p == target);
            let index = match known {
                Some(i) => i,
                None if drcs.len() < MOT_DRCS_LINKS => {
                    drcs.push((target, subpages));
                    drcs.len() - 1
                }
                None => continue,
            };
            let (tens, unit) = ((num >> 4) as usize, (num & 15) as usize);
            let (row, at) = if unit < 10 {
                (tens / 2 + 1, (tens % 2 * 10 + unit) * 2)
            } else {
                (9 + tens / 3, (tens % 3 * 6 + unit - 10) * 2)
            };
            rows[row][at + 1] = index as u8 + 1;
        }
        for (row, digits) in rows.into_iter().enumerate().skip(1) {
            c.coded
                .insert(row as u8, digits.into_iter().map(ham84).collect());
        }
        let link = |p: PageNo| [p.mag & 7, p.num >> 4, p.num & 15];
        let mut r19 = vec![];
        for i in 0..4 {
            let p = if i == 0 && !self.objects.is_empty() {
                gpop
            } else {
                none
            };
            r19.extend(link(p));
            // Subpages 0; fallback: none (bit 0); no default objects.
            r19.extend([0, 1, 0, 0, 0, 0, 0]);
        }
        c.coded.insert(19, r19.into_iter().map(ham84).collect());
        let mut r21 = vec![];
        for i in 0..8 {
            let (p, subpages) = match i {
                0 if !self.glyphs.is_empty() => (gdrcs, 0),
                0 => (none, 0),
                _ => drcs.get(i - 1).copied().unwrap_or((none, 0)),
            };
            r21.extend(link(p));
            r21.push(subpages.min(15));
        }
        r21.resize(COLS, 0);
        c.coded.insert(21, r21.into_iter().map(ham84).collect());
        c
    }

    pub fn is_empty(&self) -> bool {
        self.palette.is_none() && self.glyphs.is_empty() && self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::{unham84, unham2418};

    fn glyph(name: &str) -> Glyph {
        let mut rows = vec![".".repeat(12); 10];
        rows[0] = "#.........##".into();
        Glyph {
            name: name.into(),
            rows,
        }
    }

    #[test]
    fn palette_fills_thirteen_triplets() {
        let p = Palette {
            colours: vec!["#ff0000".into(), "#00ff00".into()],
            screen: 16,
            row: 17,
            remap: 6,
            black_bg_substitution: false,
        };
        let t = p.triplets().unwrap();
        assert_eq!(t.len(), 13);
        let w = |i: usize| t[i].0 as u32 | (t[i].1 as u32) << 6 | (t[i].2 as u32) << 11;
        // Bits 0-6 function+coding, 7-13 charset 2.
        assert_eq!(w(0) >> 7 & 0x7F, 2);
        // First colour starts at bit 4+3+14+3+4 = 28: red nibble 15.
        let bits: u128 = (0..8).map(|i| (w(i) as u128) << (18 * i)).sum();
        assert_eq!(bits >> 28 & 0xFFF, 0x00F);
        assert_eq!(bits >> 40 & 0xFFF, 0x0F0);
        assert!(
            Palette {
                colours: vec!["red".into()],
                ..p.clone()
            }
            .triplets()
            .is_err()
        );
    }

    #[test]
    fn glyph_pixels_pack_leftmost_in_bit_5() {
        let g = glyph("a");
        let ptu = g.ptu().unwrap();
        assert_eq!(ptu[0], 0x40 | 0x20);
        assert_eq!(ptu[1], 0x40 | 0x03);
        assert_eq!(ptu[2], 0x40);
        let bad = Glyph {
            name: "b".into(),
            rows: vec!["#".into()],
        };
        assert!(bad.ptu().is_err());
    }

    #[test]
    fn objects_get_distinct_pointer_slots() {
        let a = Assets {
            objects: (0..3)
                .map(|i| Object {
                    name: format!("o{i}"),
                    kind: ObjectType::Adaptive,
                    x26: vec![(0, 0x00, 1)],
                })
                .collect(),
            ..Default::default()
        };
        let l = a.validate().unwrap();
        assert_eq!(l.objects["o0"], (0x38, 0x12, 0x00));
        assert_eq!(l.objects["o1"], (0x38, 0x12, 0x10));
        assert_eq!(l.objects["o2"], (0x38, 0x12, 0x20));
        let page = a.gpop().unwrap();
        let p1 = &page.coded[&1];
        assert_eq!(unham84(p1[0]), Some(1));
        // Triplet 2 (group 0, adaptive) points at 0 and 2.
        let t2 = unham2418([p1[7], p1[8], p1[9]]).unwrap();
        assert_eq!((t2 & 0x1FF, t2 >> 9), (0, 2));
        // Triplet 5 (group 1, adaptive) holds o2 at 4, and nothing.
        let t5 = unham2418([p1[16], p1[17], p1[18]]).unwrap();
        assert_eq!((t5 & 0x1FF, t5 >> 9), (4, 0x1FF));
        // The stream: definition, body, definition, ...
        let p3 = &page.coded[&3];
        let def = unham2418([p3[1], p3[2], p3[3]]).unwrap();
        assert_eq!((def & 0x3F, def >> 6 & 0x1F, def >> 11), (0x38, 0x16, 0));
    }

    #[test]
    fn too_many_objects_or_glyphs() {
        let o = |i| Object {
            name: format!("{i}"),
            kind: ObjectType::Passive,
            x26: vec![],
        };
        let a = Assets {
            objects: (0..9).map(o).collect(),
            ..Default::default()
        };
        assert!(a.validate().is_err());
        let a = Assets {
            glyphs: (0..49).map(|i| glyph(&i.to_string())).collect(),
            ..Default::default()
        };
        assert!(a.validate().is_err());
        let a = Assets {
            glyphs: vec![glyph("x"), glyph("x")],
            ..Default::default()
        };
        assert!(a.validate().is_err());
    }

    #[test]
    fn mot_links_point_at_the_global_pages() {
        let a = Assets {
            glyphs: vec![glyph("a")],
            objects: vec![Object {
                name: "o".into(),
                kind: ObjectType::Active,
                x26: vec![],
            }],
            ..Default::default()
        };
        let mot = a.mot(PageNo::new(8, 0xE8), PageNo::new(8, 0xE9), &[]);
        let n = |row: u8, i: usize| unham84(mot.coded[&row][i]).unwrap();
        assert_eq!((n(19, 0), n(19, 1), n(19, 2)), (0, 0xE, 8));
        assert_eq!((n(19, 10), n(19, 11), n(19, 12)), (0, 0xF, 0xF));
        assert_eq!((n(21, 0), n(21, 1), n(21, 2)), (0, 0xE, 9));
        assert_eq!(n(1, 0), 0);
        assert!(mot.extra.is_empty());
    }

    #[test]
    fn mot_links_pages_to_their_own_drcs_page() {
        let drcs = PageNo::new(1, 0x0F);
        let pages = [
            (0x00, drcs, 3),
            (0x2B, drcs, 3),
            (0x35, PageNo::new(1, 0x1F), 1),
        ];
        let mot = Assets::default().mot(PageNo::new(8, 0xE8), PageNo::new(8, 0xE9), &pages);
        let n = |row: u8, i: usize| unham84(mot.coded[&row][i]).unwrap();
        // Page x00: row 1, first pair. x2B: row 9, third group. x35: row 2, second half.
        assert_eq!((n(1, 0), n(1, 1)), (0, 1));
        assert_eq!(n(9, (2 * 6 + 1) * 2 + 1), 1);
        assert_eq!(n(2, (10 + 5) * 2 + 1), 2);
        assert_eq!(n(1, 3), 0);
        // Row 21: no global DRCS, then the two pages with their subpage counts.
        assert_eq!((n(21, 1), n(21, 2)), (0xF, 0xF));
        assert_eq!((n(21, 4), n(21, 5), n(21, 6), n(21, 7)), (1, 0, 0xF, 3));
        assert_eq!((n(21, 8), n(21, 9), n(21, 10), n(21, 11)), (1, 1, 0xF, 1));
    }
}
