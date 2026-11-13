//! Extension process supervision and JSON-RPC transport.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use codie_protocol::ext::{
    ExtLogParams, ExtManifest, ExtNotification, ExtRequest, ExtResponse, ExtServerRecord,
    MAX_RPC_LINE_BYTES,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::ExtError;

type Reply = Result<serde_json::Value, String>;

/// A supervised extension process.
pub struct ExtensionClient {
    name: String,
    writer: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Reply>>>>,
    next_id: AtomicU64,
    dead: Arc<AtomicBool>,
    kill: watch::Sender<bool>,
    stderr: Arc<Mutex<VecDeque<String>>>,
}

impl std::fmt::Debug for ExtensionClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExtensionClient")
            .field("name", &self.name)
            .field("dead", &self.is_dead())
            .finish()
    }
}

struct Parts {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    child: tokio::process::Child,
}

impl ExtensionClient {
    /// Spawns an extension process from a manifest.
    ///
    /// The child receives a sanitized environment with no API keys or codie
    /// tokens. Its working directory is the manifest directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot be spawned or its pipes are
    /// unavailable.
    pub fn spawn(manifest: &ExtManifest, dir: &Path) -> Result<Self, ExtError> {
        let executable = &manifest.entrypoint[0];
        let executable = if executable.contains('/') && Path::new(executable).is_relative() {
            dir.join(executable)
        } else {
            Path::new(executable).to_path_buf()
        };
        let mut command = tokio::process::Command::new(executable);
        command
            .args(&manifest.entrypoint[1..])
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env_clear();
        for (key, value) in safe_environment(std::env::vars()) {
            command.env(key, value);
        }
        command
            .env("CODIE_EXTENSION_PROTOCOL", "1")
            .env("CODIE_EXTENSION_NAME", &manifest.name)
            .env("CODIE_EXTENSION_VERSION", &manifest.version);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| ExtError::Io(format!("cannot start `{}`: {error}", manifest.name)))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ExtError::Io("extension stdin is unavailable".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ExtError::Io("extension stdout is unavailable".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ExtError::Io("extension stderr is unavailable".to_owned()))?;
        let parts = Parts {
            stdin,
            stdout,
            stderr,
            child,
        };
        Ok(Self::start(manifest.name.clone(), parts))
    }

    fn start(name: String, parts: Parts) -> Self {
        let (writer_tx, mut writer_rx) = mpsc::unbounded_channel::<String>();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let dead = Arc::new(AtomicBool::new(false));
        let (kill_tx, mut kill_rx) = watch::channel(false);
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));

        let mut stdin = parts.stdin;
        tokio::spawn(async move {
            while let Some(line) = writer_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    break;
                }
            }
        });

        let reader_pending = Arc::clone(&pending);
        let reader_dead = Arc::clone(&dead);
        let reader_name = name.clone();
        let mut stdout = BufReader::new(parts.stdout);
        tokio::spawn(async move {
            let mut buffer = Vec::new();
            loop {
                match read_limited_line(&mut stdout, &mut buffer).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buffer);
                        let line = line.trim();
                        if line.is_empty() {
                            buffer.clear();
                            continue;
                        }
                        match serde_json::from_str::<ExtServerRecord>(line) {
                            Ok(ExtServerRecord::Response(response)) => {
                                resolve(&reader_pending, response);
                            }
                            Ok(ExtServerRecord::Notification(notification)) => {
                                handle_notification(&reader_name, notification);
                            }
                            Err(error) => {
                                tracing::warn!(
                                    extension = %reader_name,
                                    %error,
                                    "extension wrote invalid JSON-RPC"
                                );
                            }
                        }
                        buffer.clear();
                    }
                    Err(error) => {
                        tracing::warn!(
                            extension = %reader_name,
                            %error,
                            "extension output could not be read"
                        );
                        break;
                    }
                }
            }
            reader_dead.store(true, Ordering::SeqCst);
            fail_pending(&reader_pending, "extension closed its output");
        });

        let stderr_name = name.clone();
        let stderr_buffer = Arc::clone(&stderr_tail);
        let mut stderr = BufReader::new(parts.stderr);
        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match stderr.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let text = line.trim_end();
                        if text.is_empty() {
                            continue;
                        }
                        tracing::warn!(extension = %stderr_name, "{text}");
                        if let Ok(mut buffer) = stderr_buffer.lock() {
                            buffer.push_back(text.to_owned());
                            while buffer.len() > 20 {
                                buffer.pop_front();
                            }
                        }
                    }
                }
            }
        });

        let monitor_pending = Arc::clone(&pending);
        let monitor_dead = Arc::clone(&dead);
        let monitor_name = name.clone();
        let mut child = parts.child;
        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    tracing::warn!(
                        extension = %monitor_name,
                        status = ?status.ok(),
                        "extension exited"
                    );
                }
                _ = async {
                    if !*kill_rx.borrow() {
                        let _ = kill_rx.changed().await;
                    }
                } => {
                    kill_group(child.id()).await;
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
            }
            monitor_dead.store(true, Ordering::SeqCst);
            fail_pending(&monitor_pending, "extension process stopped");
        });

        Self {
            name,
            writer: writer_tx,
            pending,
            next_id: AtomicU64::new(1),
            dead,
            kill: kill_tx,
            stderr: stderr_tail,
        }
    }

    /// Returns the extension name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns true when the process is known to be gone.
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// Returns the last captured stderr lines.
    #[must_use]
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr
            .lock()
            .map(|buffer| buffer.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Sends a JSON-RPC request and waits for its response.
    ///
    /// # Errors
    ///
    /// Returns an error when the extension is dead, the request times out, or
    /// the extension replies with a JSON-RPC error. A timeout kills the child.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, ExtError> {
        self.request_cancelable(method, params, timeout, None).await
    }

    /// Sends a request that is also cancelled by `cancel`.
    ///
    /// # Errors
    ///
    /// Returns an error when the extension is dead, the request times out, or
    /// the extension replies with a JSON-RPC error.
    pub async fn request_cancelable(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
    ) -> Result<serde_json::Value, ExtError> {
        if self.is_dead() {
            return Err(ExtError::Dead(self.name.clone()));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (reply_tx, reply_rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, reply_tx);
        } else {
            return Err(ExtError::Dead(self.name.clone()));
        }
        let request = ExtRequest::new(id, method, params);
        let line = serde_json::to_string(&request)
            .map_err(|error| ExtError::Protocol(error.to_string()))?;
        if self.writer.send(line).is_err() {
            remove_pending(&self.pending, id);
            return Err(ExtError::Dead(self.name.clone()));
        }

        let result = if let Some(cancel) = cancel {
            tokio::select! {
                result = reply_rx => result,
                () = cancel.cancelled() => {
                    remove_pending(&self.pending, id);
                    self.notify("$/cancelRequest", serde_json::json!({"id": id}));
                    return Err(ExtError::Invalid("request cancelled".to_owned()));
                }
                () = tokio::time::sleep(timeout) => {
                    remove_pending(&self.pending, id);
                    self.kill();
                    return Err(ExtError::Timeout(format!(
                        "`{method}` exceeded {} ms",
                        timeout.as_millis()
                    )));
                }
            }
        } else {
            match tokio::time::timeout(timeout, reply_rx).await {
                Ok(result) => result,
                Err(_) => {
                    remove_pending(&self.pending, id);
                    self.kill();
                    return Err(ExtError::Timeout(format!(
                        "`{method}` exceeded {} ms",
                        timeout.as_millis()
                    )));
                }
            }
        };
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(ExtError::Rpc(error)),
            Err(_) => Err(ExtError::Dead(self.name.clone())),
        }
    }

    /// Sends a JSON-RPC notification.
    pub fn notify(&self, method: &str, params: serde_json::Value) {
        let notification = ExtNotification::new(method, params);
        if let Ok(line) = serde_json::to_string(&notification) {
            let _ = self.writer.send(line);
        }
    }

    /// Requests immediate termination of the process group.
    pub fn kill(&self) {
        self.dead.store(true, Ordering::SeqCst);
        let _ = self.kill.send(true);
    }
}

impl Drop for ExtensionClient {
    fn drop(&mut self) {
        self.kill();
    }
}

fn resolve(pending: &Mutex<HashMap<u64, oneshot::Sender<Reply>>>, response: ExtResponse) {
    let Some(sender) = pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(&response.id))
    else {
        return;
    };
    if let Some(error) = response.error {
        let _ = sender.send(Err(format!("{} (code {})", error.message, error.code)));
    } else {
        let _ = sender.send(Ok(response.result.unwrap_or(serde_json::Value::Null)));
    }
}

fn handle_notification(name: &str, notification: ExtNotification) {
    match notification.method.as_str() {
        "log" => match serde_json::from_value::<ExtLogParams>(notification.params) {
            Ok(log) => match log.level.as_str() {
                "error" => tracing::error!(extension = %name, "{}", log.message),
                "warn" => tracing::warn!(extension = %name, "{}", log.message),
                "debug" => tracing::debug!(extension = %name, "{}", log.message),
                _ => tracing::info!(extension = %name, "{}", log.message),
            },
            Err(error) => tracing::warn!(extension = %name, %error, "invalid log notification"),
        },
        other => tracing::debug!(extension = %name, method = %other, "ignored notification"),
    }
}

fn fail_pending(pending: &Mutex<HashMap<u64, oneshot::Sender<Reply>>>, reason: &str) {
    let senders = pending
        .lock()
        .map(|mut pending| {
            pending
                .drain()
                .map(|(_, sender)| sender)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for sender in senders {
        let _ = sender.send(Err(reason.to_owned()));
    }
}

fn remove_pending(pending: &Mutex<HashMap<u64, oneshot::Sender<Reply>>>, id: u64) {
    if let Ok(mut pending) = pending.lock() {
        pending.remove(&id);
    }
}

async fn read_limited_line<R>(reader: &mut R, buffer: &mut Vec<u8>) -> std::io::Result<usize>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(buffer.len());
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            buffer.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            return Ok(buffer.len());
        }
        buffer.extend_from_slice(available);
        let consumed = available.len();
        reader.consume(consumed);
        if buffer.len() > MAX_RPC_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JSON-RPC line exceeds {MAX_RPC_LINE_BYTES} bytes"),
            ));
        }
    }
}

/// Filters the process environment down to safe, non-secret variables.
fn safe_environment<I>(environment: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    const SAFE: [&str; 12] = [
        "PATH", "HOME", "TMPDIR", "TMP", "LANG", "LC_ALL", "LC_CTYPE", "USER", "LOGNAME", "SHELL",
        "TERM", "PWD",
    ];
    environment
        .into_iter()
        .filter(|(key, _)| SAFE.contains(&key.as_str()))
        .collect()
}

async fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        // `process_group(0)` makes the pid the group id. Shelling out keeps
        // this crate free of unsafe code; a missing group is harmless.
        let _ = tokio::process::Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pid}"))
            .kill_on_drop(true)
            .status()
            .await;
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[test]
    #[spec("EXT-014")]
    fn spec_ext_014_no_secret_environment() {
        let filtered = safe_environment([
            ("PATH".to_owned(), "/bin".to_owned()),
            ("HOME".to_owned(), "/home/me".to_owned()),
            ("DEEPSEEK_API_KEY".to_owned(), "sk-secret".to_owned()),
            ("OPENROUTER_API_KEY".to_owned(), "sk-other".to_owned()),
            ("CODIE_FOO_TOKEN".to_owned(), "token".to_owned()),
            ("SOME_TOKEN".to_owned(), "token".to_owned()),
        ]);
        assert_eq!(filtered.len(), 2);
        assert!(!filtered.iter().any(|(key, _)| key.contains("KEY")));
        assert!(!filtered.iter().any(|(key, _)| key.contains("TOKEN")));
    }
}
