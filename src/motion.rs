//! Server-side motion: ticker rows that scroll, and rows that step through animation
//! frames. ttxd drives both from its frame clock and sends each step as a row update
//! (header plus the changed rows, no erase), paid for from the page owner's flash
//! budget like any other flash. Over budget, steps are delayed, never dropped.

use crate::page::{COLS, parse_cells};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

/// Fastest motion: one step per 4 frames (12.5 steps/s).
pub const MIN_STEP: Duration = Duration::from_millis(80);
/// Spaces between the end of a ticker's text and its start coming round again.
const GAP: usize = 4;

/// A row whose text scrolls right to left. `prefix` stays put at the left (e.g. a
/// label or colour attributes); the text scrolls in the rest of the row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticker {
    pub row: u8,
    pub text: String,
    #[serde(default)]
    pub prefix: String,
    /// Cells per second, 1-12.
    #[serde(default = "default_speed")]
    pub cells_per_sec: f64,
}

fn default_speed() -> f64 {
    4.0
}

/// Rows that cycle through frames: each frame maps row numbers ("1"-"24") to markup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    pub frames: Vec<BTreeMap<String, String>>,
    /// Milliseconds per frame, at least 80.
    pub frame_ms: u64,
}

/// What a page's motion needs to remember between steps.
#[derive(Clone, Debug, Default)]
pub struct MotionState {
    tickers: Vec<(Option<SystemTime>, usize)>,
    animation: (Option<SystemTime>, usize),
}

/// One row as codes plus (column, G2) substitutions.
pub type RowCodes = (Vec<u8>, Vec<(u8, u8)>);

impl Ticker {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=24).contains(&self.row) {
            return Err(format!("ticker row {} out of range 1..=24", self.row));
        }
        if !(1.0..=12.0).contains(&self.cells_per_sec) {
            return Err("ticker speed is 1-12 cells per second".into());
        }
        let (p, _) = parse_cells(&self.prefix)?;
        if p.len() >= COLS - 4 {
            return Err("ticker prefix leaves no room to scroll".into());
        }
        let (t, _) = parse_cells(&self.text)?;
        if t.is_empty() {
            return Err("ticker text is empty".into());
        }
        Ok(())
    }

    fn step(&self) -> Duration {
        Duration::from_secs_f64(1.0 / self.cells_per_sec).max(MIN_STEP)
    }

    /// The row when the text has scrolled `offset` cells.
    pub fn row_at(&self, offset: usize) -> RowCodes {
        let (mut codes, mut g2) = parse_cells(&self.prefix).unwrap_or_default();
        let (text, text_g2) = parse_cells(&self.text).unwrap_or_default();
        let width = COLS - codes.len();
        if text.len() <= width {
            // It fits: no need to scroll.
            let start = codes.len() as u8;
            codes.extend(&text);
            g2.extend(text_g2.iter().map(|&(c, g)| (c + start, g)));
            codes.resize(COLS, b' ');
            return (codes, g2);
        }
        let n = text.len() + GAP;
        for i in 0..width {
            let k = (offset + i) % n;
            let col = codes.len() as u8;
            if k < text.len() {
                if let Some(&(_, g)) = text_g2.iter().find(|&&(c, _)| c as usize == k) {
                    g2.push((col, g));
                }
                codes.push(text[k]);
            } else {
                codes.push(b' ');
            }
        }
        (codes, g2)
    }
}

impl Animation {
    pub fn validate(&self) -> Result<(), String> {
        if self.frames.is_empty() || self.frames.len() > 100 {
            return Err("an animation has 1-100 frames".into());
        }
        if self.frame_ms < MIN_STEP.as_millis() as u64 {
            return Err(format!("frame_ms is at least {}", MIN_STEP.as_millis()));
        }
        for f in &self.frames {
            for (row, markup) in f {
                let r: u8 = row.parse().map_err(|_| format!("bad row number {row:?}"))?;
                if !(1..=24).contains(&r) {
                    return Err(format!("animation row {r} out of range 1..=24"));
                }
                let (codes, _) = parse_cells(markup)?;
                if codes.len() > COLS {
                    return Err(format!("animation row {r} is longer than {COLS} cells"));
                }
            }
        }
        Ok(())
    }
}

/// Advance a page's motion to `now`. Returns the rows to change (empty if no step is
/// due), as markup-free codes.
pub fn advance(
    tickers: &[Ticker],
    animation: Option<&Animation>,
    st: &mut MotionState,
    now: SystemTime,
) -> BTreeMap<u8, RowCodes> {
    let mut out = BTreeMap::new();
    st.tickers.resize(tickers.len(), (None, 0));
    let mut due = false;
    for (t, (next, offset)) in tickers.iter().zip(st.tickers.iter_mut()) {
        match next {
            Some(n) if now < *n => {}
            Some(n) => {
                *offset += 1;
                // Don't race to catch up after a stall.
                *next = Some((*n + t.step()).max(now));
                due = true;
            }
            None => {
                *next = Some(now + t.step());
                due = true;
            }
        }
    }
    if let Some(a) = animation {
        let (next, frame) = &mut st.animation;
        let step = Duration::from_millis(a.frame_ms);
        match next {
            Some(n) if now < *n => {}
            Some(n) => {
                *frame = (*frame + 1) % a.frames.len();
                *next = Some((*n + step).max(now));
                due = true;
            }
            None => {
                *next = Some(now + step);
                due = true;
            }
        }
    }
    if !due {
        return out;
    }
    // Every motion row goes in each step, so a step that replaces a queued one loses
    // nothing.
    if let Some(a) = animation {
        for (row, markup) in &a.frames[st.animation.1 % a.frames.len()] {
            if let (Ok(r), Ok((mut codes, g2))) = (row.parse::<u8>(), parse_cells(markup)) {
                codes.resize(COLS, b' ');
                out.insert(r, (codes, g2));
            }
        }
    }
    for (t, (_, offset)) in tickers.iter().zip(&st.tickers) {
        out.insert(t.row, t.row_at(*offset));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(r: &RowCodes) -> String {
        r.0.iter().map(|&b| b as char).collect::<String>()
    }

    #[test]
    fn ticker_scrolls_and_wraps_with_a_gap() {
        let t = Ticker {
            row: 24,
            text: "x".repeat(30) + "ABCDEFGHIJ",
            prefix: "{yellow}NYTT ".into(),
            cells_per_sec: 4.0,
        };
        t.validate().unwrap();
        let r0 = text(&t.row_at(0));
        assert!(r0.starts_with("\x03NYTT xxxx"));
        assert_eq!(r0.len(), 40);
        let r1 = text(&t.row_at(1));
        assert_eq!(&r1[6..39], &r0[7..40]);
        // After text + gap cells it is back where it started.
        assert_eq!(text(&t.row_at(40 + GAP)), r0);
    }

    #[test]
    fn short_ticker_stands_still_and_keeps_g2() {
        let t = Ticker {
            row: 1,
            text: "Ørsta".into(),
            prefix: String::new(),
            cells_per_sec: 2.0,
        };
        let r = t.row_at(5);
        assert_eq!(r.1, vec![(0, 0x69)]);
        assert_eq!(r.0.len(), 40);
    }

    #[test]
    fn long_ticker_carries_g2_along() {
        let t = Ticker {
            row: 1,
            text: format!("{}Æ", "a".repeat(45)),
            prefix: String::new(),
            cells_per_sec: 2.0,
        };
        assert!(t.row_at(0).1.is_empty());
        assert_eq!(t.row_at(10).1, vec![(35, 0x61)]);
    }

    #[test]
    fn advance_steps_on_time() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let tickers = vec![Ticker {
            row: 3,
            text: "y".repeat(50),
            prefix: String::new(),
            cells_per_sec: 4.0,
        }];
        let anim = Animation {
            frames: vec![
                BTreeMap::from([("5".into(), "A".into())]),
                BTreeMap::from([("5".into(), "B".into())]),
            ],
            frame_ms: 500,
        };
        let mut st = MotionState::default();
        let first = advance(&tickers, Some(&anim), &mut st, t0);
        assert_eq!(first.keys().copied().collect::<Vec<_>>(), vec![3, 5]);
        assert_eq!(first[&5].0[0], b'A');
        assert!(
            advance(
                &tickers,
                Some(&anim),
                &mut st,
                t0 + Duration::from_millis(100)
            )
            .is_empty()
        );
        let s = advance(
            &tickers,
            Some(&anim),
            &mut st,
            t0 + Duration::from_millis(250),
        );
        assert_eq!(s[&5].0[0], b'A'); // ticker stepped, frame not yet
        let s = advance(
            &tickers,
            Some(&anim),
            &mut st,
            t0 + Duration::from_millis(500),
        );
        assert_eq!(s[&5].0[0], b'B');
    }

    #[test]
    fn validation() {
        let mut t = Ticker {
            row: 25,
            text: "a".into(),
            prefix: String::new(),
            cells_per_sec: 4.0,
        };
        assert!(t.validate().is_err());
        t.row = 1;
        t.cells_per_sec = 50.0;
        assert!(t.validate().is_err());
        let a = Animation {
            frames: vec![],
            frame_ms: 500,
        };
        assert!(a.validate().is_err());
        let a = Animation {
            frames: vec![BTreeMap::from([("1".into(), "x".into())])],
            frame_ms: 10,
        };
        assert!(a.validate().is_err());
    }
}
