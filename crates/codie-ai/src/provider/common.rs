//! Helpers shared by provider declarations.

use std::collections::BTreeMap;

use codie_core::ThinkingLevel;

use crate::model::HeaderMap;

/// An empty request-header map.
pub fn empty_headers() -> HeaderMap {
    HeaderMap::new()
}

/// Builds OpenAI-style thinking levels.
///
/// `efforts` lists the provider-native names when the catalog declares them.
/// An empty list means the provider accepts the usual
/// `minimal`/`low`/`medium`/`high` names. A non-reasoning model gets no levels.
pub fn thinking_levels(
    reasoning: bool,
    efforts: &[&str],
) -> BTreeMap<ThinkingLevel, Option<String>> {
    if !reasoning {
        return BTreeMap::new();
    }

    if efforts.is_empty() {
        return BTreeMap::from([
            (ThinkingLevel::Off, None),
            (ThinkingLevel::Minimal, Some("minimal".to_owned())),
            (ThinkingLevel::Low, Some("low".to_owned())),
            (ThinkingLevel::Medium, Some("medium".to_owned())),
            (ThinkingLevel::High, Some("high".to_owned())),
        ]);
    }

    let mut levels = BTreeMap::from([(ThinkingLevel::Off, None)]);
    for effort in efforts {
        if let Some(level) = ThinkingLevel::parse(effort) {
            levels.insert(level, Some((*effort).to_owned()));
        }
    }
    if levels.len() == 1 {
        // The provider reported names this build does not map; keep the usual
        // levels rather than offering no reasoning control at all.
        levels.extend([
            (ThinkingLevel::Minimal, Some("minimal".to_owned())),
            (ThinkingLevel::Low, Some("low".to_owned())),
            (ThinkingLevel::Medium, Some("medium".to_owned())),
            (ThinkingLevel::High, Some("high".to_owned())),
        ]);
    }
    levels
}
