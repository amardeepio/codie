//! Semantic color themes (`TUI-008`).
//!
//! Built-in themes are `system`, `dark`, and `light`. A theme may also be a
//! TOML file with semantic tokens; hex colors are parsed at load time.

use std::path::Path;

use anyhow::{Context, Result, bail};
use ratatui::style::Color;
use serde::Deserialize;

/// Semantic colors used by every widget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Application background.
    pub background: Color,
    /// Primary body text.
    pub foreground: Color,
    /// Secondary text and metadata.
    pub muted: Color,
    /// Accent for headings, prompts, and focus.
    pub accent: Color,
    /// User message text.
    pub user: Color,
    /// Assistant message text.
    pub assistant: Color,
    /// Tool names and summaries.
    pub tool: Color,
    /// Errors and denied calls.
    pub error: Color,
    /// Success and added lines.
    pub success: Color,
    /// Added diff lines.
    pub diff_add: Color,
    /// Removed diff lines.
    pub diff_del: Color,
    /// Borders and separators.
    pub border: Color,
}

impl Palette {
    /// The default dark palette.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            background: Color::Rgb(18, 18, 22),
            foreground: Color::Rgb(226, 226, 232),
            muted: Color::Rgb(138, 138, 152),
            accent: Color::Rgb(122, 162, 247),
            user: Color::Rgb(158, 206, 106),
            assistant: Color::Rgb(226, 226, 232),
            tool: Color::Rgb(187, 154, 247),
            error: Color::Rgb(247, 118, 142),
            success: Color::Rgb(158, 206, 106),
            diff_add: Color::Rgb(158, 206, 106),
            diff_del: Color::Rgb(247, 118, 142),
            border: Color::Rgb(65, 72, 104),
        }
    }

    /// The default light palette.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            background: Color::Rgb(250, 250, 252),
            foreground: Color::Rgb(36, 40, 59),
            muted: Color::Rgb(96, 103, 128),
            accent: Color::Rgb(52, 84, 209),
            user: Color::Rgb(46, 125, 50),
            assistant: Color::Rgb(36, 40, 59),
            tool: Color::Rgb(124, 58, 237),
            error: Color::Rgb(185, 28, 28),
            success: Color::Rgb(46, 125, 50),
            diff_add: Color::Rgb(46, 125, 50),
            diff_del: Color::Rgb(185, 28, 28),
            border: Color::Rgb(160, 168, 190),
        }
    }
}

/// A loaded theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Theme name, or the file path for a custom theme.
    pub name: String,
    /// Semantic colors.
    pub palette: Palette,
}

impl Theme {
    /// Loads a theme from `system`, `dark`, `light`, or a TOML path.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown name or an invalid theme file.
    pub fn load(spec: Option<&str>) -> Result<Self> {
        match spec.unwrap_or("system") {
            "system" => Ok(Self {
                name: "system".to_owned(),
                palette: system_palette(),
            }),
            "dark" => Ok(Self {
                name: "dark".to_owned(),
                palette: Palette::dark(),
            }),
            "light" => Ok(Self {
                name: "light".to_owned(),
                palette: Palette::light(),
            }),
            path => load_file(Path::new(path)),
        }
    }

    /// Returns the background for the body text contrast check.
    #[must_use]
    pub fn body_contrast(&self) -> f64 {
        contrast_ratio(self.palette.foreground, self.palette.background)
    }
}

fn system_palette() -> Palette {
    // `COLORFGBG` ends with the terminal background index; 7 and above are
    // normally light. `CODIE_THEME` overrides detection for unusual terminals.
    if std::env::var("CODIE_THEME").as_deref() == Ok("light") {
        return Palette::light();
    }
    if std::env::var("CODIE_THEME").as_deref() == Ok("dark") {
        return Palette::dark();
    }

    let light = std::env::var("COLORFGBG").is_ok_and(|value| {
        value
            .rsplit(';')
            .next()
            .and_then(|background| background.parse::<u8>().ok())
            .is_some_and(|background| background >= 7)
    });
    if light {
        Palette::light()
    } else {
        Palette::dark()
    }
}

#[derive(Debug, Default, Deserialize)]
struct ThemeFile {
    foreground: Option<String>,
    muted: Option<String>,
    accent: Option<String>,
    user: Option<String>,
    assistant: Option<String>,
    tool: Option<String>,
    error: Option<String>,
    success: Option<String>,
    diff_add: Option<String>,
    diff_del: Option<String>,
    border: Option<String>,
    background: Option<String>,
}

fn load_file(path: &Path) -> Result<Theme> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read theme {}", path.display()))?;
    let file: ThemeFile =
        toml::from_str(&contents).with_context(|| format!("invalid theme {}", path.display()))?;
    let base = Palette::dark();

    let palette = Palette {
        background: parse_or(file.background.as_deref(), base.background, "background")?,
        foreground: parse_or(file.foreground.as_deref(), base.foreground, "foreground")?,
        muted: parse_or(file.muted.as_deref(), base.muted, "muted")?,
        accent: parse_or(file.accent.as_deref(), base.accent, "accent")?,
        user: parse_or(file.user.as_deref(), base.user, "user")?,
        assistant: parse_or(file.assistant.as_deref(), base.assistant, "assistant")?,
        tool: parse_or(file.tool.as_deref(), base.tool, "tool")?,
        error: parse_or(file.error.as_deref(), base.error, "error")?,
        success: parse_or(file.success.as_deref(), base.success, "success")?,
        diff_add: parse_or(file.diff_add.as_deref(), base.diff_add, "diff_add")?,
        diff_del: parse_or(file.diff_del.as_deref(), base.diff_del, "diff_del")?,
        border: parse_or(file.border.as_deref(), base.border, "border")?,
    };

    if contrast_ratio(palette.foreground, palette.background) < 4.5 {
        bail!(
            "theme {} has insufficient body contrast ({:.2}:1, minimum 4.5:1)",
            path.display(),
            contrast_ratio(palette.foreground, palette.background)
        );
    }

    Ok(Theme {
        name: path.display().to_string(),
        palette,
    })
}

fn parse_or(value: Option<&str>, fallback: Color, key: &str) -> Result<Color> {
    match value {
        Some(value) => parse_hex(value).with_context(|| format!("invalid `{key}` color")),
        None => Ok(fallback),
    }
}

/// Parses `#rrggbb` into an RGB color.
///
/// # Errors
///
/// Returns an error for a malformed hex string.
pub fn parse_hex(value: &str) -> Result<Color> {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        bail!("expected `#rrggbb`, got `{value}`");
    }
    let red = u8::from_str_radix(&hex[0..2], 16).context("invalid red channel")?;
    let green = u8::from_str_radix(&hex[2..4], 16).context("invalid green channel")?;
    let blue = u8::from_str_radix(&hex[4..6], 16).context("invalid blue channel")?;
    Ok(Color::Rgb(red, green, blue))
}

/// Returns the WCAG contrast ratio between two colors.
#[must_use]
pub fn contrast_ratio(first: Color, second: Color) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    let (lighter, darker) = if first > second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}

fn luminance(color: Color) -> f64 {
    let (red, green, blue) = match color {
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::Black => (0, 0, 0),
        Color::White => (255, 255, 255),
        _ => (128, 128, 128),
    };
    let channel = |value: u8| {
        let value = f64::from(value) / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[test]
    fn builtin_palettes_meet_body_contrast() {
        for theme in [
            Theme::load(Some("dark")).unwrap(),
            Theme::load(Some("light")).unwrap(),
        ] {
            assert!(
                theme.body_contrast() >= 4.5,
                "{} contrast {:.2}",
                theme.name,
                theme.body_contrast()
            );
        }
    }

    #[test]
    #[spec("TUI-008")]
    fn custom_theme_is_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(
            &path,
            "background = \"#101014\"\nforeground = \"#eeeeee\"\naccent = \"#ff0000\"\n",
        )
        .unwrap();
        let theme = Theme::load(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(theme.palette.accent, Color::Rgb(255, 0, 0));
        assert_eq!(theme.palette.background, Color::Rgb(16, 16, 20));
    }

    #[test]
    fn low_contrast_custom_theme_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(
            &path,
            "background = \"#777777\"\nforeground = \"#888888\"\n",
        )
        .unwrap();
        let error = Theme::load(Some(path.to_str().unwrap())).unwrap_err();
        assert!(error.to_string().contains("contrast"));
    }

    #[test]
    fn hex_parser_rejects_malformed_values() {
        assert!(parse_hex("nope").is_err());
        assert!(parse_hex("#12345").is_err());
        assert!(parse_hex("#zzzzzz").is_err());
    }
}
