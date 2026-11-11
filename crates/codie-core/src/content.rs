//! Message content, usage, and the vocabulary shared across the agent.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Reasoning effort requested for a model that supports thinking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    /// Disable reasoning.
    Off,
    /// Smallest supported budget.
    Minimal,
    /// Low budget.
    Low,
    /// Default budget.
    #[default]
    Medium,
    /// High budget.
    High,
    /// Extra-high budget where supported.
    XHigh,
    /// Maximum budget where supported.
    Max,
}

impl ThinkingLevel {
    /// Returns the wire spelling used in configuration and CLI flags.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Parses a level, returning `None` for unknown values.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Returns true when reasoning is requested.
    #[must_use]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// A tool declaration as advertised to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDecl {
    /// Tool name.
    pub name: String,
    /// One-paragraph description shown to the model.
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: Value,
}

/// A tool call emitted by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider-assigned opaque identifier.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parsed arguments object.
    pub arguments: Value,
}

/// One content block in a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    /// Visible model text.
    Text {
        /// Text payload.
        text: String,
        /// Provider replay signature, when the provider supplies one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Reasoning text.
    Thinking {
        /// Reasoning payload.
        thinking: String,
        /// Provider replay signature, when the provider supplies one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        /// True when the provider returned a redacted reasoning block.
        #[serde(default)]
        redacted: bool,
    },
    /// A call to a tool.
    ToolCall {
        /// Provider-assigned opaque identifier.
        id: String,
        /// Tool name.
        name: String,
        /// Parsed arguments object.
        arguments: Value,
        /// Provider replay signature, when the provider supplies one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    /// A base64-encoded image.
    Image {
        /// Base64 data.
        data: String,
        /// MIME type such as `image/png`.
        mime_type: String,
    },
}

impl Content {
    /// Creates a text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text {
            text: text.into(),
            signature: None,
        }
    }

    /// Creates a thinking block.
    #[must_use]
    pub fn thinking(thinking: impl Into<String>) -> Self {
        Self::Thinking {
            thinking: thinking.into(),
            signature: None,
            redacted: false,
        }
    }

    /// Returns the text payload, when this is a text block.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    /// Returns a tool call, when this is a tool-call block.
    #[must_use]
    pub fn as_tool_call(&self) -> Option<ToolCall> {
        match self {
            Self::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some(ToolCall {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
            }),
            _ => None,
        }
    }

    /// Returns true when this block is model-visible text.
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(self, Self::Text { .. })
    }
}

/// Why an assistant turn stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The stream is still in progress. Never persisted.
    Pending,
    /// The model finished normally.
    Stop,
    /// The output token limit was reached.
    Length,
    /// The model requested one or more tool calls.
    ToolUse,
    /// The request or stream failed.
    Error,
    /// The caller aborted the request.
    Aborted,
    /// The provider deferred the response for later retrieval.
    Deferred,
}

impl StopReason {
    /// Returns true when no further turn should be attempted.
    #[must_use]
    pub const fn is_terminal_failure(self) -> bool {
        matches!(self, Self::Error | Self::Aborted)
    }
}

/// Token usage for one request, normalized across providers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens, including reasoning tokens.
    pub output: u64,
    /// Tokens served from a provider cache.
    pub cache_read: u64,
    /// Tokens written to a provider cache.
    pub cache_write: u64,
    /// Reasoning tokens, a subset of `output`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    /// Sum of input, output, cache read, and cache write.
    pub total_tokens: u64,
    /// Cost in integer nano-USD.
    pub cost: Cost,
}

impl Usage {
    /// Creates usage and recomputes the total.
    #[must_use]
    pub fn new(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Self {
        let mut usage = Self {
            input,
            output,
            cache_read,
            cache_write,
            reasoning: None,
            total_tokens: 0,
            cost: Cost::default(),
        };
        usage.recompute_total();
        usage
    }

    /// Recomputes `total_tokens` from its components.
    pub fn recompute_total(&mut self) {
        self.total_tokens = self.input + self.output + self.cache_read + self.cache_write;
    }

    /// Returns true when every counter is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.total_tokens == 0
    }
}

/// Monetary cost in integer nano-USD (`1 USD = 1_000_000_000`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    /// Cost of uncached input tokens.
    pub input: u64,
    /// Cost of output tokens.
    pub output: u64,
    /// Cost of cache reads.
    pub cache_read: u64,
    /// Cost of cache writes.
    pub cache_write: u64,
    /// Total cost.
    pub total: u64,
    /// True when the provider reported the total instead of codie estimating it.
    #[serde(default)]
    pub total_from_provider: bool,
}

impl Cost {
    /// Creates a cost and recomputes the total.
    #[must_use]
    pub fn new(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Self {
        let mut cost = Self {
            input,
            output,
            cache_read,
            cache_write,
            total: 0,
            total_from_provider: false,
        };
        cost.recompute_total();
        cost
    }

    /// Recomputes `total` from its components.
    pub fn recompute_total(&mut self) {
        self.total = self.input + self.output + self.cache_read + self.cache_write;
    }

    /// Converts a provider-reported USD amount to nano-USD.
    #[must_use]
    pub fn nanos_from_usd(usd: f64) -> u64 {
        if !usd.is_finite() || usd <= 0.0 {
            return 0;
        }
        (usd * 1_000_000_000.0).round() as u64
    }

    /// Converts nano-USD back to USD for display.
    #[must_use]
    pub fn usd(&self) -> f64 {
        self.total as f64 / 1_000_000_000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_levels_round_trip() {
        for level in [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
            ThinkingLevel::Max,
        ] {
            assert_eq!(ThinkingLevel::parse(level.as_str()), Some(level));
        }
    }

    #[test]
    fn usage_total_is_the_sum_of_parts() {
        let usage = Usage::new(10, 20, 30, 40);
        assert_eq!(usage.total_tokens, 100);
        assert!(!usage.is_zero());
    }

    #[test]
    fn cost_is_integral_nano_usd() {
        assert_eq!(Cost::nanos_from_usd(0.00042), 420_000);
        assert_eq!(Cost::new(1, 2, 3, 4).total, 10);
    }

    #[test]
    fn tool_call_extraction_preserves_arguments() {
        let content = Content::ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: serde_json::json!({"path": "a.txt"}),
            thought_signature: None,
        };
        let call = content.as_tool_call().unwrap();
        assert_eq!(call.id, "call_1");
        assert_eq!(call.arguments["path"], "a.txt");
    }
}
