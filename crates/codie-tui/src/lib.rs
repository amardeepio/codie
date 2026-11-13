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
pub mod compaction;
pub mod editor;
pub mod keybindings;
pub mod markdown;
pub mod permissions;
pub mod plain;
pub mod render;
pub mod theme;
mod tool_view;
pub mod transcript;

pub use app::{App, TuiOptions, TuiRunMode, run};
pub use render::draw;
pub use theme::{Theme, ThemeWatcher};

/// Copies text to the system clipboard.
///
/// macOS uses `pbcopy`; Linux tries `wl-copy` and then `xclip`. Other platforms
/// return an unsupported error.
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    let candidates: &[&[&str]] = &[&["pbcopy"]];
    #[cfg(target_os = "linux")]
    let candidates: &[&[&str]] = &[&["wl-copy"], &["xclip", "-selection", "clipboard"]];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let candidates: &[&[&str]] = &[];

    let mut last_error = std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no clipboard command available",
    );

    for command in candidates {
        let mut child = match Command::new(command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        if let Some(mut stdin) = child.stdin.take()
            && let Err(error) = stdin.write_all(text.as_bytes())
        {
            last_error = error;
            let _ = child.kill();
            continue;
        }
        match child.wait() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last_error = std::io::Error::other(format!("{} exited with {status}", command[0]));
            }
            Err(error) => last_error = error,
        }
    }

    Err(last_error)
}
