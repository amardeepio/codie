//! Secret redaction for provider diagnostics (`PROV-005`).
//!
//! Trace logs may include request headers and response bodies, but they must
//! never include an API key. Everything logged by the adapter passes through
//! these helpers first.

use crate::model::HeaderMap;

/// Replaces every non-empty secret in `secrets` with `<redacted>`.
#[must_use]
pub fn redact_text(text: &str, secrets: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let mut redacted = text.to_owned();
    for secret in secrets {
        let secret = secret.as_ref();
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "<redacted>");
        }
    }
    redacted
}

/// Returns a redacted representation of one header value.
#[must_use]
pub fn redact_header(name: &str, value: &str, api_key: &str) -> String {
    if is_secret_header(name) {
        return "<redacted>".to_owned();
    }
    redact_text(value, [api_key])
}

/// Returns a redacted, single-line summary of the configured headers.
///
/// `Authorization` is always present in a real request but never in the
/// configured map, so it is added explicitly as redacted.
#[must_use]
pub fn redacted_headers(headers: &HeaderMap, api_key: &str) -> String {
    let mut parts = vec!["authorization: Bearer <redacted>".to_owned()];
    for (name, value) in headers {
        parts.push(format!("{name}: {}", redact_header(name, value, api_key)));
    }
    parts.join(", ")
}

/// Returns true for headers whose value must never be logged.
#[must_use]
pub fn is_secret_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("x-api-key")
        || name.eq_ignore_ascii_case("api-key")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_removed_from_free_text() {
        let text = "Authorization: Bearer sk-secret-123 and sk-secret-123 again";
        let redacted = redact_text(text, ["sk-secret-123"]);
        assert!(!redacted.contains("sk-secret-123"));
        assert_eq!(redacted.matches("<redacted>").count(), 2);
    }

    #[test]
    fn secret_headers_are_always_redacted() {
        assert_eq!(
            redact_header("Authorization", "Bearer sk-secret", "sk-secret"),
            "<redacted>"
        );
        assert_eq!(
            redact_header("X-Api-Key", "sk-secret", "sk-secret"),
            "<redacted>"
        );
    }

    #[test]
    fn ordinary_headers_are_kept_but_sanitized() {
        assert_eq!(redact_header("X-Title", "codie", "sk-secret"), "codie");
        assert_eq!(
            redact_header("X-Custom", "sk-secret", "sk-secret"),
            "<redacted>"
        );
    }
}
