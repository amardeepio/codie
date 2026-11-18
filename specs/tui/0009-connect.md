---
id: SPEC-TUI-009
title: Provider connection and key entry
status: accepted
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-033", "TUI-034", "TUI-035", "TUI-036", "TUI-037", "TUI-038"]
---

# Context

A first run has no provider credential. The full-screen interface must open and
offer a way to add a key instead of exiting to a command-line error. Keys are
entered in the terminal, masked by default, checked before they are stored, and
never rendered in the transcript.

# Requirements

**TUI-033 (MUST)** With no usable credential, an interactive run opens the TUI
with the connect overlay for the selected provider. Non-interactive modes keep
the actionable missing-key error.
Verification: `crates/codie-cli/tests/pty.rs::spec_tui_018_first_run_opens_the_connect_overlay`

**TUI-034 (MUST)** `/connect` and `/login` open a provider picker. Selecting a
provider opens a masked key field. Enter checks the key, paste inserts it, and
Esc cancels. `/provider` marks connected providers and offers a connect row.
Verification: `crates/codie-tui/src/app.rs::connect_lists_unconfigured_providers_and_offers_key_entry`

**TUI-035 (MUST)** A key is stored only after an accepted probe. A rejected
probe keeps the overlay open with a sanitized reason and leaves any existing
credential unchanged. Stale probe results are ignored. The masked rendering
never contains the key.
Verification: `crates/codie-tui/src/app.rs::rejected_key_does_not_overwrite_the_stored_key`,
`crates/codie-tui/src/app.rs::stale_connect_results_are_ignored`,
`crates/codie-tui/src/render.rs::connect_overlay_never_renders_a_masked_key`

**TUI-036 (MUST)** A provider authentication failure opens the connect overlay for
the active provider and never hides the failure.
Verification: `crates/codie-tui/src/app.rs::auth_errors_open_the_connect_overlay`

**TUI-037 (MUST)** Provider switching is refused while a run is streaming, with a
message telling the user to finish or cancel first.
Verification: `crates/codie-tui/src/app.rs::provider_switching_is_blocked_while_streaming`

**TUI-038 (MUST)** A successful provider or model selection is persisted so the
next launch can reuse it when configuration does not override the choice.
Verification: `crates/codie-tui/src/app.rs::model_choices_persist_the_selection`

# Interaction

- `/connect <provider>` skips the picker.
- `Ctrl+V` pastes clipboard text, `Ctrl+R` reveals the buffer, and `Ctrl+S` on
  the error screen stores the key without a check.
- A successful check switches the live client, selects the provider's default
  model, and persists a model-change entry.

# Security and performance

The key lives only in the overlay buffer and in `auth.json` after a successful
check. It is never passed to the transcript, session, logs, or a child process.
Checks run in a background task with the shared provider timeout. Closing the
overlay or starting another attempt ignores the stale result.
