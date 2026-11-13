//! A tool implemented by an extension process.

use std::sync::Arc;

use async_trait::async_trait;
use codie_core::{ExecutionMode, Tool, ToolCall, ToolContext, ToolProgress, ToolResult};
use codie_protocol::ext::{ExtExecutionMode, ExtToolCallParams, ExtToolDecl, ExtToolResult};
use tokio::sync::mpsc;

use crate::Extension;
use crate::convert;

/// One model-visible tool backed by an extension.
pub struct ExtTool {
    extension: Arc<Extension>,
    declaration: ExtToolDecl,
}

impl ExtTool {
    /// Creates a tool from an extension declaration.
    #[must_use]
    pub fn new(extension: Arc<Extension>, declaration: ExtToolDecl) -> Self {
        Self {
            extension,
            declaration,
        }
    }
}

#[async_trait]
impl Tool for ExtTool {
    fn name(&self) -> &str {
        &self.declaration.name
    }

    fn description(&self) -> &str {
        &self.declaration.description
    }

    fn schema(&self) -> serde_json::Value {
        self.declaration.parameters.clone()
    }

    fn execution_mode(&self) -> ExecutionMode {
        match self.declaration.execution {
            ExtExecutionMode::Parallel => ExecutionMode::Parallel,
            ExtExecutionMode::Sequential => ExecutionMode::Sequential,
        }
    }

    fn prompt_snippet(&self) -> &str {
        &self.declaration.description
    }

    async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        if context.cancel.is_cancelled() {
            return ToolResult::error("Tool call aborted");
        }
        let session = convert::session_info(context.session_id.as_deref(), &context.cwd, None);
        let params = ExtToolCallParams {
            session,
            call_id: call.id.clone(),
            name: self.declaration.name.clone(),
            arguments: call.arguments.clone(),
        };
        let value = match self
            .extension
            .client
            .request_cancelable(
                "tool.call",
                serde_json::to_value(params).unwrap_or(serde_json::Value::Null),
                self.extension.tool_timeout,
                Some(&context.cancel),
            )
            .await
        {
            Ok(value) => value,
            Err(crate::ExtError::Invalid(_)) => {
                return ToolResult::error("Tool call aborted");
            }
            Err(error) => {
                return ToolResult::error(format!(
                    "Extension `{}` failed tool `{}`: {error}",
                    self.extension.name, self.declaration.name
                ));
            }
        };
        let result: ExtToolResult = match serde_json::from_value(value) {
            Ok(result) => result,
            Err(error) => {
                return ToolResult::error(format!(
                    "Extension `{}` returned an invalid result for `{}`: {error}",
                    self.extension.name, self.declaration.name
                ));
            }
        };
        convert::ext_tool_result(result)
    }
}
