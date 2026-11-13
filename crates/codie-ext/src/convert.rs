//! Conversions between core types and extension wire types.

use std::path::Path;

use codie_core::{Content, Message, ToolCall, ToolResult};
use codie_protocol::ext::{
    ExtContent, ExtMessage, ExtRenderHint, ExtSessionInfo, ExtToolCall, ExtToolResult,
    MAX_TOOL_RESULT_BYTES,
};
use serde_json::{Value, json};

/// Builds the session identity passed to hooks.
#[must_use]
pub(crate) fn session_info(
    id: Option<&str>,
    cwd: &Path,
    model: Option<&codie_core::ModelRef>,
) -> ExtSessionInfo {
    ExtSessionInfo {
        id: id.map(str::to_owned),
        cwd: cwd.display().to_string(),
        model: model.map(|model| model.model.clone()),
    }
}

/// Converts a transcript into the compact form hooks receive.
#[must_use]
pub(crate) fn messages(messages: &[Message]) -> Vec<ExtMessage> {
    messages
        .iter()
        .rev()
        .take(100)
        .rev()
        .map(|message| match message {
            Message::System(system) => ExtMessage {
                role: "system".to_owned(),
                text: Some(truncate(&system.content, 4_000)),
                tool_name: None,
                tool_call_id: None,
            },
            Message::User(user) => ExtMessage {
                role: "user".to_owned(),
                text: Some(truncate(
                    &user
                        .content
                        .iter()
                        .filter_map(Content::as_text)
                        .collect::<Vec<_>>()
                        .join(""),
                    4_000,
                )),
                tool_name: None,
                tool_call_id: None,
            },
            Message::Assistant(assistant) => ExtMessage {
                role: "assistant".to_owned(),
                text: Some(truncate(&assistant.text(), 4_000)),
                tool_name: None,
                tool_call_id: None,
            },
            Message::ToolResult(result) => ExtMessage {
                role: "tool_result".to_owned(),
                text: Some(truncate(
                    &result
                        .content
                        .iter()
                        .filter_map(Content::as_text)
                        .collect::<Vec<_>>()
                        .join(""),
                    4_000,
                )),
                tool_name: Some(result.tool_name.clone()),
                tool_call_id: Some(result.tool_call_id.clone()),
            },
        })
        .collect()
}

/// Converts a core tool call into the extension shape.
#[must_use]
pub(crate) fn tool_call(call: &ToolCall) -> ExtToolCall {
    ExtToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
    }
}

/// Converts a core tool result into the extension shape.
#[must_use]
pub(crate) fn tool_result(result: &ToolResult) -> ExtToolResult {
    ExtToolResult {
        content: result
            .content
            .iter()
            .map(|content| match content {
                Content::Text { text, .. } => {
                    ExtContent::text(truncate(text, MAX_TOOL_RESULT_BYTES))
                }
                Content::Image { data, mime_type } => ExtContent::Image {
                    data: data.clone(),
                    mime_type: mime_type.clone(),
                },
                Content::Thinking { .. } | Content::ToolCall { .. } => {
                    ExtContent::text(String::new())
                }
            })
            .collect(),
        details: result.details.clone(),
        is_error: result.is_error,
        terminate: result.terminate,
        render: None,
    }
}

/// Converts an extension tool result into the core shape.
#[must_use]
pub(crate) fn ext_tool_result(result: ExtToolResult) -> ToolResult {
    let mut text_bytes = 0_usize;
    let mut truncated = false;
    let content = result
        .content
        .into_iter()
        .filter_map(|content| match content {
            ExtContent::Text { text } => {
                if text_bytes >= MAX_TOOL_RESULT_BYTES {
                    truncated = true;
                    return None;
                }
                let remaining = MAX_TOOL_RESULT_BYTES - text_bytes;
                let (text, was_truncated) = truncate_bytes(text, remaining);
                text_bytes += text.len();
                truncated |= was_truncated;
                Some(Content::text(text))
            }
            ExtContent::Image { data, mime_type } => {
                if data.len() > MAX_TOOL_RESULT_BYTES {
                    truncated = true;
                    None
                } else {
                    Some(Content::Image { data, mime_type })
                }
            }
        })
        .collect::<Vec<_>>();
    let mut content = content;
    if truncated {
        content.push(Content::text(format!(
            "\n[Extension output truncated at {MAX_TOOL_RESULT_BYTES} bytes.]"
        )));
    }
    let mut details = result.details;
    if let Some(render) = result.render {
        let render = sanitize_render(render);
        details = Some(match details {
            Some(Value::Object(mut object)) => {
                object.insert(
                    "codie_render".to_owned(),
                    serde_json::to_value(render).unwrap_or(Value::Null),
                );
                Value::Object(object)
            }
            Some(value) => json!({"value": value, "codie_render": render}),
            None => json!({"codie_render": render}),
        });
    }
    ToolResult {
        content,
        details,
        is_error: result.is_error,
        terminate: result.terminate,
        duration_ms: None,
    }
}

/// Replaces the text of the last user message, preserving images.
#[must_use]
pub(crate) fn replace_last_user_text(messages: &mut [Message], text: &str) -> bool {
    let Some(Message::User(user)) = messages
        .iter_mut()
        .rev()
        .find(|message| matches!(message, Message::User(_)))
    else {
        return false;
    };
    let images = user
        .content
        .iter()
        .filter(|content| matches!(content, Content::Image { .. }))
        .cloned()
        .collect::<Vec<_>>();
    user.content = std::iter::once(Content::text(text.to_owned()))
        .chain(images)
        .collect();
    true
}

/// Appends an extension context block to the first system message.
///
/// Existing extension context blocks are removed first so a per-request
/// injection never accumulates in the in-memory transcript.
pub(crate) fn append_context(messages: &mut [Message], extension: &str, context: &str) {
    let extension = escape_attribute(extension);
    if let Some(Message::System(system)) = messages.first_mut() {
        strip_named_context(&mut system.content, &extension);
        system.content.push_str(&format!(
            "\n\n<extension_context name=\"{extension}\">\n{}\n</extension_context>",
            context.trim()
        ));
    }
}

/// Removes every extension context block from the first system message.
pub(crate) fn strip_context(messages: &mut [Message]) {
    if let Some(Message::System(system)) = messages.first_mut() {
        while let Some(start) = system.content.find("\n\n<extension_context name=\"") {
            let Some(end) = system.content[start..].find("</extension_context>") else {
                break;
            };
            let end = start + end + "</extension_context>".len();
            system.content.replace_range(start..end, "");
        }
    }
}

fn strip_named_context(content: &mut String, extension: &str) {
    let marker = format!("\n\n<extension_context name=\"{extension}\">");
    if let Some(start) = content.find(&marker)
        && let Some(end) = content[start..].find("</extension_context>")
    {
        let end = start + end + "</extension_context>".len();
        content.replace_range(start..end, "");
    }
}

/// Removes ANSI escape sequences from a structured render hint.
#[must_use]
pub(crate) fn sanitize_render(render: ExtRenderHint) -> ExtRenderHint {
    ExtRenderHint {
        kind: render.kind,
        title: render.title.map(|title| strip_ansi(&title)),
        text: strip_ansi(&render.text),
    }
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            if characters.peek() == Some(&'[') {
                characters.next();
                for next in characters.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        output.push(character);
    }
    output
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn truncate_bytes(text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_blocks_are_replaced_not_accumulated() {
        let mut messages = vec![Message::system("system", Vec::new())];
        append_context(&mut messages, "todo", "one");
        append_context(&mut messages, "todo", "two");
        let Message::System(system) = &messages[0] else {
            panic!("system");
        };
        assert_eq!(system.content.matches("<extension_context").count(), 1);
        assert!(system.content.contains("two"));
        assert!(!system.content.contains("one"));
    }

    #[test]
    fn ansi_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    }

    #[test]
    fn last_user_text_is_replaced_without_losing_images() {
        let mut messages = vec![Message::user_with_content(vec![
            Content::text("old"),
            Content::Image {
                data: "abc".to_owned(),
                mime_type: "image/png".to_owned(),
            },
        ])];
        assert!(replace_last_user_text(&mut messages, "new"));
        let Message::User(user) = &messages[0] else {
            panic!("user");
        };
        assert_eq!(user.content[0].as_text(), Some("new"));
        assert!(matches!(user.content[1], Content::Image { .. }));
    }
}
