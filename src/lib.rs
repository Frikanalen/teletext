//! Frikanalen's teletext service.
//!
//! `ttxd` keeps the page store, schedules packets frame by frame and serves them over
//! [`link`] to `ttx-inject`, which puts them on SDI as OP-47.

pub mod api;
pub mod assets;
pub mod charset;
pub mod config;
#[cfg(feature = "decklink")]
pub mod decklink;
pub mod enhance;
pub mod feed;
pub mod ham;
pub mod link;
pub mod op47;
pub mod p830;
pub mod page;
pub mod render;
pub mod service;
