//! Interactive permission prompts (`TUI-005`, Phase 2 policy).
//!
//! Read-only tools and explicitly allowed tools never prompt. Everything else
//! sends one [`PermissionRequest`] to the UI and waits for a decision. Choosing
//! "always" adds the tool to the in-memory allow set for this run.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use codie_core::{
    ExecutionMode, Tool, ToolCall, ToolContext, ToolDecl, ToolProgress, ToolResult, Toolbox,
};
use codie_tools::is_read_only;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// A user's answer to a permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Allow this one call.
    Allow,
    /// Allow this tool for the rest of the run.
    Always,
    /// Deny the call.
    Deny,
}

/// One pending permission prompt.
#[derive(Debug)]
pub struct PermissionRequest {
    /// Tool name.
    pub tool: String,
    /// Parsed arguments.
    pub arguments: Value,
    /// Reply channel; dropping it denies the call.
    pub reply: oneshot::Sender<Decision>,
}

/// Shared permission state between the UI and tool wrappers.
#[derive(Clone)]
pub struct PermissionBroker {
    sender: mpsc::UnboundedSender<PermissionRequest>,
    allowed: Arc<Mutex<HashSet<String>>>,
}

impl PermissionBroker {
    /// Creates a broker that auto-allows the given tool names.
    #[must_use]
    pub fn new(
        sender: mpsc::UnboundedSender<PermissionRequest>,
        auto_allowed: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            sender,
            allowed: Arc::new(Mutex::new(auto_allowed.into_iter().collect())),
        }
    }

    /// Returns true when a call may run without prompting.
    #[must_use]
    pub fn is_allowed(&self, tool: &str) -> bool {
        is_read_only(tool)
            || self
                .allowed
                .lock()
                .is_ok_and(|allowed| allowed.contains(tool))
    }

    /// Remembers a tool for the rest of the run.
    pub fn allow_always(&self, tool: &str) {
        if let Ok(mut allowed) = self.allowed.lock() {
            allowed.insert(tool.to_owned());
        }
    }

    /// Asks the UI for a decision. A closed UI denies the call.
    pub async fn request(&self, tool: &str, arguments: &Value) -> Decision {
        if self.is_allowed(tool) {
            return Decision::Allow;
        }
        let (reply, answer) = oneshot::channel();
        if self
            .sender
            .send(PermissionRequest {
                tool: tool.to_owned(),
                arguments: arguments.clone(),
                reply,
            })
            .is_err()
        {
            return Decision::Deny;
        }
        match answer.await {
            Ok(Decision::Always) => {
                self.allow_always(tool);
                Decision::Always
            }
            Ok(decision) => decision,
            Err(_) => Decision::Deny,
        }
    }
}

/// A toolbox that asks for permission before mutating calls.
pub struct PermissionToolbox {
    inner: Arc<dyn Toolbox>,
    broker: PermissionBroker,
}

impl PermissionToolbox {
    /// Wraps `inner` with interactive permission checks.
    #[must_use]
    pub fn new(inner: Arc<dyn Toolbox>, broker: PermissionBroker) -> Self {
        Self { inner, broker }
    }

    /// Returns the wrapped toolbox.
    #[must_use]
    pub fn inner(&self) -> &Arc<dyn Toolbox> {
        &self.inner
    }
}

impl Toolbox for PermissionToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.inner.get(name).map(|inner| {
            Arc::new(PermissionTool {
                inner,
                broker: self.broker.clone(),
            }) as Arc<dyn Tool>
        })
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        self.inner.declarations()
    }

    fn denial(&self, name: &str) -> Option<String> {
        self.inner.denial(name)
    }
}

struct PermissionTool {
    inner: Arc<dyn Tool>,
    broker: PermissionBroker,
}

#[async_trait]
impl Tool for PermissionTool {
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
        let decision = self.broker.request(&call.name, &call.arguments).await;
        if matches!(decision, Decision::Deny) {
            return ToolResult::error(format!(
                "Permission denied for tool `{}` by the user.",
                call.name
            ));
        }
        self.inner.execute(call, context, updates).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "write"
        }

        fn description(&self) -> &str {
            "writes"
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            ToolResult::text("written")
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
            cancel: tokio_util::sync::CancellationToken::new(),
            session_id: None,
        }
    }

    fn call() -> ToolCall {
        ToolCall {
            id: "call_1".to_owned(),
            name: "write".to_owned(),
            arguments: serde_json::json!({"path": "a.txt"}),
        }
    }

    #[tokio::test]
    async fn allow_once_executes_and_deny_blocks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Arc::new(OneTool {
            tool: Arc::new(FakeTool {
                calls: Arc::clone(&calls),
            }),
        });
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let broker = PermissionBroker::new(sender, Vec::new());
        let toolbox = PermissionToolbox::new(inner, broker);
        let tool = toolbox.get("write").expect("tool");

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            async move { tool.execute(&call(), &context(), None).await }
        });
        let request = receiver.recv().await.expect("permission request");
        assert_eq!(request.tool, "write");
        request.reply.send(Decision::Allow).unwrap();
        assert!(!pending.await.unwrap().is_error);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            async move { tool.execute(&call(), &context(), None).await }
        });
        let request = receiver.recv().await.expect("permission request");
        request.reply.send(Decision::Deny).unwrap();
        assert!(pending.await.unwrap().is_error);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn always_skips_later_prompts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let inner = Arc::new(OneTool {
            tool: Arc::new(FakeTool {
                calls: Arc::clone(&calls),
            }),
        });
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let toolbox = PermissionToolbox::new(inner, PermissionBroker::new(sender, Vec::new()));
        let tool = toolbox.get("write").expect("tool");

        let pending = tokio::spawn({
            let tool = Arc::clone(&tool);
            async move { tool.execute(&call(), &context(), None).await }
        });
        let request = receiver.recv().await.expect("permission request");
        request.reply.send(Decision::Always).unwrap();
        assert!(!pending.await.unwrap().is_error);

        let result = tool.execute(&call(), &context(), None).await;
        assert!(!result.is_error);
        assert!(
            receiver.try_recv().is_err(),
            "an always-allowed tool must not prompt again"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn read_only_tools_never_prompt() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let broker = PermissionBroker::new(sender, Vec::new());
        assert!(broker.is_allowed("read"));
        assert!(!broker.is_allowed("write"));
        assert!(receiver.try_recv().is_err());
    }
}
