# Benchmark report: codie vs opencode

Generated: 2026-11-15T16:33:35.925502+00:00
Results directory: `benchmark/results/20261115-161055`

## Executive summary

| Question | codie | opencode | codie speedup |
|---|---|---|---|
| Cold binary launch (median) | 4 ms | 38 ms | 10.65x faster |
| Trivial request, first token (median) | 1.14 s | 1.79 s | 1.57x faster |
| Trivial request, total (median) | 1.15 s | 1.82 s | 1.58x faster |
| Task suite time (median) | 28.2 s | 59.0 s | 2.09x faster |
| Task suite success | 11/12 (92%) | 12/12 (100%) | - |
| Task suite cost | $0.0501 | $0.1295 | 2.58x cheaper |
| Cost per successful task | $0.0046 | $0.0108 | - |

## How much faster is codie?

All ratios compare codie with opencode. Values above 1x mean codie is faster; percentages are reductions in wall time relative to opencode. Suite values pool every run.

| Metric | codie | opencode | codie speedup | Time saved |
|---|---|---|---|---|
| Cold binary launch | 4 ms | 38 ms | 10.65x faster | 91% less time |
| Trivial request, first token | 1.14 s | 1.79 s | 1.57x faster | 36% less time |
| Trivial request, total | 1.15 s | 1.82 s | 1.58x faster | 37% less time |
| Task suite, median | 28.2 s | 59.0 s | 2.09x faster | 52% less time |
| Task suite, mean | 33.9 s | 56.0 s | 1.65x faster | 39% less time |

- **Cost:** codie spent $0.0501 versus opencode's $0.1295, so codie was **2.58x cheaper**.
- **Tokens:** codie used 161,617 input+output tokens versus opencode's 324,374, so codie used **2.01x fewer**.
- **Output size:** codie changed 2,659 lines versus opencode's 3,128, so codie wrote **1.18x fewer** lines.

## Environment and method

| Item | Value |
|---|---|
| Started | 2026-11-15T16:10:55.878963+00:00 |
| Model | `deepseek/deepseek-v4.1-flash-1017` |
| Platform | macOS-26.6.2-arm64-arm-64bit |
| Python | 3.12.11 |
| Node | v22.16.0 |
| npm | 10.9.2 |
| codie | `codie 0.9.0` |
| opencode | `opencode v2.3.0` |
| Tasks | todo-ui, feature-add, bug-fix, api-todo |
| Runs per harness per task | 3 |
| Binary launch iterations | 20 |
| Trivial model iterations | 5 |
| Task timeout | 1200 s |

Task suite:

| Task | Verifier | What it measures |
|---|---|---|
| `todo-ui` | Browser | Greenfield Next.js todo app: add, toggle, delete, filters, persistence |
| `feature-add` | Browser | Extend an existing basic todo app with filters, clear-completed, persistence |
| `bug-fix` | Hidden tests | Fix seeded logic bugs without touching tests or dependencies |
| `api-todo` | HTTP | Add a JSON CRUD API with validation and 404 handling |

Fairness rules:

- Same scaffold copied fresh for every run, including `node_modules`.
- Same pinned OpenRouter model for both harnesses.
- Same prompts, auto-approved tool permissions, and 20-minute task timeout.
- Harness order alternates each repetition.
- Work directories are outside the repository so neither harness reads codie's project config.
- `PWD` is rewritten to each work directory; opencode runs with an isolated `XDG_*` home and `--standalone`.
- The shared scaffold is hashed before and after each run; a change aborts the benchmark.

## Startup

### Binary launch

| Harness | Min | Median [95% CI] | Mean | p95 |
|---|---|---|---|---|
| codie | 3 ms | 4 ms [3 ms–4 ms] | 3 ms | 4 ms |
| opencode | 38 ms | 38 ms [38 ms–39 ms] | 38 ms | 39 ms |

`codie --version` / `opencode --version`, alternating harnesses.

### Trivial model round-trip

| Harness | First token median | Total min | Total median | Total mean | Total p95 |
|---|---|---|---|---|---|
| codie | 1.14 s | 0.70 s | 1.15 s | 1.07 s | 1.57 s |
| opencode | 1.79 s | 1.37 s | 1.82 s | 1.84 s | 2.43 s |

Prompt: `Reply with exactly: ok`, with no repository context.

## Task suite

| Task | Harness | Success | Median [95% CI] | Min–Max | Functional | Turns | Tools | First edit | Cost | Cost / success |
|---|---|---|---|---|---|---|---|---|---|---|
| todo-ui | codie | 3/3 | 56.9 s [39.0 s–60.7 s] | 39.0 s–60.7 s | 3/3 | 13 | 18 | 16.4 s | $0.0239 | $0.0080 |
| todo-ui | opencode | 3/3 | 67.7 s [63.7 s–127.1 s] | 63.7 s–127.1 s | 3/3 | 14 | 21 | 29.7 s | $0.0597 | $0.0199 |
| feature-add | codie | 3/3 | 22.5 s [18.8 s–72.4 s] | 18.8 s–72.4 s | 3/3 | 8 | 11 | 11.3 s | $0.0137 | $0.0046 |
| feature-add | opencode | 3/3 | 62.9 s [33.9 s–94.7 s] | 33.9 s–94.7 s | 3/3 | 17 | 20 | 19.4 s | $0.0306 | $0.0102 |
| bug-fix | codie | 3/3 | 11.7 s [10.8 s–13.6 s] | 10.8 s–13.6 s | 3/3 | 4 | 5 | 3.71 s | $0.0032 | $0.0011 |
| bug-fix | opencode | 3/3 | 13.1 s [13.0 s–15.3 s] | 13.0 s–15.3 s | 3/3 | 6 | 6 | 5.20 s | $0.0083 | $0.0028 |
| api-todo | codie | 2/3 | 33.9 s [20.3 s–46.5 s] | 20.3 s–46.5 s | 2/3 | 8 | 8 | 11.9 s | $0.0094 | $0.0047 |
| api-todo | opencode | 3/3 | 55.1 s [45.7 s–79.6 s] | 45.7 s–79.6 s | 3/3 | 12 | 19 | 32.7 s | $0.0309 | $0.0103 |

### Across the suite

| Harness | Success [95% CI] | Median [95% CI] | Mean | Cost | Cost / success | Input tokens | Output tokens | Changed LOC |
|---|---|---|---|---|---|---|---|---|
| codie | 11/12 [65%–99%] | 28.2 s [16.2 s–51.7 s] | 33.9 s | $0.0501 | $0.0046 | 122,489 | 39,128 | 2,659 |
| opencode | 12/12 [76%–100%] | 59.0 s [24.6 s–73.7 s] | 56.0 s | $0.1295 | $0.0108 | 274,058 | 50,316 | 3,128 |

### Telemetry

| Harness | Median turns | Median tool calls | Tool errors | Error events | Median first edit |
|---|---|---|---|---|---|
| codie | 8 | 9 | 5 | 0 | 11.6 s |
| opencode | 13 | 19 | 0 | 0 | 22.7 s |

Turns count model rounds (codie `turn_start`, opencode `step_finish`). Tool calls count executed tool invocations. First edit is the first write/edit tool call, measured from process start.

## Failed runs

### api-todo / codie / run 2

Time 33.9 s, build ok, verification failed.

```text
[{"name": "GET initial is 200", "ok": true, "detail": "status=200"}, {"name": "GET initial returns a list", "ok": true, "detail": ""}, {"name": "POST creates a todo", "ok": true, "detail": "status=201"}, {"name": "POST returns an id", "ok": true, "detail": ""}, {"name": "POST preserves text", "ok": true, "detail": "Buy milk"}, {"name": "GET lists the created todo", "ok": true, "detail": "status=200 count=1"}, {"name": "PATCH completes the todo", "ok": true, "detail": "status=200"}, {"name": "PATCH returns completed=true", "ok": true, "detail": "True"}, {"name": "POST rejects empty text with 400", "ok": false, "detail": "status=201"}, {"name": "PATCH rejects an unknown id with 404", "ok": true, "detail": "status=404"}, {"name": "DELETE removes the todo", "ok": true, "detail": "status=200"}, {"name": "DELETE confirms deletion", "ok": true, "detail": "{'deleted': True}"}, {"name": "GET no longer lists the todo", "ok": true, "detail": "count=1"}]
```

### Per-run detail

| Task | Harness | Run | Time | Success | Build | Verify | Files | LOC | Turns | Tools | Tool errors | First edit | Cost |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| todo-ui | codie | 1 | 56.9 s | ok | ok | ok | 3 | 470 | 13 | 18 | 1 | 18.4 s | $0.0077 |
| todo-ui | opencode | 1 | 67.7 s | ok | ok | ok | 3 | 544 | 14 | 21 | 0 | 25.1 s | $0.0163 |
| todo-ui | opencode | 2 | 63.7 s | ok | ok | ok | 4 | 539 | 14 | 21 | 0 | 29.7 s | $0.0133 |
| todo-ui | codie | 2 | 39.0 s | ok | ok | ok | 3 | 390 | 11 | 13 | 1 | 13.9 s | $0.0069 |
| todo-ui | codie | 3 | 60.7 s | ok | ok | ok | 4 | 438 | 16 | 20 | 1 | 16.4 s | $0.0093 |
| todo-ui | opencode | 3 | 127.1 s | ok | ok | ok | 3 | 570 | 32 | 50 | 0 | 59.4 s | $0.0301 |
| feature-add | codie | 1 | 22.5 s | ok | ok | ok | 2 | 313 | 8 | 11 | 1 | 11.3 s | $0.0040 |
| feature-add | opencode | 1 | 94.7 s | ok | ok | ok | 2 | 345 | 25 | 31 | 0 | 25.6 s | $0.0195 |
| feature-add | opencode | 2 | 62.9 s | ok | ok | ok | 2 | 364 | 17 | 20 | 0 | 19.4 s | $0.0059 |
| feature-add | codie | 2 | 18.8 s | ok | ok | ok | 2 | 301 | 6 | 7 | 0 | 6.93 s | $0.0034 |
| feature-add | codie | 3 | 72.4 s | ok | ok | ok | 2 | 331 | 18 | 22 | 1 | 23.9 s | $0.0062 |
| feature-add | opencode | 3 | 33.9 s | ok | ok | ok | 2 | 319 | 13 | 16 | 0 | 10.7 s | $0.0052 |
| bug-fix | codie | 1 | 11.7 s | ok | ok | ok | 1 | 23 | 4 | 5 | 0 | 3.36 s | $0.0013 |
| bug-fix | opencode | 1 | 13.1 s | ok | ok | ok | 1 | 23 | 6 | 7 | 0 | 5.20 s | $0.0013 |
| bug-fix | opencode | 2 | 13.0 s | ok | ok | ok | 1 | 23 | 4 | 5 | 0 | 5.10 s | $0.0049 |
| bug-fix | codie | 2 | 13.6 s | ok | ok | ok | 1 | 23 | 5 | 5 | 0 | 4.76 s | $0.0008 |
| bug-fix | codie | 3 | 10.8 s | ok | ok | ok | 1 | 23 | 4 | 4 | 0 | 3.71 s | $0.0011 |
| bug-fix | opencode | 3 | 15.3 s | ok | ok | ok | 1 | 23 | 6 | 6 | 0 | 6.84 s | $0.0020 |
| api-todo | codie | 1 | 20.3 s | ok | ok | ok | 2 | 111 | 7 | 8 | 0 | 7.79 s | $0.0039 |
| api-todo | opencode | 1 | 79.6 s | ok | ok | ok | 1 | 105 | 19 | 23 | 0 | 58.9 s | $0.0119 |
| api-todo | opencode | 2 | 55.1 s | ok | ok | ok | 2 | 162 | 12 | 19 | 0 | 32.7 s | $0.0084 |
| api-todo | codie | 2 | 33.9 s | fail | ok | fail | 2 | 117 | 10 | 12 | 0 | 11.9 s | $0.0040 |
| api-todo | codie | 3 | 46.5 s | ok | ok | ok | 1 | 119 | 8 | 8 | 0 | 32.6 s | $0.0015 |
| api-todo | opencode | 3 | 45.7 s | ok | ok | ok | 2 | 111 | 12 | 18 | 0 | 20.2 s | $0.0107 |

Edit precision for `bug-fix` (expected change: `src/lib/todos.ts` only):

- No extra files were changed in any run.

## Analysis

- Suite speed: codie's pooled median was 28.2 s versus opencode's 59.0 s, **2.09x faster than opencode**.
- `todo-ui`: codie 56.9 s vs opencode 67.7 s (**1.19x**).
- `feature-add`: codie 22.5 s vs opencode 62.9 s (**2.80x**).
- `bug-fix`: codie 11.7 s vs opencode 13.1 s (**1.13x**).
- `api-todo`: codie 33.9 s vs opencode 55.1 s (**1.63x**).
- Cost: codie spent 2.58x less than opencode ($0.0501 vs $0.1295).
- 1 of 24 runs failed verification; inspect the per-run logs.
- Verification builds run after the harness exits, so they can reuse the harness's own `.next` cache. Treat them as correctness checks, not clean-build speed.

## Limitations

- 3 run(s) per harness per task. Model output is nondeterministic.
- Bootstrap intervals with three samples are coarse; treat the bounds as indicative, not exact.
- The harnesses have different system prompts and tool descriptions; that is inherent.
- Network latency, OpenRouter routing, and machine load add noise.
- Token and cost totals come from each harness's JSON stream; missing usage understates totals.
- Browser checks use text and role selectors; unusual markup can cause a false failure.
- Hidden verification files are never present during the coding run.

## Raw data

- `benchmark/results/20261115-161055/results.json` — machine-readable metrics
- `benchmark/results/20261115-161055/logs/` — harness JSONL, build output, verifier output
