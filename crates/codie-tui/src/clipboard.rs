//! Clipboard image and text paste.
//!
//! Terminals never forward image bytes, so the app asks the operating system
//! for the clipboard directly when the paste key is pressed. Images are saved
//! to a temporary PNG and inserted as an `@"path"` attachment; text is inserted
//! like a bracketed paste.
//!
//! Reads are synchronous and happen on the UI thread. That costs a few dozen
//! milliseconds on macOS and keeps the paste handler simple; it also matches
//! how other terminal agents behave.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// How long a pasted image stays in the temporary directory.
const PASTE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Reads an image from the clipboard and saves it as a PNG.
///
/// Returns `None` when the clipboard holds no image or no supported backend is
/// available.
#[must_use]
pub fn read_image() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos_image()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unix_image()
    }
    #[cfg(windows)]
    {
        windows_image()
    }
}

/// Reads text from the clipboard.
#[must_use]
pub fn read_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    let (program, arguments) = ("pbpaste", Vec::<&str>::new());
    #[cfg(all(unix, not(target_os = "macos")))]
    let (program, arguments) = if command_exists("wl-paste") {
        ("wl-paste", vec!["--no-newline"])
    } else {
        ("xclip", vec!["-selection", "clipboard", "-o"])
    };
    #[cfg(windows)]
    let (program, arguments) = (
        "powershell",
        vec!["-NoProfile", "-Command", "Get-Clipboard -Raw"],
    );

    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    (!text.is_empty()).then_some(text)
}

fn paste_dir() -> PathBuf {
    std::env::temp_dir().join("codie-paste")
}

fn paste_path(extension: &str) -> PathBuf {
    let directory = paste_dir();
    let _ = std::fs::create_dir_all(&directory);
    sweep_old(&directory);
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    directory.join(format!("paste-{stamp}-{}.{extension}", std::process::id()))
}

/// Removes pasted images older than [`PASTE_TTL`] so the directory stays small.
fn sweep_old(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > PASTE_TTL);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_image() -> Option<PathBuf> {
    let info = Command::new("osascript")
        .args(["-e", "clipboard info"])
        .output()
        .ok()?;
    let info = String::from_utf8_lossy(&info.stdout);

    // `pngpaste` is a single-purpose binary and is more reliable than the
    // AppleScript conversion path when it is installed.
    if command_exists("pngpaste") {
        let path = paste_path("png");
        let pasted = Command::new("pngpaste")
            .arg(&path)
            .status()
            .is_ok_and(|status| status.success());
        if pasted && path.is_file() {
            return Some(path);
        }
        let _ = std::fs::remove_file(&path);
    }

    if clipboard_has(&info, "PNGf") {
        let path = paste_path("png");
        if write_clipboard_class(&path, "PNGf") {
            return Some(path);
        }
        let _ = std::fs::remove_file(&path);
    }

    if clipboard_has(&info, "TIFF") {
        let tiff = paste_path("tiff");
        if !write_clipboard_class(&tiff, "TIFF") {
            let _ = std::fs::remove_file(&tiff);
            return None;
        }
        let png = tiff.with_extension("png");
        let converted = Command::new("sips")
            .args(["-s", "format", "png"])
            .arg(&tiff)
            .arg("--out")
            .arg(&png)
            .output()
            .is_ok_and(|output| output.status.success());
        let _ = std::fs::remove_file(&tiff);
        return (converted && png.is_file()).then_some(png);
    }

    None
}

/// Returns true when the macOS clipboard description contains a type.
#[cfg(target_os = "macos")]
fn clipboard_has(info: &str, class: &str) -> bool {
    info.contains(&format!("«class {class}»"))
}

/// Writes one clipboard flavour into `path` with AppleScript.
#[cfg(target_os = "macos")]
fn write_clipboard_class(path: &Path, class: &str) -> bool {
    let script = format!(
        "set theFile to POSIX file \"{file}\"\n\
         set handle to open for access theFile with write permission\n\
         set eof handle to 0\n\
         write (the clipboard as «class {class}») to handle\n\
         close access handle",
        file = path.display(),
    );
    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .is_ok_and(|status| status.success())
        && path.is_file()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unix_image() -> Option<PathBuf> {
    if command_exists("wl-paste") {
        let types = Command::new("wl-paste").arg("--list-types").output().ok()?;
        let types = String::from_utf8_lossy(&types.stdout);
        if types.lines().any(|line| line.trim() == "image/png") {
            let output = Command::new("wl-paste")
                .args(["-t", "image/png"])
                .output()
                .ok()?;
            if output.status.success() && !output.stdout.is_empty() {
                return write_bytes(&output.stdout, "png");
            }
        }
    }
    if command_exists("xclip") {
        let output = Command::new("xclip")
            .args(["-selection", "clipboard", "-t", "image/png", "-o"])
            .output()
            .ok()?;
        if output.status.success() && !output.stdout.is_empty() {
            return write_bytes(&output.stdout, "png");
        }
    }
    None
}

#[cfg(windows)]
fn windows_image() -> Option<PathBuf> {
    let path = paste_path("png");
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $image = [System.Windows.Forms.Clipboard]::GetImage(); \
         if ($image -eq $null) {{ exit 1 }}; \
         $image.Save('{file}', [System.Drawing.Imaging.ImageFormat]::Png)",
        file = path.display(),
    );
    let saved = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .is_ok_and(|status| status.success());
    (saved && path.is_file()).then_some(path)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn write_bytes(bytes: &[u8], extension: &str) -> Option<PathBuf> {
    let path = paste_path(extension);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

#[cfg(any(all(unix, not(target_os = "macos")), target_os = "macos"))]
fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {program}")])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_paths_live_in_the_temp_directory() {
        let path = paste_path("png");
        assert!(path.starts_with(std::env::temp_dir()));
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("png")
        );
        assert!(!path.to_string_lossy().contains(' '));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn clipboard_types_are_detected() {
        assert!(clipboard_has(
            "«class PNGf», 249125, «class TIFF», 1000000",
            "PNGf"
        ));
        assert!(clipboard_has("«class TIFF», 1000000", "TIFF"));
        assert!(!clipboard_has("«class utf8», 12", "PNGf"));
    }
}
