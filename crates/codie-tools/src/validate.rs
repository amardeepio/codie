//! JSON Schema validation for tool arguments (`TOOL-002`).
//!
//! Every built-in tool declares a schema in [`codie_core::Tool::schema`]. The
//! model can still send arguments that do not satisfy it, so the registry wraps
//! each tool in a [`ValidatedTool`] that rejects invalid arguments with an
//! error result. A validation failure is a tool error, never a run failure, so
//! the model can correct the call on the next turn.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use tokio::sync::mpsc;

/// Maximum number of schema violations included in one error result.
const MAX_REPORTED_VIOLATIONS: usize = 8;

/// A tool wrapper that validates arguments before delegating.
pub struct ValidatedTool {
    inner: Arc<dyn Tool>,
    schema: serde_json::Value,
    validator: OnceLock<Result<jsonschema::Validator, String>>,
}

impl ValidatedTool {
    /// Wraps `inner`, compiling its schema lazily on the first call.
    #[must_use]
    pub fn new(inner: Arc<dyn Tool>) -> Self {
        let schema = inner.schema();
        Self {
            inner,
            schema,
            validator: OnceLock::new(),
        }
    }

    /// Returns the wrapped tool.
    #[must_use]
    pub fn inner(&self) -> &Arc<dyn Tool> {
        &self.inner
    }

    fn validate(&self, arguments: &serde_json::Value) -> Result<(), String> {
        let validator = self.validator.get_or_init(|| {
            jsonschema::validator_for(&self.schema)
                .map_err(|error| format!("tool schema is invalid: {error}"))
        });

        let validator = match validator {
            Ok(validator) => validator,
            Err(message) => return Err(message.clone()),
        };

        let violations = validator
            .iter_errors(arguments)
            .take(MAX_REPORTED_VIOLATIONS)
            .map(|error| {
                let path = error.instance_path().to_string();
                let path = if path.is_empty() {
                    "$".to_owned()
                } else {
                    path
                };
                format!("{path}: {error}")
            })
            .collect::<Vec<_>>();

        if violations.is_empty() {
            return Ok(());
        }

        Err(violations.join("; "))
    }
}

#[async_trait]
impl Tool for ValidatedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> serde_json::Value {
        self.schema.clone()
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
        if let Err(message) = self.validate(&call.arguments) {
            return ToolResult::error(format!(
                "Invalid arguments for tool `{}`: {message}",
                self.name()
            ));
        }
        self.inner.execute(call, context, updates).await
    }
}

impl std::fmt::Debug for ValidatedTool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ValidatedTool")
            .field("name", &self.inner.name())
            .finish_non_exhaustive()
    }
}
