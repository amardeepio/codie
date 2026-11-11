//! Interactive terminal interface for codie.
//!
//! Phase 2 provides an alternate-screen transcript, a fixed multiline editor,
//! streaming assistant rendering, tool blocks, a footer, themes, slash
//! commands, steering/follow-up queues, and permission prompts. Plain mode is
//! an append-only fallback for screen readers and non-interactive terminals.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod app;
pub mod commands;
pub mod editor;
pub mod keybindings;
pub mod markdown;
pub mod permissions;
pub mod plain;
pub mod render;
pub mod theme;
pub mod transcript;

pub use app::{App, TuiOptions, TuiRunMode, run};
pub use render::draw;
pub use theme::{Theme, ThemeWatcher};
