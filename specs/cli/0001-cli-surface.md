---
id: SPEC-CLI-001
title: CLI surface, configuration, and exit codes
status: implemented
version: 1.1.0
owners: ["@codie-maintainers"]
requirements: ["CLI-001", "CLI-002", "CLI-003", "CLI-004", "CLI-005", "CLI-006", "CLI-007", "CLI-008"]
---

# Context

`codie` is one binary with a one-shot run path and three management subcommands (`auth`, `models`, `config`).
Modes are explicit: the default is print when stdin/stdout is not a terminal, and interactive mode arrives in
Phase 2. Phase 1 supports `-p/--print` and `--mode json`.

# Requirements

**CLI-001 (MUST)** `--help` is generated from the same clap definitions used for parsing and documents every Phase 1 flag and subcommand.
Verification: `crates/codie-cli/tests/cli.rs::help_flag_documents_the_surface`

**CLI-002 (MUST)** Configuration layers are CLI flag, then environment, then `config.toml`, then the default. Every effective value is inspectable with `codie config get [key]`.
Verification: `crates/codie-cli/tests/cli.rs::config_get_reports_effective_values`

**CLI-003 (MUST)** Exit codes are: 0 success, 1 usage/runtime error, 2 auth check with no configured provider, 130 SIGINT, 143 SIGTERM, 129 SIGHUP.
Verification: `crates/codie-cli/tests/e2e.rs::signals_map_to_documented_exit_codes`

**CLI-004 (MUST)** `--mode json` writes only JSONL records to stdout; diagnostics go to stderr.
Verification: `crates/codie-cli/tests/e2e.rs::golden_path_end_to_end_with_a_fake_provider`

**CLI-005 (MUST)** Unknown flags fail fast with a usage error and exit 1 when no loaded extension claims them.
Verification: `crates/codie-cli/tests/cli.rs::spec_cli_005_unknown_flags_fail_fast`

**CLI-006 (MUST)** `--api-key` requires an explicit `--model` and is never written to disk.
Verification: `crates/codie-cli/tests/cli.rs::spec_prov_034_api_key_is_ephemeral`

**CLI-007 (MUST)** Startup performs no network I/O before the first output. The OpenRouter catalog refresh is spawned after the session header and runs in the background.
Verification: `crates/codie-cli/tests/cli.rs::spec_cli_007_startup_makes_no_network_call`

**CLI-008 (MUST)** Unknown long flags are deferred until extensions load. A flag claimed by an extension is passed in the initialize handshake; an unclaimed long flag, an unknown short flag, or a value-shape mismatch exits 1. Management subcommands never defer flags.
Verification: `crates/codie-cli/tests/extensions.rs::spec_ext_015_extension_flags_are_validated`

# Exit-code table

| Situation | Code |
|---|---|
| Completed run (text) | 0 |
| Usage or runtime error | 1 |
| Truncated response, aborted run, or non-completed status | 1 |
| `auth status` with no configured provider | 2 |
| JSON mode that completes | 0 (failures appear in the event stream) |
| SIGINT / SIGTERM / SIGHUP | 130 / 143 / 129 |

# Security and performance

`--api-key` is accepted only on the command line and is never echoed. `.env` is loaded only from the current
directory, or from an explicit `--env-file`; existing process variables always win.
