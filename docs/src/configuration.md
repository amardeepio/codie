# Configuration and environment

codie reads configuration from three layers. Later layers do not override earlier ones:

1. **CLI flags** (`--provider`, `--model`, `--thinking`, `--max-turns`, `--allow`, ...).
2. **Process environment**, including values loaded from `.env`.
3. **`config.toml`** in the codie config directory.
4. **Built-in defaults.**

## Environment variables

| Variable | Meaning |
|---|---|
| `DEEPSEEK_API_KEY` | DeepSeek API key. |
| `OPENROUTER_API_KEY` | OpenRouter API key. |
| `CODIE_PROVIDER` | Default provider (`deepseek` or `openrouter`). |
| `CODIE_MODEL` | Default model id, used verbatim. |
| `CODIE_THINKING` | `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, or `max`. |
| `CODIE_MAX_TURNS` | Maximum assistant requests per run. Must be greater than zero. |
| `CODIE_ALLOW` | Comma-separated mutating tools allowed in headless mode (`write`, `edit`, `bash`, or `powershell`). |
| `CODIE_BASE_URL` | Provider base URL override (proxy or local test endpoint). |
| `CODIE_CONFIG_DIR` | Config directory (default `~/.config/codie`). |
| `CODIE_DATA_DIR` | Data directory (default `~/.local/share/codie`). |
| `CODIE_CACHE_DIR` | Cache root, including the OpenRouter catalog (default `<data>/cache`). |
| `CODIE_SESSION_DIR` | Session root (default `<data>/sessions`). |
| `CODIE_NO_UPDATE_CHECK` | Set to disable the background update check in the interactive app. |
| `CODIE_ENV_FILE` | Explicit `.env` file to load when `--env-file` is absent. |
| `CODIE_RECORD_FIXTURES` | Opt in to fixture recording when built with `--features fixtures`. |
| `CODIE_FIXTURE_DIR` | Directory for recorded fixtures (default `target/fixtures`). |

CLI flags override environment values. For example, `--thinking high` wins over `CODIE_THINKING=low`.

On Windows the defaults expand under `%USERPROFILE%`: `\.config\codie`, `\.local\share\codie`,
`\.local\share\codie\cache`, and `\.local\share\codie\sessions`. Set the `CODIE_*_DIR` variables to relocate
them.

Inspect the effective values with `codie config get`, or one key with `codie config get provider`. Change a
value with `codie config set provider openrouter`, print the file path with `codie config path`, and open it in
`$VISUAL`/`$EDITOR` with `codie config edit`.
`--base-url` overrides `CODIE_BASE_URL` for a single run.

## Customization

| Resource | Location | Purpose |
|---|---|---|
| Skills | `.codie/skills/<name>/SKILL.md`, `~/.config/codie/skills/` | Reusable instruction blocks invoked with `/skill:<name>` |
| Templates | `.codie/templates/<name>.md`, `~/.config/codie/templates/` | Prompts with `$ARGUMENTS` and `$1`..`$9`, invoked with `/template:<name>` |
| System prompt | `SYSTEM.md`, `APPEND_SYSTEM.md` from the working directory upward | Replace or extend the preamble |
| Extensions | `.codie/extensions/<name>/codie.toml`, `~/.config/codie/extensions/` | Out-of-process tools, commands, and hooks |
| Flags | `--system-prompt`, `--append-system-prompt`, `--extension`, `--no-extensions` | Per-run overrides |

Project resources under `.codie/` load only for trusted projects. Approve once with `codie --approve`, or answer
the interactive trust prompt. Non-interactive runs never prompt and treat the project as untrusted. Trust is
stored in `trust.toml` by canonical path. Project extensions follow the same rule.

Load an extension for one run with `--extension <path>`, or disable every extension with
`--no-extensions`. Disable selected names with `[extensions] disabled = ["plan-mode"]` or
`codie config set extensions-disabled plan-mode`. See [Extensions](extensions.md) for the manifest and
protocol.

`@path` in a prompt attaches a file: text becomes a `<file>` block and images become image content for models
that accept it. Use `/reload` in the interactive app to pick up new skills and templates.

Network access is not enabled by default. `web_fetch` requires `--allow web_fetch` (or `--allow-all`) in
headless mode and an approval in the interactive app.

Interactive keybindings live in `keybindings.toml` under the config directory; see
[Interactive terminal](tui.md#keys).

## `.env` files

codie loads `./.env` automatically when it exists. It never searches parent directories. Use an explicit
file when you want a different location:

```bash
codie --env-file ~/.config/codie/development.env models
```

Rules:

- The file is parsed as `KEY=VALUE` lines; `#` starts a comment.
- **Existing process variables are never overwritten.** This makes CI and shell overrides predictable.
- `.env` is gitignored. Commit `.env.example`, never `.env`.
- A `.env` file can set provider keys or any `CODIE_*` variable. Treat it as a secret: it is read with the
  permissions of your user account.
- Loading a missing implicit `.env` is not an error; a missing explicit `--env-file` is.

## `config.toml`

```toml
default_provider = "deepseek"
default_model = "deepseek-flash"
thinking = "medium"
max_turns = 200

[permissions]
allow = ["edit", "bash"]   # use "powershell" on Windows

[extensions]
disabled = []
```

The config directory is `~/.config/codie` on macOS and Linux, overridable with `CODIE_CONFIG_DIR`.

## Credential storage

`codie auth login <provider>` writes `auth.json` in the data directory with mode `0600`. `codie auth status`
reports whether a provider is configured without printing the key. Environment variables are resolved after
an explicit `--api-key` and a stored credential:

```
--api-key  >  auth.json  >  provider environment variable
```

## Precedence summary

```
CLI flag
  > process environment (including .env)
    > config.toml
      > built-in default
```

For credentials specifically: `--api-key` > `auth.json` > environment variable.
