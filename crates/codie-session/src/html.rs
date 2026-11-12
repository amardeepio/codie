//! Self-contained HTML transcript export (`/export html`).

use std::fmt::Write as _;
use std::io::Write;

use codie_core::{Content, Error, Message};

use crate::session::Session;

impl Session {
    /// Writes the current branch as a self-contained HTML document.
    ///
    /// The output embeds its CSS, follows the system light/dark preference, and
    /// loads no scripts or external assets. Export is read-only.
    pub fn export_html(&self, mut writer: impl Write) -> Result<(), Error> {
        let messages = self.projection();
        let title = self.name().unwrap_or("codie session");
        let mut html = String::with_capacity(8 * 1024);

        html.push_str(
            "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        );
        let _ = writeln!(html, "<title>{}</title>", escape(title));
        html.push_str(STYLE);
        html.push_str("</head>\n<body>\n<header>\n");
        let _ = writeln!(html, "<h1>{}</h1>", escape(title));
        html.push_str("<dl>");
        let _ = writeln!(html, "<div><dt>Session</dt><dd>{}</dd></div>", self.id());
        let _ = writeln!(
            html,
            "<div><dt>Directory</dt><dd>{}</dd></div>",
            escape(self.cwd())
        );
        let _ = writeln!(
            html,
            "<div><dt>Created</dt><dd>{}</dd></div>",
            escape(&self.header().timestamp)
        );
        let _ = writeln!(
            html,
            "<div><dt>Messages</dt><dd>{}</dd></div>",
            messages.len()
        );
        let _ = writeln!(
            html,
            "<div><dt>Tokens</dt><dd>~{}</dd></div>",
            self.estimate_tokens()
        );
        html.push_str("</dl>\n</header>\n<main>\n");

        for message in &messages {
            match message {
                Message::System(_) => {}
                Message::User(user) => {
                    html.push_str("<article class=\"message user\">\n");
                    html.push_str("<div class=\"role\">You</div>\n");
                    for block in &user.content {
                        match block {
                            Content::Text { text, .. } => {
                                let _ = writeln!(html, "<pre>{}</pre>", escape(text));
                            }
                            Content::Image { .. } => {
                                html.push_str("<p class=\"chip\">[image]</p>\n");
                            }
                            _ => {}
                        }
                    }
                    html.push_str("</article>\n");
                }
                Message::Assistant(assistant) => {
                    html.push_str("<article class=\"message assistant\">\n");
                    html.push_str("<div class=\"role\">codie</div>\n");
                    for block in &assistant.content {
                        match block {
                            Content::Text { text, .. } => {
                                if !text.trim().is_empty() {
                                    let _ = writeln!(html, "<pre>{}</pre>", escape(text));
                                }
                            }
                            Content::Thinking { thinking, .. } => {
                                let _ = writeln!(
                                    html,
                                    "<details class=\"thinking\"><summary>thinking</summary>\
                                     <pre>{}</pre></details>",
                                    escape(thinking)
                                );
                            }
                            Content::ToolCall {
                                name, arguments, ..
                            } => {
                                let _ = writeln!(
                                    html,
                                    "<p class=\"chip\">{} {}</p>",
                                    escape(name),
                                    escape(&compact_arguments(arguments))
                                );
                            }
                            Content::Image { .. } => {}
                        }
                    }
                    html.push_str("</article>\n");
                }
                Message::ToolResult(result) => {
                    let class = if result.is_error {
                        "tool error"
                    } else {
                        "tool"
                    };
                    let marker = if result.is_error { "✗" } else { "✓" };
                    let _ = writeln!(
                        html,
                        "<details class=\"{class}\"><summary>{marker} {}</summary><pre>{}</pre></details>",
                        escape(&result.tool_name),
                        escape(&text_of(&result.content))
                    );
                }
            }
        }

        html.push_str("</main>\n<footer>Exported by codie ");
        html.push_str(env!("CARGO_PKG_VERSION"));
        html.push_str("</footer>\n</body>\n</html>\n");

        writer
            .write_all(html.as_bytes())
            .map_err(|error| Error::session(format!("cannot export HTML: {error}")))
    }
}

fn text_of(content: &[Content]) -> String {
    content
        .iter()
        .filter_map(Content::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_arguments(arguments: &serde_json::Value) -> String {
    let text = serde_json::to_string(arguments).unwrap_or_else(|_| "{}".to_owned());
    if text.chars().count() > 120 {
        format!("{}…", text.chars().take(119).collect::<String>())
    } else {
        text
    }
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

const STYLE: &str = r#"<style>
:root {
  color-scheme: dark light;
  --bg: #0f1115;
  --panel: #171a21;
  --fg: #e6e7eb;
  --muted: #9aa0a6;
  --accent: #7aa2f7;
  --border: #2a2f3a;
  --user: #9ece6a;
  --tool: #bb9af7;
  --error: #f7768e;
}
@media (prefers-color-scheme: light) {
  :root {
    --bg: #fbfbfd;
    --panel: #ffffff;
    --fg: #1c1f24;
    --muted: #5c6470;
    --accent: #3557b7;
    --border: #dfe3ea;
    --user: #2f7d32;
    --tool: #6a3fb5;
    --error: #b3261e;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0 auto;
  padding: 2rem 1.25rem 4rem;
  max-width: 72rem;
  background: var(--bg);
  color: var(--fg);
  font: 15px/1.6 ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
}
header { border-bottom: 1px solid var(--border); margin-bottom: 2rem; padding-bottom: 1rem; }
h1 { margin: 0 0 1rem; font-size: 1.5rem; color: var(--accent); }
dl { display: grid; gap: .35rem; margin: 0; }
dl div { display: flex; gap: .75rem; }
dt { min-width: 7rem; color: var(--muted); }
dd { margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
main { display: grid; gap: 1rem; }
.message { background: var(--panel); border: 1px solid var(--border); border-radius: .6rem; padding: 1rem; }
.message.user { border-left: 3px solid var(--user); }
.message.assistant { border-left: 3px solid var(--accent); }
.role { margin-bottom: .5rem; font-size: .78rem; font-weight: 600; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); }
pre { margin: 0 0 .6rem; white-space: pre-wrap; overflow-wrap: anywhere; font: inherit; }
.message pre:last-child { margin-bottom: 0; }
details { margin-top: .5rem; border: 1px solid var(--border); border-radius: .45rem; padding: .5rem .75rem; }
summary { cursor: pointer; color: var(--tool); }
details.error summary { color: var(--error); }
details.thinking summary { color: var(--muted); font-style: italic; }
.chip { display: inline-block; margin: .15rem .25rem .15rem 0; padding: .1rem .5rem; border: 1px solid var(--border); border-radius: 999px; color: var(--tool); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .82rem; }
footer { margin-top: 2.5rem; color: var(--muted); font-size: .8rem; }
</style>
"#;

#[cfg(test)]
mod tests {
    use codie_core::{Content, StopReason};

    use super::*;

    #[test]
    fn html_is_escaped_and_self_contained() {
        let directory = tempfile::tempdir().expect("temp dir");
        let sessions = directory.path().join("sessions");
        let mut session = Session::create(directory.path(), &sessions).expect("create session");
        session
            .append_message(&Message::user("<script>alert(1)</script>"))
            .expect("user message");
        session
            .append_message(&Message::assistant(
                "openai-completions",
                "deepseek",
                "deepseek-flash",
                vec![Content::text("hello **world**")],
                StopReason::Stop,
            ))
            .expect("assistant message");

        let mut output = Vec::new();
        session.export_html(&mut output).expect("export html");
        let html = String::from_utf8(output).expect("utf8");

        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("hello **world**"));
        assert!(!html.contains("<script src"));
    }
}
