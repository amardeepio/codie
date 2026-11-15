# Benchmark: codie vs opencode

This folder measures two things on the same machine, with the same model and the same prompts:

1. **Startup** — binary launch and a trivial model round-trip.
2. **Coding tasks** — a four-task suite with hidden, task-specific verification.

The model is pinned to `deepseek/deepseek-v4.1-flash-1017` through OpenRouter for both harnesses.

## Task suite

| Task | Verifier | What it measures |
|---|---|---|
| `todo-ui` | Headless Chrome | Greenfield Next.js todo app: add (Enter + button), toggle, delete, filters, clear-completed, localStorage persistence |
| `feature-add` | Headless Chrome | Extend a working basic todo app with filters, clear-completed, and persistence |
| `bug-fix` | Hidden Node tests | Fix three seeded logic bugs without touching tests, signatures, or dependencies |
| `api-todo` | HTTP | Add a JSON CRUD API with validation, 404s, and an in-memory store |

Each task has its own prompt under `tasks/<name>/prompt.md`. Hidden verifiers live in
`tasks/<name>/verify/` and `verifier/`; they are never present while the harness is coding.

A run counts as a success only when the harness exits cleanly, the project builds, the hidden verifier
passes, the task's forbidden files are untouched, and `package.json` is unchanged.

## Requirements

- Python 3.9+
- Node.js and npm
- Google Chrome (the browser verifier uses `playwright-core` with the system Chrome; set `CHROME_PATH` for
  other installations)
- `opencode` on `PATH`
- a built codie binary at `target/release/codie` (or pass `--build`)
- `OPENROUTER_API_KEY` in the environment or the repository `.env`

The first run creates the Next.js scaffold, installs dependencies, and installs `playwright-core` into
`benchmark/verifier/node_modules`. Later runs reuse `benchmark/.cache/next-template`.

## Run

```bash
# Full benchmark: startup + 3 runs per harness for all four tasks
./benchmark/run.sh all --build --yes

# One task, one run per harness
./benchmark/run.sh task --tasks todo-ui --runs 1 --yes

# Startup only
./benchmark/run.sh startup --yes

# Show commands without spending tokens
./benchmark/run.sh all --dry-run
```

The full suite is 24 model runs (4 tasks × 3 repetitions × 2 harnesses). Each run is capped at 20 minutes and
the build at 15 minutes. The script refuses to call the paid API without `--yes`.

## Fairness rules

- The same prompt file, scaffold, dependencies, and pinned model are used for both harnesses.
- Every run starts from a fresh copy of the scaffold, including `node_modules`.
- Tool permissions are auto-approved: codie `--allow-all`, opencode `--auto`.
- Harness order alternates each repetition.
- Work directories are created outside the repository, so neither harness reads codie's project config.
- `PWD` is rewritten to each work directory; opencode also gets an isolated `XDG_*` home and `--standalone`.
- The shared scaffold is hashed before and after every run; any change aborts the benchmark.
- codie uses `--no-session`; opencode uses `--archived`.
- Hidden verifiers run only after the harness exits.

## Telemetry

The harness parses each JSONL stream for:

- model rounds (codie `turn_start`, opencode `step_finish`)
- tool calls and tool errors
- provider error events
- time to first write/edit tool call
- input, output, reasoning, and cache-read tokens
- cost (codie nano-USD, opencode USD)

The report also computes bootstrap 95% confidence intervals for median times and Wilson intervals for success
rates.

## Results

Results are written to `benchmark/results/<timestamp>/`:

```
results/20260101-120000/
├── results.json          # raw metrics, telemetry, per-run records
├── summary.md            # compact tables
├── report.md             # detailed report
├── logs/                 # harness JSONL, build output, verifier output
└── snapshots/            # changed source files from each run
```

`results/`, `.cache/`, and `verifier/node_modules/` are gitignored. Only this README, `bench.py`, `report.py`,
`run.sh`, `REPORT.md`, `verifier/` (including its lockfile), `tasks/`, and `.gitignore` belong in git.

## Detailed report

`report.py` turns the latest `results.json` into a long-form report with per-task comparisons, confidence
intervals, verification results, telemetry, and cost per success:

```bash
python3 benchmark/report.py                    # writes results/<timestamp>/report.md
python3 benchmark/report.py --publish          # also copies it to benchmark/REPORT.md
```

[`REPORT.md`](REPORT.md) is a snapshot of the most recent full run. Raw data stays under `results/` and is not
committed.

## Caveats

- Model output is nondeterministic. Use several runs; the report shows confidence intervals rather than
  presenting a single number as exact.
- The harnesses have different system prompts and tool descriptions. That is inherent to comparing harnesses.
- Network latency, OpenRouter routing, and machine load add noise.
- Browser checks use accessible roles and visible text; unusual markup can cause a false failure. Inspect
  `results/<timestamp>/logs/verify-*.log`.
- Token and cost totals come from each harness's JSON stream; missing usage understates totals.
- Verification builds run after the harness exits and may reuse its `.next` cache. Treat them as correctness
  checks, not clean-build benchmarks.
- The benchmark calls the paid API.
