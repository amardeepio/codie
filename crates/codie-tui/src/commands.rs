//! Slash-command parsing (`TUI-003`, Phase 2 command set).

use codie_core::ThinkingLevel;

/// A parsed slash command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show key help.
    Help,
    /// Show keybindings.
    Hotkeys,
    /// Quit the interactive session.
    Quit,
    /// Start a new session.
    New,
    /// Show the current session.
    Session,
    /// Change or report the model.
    Model(Option<String>),
    /// Change or report the thinking level.
    Thinking(Option<String>),
    /// Manual compaction (Phase 3).
    Compact(Option<String>),
    /// An unknown command.
    Unknown(String),
}

/// Parses one line as a slash command.
///
/// Returns `None` when the input is not a slash command.
#[must_use]
pub fn parse(input: &str) -> Option<Command> {
    let input = input.trim();
    let rest = input.strip_prefix('/')?;
    if rest.is_empty() {
        return Some(Command::Unknown(String::new()));
    }
    let (name, argument) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(name, argument)| (name, argument.trim()));
    let argument = (!argument.is_empty()).then(|| argument.to_owned());

    Some(match name {
        "help" | "?" => Command::Help,
        "hotkeys" => Command::Hotkeys,
        "quit" | "exit" | "q" => Command::Quit,
        "new" => Command::New,
        "session" => Command::Session,
        "model" => Command::Model(argument),
        "thinking" => Command::Thinking(argument),
        "compact" => Command::Compact(argument),
        other => Command::Unknown(other.to_owned()),
    })
}

/// Parses and validates a thinking level argument.
#[must_use]
pub fn parse_thinking(value: &str) -> Option<ThinkingLevel> {
    ThinkingLevel::parse(value.trim())
}

/// Returns the slash-command names used for autocomplete.
#[must_use]
pub fn names() -> &'static [&'static str] {
    &[
        "/help",
        "/hotkeys",
        "/quit",
        "/new",
        "/session",
        "/model",
        "/thinking",
        "/compact",
    ]
}

/// Returns command names that start with `prefix`.
#[must_use]
pub fn completions(prefix: &str) -> Vec<&'static str> {
    names()
        .iter()
        .copied()
        .filter(|name| name.starts_with(prefix))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_with_and_without_arguments() {
        assert_eq!(parse("/help"), Some(Command::Help));
        assert_eq!(parse("/quit now"), Some(Command::Quit));
        assert_eq!(
            parse("/model openrouter/auto"),
            Some(Command::Model(Some("openrouter/auto".to_owned())))
        );
        assert_eq!(parse("/thinking"), Some(Command::Thinking(None)));
        assert_eq!(
            parse("/compact keep tool results"),
            Some(Command::Compact(Some("keep tool results".to_owned())))
        );
    }

    #[test]
    fn plain_text_is_not_a_command() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse(" /help"), Some(Command::Help));
    }

    #[test]
    fn unknown_commands_are_preserved() {
        assert_eq!(parse("/bogus"), Some(Command::Unknown("bogus".to_owned())));
    }

    #[test]
    fn completions_filter_by_prefix() {
        assert_eq!(completions("/he"), ["/help"]);
        assert!(completions("/").len() >= 6);
    }

    #[test]
    fn thinking_levels_are_validated() {
        assert_eq!(
            parse_thinking("high"),
            Some(codie_core::ThinkingLevel::High)
        );
        assert_eq!(parse_thinking("nope"), None);
    }
}
