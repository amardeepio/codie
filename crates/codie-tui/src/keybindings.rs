//! Configurable keybindings (`TUI-007`).
//!
//! Defaults match `docs/src/tui.md`. A `keybindings.toml` file may override any
//! action; an empty list unbinds it. Unknown action names are rejected so a
//! typo cannot silently disable a key.
//!
//! ```toml
//! [keys]
//! submit = ["ctrl+enter"]
//! newline = ["shift+enter", "ctrl+j"]
//! quit = []
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

/// An action that can be bound to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionId {
    /// Submit the editor contents.
    Submit,
    /// Insert a newline.
    Newline,
    /// Queue a follow-up message.
    FollowUp,
    /// Cancel the run, clear the editor, or quit.
    Cancel,
    /// Quit immediately.
    Quit,
    /// Expand the last tool block.
    ExpandTool,
    /// Undo an edit.
    Undo,
    /// Redo an edit.
    Redo,
    /// Kill to the end of the line.
    KillLine,
    /// Yank the kill ring.
    Yank,
    /// Recall the previous prompt.
    HistoryPrevious,
    /// Recall the next prompt.
    HistoryNext,
    /// Scroll the transcript up.
    ScrollUp,
    /// Scroll the transcript down.
    ScrollDown,
    /// Complete a slash command or file reference.
    Complete,
    /// Show command help.
    Help,
    /// Show the keybinding reference.
    Hotkeys,
    /// Toggle automatic approval for mutating tools.
    ToggleAutoApprove,
}

impl ActionId {
    /// Every action in display order.
    pub const ALL: [Self; 18] = [
        Self::Submit,
        Self::Newline,
        Self::FollowUp,
        Self::Cancel,
        Self::Quit,
        Self::ExpandTool,
        Self::Undo,
        Self::Redo,
        Self::KillLine,
        Self::Yank,
        Self::HistoryPrevious,
        Self::HistoryNext,
        Self::ScrollUp,
        Self::ScrollDown,
        Self::Complete,
        Self::Help,
        Self::Hotkeys,
        Self::ToggleAutoApprove,
    ];

    /// Returns the configuration name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Submit => "submit",
            Self::Newline => "newline",
            Self::FollowUp => "follow_up",
            Self::Cancel => "cancel",
            Self::Quit => "quit",
            Self::ExpandTool => "expand_tool",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::KillLine => "kill_line",
            Self::Yank => "yank",
            Self::HistoryPrevious => "history_previous",
            Self::HistoryNext => "history_next",
            Self::ScrollUp => "scroll_up",
            Self::ScrollDown => "scroll_down",
            Self::Complete => "complete",
            Self::Help => "help",
            Self::Hotkeys => "hotkeys",
            Self::ToggleAutoApprove => "toggle_auto_approve",
        }
    }

    /// Returns the human-readable label used by `/hotkeys`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Submit => "submit (steer while streaming)",
            Self::Newline => "insert a newline",
            Self::FollowUp => "queue a follow-up",
            Self::Cancel => "cancel the run / clear / quit",
            Self::Quit => "quit",
            Self::ExpandTool => "expand the last tool block",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::KillLine => "kill to end of line",
            Self::Yank => "yank",
            Self::HistoryPrevious => "previous prompt",
            Self::HistoryNext => "next prompt",
            Self::ScrollUp => "scroll up",
            Self::ScrollDown => "scroll down",
            Self::Complete => "complete",
            Self::Help => "help",
            Self::Hotkeys => "keybindings",
            Self::ToggleAutoApprove => "toggle auto-approve (full access)",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
    }
}

/// A normalized key combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    /// Key code.
    pub code: KeyCode,
    /// Active modifiers.
    pub modifiers: KeyModifiers,
}

impl Key {
    /// Parses a spelling such as `ctrl+o`, `shift+enter`, or `f1`.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown key or modifier.
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim().to_ascii_lowercase();
        if value.is_empty() {
            bail!("empty key binding");
        }
        let mut parts = value.split('+').map(str::trim).collect::<Vec<_>>();
        let Some(name) = parts.pop() else {
            bail!("empty key binding");
        };
        let mut modifiers = KeyModifiers::NONE;
        for modifier in parts {
            match modifier {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "alt" | "option" | "meta" => modifiers |= KeyModifiers::ALT,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                "super" | "cmd" | "command" => modifiers |= KeyModifiers::SUPER,
                other => bail!("unknown modifier `{other}` in `{value}`"),
            }
        }
        let code = match name {
            "enter" | "return" => KeyCode::Enter,
            "tab" => KeyCode::Tab,
            "backtab" | "back_tab" => KeyCode::BackTab,
            "esc" | "escape" => KeyCode::Esc,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pgup" | "pageup" | "page_up" => KeyCode::PageUp,
            "pgdn" | "pagedown" | "page_down" => KeyCode::PageDown,
            "f1" => KeyCode::F(1),
            "f2" => KeyCode::F(2),
            "f3" => KeyCode::F(3),
            "f4" => KeyCode::F(4),
            "f5" => KeyCode::F(5),
            "f6" => KeyCode::F(6),
            "f7" => KeyCode::F(7),
            "f8" => KeyCode::F(8),
            "f9" => KeyCode::F(9),
            "f10" => KeyCode::F(10),
            "f11" => KeyCode::F(11),
            "f12" => KeyCode::F(12),
            name => {
                let mut characters = name.chars();
                match (characters.next(), characters.next()) {
                    (Some(character), None) => {
                        if modifiers.contains(KeyModifiers::SHIFT) && character.is_alphabetic() {
                            KeyCode::Char(character.to_ascii_uppercase())
                        } else {
                            KeyCode::Char(character)
                        }
                    }
                    _ => bail!("unknown key `{name}`"),
                }
            }
        };
        Ok(Self { code, modifiers })
    }

    /// Returns true when this binding matches a key event.
    #[must_use]
    pub fn matches(&self, event: &KeyEvent) -> bool {
        self.code == event.code && self.modifiers == event.modifiers
    }

    /// Returns a display spelling such as `Ctrl+O`.
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_owned());
        }
        if self.modifiers.contains(KeyModifiers::SUPER) {
            parts.push("Super".to_owned());
        }
        let key = match self.code {
            KeyCode::Enter => "Enter".to_owned(),
            KeyCode::Tab => "Tab".to_owned(),
            KeyCode::Esc => "Esc".to_owned(),
            KeyCode::Backspace => "Backspace".to_owned(),
            KeyCode::Delete => "Del".to_owned(),
            KeyCode::Insert => "Ins".to_owned(),
            KeyCode::Up => "Up".to_owned(),
            KeyCode::Down => "Down".to_owned(),
            KeyCode::Left => "Left".to_owned(),
            KeyCode::Right => "Right".to_owned(),
            KeyCode::Home => "Home".to_owned(),
            KeyCode::End => "End".to_owned(),
            KeyCode::PageUp => "PgUp".to_owned(),
            KeyCode::PageDown => "PgDn".to_owned(),
            KeyCode::F(number) => format!("F{number}"),
            KeyCode::Char(' ') => "Space".to_owned(),
            KeyCode::Char(character) => character.to_string(),
            other => format!("{other:?}"),
        };
        parts.push(key);
        parts.join("+")
    }
}

/// The active keymap.
#[derive(Debug, Clone)]
pub struct Bindings {
    map: BTreeMap<ActionId, Vec<Key>>,
}

impl Default for Bindings {
    fn default() -> Self {
        let mut map = BTreeMap::new();
        let mut bind = |action: ActionId, keys: &[&str]| {
            map.insert(
                action,
                keys.iter()
                    .map(|key| Key::parse(key).expect("default binding is valid"))
                    .collect(),
            );
        };
        bind(ActionId::Submit, &["enter"]);
        bind(ActionId::Newline, &["shift+enter", "ctrl+j"]);
        bind(ActionId::FollowUp, &["alt+enter"]);
        bind(ActionId::Cancel, &["ctrl+c"]);
        bind(ActionId::Quit, &["ctrl+d"]);
        bind(ActionId::ExpandTool, &["ctrl+o"]);
        bind(ActionId::Undo, &["ctrl+z"]);
        bind(ActionId::Redo, &["ctrl+shift+z"]);
        bind(ActionId::KillLine, &["ctrl+k"]);
        bind(ActionId::Yank, &["ctrl+y"]);
        bind(ActionId::HistoryPrevious, &["ctrl+p"]);
        bind(ActionId::HistoryNext, &["ctrl+n"]);
        bind(ActionId::ScrollUp, &["pgup"]);
        bind(ActionId::ScrollDown, &["pgdn"]);
        bind(ActionId::Complete, &["tab"]);
        bind(ActionId::Help, &["f1"]);
        bind(ActionId::Hotkeys, &["f2"]);
        bind(ActionId::ToggleAutoApprove, &["shift+tab", "backtab"]);
        Self { map }
    }
}

impl Bindings {
    /// Loads defaults, then applies `path` when it exists.
    ///
    /// # Errors
    ///
    /// Returns an error for an unreadable file, unknown action, or unknown key.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let mut bindings = Self::default();
        let Some(path) = path.filter(|path| path.exists()) else {
            return Ok(bindings);
        };
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read keybindings {}", path.display()))?;
        let file: KeyFile =
            toml::from_str(&contents).with_context(|| format!("invalid {}", path.display()))?;
        for (name, keys) in file.keys {
            let action = ActionId::parse(&name)
                .with_context(|| format!("unknown keybinding action `{name}`"))?;
            let parsed = keys
                .iter()
                .map(|key| Key::parse(key))
                .collect::<Result<Vec<_>>>()
                .with_context(|| format!("invalid key for `{name}`"))?;
            bindings.map.insert(action, parsed);
        }
        Ok(bindings)
    }

    /// Returns the action bound to `event`, if any.
    #[must_use]
    pub fn action_for(&self, event: &KeyEvent) -> Option<ActionId> {
        ActionId::ALL.into_iter().find(|action| {
            self.map
                .get(action)
                .is_some_and(|keys| keys.iter().any(|key| key.matches(event)))
        })
    }

    /// Returns the keys bound to `action`.
    #[must_use]
    pub fn keys(&self, action: ActionId) -> &[Key] {
        self.map.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Returns the `/hotkeys` reference text.
    #[must_use]
    pub fn describe(&self) -> String {
        ActionId::ALL
            .into_iter()
            .map(|action| {
                let keys = self.keys(action);
                let keys = if keys.is_empty() {
                    "(unbound)".to_owned()
                } else {
                    keys.iter().map(Key::display).collect::<Vec<_>>().join(", ")
                };
                format!("{:<18} {}", keys, action.label())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    #[serde(default)]
    keys: BTreeMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;

    #[test]
    #[spec("TUI-007")]
    fn spec_tui_007_default_bindings_and_unbind() {
        let bindings = Bindings::default();
        let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(bindings.action_for(&enter), Some(ActionId::Submit));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keybindings.toml");
        std::fs::write(&path, "[keys]\nsubmit = [\"ctrl+enter\"]\nquit = []\n").unwrap();
        let bindings = Bindings::load(Some(&path)).unwrap();
        assert_eq!(bindings.action_for(&enter), None);
        assert_eq!(
            bindings.action_for(&KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
            Some(ActionId::Submit)
        );
        assert!(bindings.keys(ActionId::Quit).is_empty());
        assert!(bindings.describe().contains("(unbound)"));
    }

    #[test]
    #[spec("TUI-007")]
    fn spec_tui_007_unknown_actions_and_keys_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "[keys]\nnope = [\"enter\"]\n").unwrap();
        assert!(Bindings::load(Some(&path)).is_err());

        std::fs::write(&path, "[keys]\nsubmit = [\"hyper+enter\"]\n").unwrap();
        assert!(Bindings::load(Some(&path)).is_err());
    }
}
