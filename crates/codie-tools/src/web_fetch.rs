//! The `web_fetch` tool.

use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use reqwest::Url;
use reqwest::header::CONTENT_TYPE;
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Default response cap (2 MiB).
pub const DEFAULT_FETCH_BYTES: usize = 2 * 1024 * 1024;

/// Hard response cap (10 MiB).
pub const MAX_FETCH_BYTES: usize = 10 * 1024 * 1024;

/// Default request timeout in seconds.
pub const DEFAULT_FETCH_TIMEOUT_SECONDS: u64 = 30;

/// Hard request timeout in seconds.
pub const MAX_FETCH_TIMEOUT_SECONDS: u64 = 120;

/// Maximum redirects followed in one fetch.
pub const MAX_REDIRECTS: usize = 5;

/// Fetches one HTTP(S) URL and converts HTML to readable text.
#[derive(Debug, Clone, Copy, Default)]
pub struct WebFetchTool {
    allow_loopback: bool,
}

impl WebFetchTool {
    /// Creates a web fetch tool that blocks local and private addresses.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            allow_loopback: false,
        }
    }

    /// Creates a tool that permits loopback targets. Test seam only.
    #[must_use]
    pub const fn with_loopback_allowed() -> Self {
        Self {
            allow_loopback: true,
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a public http(s) URL and return its text. HTML is converted to readable text; \
         JavaScript is not executed. Local, private, and link-local addresses are refused. Use \
         web_fetch for documentation, changelogs, and issue pages, not for interactive browsing."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "Absolute http or https URL to fetch"
                },
                "max_bytes": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum response bytes to read; defaults to 2 MiB and is capped at 10 MiB"
                },
                "timeout": {
                    "type": "number",
                    "minimum": 1,
                    "description": "Timeout in seconds; defaults to 30 and is capped at 120"
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    fn execution_mode(&self) -> ExecutionMode {
        ExecutionMode::Parallel
    }

    async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        let Some(raw_url) = call.arguments.get("url").and_then(Value::as_str) else {
            return ToolResult::error("Missing required argument: url");
        };
        let max_bytes = match optional_limit(&call.arguments, "max_bytes", MAX_FETCH_BYTES) {
            Ok(value) => value.unwrap_or(DEFAULT_FETCH_BYTES),
            Err(message) => return ToolResult::error(message),
        };
        let timeout = match optional_timeout(&call.arguments) {
            Ok(value) => value.unwrap_or(Duration::from_secs(DEFAULT_FETCH_TIMEOUT_SECONDS)),
            Err(message) => return ToolResult::error(message),
        };

        let url = match Url::parse(raw_url) {
            Ok(url) => url,
            Err(error) => return ToolResult::error(format!("Invalid URL: {error}")),
        };
        if context.cancel.is_cancelled() {
            return ToolResult::error("Operation aborted");
        }

        let request = FetchRequest {
            max_bytes,
            allow_loopback: self.allow_loopback,
            timeout,
        };
        let cancel = context.cancel.clone();
        match tokio::time::timeout(timeout, request.run(url, &cancel)).await {
            Ok(Ok(response)) => format_response(response),
            Ok(Err(message)) => ToolResult::error(message),
            Err(_) => ToolResult::error(format!(
                "Fetch timed out after {} seconds",
                timeout.as_secs()
            )),
        }
    }
}

/// A validated fetch executed on the async runtime.
struct FetchRequest {
    max_bytes: usize,
    allow_loopback: bool,
    timeout: Duration,
}

struct FetchResponse {
    status: u16,
    content_type: String,
    final_url: String,
    text: String,
    bytes: usize,
    truncated: bool,
}

impl FetchRequest {
    async fn run(
        self,
        mut url: Url,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<FetchResponse, String> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .user_agent(concat!("codie/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("Cannot create the HTTP client: {error}"))?;

        for _ in 0..=MAX_REDIRECTS {
            if cancel.is_cancelled() {
                return Err("Operation aborted".to_owned());
            }
            validate_target(&url, self.allow_loopback).await?;
            let response = client
                .get(url.clone())
                .send()
                .await
                .map_err(|error| format!("Fetch failed: {error}"))?;

            if response.status().is_redirection() {
                let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                    return Err("Redirect response had no Location header".to_owned());
                };
                let location = location
                    .to_str()
                    .map_err(|_| "Redirect Location header is not valid text".to_owned())?;
                url = url
                    .join(location)
                    .map_err(|error| format!("Invalid redirect target: {error}"))?;
                continue;
            }

            return self.read(response, url).await;
        }

        Err(format!("Too many redirects (limit {MAX_REDIRECTS})"))
    }

    async fn read(
        self,
        response: reqwest::Response,
        final_url: Url,
    ) -> Result<FetchResponse, String> {
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();

        let mut body = Vec::new();
        let mut truncated = false;
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Cannot read the response: {error}"))?
        {
            let remaining = self.max_bytes.saturating_sub(body.len());
            if chunk.len() > remaining {
                body.extend_from_slice(&chunk[..remaining]);
                truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }

        let bytes = body.len();
        let decoded = String::from_utf8_lossy(&body);
        let text = if content_type.contains("html") {
            html_to_text(&decoded)
        } else if is_textual(&content_type) {
            decoded.into_owned()
        } else {
            format!("[binary content omitted: {content_type}]")
        };

        Ok(FetchResponse {
            status,
            content_type,
            final_url: final_url.to_string(),
            text: text.trim().to_owned(),
            bytes,
            truncated,
        })
    }
}

fn format_response(response: FetchResponse) -> ToolResult {
    let mut output = format!(
        "GET {} → {} {}\n\n{}",
        response.final_url, response.status, response.content_type, response.text
    );
    if response.truncated {
        output.push_str("\n\n[Response truncated at the byte limit.]");
    }
    ToolResult::text(output).with_details(json!({
        "status": response.status,
        "content_type": response.content_type,
        "final_url": response.final_url,
        "bytes": response.bytes,
        "truncated": response.truncated,
    }))
}

/// Rejects schemes and destinations that could reach local services.
async fn validate_target(url: &Url, allow_loopback: bool) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported URL scheme `{}`; only http and https are allowed",
            url.scheme()
        ));
    }
    let host = url.host_str().ok_or_else(|| "URL has no host".to_owned())?;
    let port = url.port_or_known_default().unwrap_or(443);

    if let Ok(address) = host.parse::<IpAddr>() {
        return check_address(address, allow_loopback);
    }

    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("Cannot resolve `{host}`: {error}"))?;
    let mut found = false;
    for address in addresses {
        found = true;
        check_address(address.ip(), allow_loopback)?;
    }
    if !found {
        return Err(format!("Cannot resolve `{host}`"));
    }
    Ok(())
}

fn check_address(address: IpAddr, allow_loopback: bool) -> Result<(), String> {
    if allow_loopback {
        return Ok(());
    }
    if let IpAddr::V6(v6) = address
        && let Some(v4) = v6.to_ipv4_mapped()
        && blocked_v4(v4)
    {
        return Err(format!("Refusing to fetch a private address: {address}"));
    }
    let blocked = match address {
        IpAddr::V4(v4) => blocked_v4(v4),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    };
    if blocked {
        Err(format!("Refusing to fetch a private address: {address}"))
    } else {
        Ok(())
    }
}

fn blocked_v4(address: std::net::Ipv4Addr) -> bool {
    address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_broadcast()
}

fn is_textual(content_type: &str) -> bool {
    content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
}

/// Removes scripts, styles, and tags, and decodes common entities.
fn html_to_text(html: &str) -> String {
    let mut without_blocks = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_lowercase();
        let Some(start) = ["<script", "<style"]
            .iter()
            .filter_map(|tag| lower.find(tag))
            .min()
        else {
            without_blocks.push_str(rest);
            break;
        };
        without_blocks.push_str(&rest[..start]);
        let closing = if lower[start..].starts_with("<script") {
            "</script>"
        } else {
            "</style>"
        };
        match lower[start..].find(closing) {
            Some(end) => rest = &rest[start + end + closing.len()..],
            None => break,
        }
    }

    let mut text = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for character in without_blocks.chars() {
        match character {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                if is_block_tag(&tag) {
                    text.push('\n');
                }
            }
            _ if in_tag => {
                if tag.len() < 24 {
                    tag.push(character);
                }
            }
            _ => text.push(character),
        }
    }

    let decoded = decode_entities(&text);
    decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_block_tag(tag: &str) -> bool {
    let name = tag
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    matches!(
        name.as_str(),
        "br" | "p"
            | "div"
            | "li"
            | "tr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "section"
            | "article"
    )
}

fn decode_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        let tail = &rest[index..];
        let Some(end) = tail.find(';').filter(|end| *end <= 10) else {
            output.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        if let Some(character) = decoded {
            output.push(character);
            rest = &tail[end + 1..];
        } else {
            output.push('&');
            rest = &tail[1..];
        }
    }
    output.push_str(rest);
    output
}

fn optional_limit(arguments: &Value, name: &str, maximum: usize) -> Result<Option<usize>, String> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|number| usize::try_from(number).ok())
            .filter(|number| *number > 0)
            .map(|number| Some(number.min(maximum)))
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

fn optional_timeout(arguments: &Value) -> Result<Option<Duration>, String> {
    match arguments.get("timeout") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_f64()
            .map(|seconds| {
                Duration::from_secs_f64(seconds.clamp(1.0, MAX_FETCH_TIMEOUT_SECONDS as f64))
            })
            .map(Some)
            .ok_or_else(|| "timeout must be a number of seconds".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_readable_text() {
        let text = html_to_text(
            "<html><head><style>body{}</style></head><body><h1>Title</h1><script>bad()</script><p>Hello &amp; bye</p></body></html>",
        );
        assert_eq!(text, "Title\nHello & bye");
        assert!(!text.contains("bad()"));
    }

    #[test]
    fn entities_decode_and_unknown_ones_survive() {
        assert_eq!(
            decode_entities("a &lt; b &#65; &#x42; &unknown;"),
            "a < b A B &unknown;"
        );
    }

    #[test]
    fn private_addresses_are_blocked() {
        assert!(check_address("127.0.0.1".parse().unwrap(), false).is_err());
        assert!(check_address("10.0.0.1".parse().unwrap(), false).is_err());
        assert!(check_address("192.168.1.1".parse().unwrap(), false).is_err());
        assert!(check_address("169.254.1.1".parse().unwrap(), false).is_err());
        assert!(check_address("::1".parse().unwrap(), false).is_err());
        assert!(check_address("fd00::1".parse().unwrap(), false).is_err());
        assert!(check_address("93.184.216.34".parse().unwrap(), false).is_ok());
        assert!(check_address("127.0.0.1".parse().unwrap(), true).is_ok());
    }

    #[test]
    fn only_http_urls_are_accepted() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let file = Url::parse("file:///etc/passwd").unwrap();
            assert!(validate_target(&file, false).await.is_err());
            let local = Url::parse("http://127.0.0.1:8080/").unwrap();
            assert!(validate_target(&local, false).await.is_err());
            assert!(validate_target(&local, true).await.is_ok());
        });
    }
}
