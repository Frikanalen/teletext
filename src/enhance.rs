//! Declarative Level 2.5 enhancements: what a publisher wants on a page (cell colours,
//! glyphs, smooth mosaics, objects, fastext links, a palette), compiled by ttxd into
//! X/26, X/27/0 and X/28/0 packets, so publishers don't hand-code triplets.
//!
//! Glyph and object names refer to the shared assets (`PUT /v1/assets`) and are
//! resolved when the page is stored, so re-send a page after renaming or reordering
//! assets.

use crate::assets::{Layout, Palette};
use crate::ham::ham84;
use crate::page::{COLS, Content, PageNo, ROWS, Triplet};
use serde::Deserialize;

/// Fastext links: the remote's coloured keys and the index key.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Links {
    pub red: Option<PageNo>,
    pub green: Option<PageNo>,
    pub yellow: Option<PageNo>,
    pub cyan: Option<PageNo>,
    pub index: Option<PageNo>,
}

/// One cell's enhancement. Colours are palette entries 0-31 (16-31 are tables 2 and 3,
/// the ones a palette defines). As everywhere in teletext, a colour holds from its cell
/// to the end of the row or the next cell that sets that colour.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub row: u8,
    pub col: u8,
    pub fg: Option<u8>,
    pub bg: Option<u8>,
    /// A shared glyph by name; it takes the foreground colour.
    pub glyph: Option<String>,
    /// A G3 smooth mosaic / line drawing character, 0x20-0x7F (Level 1.5 and up).
    pub g3: Option<u8>,
}

/// A shared object placed with its top left corner at row/col.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub object: String,
    pub row: u8,
    pub col: u8,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Enhancements {
    #[serde(default)]
    pub links: Option<Links>,
    #[serde(default)]
    pub palette: Option<Palette>,
    #[serde(default)]
    pub cells: Vec<Cell>,
    #[serde(default)]
    pub place: Vec<Place>,
}

impl Enhancements {
    pub fn is_empty(&self) -> bool {
        self.links.is_none()
            && self.palette.is_none()
            && self.cells.is_empty()
            && self.place.is_empty()
    }

    /// Add the compiled packets to `c`. `has_x26` says the publisher also sent raw
    /// X/26, which can't be mixed with cells and objects.
    pub fn apply(
        &self,
        page: PageNo,
        layout: &Layout,
        c: &mut Content,
        has_x26: bool,
    ) -> Result<(), String> {
        if let Some(l) = &self.links {
            c.add_packet(27, 0, None, Some(&x27_0(page, l)))?;
        }
        if let Some(p) = &self.palette {
            c.add_packet(28, 0, Some(&p.triplets()?), None)?;
        }
        if self.cells.is_empty() && self.place.is_empty() {
            return Ok(());
        }
        if has_x26 {
            return Err("send either x26 or cells/place, not both".into());
        }
        c.set_x26(self.triplets(layout)?)
    }

    fn triplets(&self, layout: &Layout) -> Result<Vec<Triplet>, String> {
        enum Item<'a> {
            Cell(&'a Cell),
            Place(Triplet),
        }
        let mut items: Vec<(u8, u8, u8, Item)> = vec![];
        let check = |row: u8, col: u8| {
            if !(1..=ROWS as u8).contains(&row) || col as usize >= COLS {
                Err(format!(
                    "row {row}, column {col} is off the page (rows 1-24, columns 0-39)"
                ))
            } else {
                Ok(())
            }
        };
        for cell in &self.cells {
            check(cell.row, cell.col)?;
            if cell.fg.is_some_and(|v| v > 31) || cell.bg.is_some_and(|v| v > 31) {
                return Err("colours are 0-31".into());
            }
            if cell.g3.is_some_and(|v| !(0x20..=0x7F).contains(&v)) {
                return Err("g3 characters are 0x20-0x7F".into());
            }
            if cell.g3.is_some() && cell.glyph.is_some() {
                return Err("a cell holds a glyph or a g3 character, not both".into());
            }
            if let Some(g) = &cell.glyph
                && !layout.glyphs.contains_key(g)
            {
                return Err(format!("no shared glyph {g:?}"));
            }
            items.push((cell.row, cell.col, 0, Item::Cell(cell)));
        }
        for p in &self.place {
            check(p.row, p.col)?;
            let invoke = *layout
                .objects
                .get(&p.object)
                .ok_or_else(|| format!("no shared object {:?}", p.object))?;
            items.push((p.row, p.col, 1, Item::Place(invoke)));
        }
        items.sort_by_key(|&(r, c, k, _)| (r, c, k));
        let mut out = vec![];
        let mut at: Option<u8> = None; // row the active position is on, if usable
        for (row, col, _, item) in items {
            match item {
                Item::Cell(cell) => {
                    if at != Some(row) {
                        out.push((row_address(row), 0x04, col));
                        at = Some(row);
                    }
                    if let Some(v) = cell.fg {
                        out.push((col, 0x00, v));
                    }
                    if let Some(v) = cell.bg {
                        out.push((col, 0x03, v));
                    }
                    if let Some(g) = &cell.glyph {
                        out.push((col, 0x0D, layout.glyphs[g]));
                    }
                    if let Some(v) = cell.g3 {
                        out.push((col, 0x02, v));
                    }
                }
                Item::Place(invoke) => {
                    out.push((row_address(row), 0x04, col));
                    out.push(invoke);
                    // Set the position again before any later cell on this row.
                    at = None;
                }
            }
        }
        Ok(out)
    }
}

fn row_address(row: u8) -> u8 {
    if row == 24 { 40 } else { 40 + row }
}

/// X/27/0, the 39 coded bytes after the designation: links for red, green, yellow,
/// cyan, the next-page link (unused) and index, then the link control byte (show row
/// 24) and a CRC decoders ignore. Each link is page units, tens, then subcode 3F7F with
/// the magazine as bits relative to this page's.
pub fn x27_0(own: PageNo, l: &Links) -> Vec<u8> {
    let none = PageNo::new(8, 0xFF);
    let mut out = vec![];
    for p in [l.red, l.green, l.yellow, l.cyan, None, l.index] {
        let p = p.unwrap_or(none);
        let mag = (p.mag ^ own.mag) & 7;
        for n in [
            p.num & 15,
            p.num >> 4,
            0xF,
            0x7 | (mag & 1) << 3,
            0xF,
            0x3 | (mag >> 1) << 2,
        ] {
            out.push(ham84(n));
        }
    }
    out.push(ham84(0xF));
    out.extend([0, 0]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ham::unham84;

    fn layout() -> Layout {
        let mut l = Layout::default();
        l.glyphs.insert("box".into(), 3);
        l.objects.insert("logo".into(), (0x38, 0x12, 0x10));
        l
    }

    fn cell(row: u8, col: u8) -> Cell {
        Cell {
            row,
            col,
            fg: None,
            bg: None,
            glyph: None,
            g3: None,
        }
    }

    #[test]
    fn cells_and_objects_compile_in_position_order() {
        let e = Enhancements {
            cells: vec![
                Cell {
                    fg: Some(22),
                    glyph: Some("box".into()),
                    ..cell(5, 12)
                },
                Cell {
                    g3: Some(0x35),
                    ..cell(5, 2)
                },
                Cell {
                    bg: Some(17),
                    ..cell(24, 0)
                },
            ],
            place: vec![Place {
                object: "logo".into(),
                row: 5,
                col: 8,
            }],
            ..Default::default()
        };
        assert_eq!(
            e.triplets(&layout()).unwrap(),
            vec![
                (45, 0x04, 2),
                (2, 0x02, 0x35),
                (45, 0x04, 8),
                (0x38, 0x12, 0x10),
                (45, 0x04, 12),
                (12, 0x00, 22),
                (12, 0x0D, 3),
                (40, 0x04, 0),
                (0, 0x03, 17),
            ]
        );
    }

    #[test]
    fn bad_input_says_what() {
        let l = layout();
        let e = |c: Cell| {
            Enhancements {
                cells: vec![c],
                ..Default::default()
            }
            .triplets(&l)
        };
        assert!(e(cell(0, 0)).unwrap_err().contains("off the page"));
        assert!(e(cell(1, 40)).is_err());
        assert!(
            e(Cell {
                fg: Some(32),
                ..cell(1, 0)
            })
            .is_err()
        );
        assert!(
            e(Cell {
                glyph: Some("nope".into()),
                ..cell(1, 0)
            })
            .unwrap_err()
            .contains("nope")
        );
        assert!(
            e(Cell {
                g3: Some(0x10),
                ..cell(1, 0)
            })
            .is_err()
        );
        let p = Enhancements {
            place: vec![Place {
                object: "x".into(),
                row: 1,
                col: 0,
            }],
            ..Default::default()
        };
        assert!(p.triplets(&l).is_err());
    }

    #[test]
    fn fastext_links_relative_to_own_magazine() {
        let l = Links {
            red: Some("200".parse().unwrap()),
            index: Some("100".parse().unwrap()),
            ..Default::default()
        };
        let b = x27_0("100".parse().unwrap(), &l);
        assert_eq!(b.len(), 39);
        let n: Vec<u8> = b[..36].iter().map(|&x| unham84(x).unwrap()).collect();
        // Red: page 00, magazine 2 ^ 1 = 3 -> M1 = 1, M2 = 1.
        assert_eq!(&n[..6], &[0, 0, 0xF, 0xF, 0xF, 0x7]);
        // Green: none (8FF): 8 ^ 1 = 1 -> M1 only.
        assert_eq!(&n[6..12], &[0xF, 0xF, 0xF, 0xF, 0xF, 0x3]);
        // Index: own magazine.
        assert_eq!(&n[30..36], &[0, 0, 0xF, 0x7, 0xF, 0x3]);
    }
}
