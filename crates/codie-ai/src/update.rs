//! Release update checks (`CLI-007`).
//!
//! The check is read-only: it fetches release metadata from GitHub (or an
//! explicit endpoint) and compares versions. Nothing is downloaded or executed.

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

/// Outcome of comparing the running build with the latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateStatus {
    /// Version of the running build.
    pub current: String,
    /// Latest published version.
    pub latest: String,
    /// `Less` means an update is available.
    pub ordering: Ordering,
}

impl UpdateStatus {
    /// Returns true when the published release is newer than this build.
    #[must_use]
    pub fn newer(&self) -> bool {
        self.ordering == Ordering::Less
    }
}

/// Checks the published release for a newer version.
///
/// With no endpoint, the releases API is tried first and the tag list is used
/// as a fallback. An explicit endpoint is used verbatim.
pub async fn check(endpoint: Option<&str>) -> Result<UpdateStatus> {
    let client = reqwest::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .user_agent(concat!("codie/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::provider(format!("cannot create the update client: {error}")))?;

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

    Ok(UpdateStatus {
        current,
        latest,
        ordering,
    })
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
    let target = release_target()
        .ok_or_else(|| Error::provider("self-update is not supported on this platform"))?;
    let client = reqwest::Client::builder()
        .timeout(INSTALL_TIMEOUT)
        .user_agent(concat!("codie/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| Error::provider(format!("cannot create the update client: {error}")))?;

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
        let Some(bytes) = download_optional(&client, &url).await? else {
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
        "no prebuilt binary is published for codie {version}; reinstall with the install script"
    )))
}

async fn download_optional(client: &reqwest::Client, url: &str) -> Result<Option<Vec<u8>>> {
    let response = client
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
    let bytes = response
        .bytes()
        .await
        .map_err(|error| Error::provider(format!("cannot read {url}: {error}")))?;
    Ok(Some(bytes.to_vec()))
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
    }
}
