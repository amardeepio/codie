---
id: SPEC-TUI-004
title: Permission choices and full-access mode
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TUI-018", "TUI-019"]
---

# Context

The permission overlay must be unambiguous and keyboard-only. Users expect `y`/`n` (case-insensitive) rather than
tool-specific letters, and a quick way to stop being prompted during a trusted session. This spec covers the
prompt keys and the runtime full-access toggle; the policy contract remains in `SPEC-SEC-001`.

# Normative behavior

**TUI-018 (MUST)** When a permission prompt is visible, `y` or `Y` allows the call, `n` or `N` denies it, `A`
allows the tool for the rest of the run, `Enter` allows, and `Esc` denies. The prompt text names these keys.
Verification: `crates/codie-tui/src/app.rs::spec_tui_018_permission_prompt_accepts_y_and_n`

**TUI-019 (MUST)** `Shift+Tab` toggles full-access mode. While enabled, mutating tools run without sending a
permission request; disabling it restores prompts. Toggling it while a prompt is visible allows that call. The
footer shows `FULL ACCESS` while the mode is on.
Verification: `crates/codie-tui/src/app.rs::spec_tui_019_shift_tab_toggles_full_access`

# State machine / data shapes

| Key | Decision |
|---|---|
| `y` / `Y` | allow once |
| `n` / `N` / `Esc` | deny |
| `A` | allow this tool for the rest of the run |
| `Enter` | allow once |
| `Shift+Tab` | toggle full access for the session |

Full access is shared between the UI and the tool wrapper through an atomic flag on the permission broker. It is
in-memory only and ends with the process.

# Edge cases

- `Shift+Tab` is delivered as `BackTab` by some terminals and as `Tab+Shift` by others; both toggle the mode.
- Turning full access on while a prompt is queued allows the visible call and skips later prompts.
- Turning it off does not revoke tools already allowed with `A`.
- Plain mode accepts `y`/`Y`/`a`/`A` from stdin and treats anything else as a denial.

# Non-goals

Persisting full access across runs, per-tool trust policies, and project-level trust files are Phase 4 work.

# Security and performance

Full access is an explicit, visible opt-in and never enabled by default. It bypasses prompts, not the tool
policy object itself, and the footer makes the state visible. Toggling is O(1) and sends no I/O.
