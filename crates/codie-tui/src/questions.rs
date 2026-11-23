//! The `question` tool and its interactive broker.
//!
//! A child run can need a decision the parent cannot make for it ("which
//! breakpoint matters?"). The tool sends one request to the interface and
//! returns the answer as text; a dismissed prompt returns an explicit refusal
//! so the model continues instead of guessing.

use std::sync::Arc;

use async_trait::async_trait;
use codie_core::{
    Content, ExecutionMode, Tool, ToolCall, ToolContext, ToolDecl, ToolProgress, ToolResult,
    Toolbox,
};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

/// Wire name of the host question tool.
pub(crate) const QUESTION_TOOL_NAME: &str = "question";

/// One pending question shown by the interface.
#[derive(Debug)]
pub struct QuestionRequest {
    /// Question text.
    pub question: String,
    /// Agent type that asked, when delegated.
    pub agent: Option<String>,
    /// Session id of the asking run, when persisted.
    pub session_id: Option<String>,
    /// Reply channel; dropping it dismisses the question.
    pub reply: oneshot::Sender<Option<String>>,
}

/// Shared handle used by the tool to ask the interface.
#[derive(Clone, Debug)]
pub(crate) struct QuestionBroker {
    sender: mpsc::UnboundedSender<QuestionRequest>,
}

impl QuestionBroker {
    /// Creates a broker over the interface channel.
    #[must_use]
    pub(crate) fn new(sender: mpsc::UnboundedSender<QuestionRequest>) -> Self {
        Self { sender }
    }

    /// Asks one question, returning the answer or `None` when dismissed.
    pub(crate) async fn ask(
        &self,
        question: &str,
        agent: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<String> {
        let (reply, answer) = oneshot::channel();
        if self
            .sender
            .send(QuestionRequest {
                question: question.to_owned(),
                agent: agent.map(str::to_owned),
                session_id: session_id.map(str::to_owned),
                reply,
            })
            .is_err()
        {
            return None;
        }
        answer.await.ok().flatten()
    }
}

/// A toolbox wrapper that adds `question` to the parent and its children.
pub(crate) struct QuestionToolbox {
    inner: Arc<dyn Toolbox>,
    broker: QuestionBroker,
}

impl QuestionToolbox {
    /// Wraps `inner`, adding the host question tool.
    #[must_use]
    pub(crate) fn new(inner: Arc<dyn Toolbox>, broker: QuestionBroker) -> Self {
        Self { inner, broker }
    }
}

impl Toolbox for QuestionToolbox {
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if name == QUESTION_TOOL_NAME {
            return Some(Arc::new(QuestionTool::new(self.broker.clone())));
        }
        self.inner.get(name)
    }

    fn declarations(&self) -> Vec<ToolDecl> {
        let mut declarations = self.inner.declarations();
        if !declarations
            .iter()
            .any(|declaration| declaration.name == QUESTION_TOOL_NAME)
        {
            declarations.push(QuestionTool::declaration());
        }
        declarations
    }

    fn denial(&self, name: &str) -> Option<String> {
        self.inner.denial(name)
    }
}

/// The `question` tool itself.
struct QuestionTool {
    broker: QuestionBroker,
}

impl QuestionTool {
    fn new(broker: QuestionBroker) -> Self {
        Self { broker }
    }

    fn declaration() -> ToolDecl {
        ToolDecl {
            name: QUESTION_TOOL_NAME.to_owned(),
            description: "Ask the user one clarifying question. Use this when a decision blocks \
                          progress and guessing would be costly. Returns the answer, or a notice \
                          that the user dismissed the prompt."
                .to_owned(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The question to ask; include the options when there are few."
                    }
                },
                "required": ["question"],
                "additionalProperties": false
            }),
        }
    }
}

#[async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &str {
        QUESTION_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Ask the user one clarifying question."
    }

    fn schema(&self) -> Value {
        Self::declaration().parameters
    }

    fn execution_mode(&self) -> ExecutionMode {
        // Waiting for a human must not hold a parallel batch open behind it.
        ExecutionMode::Sequential
    }

    async fn execute(
        &self,
        call: &ToolCall,
        _context: &ToolContext,
        _updates: Option<mpsc::UnboundedSender<ToolProgress>>,
    ) -> ToolResult {
        let Some(question) = call
            .arguments
            .get("question")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|question| !question.is_empty())
        else {
            return ToolResult::error("`question` must be a non-empty string.");
        };
        match self
            .broker
            .ask(
                question,
                _context.agent.as_deref(),
                _context.session_id.as_deref(),
            )
            .await
        {
            Some(answer) if !answer.trim().is_empty() => ToolResult {
                content: vec![Content::text(format!(
                    "The user answered \"{question}\": {answer}"
                ))],
                details: Some(json!({"question": question, "answer": answer})),
                is_error: false,
                terminate: false,
                duration_ms: None,
            },
            _ => ToolResult::text("The user dismissed the question."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codie_testkit::spec;
    use tokio::sync::mpsc;

    #[tokio::test]
    #[spec("AGENT-035")]
    async fn answers_round_trip_through_the_broker() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let broker = QuestionBroker::new(sender);
        let asking = broker.clone();
        let pending =
            tokio::spawn(async move { asking.ask("Which breakpoint matters?", None, None).await });

        let request = receiver.recv().await.expect("question request");
        assert_eq!(request.question, "Which breakpoint matters?");
        request.reply.send(Some("768px".to_owned())).unwrap();

        assert_eq!(pending.await.unwrap().as_deref(), Some("768px"));
    }

    #[tokio::test]
    async fn dismissal_returns_none() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let broker = QuestionBroker::new(sender);
        let asking = broker.clone();
        let pending = tokio::spawn(async move { asking.ask("Ship it?", None, None).await });

        let request = receiver.recv().await.expect("question request");
        request.reply.send(None).unwrap();
        assert_eq!(pending.await.unwrap(), None);
    }

    #[tokio::test]
    #[spec("TUI-045")]
    async fn tool_reports_the_answer_and_dismissal() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let tool = QuestionTool::new(QuestionBroker::new(sender));
        let call = ToolCall {
            id: "q1".to_owned(),
            name: QUESTION_TOOL_NAME.to_owned(),
            arguments: serde_json::json!({"question": "Which header?"}),
        };
        let running = tokio::spawn({
            let call = call.clone();
            async move { tool.execute(&call, &ToolContext::default(), None).await }
        });
        let request = receiver.recv().await.expect("question request");
        request
            .reply
            .send(Some("The compact one.".to_owned()))
            .unwrap();
        let result = running.await.unwrap();
        assert!(!result.is_error);
        let text = result.content[0].as_text().unwrap();
        assert!(text.contains("The compact one."), "{text}");
        assert_eq!(result.details.unwrap()["answer"], "The compact one.");

        let (sender, mut receiver) = mpsc::unbounded_channel();
        let tool = QuestionTool::new(QuestionBroker::new(sender));
        let running = tokio::spawn({
            let call = call.clone();
            async move { tool.execute(&call, &ToolContext::default(), None).await }
        });
        receiver
            .recv()
            .await
            .expect("question request")
            .reply
            .send(None)
            .unwrap();
        let result = running.await.unwrap();
        assert!(result.content[0].as_text().unwrap().contains("dismissed"));
    }

    #[test]
    fn toolbox_adds_one_declaration() {
        let (sender, _receiver) = mpsc::unbounded_channel();
        let toolbox = QuestionToolbox::new(
            Arc::new(codie_tools::Registry::new(Vec::new())),
            QuestionBroker::new(sender),
        );
        let names = toolbox
            .declarations()
            .into_iter()
            .map(|declaration| declaration.name)
            .collect::<Vec<_>>();
        assert_eq!(names, vec![QUESTION_TOOL_NAME.to_owned()]);
        assert!(toolbox.get(QUESTION_TOOL_NAME).is_some());
    }
}
