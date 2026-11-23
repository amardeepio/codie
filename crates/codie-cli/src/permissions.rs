//! Interactive permission handling for RPC mode (`PROTO-007`).
//!
//! Version 1 of the protocol runs tools with a headless policy: mutating tools
//! are denied unless they were pre-approved with `--allow`. Version 2 lets a
//! client opt into `ask` or `allow` mode; in `ask` mode each mutating call that
//! is not already allowed is sent to the client as a `permission_request` and
//! the tool waits for `permission_response`.
//!
//! The gate is deliberately transport-agnostic: it emits [`PermissionPrompt`]s
//! on a channel and resolves each one with a oneshot reply. Cancelling the run
//! or dropping the receiver denies the call.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use codie_core::{
    ExecutionMode, Tool, ToolCall, ToolContext, ToolDecl, ToolProgress, ToolResult, Toolbox,
};
use codie_protocol::{PermissionDecision, PermissionMode};
use codie_tools::{Policy, is_read_only};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// One mutating tool call waiting for a client decision.
pub(crate) struct PermissionPrompt {
    pub(crate) tool: String,
    pub(crate) tool_call_id: String,
    pub(crate) arguments: Value,
    pub(crate) reply: oneshot::Sender<PermissionDecision>,
}

const MODE_HEADLESS: u8 = 0;
const MODE_ASK: u8 = 1;
const MODE_ALLOW: u8 = 2;

fn encode_mode(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Headless => MODE_HEADLESS,
        PermissionMode::Ask => MODE_ASK,
        PermissionMode::Allow => MODE_ALLOW,
    }
}

fn decode_mode(value: u8) -> PermissionMode {
    match value {
        MODE_ASK => PermissionMode::Ask,
        MODE_ALLOW => PermissionMode::Allow,
        _ => PermissionMode::Headless,
    }
}

/// Shared decision state between the RPC loop and tool wrappers.
#[derive(Clone)]
pub(crate) struct PermissionGate {
    sender: mpsc::UnboundedSender<PermissionPrompt>,
    /// Tools pre-approved by configuration or `--allow`; replaced by
    /// `set_config allow`.
    configured: Arc<Mutex<HashSet<String>>>,
    /// Tools approved with `allow_always` during this session; never revoked.
    granted: Arc<Mutex<HashSet<String>>>,
    mode: Arc<AtomicU8>,
}

impl PermissionGate {
    /// Creates a gate that starts in headless mode with `allowed` pre-approved.
    pub(crate) fn new(
        sender: mpsc::UnboundedSender<PermissionPrompt>,
        allowed: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            sender,
            configured: Arc::new(Mutex::new(allowed.into_iter().collect())),
            granted: Arc::new(Mutex::new(HashSet::new())),
            mode: Arc::new(AtomicU8::new(MODE_HEADLESS)),
        }
    }

    pub(crate) fn mode(&self) -> PermissionMode {
        decode_mode(self.mode.load(Ordering::SeqCst))
    }

    pub(crate) fn set_mode(&self, mode: PermissionMode) {
        self.mode.store(encode_mode(mode), Ordering::SeqCst);
    }

    /// Replaces the configured allow list, keeping session grants.
    pub(crate) fn set_configured(&self, allowed: impl IntoIterator<Item = String>) {
        if let Ok(mut configured) = self.configured.lock() {
            *configured = allowed.into_iter().collect();
        }
    }

    /// Returns true when `tool` may run without asking.
    pub(crate) fn is_allowed(&self, tool: &str) -> bool {
        self.mode() == PermissionMode::Allow
            || is_read_only(tool)
            || self
                .configured
                .lock()
                .is_ok_and(|allowed| allowed.contains(tool))
            || self
                .granted
                .lock()
                .is_ok_and(|allowed| allowed.contains(tool))
    }

    /// Pre-approves `tool` for the rest of the session.
    pub(crate) fn allow_always(&self, tool: &str) {
        if let Ok(mut granted) = self.granted.lock() {
            granted.insert(tool.to_owned());
        }
    }

    /// Asks the client for a decision.
    ///
    /// A dropped channel, a closed reply, an aborted run, or an explicit
    /// `deny` all result in [`PermissionDecision::Deny`].
    pub(crate) async fn request(
        &self,
        tool: &str,
        tool_call_id: &str,
        arguments: &Value,
        cancel: &CancellationToken,
    ) -> PermissionDecision {
        if self.is_allowed(tool) {
            return PermissionDecision::AllowOnce;
        }

        let (reply, answer) = oneshot::channel();
        let prompt = PermissionPrompt {
            tool: tool.to_owned(),
            tool_call_id: tool_call_id.to_owned(),
            arguments: arguments.clone(),
            reply,
        };
        if self.sender.send(prompt).is_err() {
            return PermissionDecision::Deny;
        }

        tokio::select! {
            answer = answer => match answer {
                Ok(PermissionDecision::AllowAlways) => {
                    self.allow_always(tool);
                    PermissionDecision::AllowAlways
                }
                Ok(decision) => decision,
                Err(_) => PermissionDecision::Deny,
            },
            () = cancel.cancelled() => PermissionDecision::Deny,
        }
    }
}

/// A toolbox that enforces the policy and asks before mutating calls.
pub(crate) struct RpcToolbox {
    registry: Arc<dyn Toolbox>,
    policy: Policy,
    gate: PermissionGate,
}

impl RpcToolbox {
    /// Wraps `registry` with `allowed` pre-approved and the given gate.
    pub(crate) fn new(
        registry: Arc<dyn Toolbox>,
        allowed: impl IntoIterator<Item = String>,
        gate: PermissionGate,
    ) -> Self {
        Self {
            registry,
            policy: Policy::headless(allowed),
            gate,
        }
    }
}

impl Toolbox for RpcToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let inner = self.registry.get(name)?;
        Some(Arc::new(GatedTool {
            inner,
            gate: self.gate.clone(),
            pre_allowed: self.policy.allow.contains(name),
        }))
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        self.registry.declarations()
    }

    fn denial(&self, name: &str) -> Option<String> {
        if is_read_only(name) {
            return None;
        }
        if self.policy.deny.contains(name) {
            return self.policy.denial(name);
        }
        if self.policy.allow.contains(name) {
            return None;
        }
        match self.gate.mode() {
            PermissionMode::Headless => self.policy.denial(name),
            PermissionMode::Ask | PermissionMode::Allow => None,
        }
    }
}

struct GatedTool {
    inner: Arc<dyn Tool>,
    gate: PermissionGate,
    pre_allowed: bool,
}

#[async_trait]
impl Tool for GatedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn execution_mode(&self) -> ExecutionMode {
        self.inner.execution_mode()
    }

    fn prompt_snippet(&self) -> &str {
        self.inner.prompt_snippet()
    }

    async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
        updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        let needs_decision = !self.pre_allowed
            && !is_read_only(&call.name)
            && self.gate.mode() != PermissionMode::Allow;
        if needs_decision {
            let decision = self
                .gate
                .request(&call.name, &call.id, &call.arguments, &context.cancel)
                .await;
            if decision == PermissionDecision::Deny {
                return ToolResult::error(format!(
                    "Permission denied for tool `{}` by the user.",
                    call.name
                ));
            }
        }
        self.inner.execute(call, context, updates).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    struct FakeTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "edit"
        }

        fn description(&self) -> &str {
            "edits"
        }

        fn schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(
            &self,
            _call: &ToolCall,
            _context: &ToolContext,
            _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
        ) -> ToolResult {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            ToolResult::text("edited")
        }
    }

    struct OneTool {
        tool: Arc<dyn Tool>,
    }

    impl Toolbox for OneTool {
        fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
            (name == self.tool.name()).then(|| Arc::clone(&self.tool))
        }

        fn declarations(&self) -> Vec<ToolDecl> {
            Vec::new()
        }
    }

    fn context() -> ToolContext {
        ToolContext {
            cwd: std::path::PathBuf::from("."),
            cancel: CancellationToken::new(),
            session_id: None,
            delegation_depth: 0,
            agent: None,
        }
    }

    fn call() -> ToolCall {
        ToolCall {
            id: "call-1".to_owned(),
            name: "edit".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
        }
    }

    fn toolbox(gate: PermissionGate) -> RpcToolbox {
        let calls = Arc::new(AtomicUsize::new(0));
        RpcToolbox::new(
            Arc::new(OneTool {
                tool: Arc::new(FakeTool { calls }),
            }),
            Vec::new(),
            gate,
        )
    }

    #[tokio::test]
    #[spec("PROTO-007")]
    async fn spec_proto_007_headless_mode_denies_mutations() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let gate = PermissionGate::new(sender, Vec::new());
        let toolbox = toolbox(gate);
        assert!(toolbox.denial("edit").is_some());
        assert!(toolbox.denial("read").is_none());
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    #[spec("PROTO-007")]
    async fn spec_proto_007_ask_mode_prompts_and_honors_decisions() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let gate = PermissionGate::new(sender, Vec::new());
        gate.set_mode(PermissionMode::Ask);
        let toolbox = toolbox(gate.clone());
        assert!(toolbox.denial("edit").is_none());

        let tool = toolbox.get("edit").expect("tool");

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            async move { tool.execute(&call(), &context(), None).await }
        });
        let prompt = receiver.recv().await.expect("permission prompt");
        assert_eq!(prompt.tool, "edit");
        assert_eq!(prompt.tool_call_id, "call-1");
        assert_eq!(prompt.arguments["path"], "a.txt");
        prompt.reply.send(PermissionDecision::Deny).unwrap();
        let denied = pending.await.unwrap();
        assert!(denied.is_error);
        assert!(denied.content[0].as_text().unwrap().contains("denied"));

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            async move { tool.execute(&call(), &context(), None).await }
        });
        let prompt = receiver.recv().await.expect("permission prompt");
        prompt.reply.send(PermissionDecision::AllowAlways).unwrap();
        assert!(!pending.await.unwrap().is_error);
        assert!(gate.is_allowed("edit"));

        // allow_always skips the next prompt.
        let result = tool.execute(&call(), &context(), None).await;
        assert!(!result.is_error);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    #[spec("PROTO-007")]
    async fn spec_proto_007_abort_denies_a_pending_prompt() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let gate = PermissionGate::new(sender, Vec::new());
        gate.set_mode(PermissionMode::Ask);
        let toolbox = toolbox(gate);
        let tool = toolbox.get("edit").expect("tool");
        let cancel = CancellationToken::new();

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            let cancel = cancel.clone();
            async move {
                tool.execute(
                    &call(),
                    &ToolContext {
                        cwd: std::path::PathBuf::from("."),
                        cancel,
                        session_id: None,
                        delegation_depth: 0,
                        agent: None,
                    },
                    None,
                )
                .await
            }
        });
        let _prompt = receiver.recv().await.expect("permission prompt");
        cancel.cancel();
        let denied = pending.await.unwrap();
        assert!(denied.is_error);
    }

    #[tokio::test]
    #[spec("PROTO-007")]
    async fn spec_proto_007_allow_mode_skips_prompts() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let gate = PermissionGate::new(sender, Vec::new());
        gate.set_mode(PermissionMode::Allow);
        let toolbox = toolbox(gate);
        let result = toolbox
            .get("edit")
            .expect("tool")
            .execute(&call(), &context(), None)
            .await;
        assert!(!result.is_error);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    #[spec("PROTO-009")]
    fn spec_proto_009_config_allow_changes_replace_pre_approval() {
        let (sender, _receiver) = mpsc::unbounded_channel();
        let gate = PermissionGate::new(sender, vec!["edit".to_owned()]);
        assert!(gate.is_allowed("edit"));
        assert!(!gate.is_allowed("write"));

        gate.allow_always("bash");
        gate.set_configured(vec!["write".to_owned()]);
        assert!(
            !gate.is_allowed("edit"),
            "removed entries stop being allowed"
        );
        assert!(gate.is_allowed("write"), "new entries are allowed");
        assert!(
            gate.is_allowed("bash"),
            "session grants survive a configuration change"
        );
    }
}
