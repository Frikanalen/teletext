//! Page previews as PNG, at Level 1 or with the Level 1.5/2.5 enhancements.
//!
//!   GET /v1/pages/{page}.png?level=1|1.5|2.5&subpage=N
//!
//! A preview draws what ttxd holds for a page the way a decoder would compose it: the
//! Level 1 rows with their spacing attributes, then the page format (X/28/0, or the
//! magazine palette), and the X/26 enhancements, including shared objects, shared
//! glyphs (GDRCS) and glyphs on the page's own DRCS page. It follows the web viewer's
//! composer (frontend `src/components/teletext/compose.ts`), which is checked cell for
//! cell against libzvbi. Characters come from Bedstead (CC0), at 12x20 pixels a cell.
//!
//! Not drawn: G3 smooth mosaic and line drawing characters show as a hatched cell,
//! since Bedstead has no G3 glyphs; flashing cells show in their "on" phase; side
//! panels.

use crate::assets::{Assets, GLYPH_H};
use crate::ham::unham2418;
use crate::page::{COLS, Content, PageNo, Triplet, merge_x26};
use crate::service::Service;

pub const CELL_W: usize = 12;
pub const CELL_H: usize = 20;
const ROWS: usize = 25;
pub const WIDTH: usize = COLS * CELL_W;
pub const HEIGHT: usize = ROWS * CELL_H;
const TRANSPARENT: u8 = 8;

const FONT_CHARS: &str = include_str!("bedstead.txt");
const FONT_DATA: &[u8] = include_bytes!("bedstead.bin");
const GLYPH_BYTES: usize = CELL_W * CELL_H / 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    L1,
    L15,
    L25,
}

impl std::str::FromStr for Level {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "1" => Ok(Level::L1),
            "1.5" => Ok(Level::L15),
            "2.5" => Ok(Level::L25),
            _ => Err(format!("level {s:?}: use 1, 1.5 or 2.5")),
        }
    }
}

#[derive(Clone, Debug)]
struct Cell {
    ch: char,
    /// Block mosaic as bits 0-5, or None.
    mosaic: Option<u8>,
    separated: bool,
    /// 12x10 pixels, bit 11 = left.
    drcs: Option<[u16; GLYPH_H]>,
    g3: bool,
    fg: u8,
    bg: u8,
    conceal: bool,
    boxed: bool,
    dh: bool,
    dw: bool,
    covered: bool,
}

fn blank(bg: u8) -> Cell {
    Cell {
        ch: ' ',
        mosaic: None,
        separated: false,
        drcs: None,
        g3: false,
        fg: 7,
        bg,
        conceal: false,
        boxed: false,
        dh: false,
        dw: false,
        covered: false,
    }
}

// Colour tables 0-3 (ETS 300 706 §12.4), 12-bit RGB as 0xRGB.
const DEFAULT_CLUT: [u16; 32] = [
    0x000, 0xf00, 0x0f0, 0xff0, 0x00f, 0xf0f, 0x0ff, 0xfff, 0x000, 0x700, 0x070, 0x770, 0x007,
    0x707, 0x077, 0x777, 0xf05, 0xf70, 0x0f7, 0xffb, 0x0ca, 0x500, 0x652, 0xc77, 0x333, 0xf77,
    0x7f7, 0xff7, 0x77f, 0xf7f, 0x7ff, 0xddd,
];
// Colour table remapping (§9.4.2 table 32): (foreground table, background table).
const REMAP: [(u8, u8); 8] = [
    (0, 0),
    (0, 1),
    (0, 2),
    (1, 1),
    (1, 2),
    (2, 1),
    (2, 2),
    (3, 3),
];

struct Format {
    palette: [[u8; 3]; 32],
    screen: u8,
    row: u8,
    black_bg_sub: bool,
    fg_table: u8,
    bg_table: u8,
}

fn read_bits(words: &[u32]) -> impl FnMut(usize) -> u32 + '_ {
    let mut i = 0usize;
    move |n| {
        let mut v = 0;
        for k in 0..n {
            let w = words.get(i / 18).copied().unwrap_or(0);
            v |= ((w >> (i % 18)) & 1) << k;
            i += 1;
        }
        v
    }
}

fn format(words: Option<Vec<u32>>) -> Format {
    let rgb = |v: u16| {
        [
            ((v >> 8) & 15) as u8 * 17,
            ((v >> 4) & 15) as u8 * 17,
            (v & 15) as u8 * 17,
        ]
    };
    let mut f = Format {
        palette: DEFAULT_CLUT.map(rgb),
        screen: 0,
        row: 0,
        black_bg_sub: false,
        fg_table: 0,
        bg_table: 0,
    };
    let Some(words) = words else { return f };
    let mut take = read_bits(&words);
    take(4 + 3 + 7 + 7 + 7); // function, coding, G0 sets, side panels
    for i in 0..16 {
        let v = take(12);
        f.palette[16 + i] = [
            (v & 15) as u8 * 17,
            ((v >> 4) & 15) as u8 * 17,
            ((v >> 8) & 15) as u8 * 17,
        ];
    }
    f.screen = take(5) as u8;
    f.row = take(5) as u8;
    f.black_bg_sub = take(1) != 0;
    (f.fg_table, f.bg_table) = REMAP[take(3) as usize];
    f
}

/// Decode 13 coded triplets (39 bytes) to 18-bit words.
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks(3)
        .map(|t| unham2418([t[0], t[1], t[2]]).unwrap_or(0))
        .collect()
}

fn g0(b: u8, national: bool) -> char {
    if b == 0x7F {
        return '■';
    }
    if national {
        crate::charset::decode_g0(b)
    } else {
        b as char
    }
}

const G2: &str = concat!(
    " ¡¢£$¥#§¤‘“«←↑→↓",
    "°±²³×µ¶·÷’”»¼½¾¿",
    "                ",
    "―¹®©™♪₠‰α   ⅛⅜⅝⅞",
    "ΩÆĐªĦ ĲĿŁØŒºÞŦŊŉ",
    "ĸæđðħıĳŀłøœßþŧŋ■",
);

fn g2(b: u8) -> char {
    G2.chars()
        .nth((b & 0x7F).saturating_sub(0x20) as usize)
        .unwrap_or(' ')
}

/// PTU `n` of a DRCS page's rows (row (n / 2) + 1, 20 bytes from (n % 2) * 20).
fn drcs_from_rows(rows: &[Vec<u8>], n: u8) -> Option<[u16; GLYPH_H]> {
    let row = rows.get(n as usize / 2)?;
    let off = (n as usize % 2) * 20;
    let mut out = [0u16; GLYPH_H];
    for (y, o) in out.iter_mut().enumerate() {
        let a = *row.get(off + 2 * y)? as u16;
        let b = *row.get(off + 2 * y + 1)? as u16;
        *o = (a & 0x3F) << 6 | (b & 0x3F);
    }
    Some(out)
}

struct Ctx<'a> {
    svc: &'a Service,
    page: PageNo,
    content: &'a Content,
    cells: Vec<Vec<Cell>>,
    fg_mark: Vec<[bool; COLS]>,
    bg_mark: Vec<[bool; COLS]>,
}

impl Ctx<'_> {
    fn assets(&self) -> &Assets {
        &self.svc.assets
    }

    /// A glyph by DRCS triplet data: bit 6 set = the page's own (normal) DRCS page,
    /// clear = the shared GDRCS glyphs.
    fn drcs(&self, data: u8, sub: u8) -> Option<[u16; GLYPH_H]> {
        let n = data & 0x3F;
        if data & 0x40 == 0 {
            let g = self.assets().glyphs.get(n as usize)?;
            let ptu = g.ptu().ok()?;
            return drcs_from_rows(&[ptu.to_vec()], 0);
        }
        // Normal DRCS: the page named when this page was PUT (`drcs_page`, which goes
        // out in the MOT), or else the one its X/27/4 link names (DRCS link, slot 3).
        let named = self.svc.pages.get(&self.page).and_then(|p| p.drcs_page);
        let no = match named {
            Some(no) => no,
            None => {
                let (_, _, raw) = self
                    .content
                    .extra
                    .iter()
                    .find(|(p, d, _)| (*p, *d) == (27, 4))?;
                let w = words(raw);
                let link = w.get(6).copied()?; // slot 3: triplets 6 (page) and 7 (subcode)
                // libzvbi's reading: units in bits 7-10, tens in bits 15-17.
                let num = ((link >> 7) & 15) as u8 | (((link >> 15) & 7) as u8) << 4;
                PageNo::new(self.page.mag, num)
            }
        };
        let drcs_page = self.svc.pages.get(&no)?;
        let content = if drcs_page.subpages.len() > 1 {
            drcs_page.subpages.get((sub as usize).checked_sub(1)?)?
        } else {
            drcs_page.subpages.first()?
        };
        drcs_from_rows(&content.rows, n)
    }

    fn forward(&mut self, r: usize, c: usize, fg: bool, set: impl Fn(&mut Cell)) {
        for k in c..COLS {
            let stop = if fg {
                self.fg_mark[r][k]
            } else {
                self.bg_mark[r][k]
            };
            if k > c && stop {
                break;
            }
            set(&mut self.cells[r][k]);
        }
    }

    /// Apply X/26 triplets with their origin at (row0, col0): 0 for the page's own,
    /// the invocation point for an object. `depth` stops objects invoking themselves.
    fn enhance(&mut self, triplets: &[Triplet], row0: usize, col0: usize, depth: u8) {
        let (mut row, mut col) = (row0, col0);
        let mut drcs_sub = [0u8; 2];
        let row_of = |a: u8| if a == 40 { 24 } else { a as usize - 40 };
        for &(a, m, d) in triplets {
            if a >= 40 {
                match m {
                    0x01 | 0x04 => {
                        row = row0 + row_of(a);
                        col = if m == 0x04 && (d as usize) < COLS {
                            col0 + d as usize
                        } else {
                            col0
                        };
                    }
                    0x07 => (row, col) = (0, 0),
                    0x11..=0x13 if depth < 2 => {
                        if let Some(o) = self.object((a, m, d)) {
                            self.enhance(&o, row, col, depth + 1);
                        }
                    }
                    0x15..=0x17 if depth > 0 => return, // the next object's definition
                    0x18 => drcs_sub[(d >> 6) as usize & 1] = d & 15,
                    0x1F => return,
                    _ => {}
                }
                continue;
            }
            let c = col0 + a as usize;
            col = c;
            if row >= ROWS || c >= COLS {
                continue;
            }
            match m {
                0x00 if d >> 5 == 0 => self.forward(row, c, true, |cell| cell.fg = d & 31),
                0x03 if d >> 5 == 0 => self.forward(row, c, false, |cell| cell.bg = d & 31),
                0x01 if d & 0x20 != 0 => {
                    let cell = &mut self.cells[row][c];
                    (cell.ch, cell.drcs, cell.g3) = (' ', None, false);
                    cell.mosaic = Some(d & 0x1F | (d & 0x40) >> 1);
                }
                0x02 | 0x0B => {
                    let cell = &mut self.cells[row][c];
                    (cell.ch, cell.mosaic, cell.drcs, cell.g3) = (' ', None, None, true);
                }
                0x09 if d >= 0x20 => self.set_char(row, c, g0(d, false)),
                0x0C => {
                    let r = row;
                    self.forward(row, c, true, |cell| {
                        cell.dh = d & 1 != 0 && r > 0 && r < 23;
                        cell.boxed = d & 2 != 0;
                        cell.conceal = d & 4 != 0;
                        cell.dw = d & 0x40 != 0;
                        if d & 8 != 0 {
                            std::mem::swap(&mut cell.fg, &mut cell.bg);
                        }
                    });
                }
                0x0D => {
                    let glyph = self.drcs(d, drcs_sub[(d >> 6) as usize & 1]);
                    let cell = &mut self.cells[row][c];
                    (cell.ch, cell.mosaic, cell.g3) = (' ', None, false);
                    cell.drcs = glyph;
                }
                0x0F if d >= 0x20 => self.set_char(row, c, g2(d)),
                0x10..=0x1F if d >= 0x20 => self.set_char(row, c, g0(d, false)),
                _ => {}
            }
        }
        let _ = col;
    }

    fn set_char(&mut self, r: usize, c: usize, ch: char) {
        let cell = &mut self.cells[r][c];
        (cell.ch, cell.mosaic, cell.drcs, cell.g3) = (ch, None, None, false);
    }

    /// A shared object's triplets by its invocation triplet.
    fn object(&self, invoke: Triplet) -> Option<Vec<Triplet>> {
        let layout = self.assets().validate().ok()?;
        let name = layout.objects.iter().find(|(_, t)| **t == invoke)?.0;
        Some(
            self.assets()
                .objects
                .iter()
                .find(|o| &o.name == name)?
                .x26
                .clone(),
        )
    }
}

/// Compose and draw subpage `sub` of `no` as RGB pixels (WIDTH x HEIGHT).
pub fn render(
    svc: &Service,
    no: PageNo,
    sub: usize,
    level: Level,
    row0: &[u8; 40],
) -> Option<Vec<u8>> {
    let stored = svc.pages.get(&no)?;
    let content = stored.subpages.get(sub)?;
    let x28 = content
        .extra
        .iter()
        .find(|(p, d, _)| (*p, *d) == (28, 0))
        .map(|(_, _, b)| words(b));
    let fmt = match level {
        Level::L25 => format(x28.or_else(|| {
            let p = svc.assets.palette.as_ref()?;
            Some(
                p.triplets()
                    .ok()?
                    .iter()
                    .map(|&(a, m, d)| a as u32 | (m as u32) << 6 | (d as u32) << 11)
                    .collect(),
            )
        })),
        _ => format(None),
    };
    let x26: Vec<Triplet> = if level >= Level::L15 {
        merge_x26(&content.x26, &content.g2)
    } else {
        vec![]
    };
    let x26: Vec<Triplet> = if level < Level::L25 {
        // Level 1.5: characters only.
        x26.into_iter()
            .filter(|&(a, m, _)| a >= 40 || matches!(m, 0x02 | 0x09 | 0x0F | 0x10..=0x1F))
            .collect()
    } else {
        x26
    };

    // Full screen and full row colours.
    let mut screen = fmt.screen;
    let mut row_colour = [fmt.row; ROWS];
    {
        let mut row = 0;
        for &(a, m, d) in &x26 {
            if a < 40 {
                continue;
            }
            if m == 0x1F {
                break;
            }
            if m == 0x00 && d >> 5 == 0 {
                screen = d & 31;
            }
            if m == 0x01 || m == 0x04 {
                row = if a == 40 { 24 } else { a as usize - 40 };
            }
            if m == 0x01 {
                let to = if d >> 5 == 3 { ROWS } else { row + 1 };
                for rc in row_colour.iter_mut().take(to).skip(row) {
                    *rc = d & 31;
                }
            }
        }
    }

    let mut ctx = Ctx {
        svc,
        page: no,
        content,
        cells: vec![],
        fg_mark: vec![[false; COLS]; ROWS],
        bg_mark: vec![[false; COLS]; ROWS],
    };
    let mut skip_next = false;
    #[allow(clippy::needless_range_loop)] // r also indexes cells and marks
    for r in 0..ROWS {
        let fg_col = |c: u8| fmt.fg_table * 8 + c;
        let bg_col = |c: u8| {
            if c == 0 && fmt.black_bg_sub {
                row_colour[r]
            } else {
                fmt.bg_table * 8 + c
            }
        };
        if skip_next {
            skip_next = false;
            let above = ctx.cells[r - 1].clone();
            ctx.cells.push(
                above
                    .iter()
                    .map(|a| Cell {
                        boxed: a.boxed,
                        covered: a.dh,
                        ..blank(a.bg)
                    })
                    .collect(),
            );
            continue;
        }
        let bytes: &[u8] = if r == 0 { row0 } else { &content.rows[r - 1] };
        let (mut fg, mut bg, mut mosaics, mut separated, mut hold) =
            (7u8, 0u8, false, false, false);
        let (mut held, mut held_sep) = (0x20u8, false);
        let (mut conceal, mut boxed, mut dh, mut dw, mut any_double) =
            (false, false, false, false, false);
        let mut row = vec![];
        for (c, &b) in bytes.iter().take(COLS).enumerate() {
            let b = b & 0x7F;
            if b < 0x20 {
                match b {
                    0x0C => {
                        if dh || dw {
                            held = 0x20;
                        }
                        (dh, dw) = (false, false);
                    }
                    0x18 => conceal = true,
                    0x19 => separated = false,
                    0x1A => separated = true,
                    0x1C => {
                        bg = 0;
                        ctx.bg_mark[r][c] = true;
                    }
                    0x1D => {
                        bg = fg;
                        ctx.bg_mark[r][c] = true;
                    }
                    0x1E => hold = true,
                    _ => {}
                }
            }
            let mut cell = blank(bg_col(bg));
            cell.fg = fg_col(fg);
            cell.conceal = conceal;
            cell.boxed = boxed;
            cell.dh = dh && r > 0 && r < 23;
            cell.dw = dw && c < COLS - 1;
            if b < 0x20 {
                if hold && mosaics {
                    cell.mosaic = Some(held);
                    cell.separated = held_sep;
                }
            } else if mosaics && b & 0x20 != 0 {
                cell.mosaic = Some(b);
                cell.separated = separated;
                held = b;
                held_sep = separated;
            } else {
                cell.ch = g0(b, true);
            }
            if let Some(m) = cell.mosaic {
                cell.mosaic = Some(m & 0x1F | (m & 0x40) >> 1);
            }
            any_double |= cell.dh;
            row.push(cell);
            if b < 0x20 {
                match b {
                    0x00..=0x07 | 0x10..=0x17 => {
                        let to_mosaics = b >= 0x10;
                        if to_mosaics != mosaics {
                            held = 0x20;
                        }
                        mosaics = to_mosaics;
                        fg = b & 7;
                        conceal = false;
                        if c + 1 < COLS {
                            ctx.fg_mark[r][c + 1] = true;
                        }
                    }
                    0x0A => boxed = false,
                    0x0B => boxed = true,
                    0x0D..=0x0F => {
                        let (ndh, ndw) = (b != 0x0E, b != 0x0D);
                        if ndh != dh || ndw != dw {
                            held = 0x20;
                        }
                        (dh, dw) = (ndh, ndw);
                    }
                    0x1F => hold = false,
                    _ => {}
                }
            }
        }
        if any_double {
            skip_next = true;
        }
        ctx.cells.push(row);
    }
    ctx.enhance(&x26, 0, 0, 0);

    for r in 0..ROWS {
        for c in 0..COLS {
            let (cdw, cdh, cov) = {
                let cell = &ctx.cells[r][c];
                (cell.dw, cell.dh, cell.covered)
            };
            if cov {
                continue;
            }
            if cdw && c + 1 < COLS {
                ctx.cells[r][c + 1].covered = true;
            }
            if cdh && r + 1 < ROWS {
                ctx.cells[r + 1][c].covered = true;
                if cdw && c + 1 < COLS {
                    ctx.cells[r + 1][c + 1].covered = true;
                }
            }
        }
    }

    Some(draw(&ctx.cells, &fmt.palette, screen))
}

fn glyph_index(ch: char) -> Option<usize> {
    FONT_CHARS.chars().position(|c| c == ch)
}

fn draw(cells: &[Vec<Cell>], palette: &[[u8; 3]; 32], screen: u8) -> Vec<u8> {
    let colour = |i: u8| {
        if i == TRANSPARENT {
            palette[screen as usize & 31]
        } else {
            palette[i as usize & 31]
        }
    };
    let mut out = vec![0u8; WIDTH * HEIGHT * 3];
    for (r, row) in cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if cell.covered {
                continue;
            }
            let (sx, sy) = (1 + cell.dw as usize, 1 + cell.dh as usize);
            let (fg, bg) = (colour(cell.fg), colour(cell.bg));
            let g = if cell.mosaic.is_none()
                && cell.drcs.is_none()
                && cell.ch != ' '
                && !cell.conceal
            {
                glyph_index(cell.ch).or_else(|| glyph_index('?'))
            } else {
                None
            };
            for y in 0..CELL_H * sy {
                let py = r * CELL_H + y;
                if py >= HEIGHT {
                    break;
                }
                let gy = y / sy;
                for x in 0..CELL_W * sx {
                    let px = c * CELL_W + x;
                    if px >= WIDTH {
                        break;
                    }
                    let gx = x / sx;
                    let on = if cell.conceal {
                        false
                    } else if let Some(m) = cell.mosaic {
                        mosaic_on(m, cell.separated, gx, gy)
                    } else if let Some(d) = cell.drcs {
                        (d[gy / 2] >> (11 - gx)) & 1 != 0
                    } else if cell.g3 {
                        (gx + gy) % 4 == 0
                    } else if let Some(g) = g {
                        let bit = gy * CELL_W + gx;
                        (FONT_DATA[g * GLYPH_BYTES + bit / 8] >> (bit % 8)) & 1 != 0
                    } else {
                        false
                    };
                    let i = (py * WIDTH + px) * 3;
                    out[i..i + 3].copy_from_slice(if on { &fg } else { &bg });
                }
            }
        }
    }
    out
}

fn mosaic_on(pattern: u8, separated: bool, x: usize, y: usize) -> bool {
    let bx = usize::from(x >= 6);
    let by = if y < 6 {
        0
    } else if y < 14 {
        1
    } else {
        2
    };
    if (pattern >> (by * 2 + bx)) & 1 == 0 {
        return false;
    }
    if !separated {
        return true;
    }
    let y_end = [6, 14, 20][by];
    x - bx * 6 >= 2 && y < y_end - 2
}

/// Encode RGB pixels as PNG, `scale_x` times wider (a teletext page is shown about
/// 4:3, so 480x500 pixels look right drawn 4/3 wider: scale 4 of 3 is approximated as
/// doubling both and is left to the caller).
pub fn png(rgb: &[u8], scale: usize) -> Vec<u8> {
    let (w, h) = (WIDTH * scale, HEIGHT * scale);
    let mut big = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let i = ((y / scale) * WIDTH + x / scale) * 3;
            big.extend_from_slice(&rgb[i..i + 3]);
        }
    }
    let mut out = vec![];
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(&big).expect("png data");
    }
    out
}

/// Header row as a decoder shows it: page number in columns 0-7, then ttxd's header.
pub fn header_row(no: PageNo, text: &[u8; 32]) -> [u8; 40] {
    let mut r = [b' '; 40];
    let p = format!("P{no}");
    r[1..1 + p.len()].copy_from_slice(p.as_bytes());
    r[8..].copy_from_slice(text);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{Glyph, Object, ObjectType, Palette};
    use crate::service::StoredPage;
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    fn px(rgb: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * WIDTH + x) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    }

    fn svc_with(c: Content) -> Service {
        let mut s = Service::new("FRIKANALEN", BTreeMap::new());
        s.pages.insert(
            PageNo::new(2, 0),
            StoredPage::new("t", vec![c], 10, None, SystemTime::now()),
        );
        s
    }

    #[test]
    fn level_1_text_and_mosaics() {
        let c = Content::from_markup(&["{yellow}Hei".into(), "{gred}{x:7F}".into()]).unwrap();
        let s = svc_with(c);
        let rgb = render(&s, PageNo::new(2, 0), 0, Level::L1, &[b' '; 40]).unwrap();
        // Row 2, column 1: a full red block.
        assert_eq!(px(&rgb, 12 + 6, 2 * CELL_H + 10), [255, 0, 0]);
        // Row 1, column 0 is the attribute: black.
        assert_eq!(px(&rgb, 6, CELL_H + 10), [0, 0, 0]);
        // Some yellow pixel in "H" at row 1, column 1.
        let yellow =
            (0..CELL_W).any(|x| (0..CELL_H).any(|y| px(&rgb, 12 + x, CELL_H + y) == [255, 255, 0]));
        assert!(yellow);
    }

    #[test]
    fn objects_glyphs_and_magazine_palette_at_level_25() {
        let mut c = Content::from_markup(&["Logo".into()]).unwrap();
        c.set_x26(vec![(41, 0x04, 10), (0x38, 0x12, 0x00)]).unwrap();
        let mut s = svc_with(c);
        s.set_assets(Assets {
            palette: Some(Palette {
                colours: vec!["#000000".into(), "#ff00ff".into()],
                screen: 0,
                row: 0,
                remap: 0,
                black_bg_substitution: false,
            }),
            glyphs: vec![Glyph {
                name: "full".into(),
                rows: vec!["#".repeat(12); 10],
            }],
            objects: vec![Object {
                name: "logo".into(),
                kind: ObjectType::Adaptive,
                x26: vec![(0, 0x00, 17), (0, 0x0D, 0)],
            }],
        });
        let no = PageNo::new(2, 0);
        let rgb = render(&s, no, 0, Level::L25, &[b' '; 40]).unwrap();
        assert_eq!(px(&rgb, 10 * CELL_W + 5, CELL_H + 5), [255, 0, 255]);
        let rgb = render(&s, no, 0, Level::L1, &[b' '; 40]).unwrap();
        assert_eq!(px(&rgb, 10 * CELL_W + 5, CELL_H + 5), [0, 0, 0]);
        assert!(png(&rgb, 1).starts_with(b"\x89PNG"));
    }

    #[test]
    fn own_drcs_page_is_the_one_the_page_names() {
        let mut c = Content::from_markup(&["{red}Logo".into()]).unwrap();
        c.set_x26(vec![(41, 0x04, 0), (10, 0x0D, 0x40)]).unwrap();
        let mut s = svc_with(c);
        let (no, drcs) = (PageNo::new(2, 0), PageNo::new(2, 0x0F));
        // One tile, every pixel lit.
        let tiles = Content {
            rows: vec![vec![0x7F; 40]],
            ..Default::default()
        };
        s.pages.insert(
            drcs,
            StoredPage::new("t", vec![tiles], 10, None, SystemTime::now()),
        );
        let at = |s: &Service| {
            let rgb = render(s, no, 0, Level::L25, &[b' '; 40]).unwrap();
            px(&rgb, 10 * CELL_W + 5, CELL_H + 5)
        };
        assert_eq!(at(&s), [0, 0, 0]);
        s.pages.get_mut(&no).unwrap().drcs_page = Some(drcs);
        assert_eq!(at(&s), [255, 0, 0]);
    }
}
