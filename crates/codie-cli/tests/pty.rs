//! PTY integration tests for the interactive terminal (`TUI-009`, `TUI-010`).

#![cfg(unix)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use codie_testkit::{TempHome, spec};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

struct PtySession {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    buffer: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

impl PtySession {
    fn spawn(args: &[&str]) -> Self {
        let home = TempHome::new().expect("temp home");
        let work = tempfile::tempdir().expect("temp work");
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 30,
                cols: 110,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_codie"));
        for (key, value) in home.envs() {
            command.env(key, value);
        }
        command.env("DEEPSEEK_API_KEY", "dummy");
        command.env("OPENROUTER_API_KEY", "");
        command.env("TERM", "xterm-256color");
        command.args(["--no-session", "--cwd"]);
        command.arg(work.path());
        command.args(args);

        let child = pair.slave.spawn_command(command).expect("spawn codie");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("pty reader");
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let reader_buffer = Arc::clone(&buffer);
        let reader = std::thread::spawn(move || {
            let mut chunk = [0_u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        reader_buffer
                            .lock()
                            .expect("pty buffer")
                            .extend_from_slice(&chunk[..count]);
                    }
                }
            }
        });
        let writer = pair.master.take_writer().expect("pty writer");
        Self {
            master: Some(pair.master),
            writer: Some(writer),
            child,
            buffer,
            reader: Some(reader),
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.buffer.lock().expect("pty buffer")).into_owned()
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if self.text().contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    fn write(&mut self, text: &str) {
        let writer = self.writer.as_mut().expect("pty writer");
        writer.write_all(text.as_bytes()).expect("write pty");
        writer.flush().expect("flush pty");
    }

    fn signal(&self, signal: &str) {
        let pid = self.child.process_id().expect("child pid");
        let status = std::process::Command::new("kill")
            .arg(format!("-{signal}"))
            .arg(pid.to_string())
            .status()
            .expect("send signal");
        assert!(status.success(), "kill -{signal} failed");
    }

    fn wait(&mut self, timeout: Duration) -> Option<u32> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status.exit_code());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        None
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.writer.take();
        self.master.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
#[spec("TUI-009")]
fn spec_tui_009_terminal_restored_on_signals() {
    for (signal, expected) in [("INT", 130), ("TERM", 143)] {
        let mut session = PtySession::spawn(&[]);
        assert!(
            session.wait_for("prompt", Duration::from_secs(10)),
            "the TUI did not start: {:?}",
            session.text()
        );

        session.signal(signal);
        let code = session.wait(Duration::from_secs(10));
        assert_eq!(code, Some(expected), "SIG{signal} exit code");

        let text = session.text();
        assert!(
            text.contains("\u{1b}[?1049l"),
            "the alternate screen was not left: {text:?}"
        );
        assert!(
            text.contains("\u{1b}[?25h"),
            "the cursor was not restored: {text:?}"
        );
    }
}

#[test]
#[spec("TUI-010")]
fn spec_tui_010_plain_mode_is_append_only() {
    let mut session = PtySession::spawn(&["--tui-mode", "plain"]);
    assert!(
        session.wait_for("plain mode", Duration::from_secs(10)),
        "plain mode did not start: {:?}",
        session.text()
    );
    session.write("/quit\r");
    let code = session.wait(Duration::from_secs(10));
    assert_eq!(code, Some(0), "plain mode should exit 0");

    let text = session.text();
    assert!(
        !text.contains("\u{1b}[?1049h"),
        "plain mode entered the alternate screen"
    );
    assert!(!text.contains("\u{1b}[?1049l"));
    assert!(text.contains("plain mode"));
}
