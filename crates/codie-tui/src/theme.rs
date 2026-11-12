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
    /// Raised surfaces such as user messages.
    pub surface: Color,
    /// Stronger surfaces such as inline code.
    pub surface_alt: Color,
    /// Primary body text.
    pub foreground: Color,
    /// Secondary text and metadata.
    pub muted: Color,
    /// Primary interactive color: prompts, focus, and selection.
    pub primary: Color,
    /// Secondary interactive color: models and tool summaries.
    pub secondary: Color,
    /// Accent for headings and code keywords.
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
    /// Warnings and strong emphasis.
    pub warning: Color,
    /// Informational text and ordered-list markers.
    pub info: Color,
    /// Added diff lines.
    pub diff_add: Color,
    /// Removed diff lines.
    pub diff_del: Color,
    /// Borders and separators.
    pub border: Color,
    /// Focused borders and active separators.
    pub border_active: Color,
    /// Low-contrast borders.
    pub border_subtle: Color,
    /// Markdown headings.
    pub heading: Color,
    /// Markdown links.
    pub link: Color,
    /// Inline code and code strings.
    pub code: Color,
    /// Markdown strong text.
    pub strong: Color,
    /// Markdown emphasis and syntax types.
    pub emphasis: Color,
    /// Block quotes.
    pub quote: Color,
    /// Unordered-list markers.
    pub list: Color,
    /// Ordered-list numbers.
    pub list_enum: Color,
    /// Horizontal rules.
    pub rule: Color,
}

impl Palette {
    /// The default dark palette.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            background: Color::Rgb(10, 10, 10),
            surface: Color::Rgb(20, 20, 20),
            surface_alt: Color::Rgb(30, 30, 30),
            foreground: Color::Rgb(238, 238, 238),
            muted: Color::Rgb(128, 128, 128),
            primary: Color::Rgb(250, 178, 131),
            secondary: Color::Rgb(92, 156, 245),
            accent: Color::Rgb(157, 124, 216),
            user: Color::Rgb(238, 238, 238),
            assistant: Color::Rgb(238, 238, 238),
            tool: Color::Rgb(92, 156, 245),
            error: Color::Rgb(224, 108, 117),
            success: Color::Rgb(127, 216, 143),
            warning: Color::Rgb(245, 167, 66),
            info: Color::Rgb(86, 182, 194),
            diff_add: Color::Rgb(79, 214, 190),
            diff_del: Color::Rgb(197, 59, 83),
            border: Color::Rgb(72, 72, 72),
            border_active: Color::Rgb(96, 96, 96),
            border_subtle: Color::Rgb(60, 60, 60),
            heading: Color::Rgb(157, 124, 216),
            link: Color::Rgb(250, 178, 131),
            code: Color::Rgb(127, 216, 143),
            strong: Color::Rgb(245, 167, 66),
            emphasis: Color::Rgb(229, 192, 123),
            quote: Color::Rgb(229, 192, 123),
            list: Color::Rgb(250, 178, 131),
            list_enum: Color::Rgb(86, 182, 194),
            rule: Color::Rgb(128, 128, 128),
        }
    }

    /// The default light palette.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            background: Color::Rgb(255, 255, 255),
            surface: Color::Rgb(250, 250, 250),
            surface_alt: Color::Rgb(245, 245, 245),
            foreground: Color::Rgb(26, 26, 26),
            muted: Color::Rgb(107, 107, 107),
            primary: Color::Rgb(41, 104, 195),
            secondary: Color::Rgb(123, 91, 182),
            accent: Color::Rgb(138, 90, 0),
            user: Color::Rgb(26, 26, 26),
            assistant: Color::Rgb(26, 26, 26),
            tool: Color::Rgb(123, 91, 182),
            error: Color::Rgb(209, 56, 61),
            success: Color::Rgb(47, 125, 69),
            warning: Color::Rgb(168, 95, 0),
            info: Color::Rgb(31, 111, 122),
            diff_add: Color::Rgb(30, 114, 92),
            diff_del: Color::Rgb(197, 59, 83),
            border: Color::Rgb(184, 184, 184),
            border_active: Color::Rgb(160, 160, 160),
            border_subtle: Color::Rgb(212, 212, 212),
            heading: Color::Rgb(138, 90, 0),
            link: Color::Rgb(41, 104, 195),
            code: Color::Rgb(47, 125, 69),
            strong: Color::Rgb(168, 95, 0),
            emphasis: Color::Rgb(138, 100, 16),
            quote: Color::Rgb(138, 100, 16),
            list: Color::Rgb(41, 104, 195),
            list_enum: Color::Rgb(31, 111, 122),
            rule: Color::Rgb(107, 107, 107),
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

/// Watches a TOML theme file and reloads it when its modification time changes.
#[derive(Debug, Clone)]
pub struct ThemeWatcher {
    path: std::path::PathBuf,
    modified: Option<std::time::SystemTime>,
}

impl ThemeWatcher {
    /// Creates a watcher for `path`.
    #[must_use]
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        let path = path.into();
        let modified = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok();
        Self { path, modified }
    }

    /// Returns the reloaded theme when the file changed and still parses.
    ///
    /// Invalid edits are rejected and the previous theme stays active.
    #[must_use]
    pub fn poll(&mut self) -> Option<Theme> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|metadata| metadata.modified())
            .ok()?;
        if self.modified == Some(modified) {
            return None;
        }
        self.modified = Some(modified);
        match Theme::load(Some(&self.path.display().to_string())) {
            Ok(theme) => Some(theme),
            Err(error) => {
                tracing::warn!(%error, "theme reload failed; keeping the previous theme");
                None
            }
        }
    }

    /// Returns the watched path.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
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
    warning: Option<String>,
    info: Option<String>,
    diff_add: Option<String>,
    diff_del: Option<String>,
    border: Option<String>,
    border_active: Option<String>,
    border_subtle: Option<String>,
    background: Option<String>,
    surface: Option<String>,
    surface_alt: Option<String>,
    primary: Option<String>,
    secondary: Option<String>,
    heading: Option<String>,
    link: Option<String>,
    code: Option<String>,
    strong: Option<String>,
    emphasis: Option<String>,
    quote: Option<String>,
    list: Option<String>,
    list_enum: Option<String>,
    rule: Option<String>,
}

fn load_file(path: &Path) -> Result<Theme> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read theme {}", path.display()))?;
    let file: ThemeFile =
        toml::from_str(&contents).with_context(|| format!("invalid theme {}", path.display()))?;
    let base = Palette::dark();

    let accent = parse_or(file.accent.as_deref(), base.accent, "accent")?;
    let primary = parse_or(
        file.primary.as_deref(),
        if file.accent.is_some() {
            accent
        } else {
            base.primary
        },
        "primary",
    )?;
    let heading = parse_or(
        file.heading.as_deref(),
        if file.accent.is_some() {
            accent
        } else {
            base.heading
        },
        "heading",
    )?;
    let success = parse_or(file.success.as_deref(), base.success, "success")?;
    let warning = parse_or(file.warning.as_deref(), base.warning, "warning")?;
    let info = parse_or(file.info.as_deref(), base.info, "info")?;
    let muted = parse_or(file.muted.as_deref(), base.muted, "muted")?;

    let palette = Palette {
        background: parse_or(file.background.as_deref(), base.background, "background")?,
        surface: parse_or(file.surface.as_deref(), base.surface, "surface")?,
        surface_alt: parse_or(file.surface_alt.as_deref(), base.surface_alt, "surface_alt")?,
        foreground: parse_or(file.foreground.as_deref(), base.foreground, "foreground")?,
        muted,
        primary,
        secondary: parse_or(file.secondary.as_deref(), base.secondary, "secondary")?,
        accent,
        user: parse_or(file.user.as_deref(), base.user, "user")?,
        assistant: parse_or(file.assistant.as_deref(), base.assistant, "assistant")?,
        tool: parse_or(file.tool.as_deref(), base.tool, "tool")?,
        error: parse_or(file.error.as_deref(), base.error, "error")?,
        success,
        warning,
        info,
        diff_add: parse_or(file.diff_add.as_deref(), base.diff_add, "diff_add")?,
        diff_del: parse_or(file.diff_del.as_deref(), base.diff_del, "diff_del")?,
        border: parse_or(file.border.as_deref(), base.border, "border")?,
        border_active: parse_or(
            file.border_active.as_deref(),
            base.border_active,
            "border_active",
        )?,
        border_subtle: parse_or(
            file.border_subtle.as_deref(),
            base.border_subtle,
            "border_subtle",
        )?,
        heading,
        link: parse_or(file.link.as_deref(), primary, "link")?,
        code: parse_or(file.code.as_deref(), success, "code")?,
        strong: parse_or(file.strong.as_deref(), warning, "strong")?,
        emphasis: parse_or(file.emphasis.as_deref(), warning, "emphasis")?,
        quote: parse_or(file.quote.as_deref(), warning, "quote")?,
        list: parse_or(file.list.as_deref(), primary, "list")?,
        list_enum: parse_or(file.list_enum.as_deref(), info, "list_enum")?,
        rule: parse_or(file.rule.as_deref(), muted, "rule")?,
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
    fn builtin_text_colors_meet_contrast() {
        for theme in [
            Theme::load(Some("dark")).unwrap(),
            Theme::load(Some("light")).unwrap(),
        ] {
            let palette = theme.palette;
            for color in [
                palette.foreground,
                palette.muted,
                palette.primary,
                palette.secondary,
                palette.accent,
                palette.error,
                palette.success,
                palette.warning,
                palette.info,
                palette.heading,
                palette.link,
                palette.code,
                palette.strong,
                palette.emphasis,
                palette.quote,
                palette.list,
                palette.list_enum,
                palette.rule,
            ] {
                let ratio = contrast_ratio(color, palette.background);
                assert!(
                    ratio >= 4.5,
                    "{} color {color:?} contrast {ratio:.2}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn custom_theme_parses_semantic_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(
            &path,
            "primary = \"#ff0000\"\nheading = \"#00ff00\"\nsurface = \"#010203\"\nstrong = \"#040506\"\n",
        )
        .unwrap();
        let theme = Theme::load(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(theme.palette.primary, Color::Rgb(255, 0, 0));
        assert_eq!(theme.palette.heading, Color::Rgb(0, 255, 0));
        assert_eq!(theme.palette.surface, Color::Rgb(1, 2, 3));
        assert_eq!(theme.palette.strong, Color::Rgb(4, 5, 6));
    }

    #[test]
    fn custom_accent_drives_primary_and_heading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(&path, "accent = \"#ff0000\"\n").unwrap();
        let theme = Theme::load(Some(path.to_str().unwrap())).unwrap();
        assert_eq!(theme.palette.primary, Color::Rgb(255, 0, 0));
        assert_eq!(theme.palette.heading, Color::Rgb(255, 0, 0));
    }

    #[test]
    #[spec("TUI-008")]
    fn spec_tui_008_theme_hot_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("theme.toml");
        std::fs::write(&path, "accent = \"#ff0000\"\n").unwrap();
        let mut watcher = ThemeWatcher::new(&path);
        assert!(
            watcher.poll().is_none(),
            "an unchanged file must not reload"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "accent = \"#00ff00\"\n").unwrap();
        let theme = watcher.poll().expect("the changed theme reloads");
        assert_eq!(theme.palette.accent, Color::Rgb(0, 255, 0));
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
