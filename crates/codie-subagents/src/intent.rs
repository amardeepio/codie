//! Deterministic delegation intent and mode resolution.
//!
//! This is a phrase detector, not a classifier: the same prompt always yields
//! the same result, and the patterns are frozen by `AGENT-029`.

use std::sync::LazyLock;

use regex::Regex;

/// How hard the host should push delegation for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DelegationMode {
    /// Delegation is refused for the session.
    Off,
    /// The model decides whether to call `task`.
    #[default]
    Auto,
    /// The model is told to delegate; one follow-up may nudge it.
    Force,
}

impl DelegationMode {
    /// Returns the wire and configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "auto",
            Self::Force => "force",
        }
    }

    /// Parses a mode, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "disable" | "disabled" => Some(Self::Off),
            "on" | "auto" | "automatic" => Some(Self::Auto),
            "force" | "forced" | "always" => Some(Self::Force),
            _ => None,
        }
    }
}

/// What the user's prompt asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DelegationIntent {
    /// No delegation phrase was found.
    #[default]
    None,
    /// The prompt asked to use subagents.
    Requested,
    /// The prompt asked not to use subagents.
    Suppressed,
}

/// The effective mode for one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DelegationPlan {
    /// Mode after applying precedence.
    pub mode: DelegationMode,
    /// What the prompt detector found.
    pub intent: DelegationIntent,
}

impl DelegationPlan {
    /// Returns true when the run should be nudged to delegate.
    #[must_use]
    pub const fn is_forced(self) -> bool {
        matches!(self.mode, DelegationMode::Force)
    }
}

/// Characters inspected at the start of a prompt.
pub const INTENT_WINDOW: usize = 500;

/// Persisted system message appended when delegation is forced (`PRMPT-014`).
pub const FORCED_DIRECTIVE: &str = "<delegation mode=\"force\">The user requested delegation. Prefer the \
`task` tool for independent subtasks. Delegation is faster only when subtasks are independent; do not delegate \
concurrent edits to the same files.</delegation>";

/// Follow-up sent once when a forced run does not delegate on its first turn.
pub const FORCED_NUDGE: &str = "You were asked to use subagents for this task. Call the `task` tool \
for the independent parts now, or explain briefly why delegation is not appropriate.";

/// Tracks the one follow-up allowed when delegation is forced (`AGENT-030`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NudgeState {
    used: bool,
}

impl NudgeState {
    /// Allows one more nudge, for example after a new prompt.
    pub fn reset(&mut self) {
        self.used = false;
    }

    /// Returns the follow-up when a forced run did not delegate, at most once.
    pub fn after_turn(&mut self, forced: bool, delegated: bool) -> Option<&'static str> {
        if forced && !delegated && !self.used {
            self.used = true;
            Some(FORCED_NUDGE)
        } else {
            None
        }
    }
}

/// Returns the persisted directive for a mode change.
#[must_use]
pub fn mode_directive(mode: DelegationMode) -> &'static str {
    match mode {
        DelegationMode::Off => {
            "<delegation mode=\"off\">Subagents are disabled for this session; do not call `task`.</delegation>"
        }
        DelegationMode::Auto => {
            "<delegation mode=\"auto\">Delegate only self-contained, independent subtasks.</delegation>"
        }
        DelegationMode::Force => FORCED_DIRECTIVE,
    }
}

static NEGATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:don'?t|do\s+not|dont|never|no|without|avoid)\s+(?:use\s+|using\s+|the\s+|any\s+|more\s+)*(?:sub-?agents?|agents?|delegat(?:e|ion|ing))\b",
    )
    .expect("valid negation pattern")
});

static AFFIRMATIVE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\buse\s+(?:the\s+)?sub-?agents?\b",
        r"\bwith\s+(?:the\s+)?sub-?agents?\b",
        r"\buse\s+(?:multiple|parallel)\s+agents?\b",
        r"\b(?:spawn|launch|start)\s+(?:the\s+|some\s+|multiple\s+)?(?:sub-?)?agents?\b",
        r"\bdelegate\s+(?:this|it|the\s+task|the\s+work)\b",
        r"\brun\s+(?:it|this|these)\s+in\s+parallel\b",
        r"\bparallel(?:ize|ise)\s+(?:this|the\s+task|the\s+work)\b",
        r"\bsplit\s+(?:this|the\s+task|the\s+work)\s+(?:in|into)\s+(?:parallel\s+)?agents?\b",
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("valid affirmative pattern"))
    .collect()
});

/// Detects delegation intent in user text.
///
/// Only the first [`INTENT_WINDOW`] characters are inspected, and fenced code
/// blocks are ignored. Negation wins over any affirmative phrase.
#[must_use]
pub fn detect_intent(prompt: &str) -> DelegationIntent {
    let without_code = strip_fenced_code(prompt);
    let window: String = without_code.chars().take(INTENT_WINDOW).collect();
    let lowered = window.to_ascii_lowercase();

    if NEGATION.is_match(&lowered) {
        return DelegationIntent::Suppressed;
    }
    if AFFIRMATIVE.iter().any(|pattern| pattern.is_match(&lowered)) {
        return DelegationIntent::Requested;
    }
    DelegationIntent::None
}

/// Resolves the effective mode using the precedence in `AGENT-028`.
#[must_use]
pub fn effective_mode(
    enabled: bool,
    session_mode: DelegationMode,
    force_once: bool,
    intent: DelegationIntent,
) -> DelegationPlan {
    let mode = if !enabled {
        DelegationMode::Off
    } else {
        match intent {
            DelegationIntent::Suppressed => DelegationMode::Off,
            DelegationIntent::Requested => DelegationMode::Force,
            DelegationIntent::None => match session_mode {
                DelegationMode::Off => DelegationMode::Off,
                DelegationMode::Force => DelegationMode::Force,
                DelegationMode::Auto if force_once => DelegationMode::Force,
                DelegationMode::Auto => DelegationMode::Auto,
            },
        }
    };
    DelegationPlan { mode, intent }
}

/// Removes fenced code blocks, including their fence lines.
fn strip_fenced_code(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut fence: Option<&str> = None;
    for line in input.lines() {
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if let Some(marker) = marker {
            fence = Some(marker);
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }
    output
}
