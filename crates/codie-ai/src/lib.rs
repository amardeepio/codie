//! Provider abstractions and wire adapters for codie.
//!
//! The public surface is intentionally small in Phase 0: provider and API
//! identifiers plus the API used by every v1.0 provider. The `Provider` trait,
//! model catalog, auth, and streaming land in Phase 1 behind the contract in
//! `specs/provider/`.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::borrow::Cow;

/// A provider identifier such as `deepseek` or `openrouter`.
pub type ProviderId = Cow<'static, str>;

/// A wire-protocol identifier such as `openai-completions`.
pub type ApiId = Cow<'static, str>;

/// The OpenAI-compatible chat-completions API used by every v1.0 provider.
pub const OPENAI_COMPLETIONS: &str = "openai-completions";

/// Returns the wire API used by every provider supported in v1.0.
#[must_use]
pub const fn default_api() -> &'static str {
    OPENAI_COMPLETIONS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_uses_openai_completions() {
        assert_eq!(default_api(), OPENAI_COMPLETIONS);
        assert_eq!(default_api(), "openai-completions");
    }
}
