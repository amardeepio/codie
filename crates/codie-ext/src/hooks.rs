//! Bridge from extension hook requests to the core agent loop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use codie_core::{HookContext, Hooks, Message, ModelRef, ToolCall, ToolResult};
use codie_protocol::ext::{ExtHook, ExtHookParams};

use crate::Extension;
use crate::convert;

/// Hook adapter shared by the CLI, TUI, plain mode, and RPC.
pub struct ExtensionHooks {
    extensions: Vec<Arc<Extension>>,
    cwd: PathBuf,
    session_id: Arc<Mutex<Option<String>>>,
}

pub(crate) fn build(
    extensions: &[Arc<Extension>],
    cwd: PathBuf,
    session_id: Arc<Mutex<Option<String>>>,
) -> Option<Arc<dyn Hooks>> {
    let interested = extensions
        .iter()
        .filter(|extension| !extension.hooks.is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if interested.is_empty() {
        return None;
    }
    Some(Arc::new(ExtensionHooks {
        extensions: interested,
        cwd,
        session_id,
    }))
}

impl ExtensionHooks {
    fn session(&self, model: Option<&ModelRef>) -> codie_protocol::ext::ExtSessionInfo {
        let id = self.session_id.lock().ok().and_then(|id| id.clone());
        convert::session_info(id.as_deref(), &self.cwd, model)
    }

    async fn run_hook(
        &self,
        hook: ExtHook,
        params: ExtHookParams,
    ) -> Vec<codie_protocol::ext::ExtHookResult> {
        let mut results = Vec::new();
        for extension in &self.extensions {
            if !extension.hooks.contains(&hook) {
                continue;
            }
            match extension.call_hook(hook, params.clone()).await {
                Ok(result) => results.push(result),
                Err(error) => {
                    tracing::warn!(
                        extension = %extension.name,
                        hook = hook.as_str(),
                        %error,
                        "extension hook failed"
                    );
                }
            }
        }
        results
    }
}

#[async_trait]
impl Hooks for ExtensionHooks {
    async fn transform_context(&self, mut context: HookContext) -> Result<HookContext, String> {
        convert::strip_context(&mut context.messages);
        let session = self.session(Some(&context.model));
        let params = ExtHookParams {
            session,
            text: last_user_text(&context.messages),
            messages: convert::messages(&context.messages),
            ..ExtHookParams::default()
        };
        let mut contexts = Vec::new();
        for result in self.run_hook(ExtHook::BeforeAgentStart, params).await {
            if result.block {
                return Err(result
                    .reason
                    .unwrap_or_else(|| "blocked by an extension".to_owned()));
            }
            if let Some(prompt) = result.prompt {
                let _ = convert::replace_last_user_text(&mut context.messages, &prompt);
            }
            if let Some(extra) = result.context {
                contexts.push(extra);
            }
        }
        append_contexts(&mut context.messages, contexts);
        Ok(context)
    }

    async fn prepare_request(&self, mut context: HookContext) -> Result<HookContext, String> {
        convert::strip_context(&mut context.messages);
        let session = self.session(Some(&context.model));
        let params = ExtHookParams {
            session,
            messages: convert::messages(&context.messages),
            ..ExtHookParams::default()
        };
        let mut contexts = Vec::new();
        for result in self.run_hook(ExtHook::Context, params).await {
            if result.block {
                return Err(result
                    .reason
                    .unwrap_or_else(|| "blocked by an extension".to_owned()));
            }
            if let Some(extra) = result.context {
                contexts.push(extra);
            }
        }
        append_contexts(&mut context.messages, contexts);
        Ok(context)
    }

    async fn convert_to_llm(&self, context: HookContext) -> Result<HookContext, String> {
        Ok(context)
    }

    async fn prepare_next_turn(&self, mut context: HookContext) -> Result<HookContext, String> {
        convert::strip_context(&mut context.messages);
        Ok(context)
    }

    async fn finish_turn(&self, mut context: HookContext) -> Result<HookContext, String> {
        convert::strip_context(&mut context.messages);
        let session = self.session(Some(&context.model));
        let params = ExtHookParams {
            session,
            messages: convert::messages(&context.messages),
            assistant_text: last_assistant_text(&context.messages),
            ..ExtHookParams::default()
        };
        let mut contexts = Vec::new();
        for result in self.run_hook(ExtHook::TurnEnd, params).await {
            if let Some(extra) = result.context {
                contexts.push(extra);
            }
        }
        append_contexts(&mut context.messages, contexts);
        Ok(context)
    }

    async fn before_tool(&self, call: &ToolCall) -> Result<(), String> {
        let params = ExtHookParams {
            session: self.session(None),
            call: Some(convert::tool_call(call)),
            ..ExtHookParams::default()
        };
        for result in self.run_hook(ExtHook::BeforeToolCall, params).await {
            if result.block {
                return Err(result.reason.unwrap_or_else(|| {
                    format!("tool `{}` was blocked by an extension", call.name)
                }));
            }
        }
        Ok(())
    }

    async fn after_tool(&self, call: &ToolCall, result: ToolResult) -> Result<ToolResult, String> {
        let mut current = result;
        let params = ExtHookParams {
            session: self.session(None),
            call: Some(convert::tool_call(call)),
            result: Some(convert::tool_result(&current)),
            ..ExtHookParams::default()
        };
        for hook_result in self.run_hook(ExtHook::AfterToolCall, params).await {
            if let Some(result) = hook_result.result {
                current = convert::ext_tool_result(result);
            }
        }
        Ok(current)
    }
}

fn append_contexts(messages: &mut [Message], contexts: Vec<String>) {
    if contexts.is_empty() {
        return;
    }
    convert::append_context(messages, "extensions", &contexts.join("\n\n"));
}

fn last_user_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        Message::User(user) => Some(
            user.content
                .iter()
                .filter_map(codie_core::Content::as_text)
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    })
}

fn last_assistant_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|message| match message {
        Message::Assistant(assistant) => Some(assistant.text()),
        _ => None,
    })
}
