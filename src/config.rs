use crate::page::PageNo;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    /// HTTP API, e.g. "0.0.0.0:8080".
    pub listen: String,
    /// Raw T42 packet stream for debugging (42-byte records), e.g. "127.0.0.1:4242".
    pub t42_listen: Option<String>,
    /// Public read-only feed for the web viewer (GET /teletext/feed), e.g. "0.0.0.0:8081".
    pub public_listen: Option<String>,
    /// Most viewers the public feed serves at once.
    #[serde(default = "default_feed_clients")]
    pub public_max_clients: usize,
    /// Where pages are kept across restarts.
    pub snapshot: PathBuf,
    #[serde(default = "default_name")]
    pub service_name: String,
    /// Page a decoder shows first; sent in packet 8/30.
    #[serde(default = "default_initial_page")]
    pub initial_page: PageNo,
    /// Network identification code in packet 8/30 (0 = none registered).
    #[serde(default)]
    pub network_id: u16,
    /// Where ttx-inject connects for frames, e.g. "0.0.0.0:4243". Without it, or while no
    /// injector is connected, a 50 Hz timer stands in for the frame clock.
    pub inject_listen: Option<String>,
    #[serde(default, rename = "publisher")]
    pub publishers: Vec<Publisher>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Publisher {
    pub name: String,
    #[serde(default)]
    pub token: String,
    /// Read the token from this environment variable instead (e.g. from a Kubernetes Secret).
    #[serde(default)]
    pub token_env: Option<String>,
    /// Page ranges this publisher may write, e.g. ["200-29F", "888"].
    #[serde(default)]
    pub pages: Vec<String>,
    #[serde(default)]
    pub subtitles: bool,
    /// May set the 8/30 status display (e.g. to the programme on air).
    #[serde(default)]
    pub status_display: bool,
    /// Flash budget in packets per second (a frame carries 5, so 250/s is the whole line).
    #[serde(default = "default_flash")]
    pub flash_per_sec: f64,
}

fn default_feed_clients() -> usize {
    1000
}
fn default_name() -> String {
    "FRIKANALEN".into()
}
fn default_initial_page() -> PageNo {
    PageNo::new(1, 0x00)
}
fn default_flash() -> f64 {
    100.0
}

impl Config {
    /// Fill in tokens given by `token_env`, and refuse publishers without a token.
    pub fn resolve_tokens(&mut self) -> anyhow::Result<()> {
        for p in &mut self.publishers {
            if let Some(var) = &p.token_env {
                p.token = std::env::var(var)
                    .map_err(|_| anyhow::anyhow!("publisher {}: {var} is not set", p.name))?;
            }
            if p.token.is_empty() {
                anyhow::bail!("publisher {} has no token", p.name);
            }
        }
        Ok(())
    }
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
            token_env: None,
            pages: vec!["200-29F".into(), "888".into()],
            subtitles: false,
            status_display: false,
            flash_per_sec: 1.0,
        };
        assert!(p.may_write("200".parse().unwrap()));
        assert!(p.may_write("250".parse().unwrap()));
        assert!(p.may_write("888".parse().unwrap()));
        assert!(!p.may_write("300".parse().unwrap()));
        assert!(!p.may_write("100".parse().unwrap()));
    }
}
