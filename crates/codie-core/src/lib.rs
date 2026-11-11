//! Core types shared by every codie crate.
//!
//! `codie-core` deliberately contains no I/O, HTTP, terminal, or filesystem
//! code. It defines the vocabulary — messages, events, errors, and the stable
//! names of configuration locations — that the agent loop, providers, tools,
//! and interfaces agree on.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod error;

pub use error::{Error, ErrorKind, Result};

/// The codie version compiled into this build.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
