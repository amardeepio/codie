//! Release update checks (`CLI-007`).
//!
//! The check is read-only: it fetches release metadata from GitHub (or an
//! explicit endpoint), compares versions, and reports whether a prebuilt
//! archive exists for this platform. Nothing is downloaded or executed by a
//! check.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

use codie_core::{Error, Result};

/// Endpoint for the latest GitHub release.
pub const RELEASES_URL: &str = "https://api.github.com/repos/amardeepio/codie/releases/latest";
/// Fallback endpoint listing tags, used before the first release exists.
pub const TAGS_URL: &str = "https://api.github.com/repos/amardeepio/codie/tags";
/// Network timeout for a check.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// Network timeout for downloading an update.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
/// Base URL for release artifacts.
pub const RELEASE_DOWNLOAD: &str = "https://github.com/amardeepio/codie/releases/download";
/// Installer command used when no prebuilt archive is published.
pub const INSTALLER_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/amardeepio/codie/main/install.sh | sh";

/// Outcome of comparing the running build with the latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    /// Version of the running build.
    pub current: String,
    /// Latest published version.
    pub latest: String,
    /// `Less` means an update is available.
    pub ordering: Ordering,
    /// Whether a prebuilt archive is published for this platform.
    ///
    /// `Some(false)` means a newer release exists but cannot be installed in
    /// place, so the user has to run the installer. `None` means the check
    /// could not confirm either way.
    pub installable: Option<bool>,
}

impl UpdateStatus {
    /// Returns true when the published release is newer than this build.
    #[must_use]
    pub fn newer(&self) -> bool {
        self.ordering == Ordering::Less
    }

    /// Returns true when a newer release can be installed in place.
    #[must_use]
    pub fn installable_release(&self) -> bool {
        self.newer() && self.installable != Some(false)
    }
}

/// Download progress reported while an update archive is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateProgress {
    /// Bytes received so far.
    pub downloaded: u64,
    /// Total archive size, when the server reports a content length.
    pub total: Option<u64>,
}

impl UpdateProgress {
    /// Completion percentage (0–100) when the total size is known.
    #[must_use]
    pub fn percent(&self) -> Option<u8> {
        let total = self.total.filter(|total| *total > 0)?;
        let percent = self.downloaded.min(total).saturating_mul(100) / total;
        Some(u8::try_from(percent).unwrap_or(100))
    }
}

/// Checks the published release for a newer version.
///
/// With no endpoint, the releases API is tried first and the tag list is used
/// as a fallback. An explicit endpoint is used verbatim. When a newer release
/// is found, the check also probes for a prebuilt archive for this platform so
/// the caller does not advertise an update that cannot be installed.
pub async fn check(endpoint: Option<&str>) -> Result<UpdateStatus> {
    let client = http_client(CHECK_TIMEOUT)?;

    let latest = match endpoint {
        Some(url) => fetch_latest(&client, url).await?,
        None => match fetch_latest(&client, RELEASES_URL).await {
            Ok(version) => version,
            Err(_) => fetch_latest(&client, TAGS_URL).await?,
        },
    };

    let current = env!("CARGO_PKG_VERSION").to_owned();
    let ordering = compare_versions(&current, &latest).ok_or_else(|| {
        Error::provider(format!(
            "cannot compare versions `{current}` and `{latest}`"
        ))
    })?;

    // Mirrors and tests point at their own metadata; probing the real release
    // download for those would be wrong, so report availability as unknown.
    let installable = if ordering == Ordering::Less && endpoint.is_none() {
        asset_available_with(&client, RELEASE_DOWNLOAD, &latest)
            .await
            .ok()
    } else {
        None
    };

    Ok(UpdateStatus {
        current,
        latest,
        ordering,
        installable,
    })
}

fn http_client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("codie/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::provider(format!("cannot create the update client: {error}")))
}

/// Returns whether a prebuilt archive is published for this platform.
///
/// A missing archive means `/update` cannot install the release in place and
/// the installer has to build it from source instead.
pub async fn asset_available(version: &str) -> Result<bool> {
    let client = http_client(CHECK_TIMEOUT)?;
    asset_available_with(&client, RELEASE_DOWNLOAD, version).await
}

async fn asset_available_with(client: &reqwest::Client, base: &str, version: &str) -> Result<bool> {
    let Some(target) = release_target() else {
        return Ok(false);
    };

    for extension in ["tar.gz", "tar.xz"] {
        let name = asset_name(version, target, extension);
        let url = format!("{base}/v{version}/{name}");
        let response = client
            .head(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| Error::provider(format!("cannot check {url}: {error}")))?;

        let status = response.status();
        if status.is_success() {
            return Ok(true);
        }
        if status != reqwest::StatusCode::NOT_FOUND {
            return Err(Error::provider(format!(
                "cannot check {url}: HTTP {status}"
            )));
        }
    }

    Ok(false)
}

async fn fetch_latest(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| Error::provider(format!("cannot fetch {url}: {error}")))?;

    if !response.status().is_success() {
        return Err(Error::provider(format!(
            "cannot fetch {url}: HTTP {}",
            response.status()
        )));
    }

    let value: Value = response.json().await.map_err(|error| {
        Error::provider(format!("invalid release metadata from {url}: {error}"))
    })?;

    latest_from_payload(&value)
        .ok_or_else(|| Error::provider(format!("no release version in {url}")))
}

/// Extracts a version from a release object or a tag array.
#[must_use]
pub fn latest_from_payload(value: &Value) -> Option<String> {
    if let Some(tag) = value.get("tag_name").and_then(Value::as_str) {
        return Some(normalize_version(tag));
    }
    value.as_array()?.iter().find_map(|entry| {
        entry
            .get("name")
            .and_then(Value::as_str)
            .map(normalize_version)
            .filter(|version| !version.is_empty())
    })
}

/// Compares dotted numeric versions, ignoring pre-release suffixes.
#[must_use]
pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    Some(parse_version(left)?.cmp(&parse_version(right)?))
}

fn normalize_version(version: &str) -> String {
    version.trim().trim_start_matches('v').to_owned()
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let version = normalize_version(version);
    let mut parts = version.split(['.', '-', '+']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// Returns the release target triple for this build.
#[must_use]
pub fn release_target() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        _ => None,
    }
}

/// Returns the release asset name for a version, target, and extension.
#[must_use]
pub fn asset_name(version: &str, target: &str, extension: &str) -> String {
    format!("codie-v{version}-{target}.{extension}")
}

/// Downloads and installs a release, replacing the running executable.
///
/// The archive is checksum-verified before extraction. Installation fails
/// closed when no prebuilt artifact is published for this version, rather than
/// silently falling back to building from source inside the agent.
pub async fn install(version: &str) -> Result<()> {
    install_with_progress(version, |_| {}).await
}

/// Downloads and installs a release while reporting archive download progress.
///
/// `on_progress` receives the bytes received and, when the server reports it,
/// the total archive size. Checksum and extraction steps are not reported.
pub async fn install_with_progress(
    version: &str,
    mut on_progress: impl FnMut(UpdateProgress) + Send,
) -> Result<()> {
    let target = release_target().ok_or_else(|| {
        Error::provider(format!(
            "self-update is not supported on this platform; reinstall with:\n{INSTALLER_COMMAND}"
        ))
    })?;
    let client = http_client(INSTALL_TIMEOUT)?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let directory =
        std::env::temp_dir().join(format!("codie-update-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&directory)
        .map_err(|error| Error::provider(format!("cannot create an update directory: {error}")))?;
    let _cleanup = Cleanup(directory.clone());

    for extension in ["tar.gz", "tar.xz"] {
        let name = asset_name(version, target, extension);
        let url = format!("{RELEASE_DOWNLOAD}/v{version}/{name}");
        let Some(bytes) = download_optional_with_progress(&client, &url, &mut on_progress).await?
        else {
            continue;
        };

        let checksum_url = format!("{url}.sha256");
        let checksum = match download_optional(&client, &checksum_url).await? {
            Some(bytes) => String::from_utf8(bytes).map_err(|error| {
                Error::provider(format!("invalid checksum file {checksum_url}: {error}"))
            })?,
            None => {
                let sums_url = format!("{RELEASE_DOWNLOAD}/v{version}/sha256.sum");
                let sums = download_text(&client, &sums_url).await?;
                sums.lines()
                    .find(|line| line.contains(&name))
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        Error::provider(format!("no checksum for {name} in {sums_url}"))
                    })?
            }
        };
        verify_sha256(&bytes, &checksum)?;

        let archive = directory.join(&name);
        std::fs::write(&archive, &bytes).map_err(|error| {
            Error::provider(format!("cannot write {}: {error}", archive.display()))
        })?;
        extract(&archive, &directory)?;

        let binary = find_binary(&directory)
            .ok_or_else(|| Error::provider(format!("no codie binary found in {name}")))?;
        replace_running_binary(&binary)?;
        return Ok(());
    }

    Err(Error::provider(format!(
        "no prebuilt binary is published for codie {version} on this platform; reinstall with:\n{INSTALLER_COMMAND}"
    )))
}

async fn download_optional(client: &reqwest::Client, url: &str) -> Result<Option<Vec<u8>>> {
    download_optional_with_progress(client, url, |_| {}).await
}

async fn download_optional_with_progress(
    client: &reqwest::Client,
    url: &str,
    mut on_progress: impl FnMut(UpdateProgress),
) -> Result<Option<Vec<u8>>> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| Error::provider(format!("cannot fetch {url}: {error}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(Error::provider(format!(
            "cannot fetch {url}: HTTP {}",
            response.status()
        )));
    }

    let total = response.content_length();
    // Chunked responses omit the length; report bytes received instead.
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(64 * 1024 * 1024) as usize);
    let mut downloaded = 0_u64;
    on_progress(UpdateProgress { downloaded, total });
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Error::provider(format!("cannot read {url}: {error}")))?
    {
        bytes.extend_from_slice(&chunk);
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        on_progress(UpdateProgress { downloaded, total });
    }
    Ok(Some(bytes))
}

async fn download_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let bytes = download_optional(client, url)
        .await?
        .ok_or_else(|| Error::provider(format!("missing checksum: {url}")))?;
    String::from_utf8(bytes)
        .map_err(|error| Error::provider(format!("invalid checksum file {url}: {error}")))
}

/// Verifies a `sha256sum`-style checksum line against downloaded bytes.
pub fn verify_sha256(bytes: &[u8], checksum: &str) -> Result<()> {
    let expected = checksum
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim();
    if expected.len() != 64
        || !expected
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(Error::provider("malformed update checksum"));
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::provider("update checksum verification failed"));
    }
    Ok(())
}

fn extract(archive: &Path, directory: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(directory)
        .status()
        .map_err(|error| Error::provider(format!("cannot run tar: {error}")))?;
    if !status.success() {
        return Err(Error::provider(format!(
            "cannot extract {}",
            archive.display()
        )));
    }
    Ok(())
}

fn find_binary(directory: &Path) -> Option<PathBuf> {
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new("codie"))
            {
                return Some(path);
            }
        }
    }
    None
}

fn replace_running_binary(source: &Path) -> Result<()> {
    let current = std::env::current_exe()
        .map_err(|error| Error::provider(format!("cannot locate the running binary: {error}")))?;
    let staged = current.with_file_name(format!(".codie-update-{}", std::process::id()));

    std::fs::copy(source, &staged).map_err(|error| {
        Error::provider(format!(
            "cannot stage the update at {}: {error}",
            staged.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(
            |error| Error::provider(format!("cannot make the update executable: {error}")),
        )?;
    }
    std::fs::rename(&staged, &current).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        Error::provider(format!("cannot replace {}: {error}", current.display()))
    })?;
    Ok(())
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("0.4.0", "0.5.0"), Some(Ordering::Less));
        assert_eq!(compare_versions("v0.5.0", "0.5.0"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.0.0", "0.9.9"), Some(Ordering::Greater));
    }

    #[test]
    fn release_and_tag_payloads_are_supported() {
        let release = serde_json::json!({"tag_name": "v0.5.0"});
        assert_eq!(latest_from_payload(&release).as_deref(), Some("0.5.0"));
        let tags = serde_json::json!([{"name": "v0.5.0"}, {"name": "v0.4.0"}]);
        assert_eq!(latest_from_payload(&tags).as_deref(), Some("0.5.0"));
    }

    #[test]
    fn release_assets_follow_the_installer_naming() {
        assert_eq!(
            asset_name("0.5.0", "aarch64-apple-darwin", "tar.gz"),
            "codie-v0.5.0-aarch64-apple-darwin.tar.gz"
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            release_target(),
            Some(if cfg!(target_arch = "aarch64") {
                "aarch64-apple-darwin"
            } else {
                "x86_64-apple-darwin"
            })
        );
    }

    #[test]
    fn checksums_are_verified() {
        let payload = b"codie";
        let hash = format!("{:x}", Sha256::digest(payload));
        assert!(verify_sha256(payload, &format!("{hash}  codie.tar.gz")).is_ok());
        assert!(verify_sha256(payload, &format!("{}  codie.tar.gz", "0".repeat(64))).is_err());
        assert!(verify_sha256(payload, "not-a-checksum").is_err());
    }

    #[tokio::test]
    async fn check_uses_an_explicit_endpoint() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"tag_name": "v9.9.9"})),
            )
            .mount(&server)
            .await;

        let status = check(Some(&server.uri())).await.expect("check");
        assert!(status.newer());
        assert_eq!(status.latest, "9.9.9");
        // A mirror may not publish the release artifacts this build expects.
        assert_eq!(status.installable, None);
    }

    #[tokio::test]
    async fn asset_availability_probes_release_archives() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        if release_target().is_none() {
            return;
        }

        let client = http_client(CHECK_TIMEOUT).expect("client");

        let published = MockServer::start().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&published)
            .await;
        assert!(
            asset_available_with(&client, &published.uri(), "0.6.0")
                .await
                .expect("availability")
        );

        let missing = MockServer::start().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&missing)
            .await;
        assert!(
            !asset_available_with(&client, &missing.uri(), "0.6.0")
                .await
                .expect("availability")
        );
    }

    #[tokio::test]
    async fn downloads_report_progress_while_streaming() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let payload = vec![7_u8; 4096];
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(payload.clone()))
            .mount(&server)
            .await;

        let client = http_client(CHECK_TIMEOUT).expect("client");
        let mut events = Vec::new();
        let bytes = download_optional_with_progress(&client, &server.uri(), |event| {
            events.push(event);
        })
        .await
        .expect("download")
        .expect("present");

        assert_eq!(bytes, payload);
        assert!(
            events.len() >= 2,
            "expected a start and a chunk: {events:?}"
        );
        assert_eq!(events[0].downloaded, 0);
        let last = events.last().expect("events");
        assert_eq!(last.downloaded, 4096);
        assert_eq!(last.percent(), Some(100));
    }

    #[tokio::test]
    async fn downloads_treat_404_as_missing() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = http_client(CHECK_TIMEOUT).expect("client");
        assert!(
            download_optional_with_progress(&client, &server.uri(), |_| {})
                .await
                .expect("download")
                .is_none()
        );
    }

    #[test]
    fn update_progress_reports_percentages_when_the_size_is_known() {
        let half = UpdateProgress {
            downloaded: 5 * 1024 * 1024,
            total: Some(10 * 1024 * 1024),
        };
        assert_eq!(half.percent(), Some(50));

        let done = UpdateProgress {
            downloaded: 20,
            total: Some(10),
        };
        assert_eq!(done.percent(), Some(100), "overshoot must clamp");

        let unknown = UpdateProgress {
            downloaded: 1234,
            total: None,
        };
        assert_eq!(unknown.percent(), None);

        let empty = UpdateProgress {
            downloaded: 0,
            total: Some(0),
        };
        assert_eq!(empty.percent(), None, "a zero total must not divide");
    }

    #[test]
    fn only_releases_with_archives_are_installable() {
        let pending = UpdateStatus {
            current: "0.5.0".to_owned(),
            latest: "0.6.0".to_owned(),
            ordering: Ordering::Less,
            installable: Some(false),
        };
        assert!(pending.newer());
        assert!(!pending.installable_release());

        let ready = UpdateStatus {
            installable: Some(true),
            ..pending
        };
        assert!(ready.installable_release());

        let current = UpdateStatus {
            current: "0.6.0".to_owned(),
            latest: "0.6.0".to_owned(),
            ordering: Ordering::Equal,
            installable: None,
        };
        assert!(!current.installable_release());
    }
}
