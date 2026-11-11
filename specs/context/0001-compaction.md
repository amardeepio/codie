---
id: SPEC-CTX-001
title: Context compaction and token estimates
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["CTX-002", "CTX-003", "CTX-004", "CTX-005", "CTX-007"]
---

# Context

A long session eventually exceeds the model's context window. Compaction replaces the model-visible prefix with
a summary while retaining the append-only file, and token estimates let the UI show how close the session is to
the limit. This spec covers the estimate, the cut point, the summary shape, append-only persistence, and the
manual `/compact` command.

Automatic threshold triggering (`CTX-001`) and one-shot overflow recovery (`CTX-006`) are deferred to the next
Phase 3 increment; they are not claimed here.

# Normative behavior

**CTX-002 (MUST)** Token estimation uses `chars / 4` for text, thinking, and tool-call JSON, counts an image as
4,800 characters, and reports a context percentage capped at 100. It is never used for billing.
Verification: `crates/codie-session/tests/compaction.rs::spec_ctx_002_token_estimate_uses_chars_and_images`

**CTX-003 (MUST)** The retained tail never starts at a tool result; a compaction requested with a tool result as
its first kept entry is refused.
Verification: `crates/codie-session/tests/compaction.rs::spec_ctx_003_compaction_cannot_start_at_a_tool_result`

**CTX-004 (MUST)** A summary prompt requests exactly the sections Goal, Constraints, Progress, Key Decisions,
Next Steps, Critical Context, plus `<read-files>` and `<modified-files>`; thinking blocks are omitted and tool
output is truncated to 2,000 characters.
Verification: `crates/codie-tui/src/compaction.rs::spec_ctx_004_summary_prompt_has_the_fixed_structure`

**CTX-005 (MUST)** Compaction is append-only: a `compaction` entry records the summary, the first retained entry,
`tokens_before`, optional host details, and optional usage. The projection replaces the hidden prefix with the
summary after the system message. An unknown or non-message first kept entry is refused.
Verification: `crates/codie-session/tests/compaction.rs::spec_ctx_005_compaction_is_append_only_and_rewrites_the_projection`

**CTX-007 (MUST)** Manual `/compact [instructions]` records the user's instructions for the compaction request
and refuses to start one when no session is active.
Verification: `crates/codie-tui/src/app.rs::spec_ctx_007_manual_compact_requires_a_session`

# State machine / data shapes

```json
{"type":"compaction","id":"...","parent_id":"...","timestamp":"...","summary":"## Goal\n...","first_kept_entry_id":"...","tokens_before":24120,"details":{"instructions":"keep tests"},"usage":{...}}
```

Projection order after compaction is: system message (always retained), summary as a synthetic user message,
then every retained message through the leaf. A subsequent compaction on the same branch applies the same rule,
so only the newest summary is model-visible.

# Edge cases

- A session with no messages returns "there is nothing to compact".
- A cut point inside the system message is impossible; the system message is retained by construction.
- Images and reasoning text count toward the estimate even when they are omitted from the summary prompt.
- A provider error or empty summary leaves the session unchanged and reports the failure.
- The retained tail budget is 20,000 tokens; the reserve budget exposed for automatic triggering is 16,384.

# Non-goals

Automatic threshold compaction and one-shot overflow recovery are deferred. HTML export and RPC compaction are
Phase 5. Summary quality evaluation is not specified.

# Security and performance

Summaries contain model output and are stored alongside prompts and tool results. The summarization request is
bounded by the same provider stream and cancellation path as a normal turn. Compaction adds one entry and a new
projection pass; the file is never rewritten.
