//! Interactive terminal interface for codie.
//!
//! Phase 2 provides an alternate-screen transcript, a fixed multiline editor,
//! streaming assistant rendering, tool blocks, a footer, themes, slash
//! commands, steering/follow-up queues, and permission prompts. Plain mode is
//! an append-only fallback for screen readers and non-interactive terminals.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod app;
pub mod clipboard;
pub mod commands;
pub mod compaction;
pub mod editor;
pub mod keybindings;
pub mod markdown;
pub mod permissions;
pub mod plain;
mod questions;
pub mod render;
mod subagents;
pub mod theme;
mod tool_view;
pub mod transcript;

pub use app::{App, TuiOptions, TuiRunMode, run};
pub use render::draw;
pub use theme::{Theme, ThemeWatcher};

/// How long a clipboard helper may take before codie stops waiting.
///
/// A stalled helper must never block the caller forever; `copy_to_clipboard`
/// runs on a blocking thread, but it still has to return so the notice can be
/// shown and the child process can be cleaned up.
const CLIPBOARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Copies text to the system clipboard.
///
/// macOS uses `pbcopy`, Linux tries `wl-copy` and then `xclip`, and Windows
/// uses `clip`. Other platforms return an unsupported error.
///
/// The call blocks, so it must not run on the render or event-loop thread;
/// the TUI dispatches it to a blocking task. Every helper is killed after two
/// seconds, including one that never reads its standard input.
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[&[&str]] = &[&["pbcopy"]];
    #[cfg(target_os = "linux")]
    const CANDIDATES: &[&[&str]] = &[&["wl-copy"], &["xclip", "-selection", "clipboard"]];
    #[cfg(target_os = "windows")]
    const CANDIDATES: &[&[&str]] = &[&["clip"]];
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    const CANDIDATES: &[&[&str]] = &[];

    copy_with(text, CANDIDATES, CLIPBOARD_TIMEOUT)
}

/// Copies `text` through the first candidate command that succeeds, killing a
/// helper that exceeds `timeout`.
fn copy_with(
    text: &str,
    candidates: &[&[&str]],
    timeout: std::time::Duration,
) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Instant;

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
        if let Some(mut stdin) = child.stdin.take() {
            // Writing can block when the helper stops draining the pipe, so
            // feed it from a thread and give up on the same deadline.
            let bytes = text.as_bytes().to_vec();
            let (written, write_result) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = written.send(stdin.write_all(&bytes));
            });
            match write_result.recv_timeout(timeout) {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    last_error = error;
                    let _ = child.kill();
                    let _ = child.wait();
                    continue;
                }
                Err(_) => {
                    last_error = std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("{} did not read the clipboard data in time", command[0]),
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    continue;
                }
            }
        }
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    last_error =
                        std::io::Error::other(format!("{} exited with {status}", command[0]));
                    break;
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(None) => {
                    last_error = std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("{} did not exit in time", command[0]),
                    );
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                Err(error) => {
                    last_error = error;
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }

    Err(last_error)
}

#[cfg(all(test, unix))]
mod tests {
    use super::copy_with;
    use std::time::{Duration, Instant};

    #[test]
    fn clipboard_helper_success_is_reported() {
        let command: &[&str] = &["sh", "-c", "cat > /dev/null"];
        assert!(copy_with("hello", &[command], Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn clipboard_helper_is_killed_when_it_hangs() {
        let command: &[&str] = &["sh", "-c", "sleep 30"];
        let started = Instant::now();
        let result = copy_with("hello", &[command], Duration::from_millis(50));
        assert!(result.is_err(), "a hung helper must report a failure");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a hung helper must not block the caller"
        );
    }
}
