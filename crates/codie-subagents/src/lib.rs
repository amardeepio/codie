//! In-process subagent delegation.
//!
//! The `task` tool starts a child [`codie_core::Agent`] run with its own
//! transcript, model, turn budget, cancellation token, and tool subset. Child
//! progress is reported through the parent tool's progress channel, so the
//! parent event stream is untouched (`SPEC-AGENT-003`).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod intent;
pub mod runner;
pub mod tool;

mod budget;

pub use config::{
    AgentTypeConfig, DEFAULT_MAX_DEPTH_LIMIT, DEFAULT_MAX_PARALLEL_LIMIT, DEFAULT_MAX_TURNS,
    DEFAULT_RESULT_MAX_BYTES, DEFAULT_TIMEOUT_SECS, SubagentConfig, ToolSelection,
};
pub use intent::{
    DelegationIntent, DelegationMode, DelegationPlan, FORCED_DIRECTIVE, FORCED_NUDGE,
    INTENT_WINDOW, NudgeState, detect_intent, effective_mode, mode_directive,
};
pub use runner::{RunSnapshot, RunState, SubagentRuntime, SubagentToolbox};
pub use tool::{
    TASK_DESCRIPTION, TASK_TOOL_NAME, TaskArgs, TaskOutcome, TaskTool, describe_types, schema,
};
