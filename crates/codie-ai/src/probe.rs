//! Best-effort API-key verification.
//!
//! A key is checked only when the user asks to connect or verify a provider.
//! Startup and `codie auth status` stay offline. Probes deliberately use small,
//! read-only endpoints and never include the key in an error message.
//!
//! OpenRouter's `/models` endpoint is public, so an invalid key would still
//! return 200. OpenRouter therefore uses `/key`, which requires authentication.
//! Gemini uses its native model list, which accepts the key as a query
//! parameter, and the remaining OpenAI-compatible providers use `/models`.

use std::time::Duration;

use serde_json::Value;

use crate::provider::{ProbeKind, find};
use crate::redact::redact_text;

/// How long one key check may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// The result of checking one API key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The provider accepted the key.
    Valid,
    /// The provider accepted the key but is throttling the check.
    Throttled {
        /// Provider message or a generic explanation.
        message: String,
    },
    /// The provider rejected the key.
    Invalid {
        /// HTTP status returned by the provider, or zero for a local error.
        status: u16,
        /// Sanitized explanation.
        message: String,
    },
    /// The provider does not expose an endpoint this client can check.
    Unsupported {
        /// Sanitized explanation.
        message: String,
    },
    /// The check could not reach the provider or received an unexpected reply.
    Unreachable {
        /// Sanitized explanation.
        message: String,
    },
}

impl ProbeOutcome {
    /// Returns true when the key is usable and may be stored.
    #[must_use]
    pub const fn accepted(&self) -> bool {
        matches!(self, Self::Valid | Self::Throttled { .. })
    }

    /// Returns a one-line description suitable for a transcript or status line.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Valid => "key accepted".to_owned(),
            Self::Throttled { message } => format!("key accepted, but rate limited: {message}"),
            Self::Invalid { message, .. } => format!("key rejected: {message}"),
            Self::Unsupported { message } => format!("cannot check this key: {message}"),
            Self::Unreachable { message } => format!("could not verify the key: {message}"),
        }
    }
}

/// Checks `key` against `provider` without storing it.
///
/// `base_url` is the same override used by the provider client, so a probe
/// against a proxy checks the endpoint the run will actually use.
pub async fn probe_key(provider: &str, key: &str, base_url: Option<&str>) -> ProbeOutcome {
    let Some(spec) = find(provider) else {
        return ProbeOutcome::Unsupported {
            message: format!("unknown provider `{provider}`"),
        };
    };

    let key = key.trim();
    if key.is_empty() && spec.requires_key {
        return ProbeOutcome::Invalid {
            status: 0,
            message: "API key must not be empty".to_owned(),
        };
    }

    let client = match reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .user_agent(format!("codie/{}", codie_core::VERSION))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ProbeOutcome::Unreachable {
                message: sanitize(&error.to_string(), key),
            };
        }
    };

    let base = base_url
        .map(str::to_owned)
        .unwrap_or_else(|| spec.base_url.to_owned());
    let request = match spec.probe {
        ProbeKind::OpenRouterKey => client
            .get(format!("{}/key", base.trim_end_matches('/')))
            .bearer_auth(key),
        ProbeKind::GeminiNative if base_url.is_none() => match gemini_probe_url(key) {
            Ok(url) => client.get(url),
            Err(error) => {
                return ProbeOutcome::Unreachable {
                    message: sanitize(&error.to_string(), key),
                };
            }
        },
        ProbeKind::Models | ProbeKind::GeminiNative => {
            let request = client.get(format!("{}/models", base.trim_end_matches('/')));
            if key.is_empty() {
                request
            } else {
                request.bearer_auth(key)
            }
        }
    };

    match request.send().await {
        Ok(response) => {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            classify(status, &body, key)
        }
        Err(error) => ProbeOutcome::Unreachable {
            message: sanitize(&error.to_string(), key),
        },
    }
}

fn gemini_probe_url(key: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse("https://generativelanguage.googleapis.com/v1beta/models")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("key", key);
    Ok(url)
}

fn classify(status: u16, body: &str, key: &str) -> ProbeOutcome {
    let message = provider_message(body)
        .unwrap_or_else(|| body.to_owned())
        .trim()
        .to_owned();
    let message = sanitize(&truncate(&message), key);

    match status {
        200..=299 => ProbeOutcome::Valid,
        401 | 403 => ProbeOutcome::Invalid {
            status,
            message: if message.is_empty() {
                "the provider rejected the key".to_owned()
            } else {
                message
            },
        },
        400 if looks_like_auth_error(&message) => ProbeOutcome::Invalid { status, message },
        429 => ProbeOutcome::Throttled {
            message: if message.is_empty() {
                "try again shortly".to_owned()
            } else {
                message
            },
        },
        404 | 405 => ProbeOutcome::Unsupported {
            message: "the provider does not expose a model list at this endpoint".to_owned(),
        },
        _ => ProbeOutcome::Unreachable {
            message: format!("provider returned HTTP {status}: {message}"),
        },
    }
}

fn looks_like_auth_error(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    lowered.contains("api key")
        || lowered.contains("api_key")
        || lowered.contains("apikey")
        || lowered.contains("unauthorized")
        || lowered.contains("authentication")
        || lowered.contains("invalid key")
        || lowered.contains("invalid token")
}

fn provider_message(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Some(message.to_owned());
    }
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Some(message.to_owned());
    }
    for field in ["message", "detail", "error_description"] {
        if let Some(message) = value.get(field).and_then(Value::as_str) {
            return Some(message.to_owned());
        }
    }
    None
}

fn truncate(message: &str) -> String {
    let collapsed = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let truncated = chars.by_ref().take(240).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn sanitize(message: &str, key: &str) -> String {
    let message = message.trim();
    if message.is_empty() {
        return String::new();
    }
    redact_text(message, [key])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn models_endpoint_accepts_a_valid_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer sk-good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"id": "deepseek-flash"}]
            })))
            .mount(&server)
            .await;

        let outcome = probe_key("deepseek", "sk-good", Some(&server.uri())).await;
        assert_eq!(outcome, ProbeOutcome::Valid);
    }

    #[tokio::test]
    async fn rejected_keys_are_reported_without_the_secret() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"message": "Invalid API key sk-very-secret"}
            })))
            .mount(&server)
            .await;

        let outcome = probe_key("deepseek", "sk-very-secret", Some(&server.uri())).await;
        assert!(!outcome.accepted());
        let message = outcome.summary();
        assert!(message.contains("key rejected"), "{message}");
        assert!(!message.contains("sk-very-secret"), "{message}");
    }

    #[tokio::test]
    async fn rate_limits_are_accepted_with_a_warning() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "error": {"message": "slow down"}
            })))
            .mount(&server)
            .await;

        let outcome = probe_key("deepseek", "sk-good", Some(&server.uri())).await;
        assert!(outcome.accepted());
        assert!(outcome.summary().contains("rate limited"), "{outcome:?}");
    }

    #[tokio::test]
    async fn openrouter_uses_the_authenticated_key_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/key"))
            .and(header("authorization", "Bearer sk-or-good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {"label": "test"}
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let outcome = probe_key("openrouter", "sk-or-good", Some(&server.uri())).await;
        assert_eq!(outcome, ProbeOutcome::Valid);
    }

    #[tokio::test]
    async fn empty_keys_never_reach_the_network() {
        let outcome = probe_key("deepseek", "   ", None).await;
        assert!(matches!(outcome, ProbeOutcome::Invalid { status: 0, .. }));
    }
}
