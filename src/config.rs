use crate::page::PageNo;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// HTTP API, e.g. "0.0.0.0:8080".
    pub listen: String,
    /// Raw T42 packet stream for debugging (42-byte records), e.g. "127.0.0.1:4242".
    pub t42_listen: Option<String>,
    /// Where pages are kept across restarts.
    pub snapshot: PathBuf,
    #[serde(default = "default_name")]
    pub service_name: String,
    /// Line numbers put in the five OP-47 descriptors.
    #[serde(default = "default_lines")]
    pub op47_lines: [u8; 5],
    /// DeckLink device to output on, e.g. "DeckLink Duo (2)". Without it a 50 Hz timer
    /// stands in for the frame clock (needs the `decklink` build feature).
    pub decklink_device: Option<String>,
    /// VANC line carrying the OP-47 packet.
    #[serde(default = "default_vanc_line")]
    pub vanc_line: u32,
    #[serde(default, rename = "publisher")]
    pub publishers: Vec<Publisher>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Publisher {
    pub name: String,
    pub token: String,
    /// Page ranges this publisher may write, e.g. ["200-29F", "888"].
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub subtitles: bool,
    /// Flash budget in packets per second (a frame carries 5, so 250/s is the whole line).
    #[serde(default = "default_flash")]
    pub flash_per_sec: f64,
}

fn default_name() -> String {
    "FRIKANALEN".into()
}
fn default_lines() -> [u8; 5] {
    [7, 8, 9, 10, 11]
}
fn default_vanc_line() -> u32 {
    9
}
fn default_flash() -> f64 {
    100.0
}

impl Publisher {
    pub fn may_write(&self, page: PageNo) -> bool {
        self.pages.iter().any(|r| {
            let (a, b) = r.split_once('-').unwrap_or((r, r));
            match (a.parse::<PageNo>(), b.parse::<PageNo>()) {
                (Ok(a), Ok(b)) => (a..=b).contains(&page),
                _ => false,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges() {
        let p = Publisher {
            name: "s".into(),
            token: "t".into(),
            pages: vec!["200-29F".into(), "888".into()],
            subtitles: false,
            flash_per_sec: 1.0,
        };
        assert!(p.may_write("200".parse().unwrap()));
        assert!(p.may_write("250".parse().unwrap()));
        assert!(p.may_write("888".parse().unwrap()));
        assert!(!p.may_write("300".parse().unwrap()));
        assert!(!p.may_write("100".parse().unwrap()));
    }
}
