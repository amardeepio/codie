//! Extension discovery and manifest parsing.

use std::fs;
use std::path::{Path, PathBuf};

use codie_protocol::ext::{
    EXT_MANIFEST_FILE, EXT_PROTOCOL_VERSION, EXTENSIONS_DIR, ExtManifest, MAX_EXTENSIONS,
    MAX_MANIFEST_BYTES,
};

/// One validated manifest and the directory containing it.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    /// Parsed manifest.
    pub manifest: ExtManifest,
    /// Directory containing `codie.toml`.
    pub dir: PathBuf,
    /// Manifest path.
    pub path: PathBuf,
}

/// Reads and validates one manifest file.
///
/// # Errors
///
/// Returns a human-readable message for unsafe paths, oversize files, invalid
/// TOML, unsupported protocol versions, or an empty entrypoint.
pub fn read_manifest(path: &Path) -> Result<LoadedManifest, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "{} is larger than {MAX_MANIFEST_BYTES} bytes",
            path.display()
        ));
    }
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let manifest: ExtManifest = toml::from_str(&contents)
        .map_err(|error| format!("invalid manifest {}: {error}", path.display()))?;
    validate(&manifest)?;
    let dir = path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok(LoadedManifest {
        manifest,
        dir,
        path: path.to_path_buf(),
    })
}

fn validate(manifest: &ExtManifest) -> Result<(), String> {
    if manifest.name.trim().is_empty() || manifest.name.len() > 64 {
        return Err("manifest name must be 1..64 characters".to_owned());
    }
    if !manifest
        .name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(format!(
            "manifest name `{}` may contain only letters, digits, `_`, and `-`",
            manifest.name
        ));
    }
    if manifest.version.trim().is_empty() {
        return Err("manifest version must not be empty".to_owned());
    }
    if manifest.protocol != EXT_PROTOCOL_VERSION {
        return Err(format!(
            "protocol {} is not supported (host speaks {EXT_PROTOCOL_VERSION})",
            manifest.protocol
        ));
    }
    if manifest.entrypoint.is_empty() || manifest.entrypoint[0].trim().is_empty() {
        return Err("manifest entrypoint must name an executable".to_owned());
    }
    if manifest
        .entrypoint
        .iter()
        .any(|argument| argument.contains('\0'))
    {
        return Err("manifest entrypoint contains a NUL byte".to_owned());
    }
    if manifest.limits.hook_timeout_ms == 0 || manifest.limits.tool_timeout_ms == 0 {
        return Err("manifest timeouts must be greater than zero".to_owned());
    }
    Ok(())
}

/// Discovers user, project, and explicit extensions.
///
/// Precedence is explicit > project > user for the same manifest name.
/// Disabled names are removed before any process is started.
#[must_use]
pub fn discover(
    config_dir: &Path,
    project_root: Option<&Path>,
    explicit: &[PathBuf],
    disabled: &[String],
    cwd: &Path,
) -> (Vec<LoadedManifest>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut found: Vec<LoadedManifest> = Vec::new();

    scan_root(&config_dir.join(EXTENSIONS_DIR), &mut found, &mut warnings);
    if let Some(root) = project_root {
        scan_root(
            &root
                .join(codie_core::config::PROJECT_DIR)
                .join(EXTENSIONS_DIR),
            &mut found,
            &mut warnings,
        );
    }
    for path in explicit {
        let path = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };
        let path = if path.is_dir() {
            path.join(EXT_MANIFEST_FILE)
        } else {
            path
        };
        match read_manifest(&path) {
            Ok(manifest) => found.push(manifest),
            Err(error) => warnings.push(error),
        }
    }

    found.retain(|item| !disabled.iter().any(|name| name == &item.manifest.name));

    let mut by_name: Vec<LoadedManifest> = Vec::new();
    for item in found {
        if let Some(previous) = by_name
            .iter_mut()
            .find(|previous| previous.manifest.name == item.manifest.name)
        {
            warnings.push(format!(
                "extension `{}` is defined by {} and {}; using {}",
                item.manifest.name,
                previous.path.display(),
                item.path.display(),
                item.path.display()
            ));
            *previous = item;
        } else {
            by_name.push(item);
        }
    }
    by_name.sort_by(|left, right| left.manifest.name.cmp(&right.manifest.name));
    if by_name.len() > MAX_EXTENSIONS {
        warnings.push(format!(
            "only the first {MAX_EXTENSIONS} extensions are loaded"
        ));
        by_name.truncate(MAX_EXTENSIONS);
    }
    (by_name, warnings)
}

fn scan_root(root: &Path, found: &mut Vec<LoadedManifest>, warnings: &mut Vec<String>) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            warnings.push(format!("cannot read {}: {error}", root.display()));
            return;
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let manifest = if path.is_dir() {
            path.join(EXT_MANIFEST_FILE)
        } else if path.file_name().and_then(|name| name.to_str()) == Some(EXT_MANIFEST_FILE) {
            path
        } else {
            continue;
        };
        match read_manifest(&manifest) {
            Ok(loaded) => found.push(loaded),
            Err(error) => warnings.push(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    fn write_extension(root: &Path, directory: &str, name: &str) {
        let dir = root.join(directory);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(EXT_MANIFEST_FILE),
            format!(
                "name = \"{name}\"\nversion = \"0.1.0\"\nprotocol = 1\nentrypoint = [\"python3\", \"ext.py\"]\ncapabilities = [\"tools\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("ext.py"), "print('{}')\n").unwrap();
    }

    #[test]
    #[spec("EXT-006")]
    fn spec_ext_006_rejects_bad_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(EXT_MANIFEST_FILE);
        std::fs::write(
            &path,
            "name = \"bad\"\nversion = \"0.1.0\"\nprotocol = 2\nentrypoint = [\"python3\", \"x.py\"]\n",
        )
        .unwrap();
        assert!(read_manifest(&path).is_err());
        std::fs::write(
            &path,
            "name = \"bad name\"\nversion = \"0.1.0\"\nprotocol = 1\nentrypoint = [\"python3\", \"x.py\"]\n",
        )
        .unwrap();
        assert!(read_manifest(&path).is_err());
    }

    #[test]
    #[spec("EXT-007")]
    fn spec_ext_007_discovery_precedence_and_trust() {
        let home = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        write_extension(&home.path().join(EXTENSIONS_DIR), "shared", "shared");
        write_extension(&home.path().join(EXTENSIONS_DIR), "user-only", "user-only");

        let codie = project.path().join(codie_core::config::PROJECT_DIR);
        write_extension(&codie.join(EXTENSIONS_DIR), "shared", "shared");
        write_extension(&codie.join(EXTENSIONS_DIR), "project-only", "project-only");

        let (untrusted, _) = discover(home.path(), None, &[], &[], project.path());
        let names = untrusted
            .iter()
            .map(|item| item.manifest.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["shared", "user-only"]);

        let (trusted, warnings) =
            discover(home.path(), Some(project.path()), &[], &[], project.path());
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("defined by")),
            "warnings: {warnings:?}"
        );
        let names = trusted
            .iter()
            .map(|item| item.manifest.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["project-only", "shared", "user-only"]);
        let shared = trusted
            .iter()
            .find(|item| item.manifest.name == "shared")
            .unwrap();
        assert!(shared.path.starts_with(project.path()));

        let (disabled, _) = discover(
            home.path(),
            Some(project.path()),
            &[],
            &["shared".to_owned()],
            project.path(),
        );
        assert!(disabled.iter().all(|item| item.manifest.name != "shared"));
    }
}
