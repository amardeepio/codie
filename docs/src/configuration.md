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
| `CODIE_ALLOW` | Comma-separated mutating tools allowed in headless mode. |
| `CODIE_CONFIG_DIR` | Config directory (default `~/.config/codie`). |
| `CODIE_DATA_DIR` | Data directory (default `~/.local/share/codie`). |
| `CODIE_SESSION_DIR` | Session root (default `<data>/sessions`). |
| `CODIE_ENV_FILE` | Explicit `.env` file to load when `--env-file` is absent. |

CLI flags override environment values. For example, `--thinking high` wins over `CODIE_THINKING=low`.

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
allow = ["edit", "bash"]
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
