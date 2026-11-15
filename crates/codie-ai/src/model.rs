//! Model descriptors, pricing, and the provider compatibility matrix.
//!
//! A [`Model`] carries everything the adapter needs to build a request and to
//! account for its cost. Capabilities live on the model rather than on the
//! provider because one OpenAI-compatible endpoint can front models with
//! different request features (`PROV-019`).

use std::collections::BTreeMap;

use codie_core::ThinkingLevel;

use crate::{ApiId, OPENAI_COMPLETIONS, ProviderId};

/// An ordered map of extra HTTP headers sent with every request.
///
/// Header names are stored as written by the caller. HTTP header names are
/// case-insensitive, so consumers must compare them accordingly.
pub type HeaderMap = BTreeMap<String, String>;

/// The provider identifier used by DeepSeek.
pub const PROVIDER_DEEPSEEK: &str = "deepseek";

/// The provider identifier used by OpenRouter.
pub const PROVIDER_OPENROUTER: &str = "openrouter";

/// The provider identifier used by Google Gemini.
pub const PROVIDER_GEMINI: &str = "gemini";

/// The default `HTTP-Referer` attribution value for OpenRouter.
pub const DEFAULT_HTTP_REFERER: &str = "https://github.com/amardeepio/codie";

/// The default `X-Title` attribution value for OpenRouter.
pub const DEFAULT_X_TITLE: &str = "codie";

/// The request field that carries the maximum output token limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxTokensField {
    /// Send the limit as `max_tokens` (DeepSeek and Gemini).
    MaxTokens,
    /// Send the limit as `max_completion_tokens` (OpenRouter and newer OpenAI).
    MaxCompletionTokens,
}

/// How a provider expects reasoning controls to be encoded (`PROV-011`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingFormat {
    /// Send a top-level `reasoning_effort` string.
    OpenAi,
    /// Send `reasoning: { effort: "..." }`, with `off` mapped to `"none"`.
    OpenRouter,
    /// Send `thinking: { type: "enabled" | "disabled" }` plus an optional
    /// `reasoning_effort`.
    DeepSeek,
}

/// Per-model request capabilities.
///
/// Fields are consulted only by [`crate::openai`]; a capability set to `false`
/// means the corresponding wire field is omitted rather than sent with a null
/// or default value (`PROV-019`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Compat {
    /// The provider accepts the OpenAI `store` request field.
    pub supports_store: bool,
    /// The provider accepts a `developer` role instead of `system`.
    pub supports_developer_role: bool,
    /// The field used for the maximum output token limit.
    pub max_tokens_field: MaxTokensField,
    /// Assistant messages must replay reasoning as `reasoning_content`.
    pub requires_reasoning_content_on_assistant_messages: bool,
    /// How thinking controls are encoded.
    pub thinking_format: ThinkingFormat,
    /// The provider accepts `stream_options: { include_usage: true }`.
    pub supports_usage_in_streaming: bool,
    /// The provider always reports `finish_reason` on a successful stream.
    ///
    /// When this is `false`, an EOF without a finish reason is finalized as a
    /// normal stop instead of a protocol error (`PROV-012`).
    pub supports_finish_reason: bool,
}

impl Compat {
    /// The compatibility set used by DeepSeek.
    #[must_use]
    pub const fn deepseek() -> Self {
        Self {
            supports_store: false,
            supports_developer_role: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            requires_reasoning_content_on_assistant_messages: true,
            thinking_format: ThinkingFormat::DeepSeek,
            supports_usage_in_streaming: true,
            supports_finish_reason: true,
        }
    }

    /// The compatibility set used by OpenRouter.
    #[must_use]
    pub const fn openrouter() -> Self {
        Self {
            supports_store: false,
            supports_developer_role: false,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            requires_reasoning_content_on_assistant_messages: false,
            thinking_format: ThinkingFormat::OpenRouter,
            supports_usage_in_streaming: true,
            supports_finish_reason: true,
        }
    }

    /// The compatibility set used by Google Gemini's OpenAI-compatible endpoint.
    ///
    /// Gemini accepts `max_tokens` (with `max_completion_tokens` as an alias),
    /// streams usage, and controls reasoning with a top-level
    /// `reasoning_effort`. It has no `store` field and no `developer` role.
    #[must_use]
    pub const fn gemini() -> Self {
        Self {
            supports_store: false,
            supports_developer_role: false,
            max_tokens_field: MaxTokensField::MaxTokens,
            requires_reasoning_content_on_assistant_messages: false,
            thinking_format: ThinkingFormat::OpenAi,
            supports_usage_in_streaming: true,
            supports_finish_reason: true,
        }
    }

    /// The compatibility set used by the future native OpenAI format.
    #[must_use]
    pub const fn openai() -> Self {
        Self {
            supports_store: true,
            supports_developer_role: true,
            max_tokens_field: MaxTokensField::MaxCompletionTokens,
            requires_reasoning_content_on_assistant_messages: false,
            thinking_format: ThinkingFormat::OpenAi,
            supports_usage_in_streaming: true,
            supports_finish_reason: true,
        }
    }
}

impl Default for Compat {
    fn default() -> Self {
        Self::openai()
    }
}

/// Model pricing in USD per one million tokens.
///
/// The adapter converts these prices to integer nano-USD when estimating cost
/// (`MSG-006`). Cache prices may be zero when the provider publishes none.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModelCost {
    /// Uncached input price per million tokens.
    pub input: f64,
    /// Output price per million tokens.
    pub output: f64,
    /// Cache-read price per million tokens.
    pub cache_read: f64,
    /// Cache-write price per million tokens.
    pub cache_write: f64,
}

/// A selectable model and everything needed to call it.
#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// Model identifier used verbatim on the wire.
    pub id: String,
    /// Human-readable model name.
    pub name: String,
    /// Provider that serves this model.
    pub provider: ProviderId,
    /// Wire API identifier, normally [`OPENAI_COMPLETIONS`].
    pub api: ApiId,
    /// Base URL of the provider API.
    pub base_url: String,
    /// Maximum context window in tokens.
    pub context_window: u32,
    /// Maximum output tokens, or zero when the provider declares no limit.
    pub max_output_tokens: u32,
    /// True when the model supports reasoning.
    pub reasoning: bool,
    /// True when the provider does not allow reasoning to be disabled.
    pub reasoning_mandatory: bool,
    /// Supported thinking levels and their provider-native effort names.
    ///
    /// A `None` value means the level is supported but the provider accepts no
    /// explicit effort name for it; the adapter then either omits the effort or
    /// falls back to the level spelling, depending on `thinking_format`.
    pub thinking_levels: BTreeMap<ThinkingLevel, Option<String>>,
    /// Pricing used for cost estimation.
    pub cost: ModelCost,
    /// Request capabilities for this model.
    pub compat: Compat,
    /// Extra HTTP headers for this model.
    pub headers: HeaderMap,
}

impl Model {
    /// Returns the provider-native thinking effort for `level`.
    ///
    /// Unsupported levels are clamped down to the nearest lower supported
    /// level. `off` always returns `None`.
    #[must_use]
    pub fn thinking_effort(&self, level: ThinkingLevel) -> Option<String> {
        if level == ThinkingLevel::Off {
            return None;
        }
        let effective = if self.thinking_levels.contains_key(&level) {
            level
        } else {
            self.thinking_levels
                .keys()
                .rev()
                .copied()
                .find(|candidate| *candidate < level && *candidate != ThinkingLevel::Off)?
        };
        Some(
            self.thinking_levels
                .get(&effective)
                .cloned()
                .flatten()
                .unwrap_or_else(|| effective.as_str().to_owned()),
        )
    }

    /// Returns true when the model supports reasoning at `level`.
    #[must_use]
    pub fn supports_thinking_level(&self, level: ThinkingLevel) -> bool {
        (level == ThinkingLevel::Off && !self.reasoning_mandatory)
            || self.thinking_levels.contains_key(&level)
    }
}

fn provider_id(value: &str) -> ProviderId {
    ProviderId::Owned(value.to_owned())
}

fn api_id() -> ApiId {
    ApiId::Borrowed(OPENAI_COMPLETIONS)
}

fn deepseek_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Off, None),
        (ThinkingLevel::Minimal, Some("minimal".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, None),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

fn openrouter_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Off, Some("none".to_owned())),
        (ThinkingLevel::Minimal, Some("minimal".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

/// Gemini 4 reasoning levels. `minimal` is mapped to `low` by the provider.
fn gemini4_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Minimal, Some("low".to_owned())),
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

/// Gemini 4 Hydrogen additionally accepts `xhigh`.
fn gemini4_hydrogen_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    let mut levels = gemini4_thinking_levels();
    levels.insert(ThinkingLevel::XHigh, Some("xhigh".to_owned()));
    levels
}

/// Gemini 3 reasoning levels. `minimal` is rejected by these models.
fn gemini3_thinking_levels() -> BTreeMap<ThinkingLevel, Option<String>> {
    BTreeMap::from([
        (ThinkingLevel::Low, Some("low".to_owned())),
        (ThinkingLevel::Medium, Some("medium".to_owned())),
        (ThinkingLevel::High, Some("high".to_owned())),
    ])
}

/// Returns the base URL for a known provider.
#[must_use]
pub fn provider_base_url(provider: &str) -> &'static str {
    if provider.eq_ignore_ascii_case(PROVIDER_DEEPSEEK) {
        "https://api.deepseek.com"
    } else if provider.eq_ignore_ascii_case(PROVIDER_OPENROUTER) {
        "https://openrouter.ai/api/v1"
    } else if provider.eq_ignore_ascii_case(PROVIDER_GEMINI) {
        "https://generativelanguage.googleapis.com/v1beta/openai"
    } else {
        ""
    }
}

/// Returns the compatibility defaults for a known provider.
#[must_use]
pub fn provider_compat(provider: &str) -> Compat {
    if provider.eq_ignore_ascii_case(PROVIDER_DEEPSEEK) {
        Compat::deepseek()
    } else if provider.eq_ignore_ascii_case(PROVIDER_OPENROUTER) {
        Compat::openrouter()
    } else if provider.eq_ignore_ascii_case(PROVIDER_GEMINI) {
        Compat::gemini()
    } else {
        Compat::openai()
    }
}

/// Returns the built-in model catalog for Phase 1.
///
/// DeepSeek pricing is USD per million tokens (`deepseek-flash`:
/// $0.30 input, $1.20 output, $0.006 cache read). `openrouter/auto` is a safe
/// default with unknown (zero) pricing so cost accounting never invents a
/// price for a model OpenRouter has not resolved yet.
#[must_use]
pub fn builtin_models() -> Vec<Model> {
    vec![
        Model {
            id: "deepseek-flash".to_owned(),
            name: "DeepSeek Flash".to_owned(),
            provider: provider_id(PROVIDER_DEEPSEEK),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_DEEPSEEK).to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: deepseek_thinking_levels(),
            cost: ModelCost {
                input: 0.30,
                output: 1.20,
                cache_read: 0.006,
                cache_write: 0.0,
            },
            compat: Compat::deepseek(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek-v4-pro".to_owned(),
            name: "DeepSeek V4 Pro".to_owned(),
            provider: provider_id(PROVIDER_DEEPSEEK),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_DEEPSEEK).to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: deepseek_thinking_levels(),
            cost: ModelCost {
                input: 1.32,
                output: 3.96,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            compat: Compat::deepseek(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "openrouter/auto".to_owned(),
            name: "OpenRouter Auto".to_owned(),
            provider: provider_id(PROVIDER_OPENROUTER),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_OPENROUTER).to_owned(),
            context_window: 128_000,
            max_output_tokens: 8192,
            reasoning: false,
            reasoning_mandatory: false,
            thinking_levels: openrouter_thinking_levels(),
            cost: ModelCost::default(),
            compat: Compat::openrouter(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "deepseek/deepseek-v4.1-flash-1017".to_owned(),
            name: "DeepSeek V4.1 Flash (OpenRouter)".to_owned(),
            provider: provider_id(PROVIDER_OPENROUTER),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_OPENROUTER).to_owned(),
            context_window: 1_000_000,
            max_output_tokens: 384_000,
            reasoning: true,
            reasoning_mandatory: false,
            thinking_levels: openrouter_thinking_levels(),
            // OpenRouter reports the billed cost per request, so the catalog
            // price is only an estimate for offline planning.
            cost: ModelCost::default(),
            compat: Compat::openrouter(),
            headers: HeaderMap::new(),
        },
        // Gemini pricing is USD per million tokens as published on
        // 2026-11-15. Gemini 4 prices double on 2026-12-01; update the
        // catalog when the new prices take effect. Cache writes are billed as
        // storage per hour, so the per-token write price stays zero here.
        Model {
            id: "gemini-4-hydrogen".to_owned(),
            name: "Gemini 4 Hydrogen".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_GEMINI).to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_hydrogen_thinking_levels(),
            cost: ModelCost {
                input: 1.50,
                output: 7.50,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-4-argon".to_owned(),
            name: "Gemini 4 Argon".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_GEMINI).to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_thinking_levels(),
            cost: ModelCost {
                input: 2.00,
                output: 10.00,
                cache_read: 0.10,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-4-carbon".to_owned(),
            name: "Gemini 4 Carbon".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_GEMINI).to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini4_thinking_levels(),
            cost: ModelCost {
                input: 2.00,
                output: 10.00,
                cache_read: 0.10,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-3.8-flash".to_owned(),
            name: "Gemini 3.8 Flash".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_GEMINI).to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini3_thinking_levels(),
            cost: ModelCost {
                input: 0.75,
                output: 3.75,
                cache_read: 0.075,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
        Model {
            id: "gemini-3.5-flash-lite".to_owned(),
            name: "Gemini 3.5 Flash-Lite".to_owned(),
            provider: provider_id(PROVIDER_GEMINI),
            api: api_id(),
            base_url: provider_base_url(PROVIDER_GEMINI).to_owned(),
            context_window: 1_048_576,
            max_output_tokens: 65_536,
            reasoning: true,
            reasoning_mandatory: true,
            thinking_levels: gemini3_thinking_levels(),
            cost: ModelCost {
                input: 0.30,
                output: 2.50,
                cache_read: 0.03,
                cache_write: 0.0,
            },
            compat: Compat::gemini(),
            headers: HeaderMap::new(),
        },
    ]
}

/// Finds a built-in model by provider and identifier.
///
/// The identifier is matched verbatim, which matters for OpenRouter's
/// namespaced IDs (`PROV-020`).
#[must_use]
pub fn find_model(provider: &str, id: &str) -> Option<Model> {
    builtin_models()
        .into_iter()
        .find(|model| model.provider.eq_ignore_ascii_case(provider) && model.id == id)
}

/// Builds a conservative model descriptor for an unknown model identifier.
///
/// The fallback assumes a 128k context window, 8,192 output tokens, no
/// reasoning, and zero pricing. The provider's default compatibility set is
/// used so request gating still behaves sensibly.
#[must_use]
pub fn fallback_model(provider: &str, id: &str) -> Model {
    Model {
        id: id.to_owned(),
        name: id.to_owned(),
        provider: provider_id(provider),
        api: api_id(),
        base_url: provider_base_url(provider).to_owned(),
        context_window: 128_000,
        max_output_tokens: 8192,
        reasoning_mandatory: false,
        reasoning: false,
        thinking_levels: BTreeMap::new(),
        cost: ModelCost::default(),
        compat: provider_compat(provider),
        headers: HeaderMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_contains_the_launch_models() {
        let models = builtin_models();
        assert!(models.iter().any(|model| model.id == "deepseek-flash"));
        assert!(models.iter().any(|model| model.id == "deepseek-v4-pro"));
        assert!(models.iter().any(|model| model.id == "openrouter/auto"));
        assert!(models.iter().any(|model| model.id == "gemini-4-hydrogen"));

        let hydrogen = find_model("gemini", "gemini-4-hydrogen").expect("hydrogen is built in");
        assert_eq!(hydrogen.context_window, 1_048_576);
        assert!(hydrogen.reasoning_mandatory);
        assert_eq!(hydrogen.cost.input, 1.50);
        assert_eq!(
            hydrogen.thinking_effort(ThinkingLevel::XHigh),
            Some("xhigh".into())
        );

        let flash = find_model("deepseek", "deepseek-flash").expect("flash is built in");
        assert_eq!(flash.context_window, 1_000_000);
        assert_eq!(flash.max_output_tokens, 384_000);
        assert!(flash.reasoning);
        assert_eq!(flash.cost.input, 0.30);
        assert_eq!(flash.cost.cache_read, 0.006);
    }

    #[test]
    fn thinking_levels_clamp_down() {
        let flash = find_model("deepseek", "deepseek-flash").unwrap();
        assert_eq!(
            flash.thinking_effort(ThinkingLevel::High),
            Some("high".into())
        );
        assert_eq!(
            flash.thinking_effort(ThinkingLevel::XHigh),
            Some("high".into())
        );
        assert_eq!(
            flash.thinking_effort(ThinkingLevel::Max),
            Some("high".into())
        );
        assert_eq!(flash.thinking_effort(ThinkingLevel::Off), None);
        assert_eq!(
            flash.thinking_effort(ThinkingLevel::Minimal),
            Some("minimal".into())
        );
    }

    #[test]
    fn fallback_is_conservative() {
        let model = fallback_model("deepseek", "future-model");
        assert_eq!(model.context_window, 128_000);
        assert_eq!(model.max_output_tokens, 8192);
        assert_eq!(model.cost, ModelCost::default());
        assert!(!model.reasoning);
    }
}
