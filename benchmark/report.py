#!/usr/bin/env python3
"""Render a detailed markdown report from a benchmark results directory.

Usage:
  python3 benchmark/report.py                    # latest results directory
  python3 benchmark/report.py benchmark/results/20260101-120000
  python3 benchmark/report.py <dir> --publish    # also write benchmark/REPORT.md

Reports include per-task comparisons, bootstrap confidence intervals, telemetry
(turns, tool calls, first edit), cost per success, and verification details.
"""

from __future__ import annotations

import argparse
import json
import random
import statistics
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple

BENCH_DIR = Path(__file__).resolve().parent
RESULTS_DIR = BENCH_DIR / "results"
HARNESSES = ("codie", "opencode")
TASK_ORDER = ["todo-ui", "feature-add", "bug-fix", "api-todo"]


def latest_results_dir() -> Path:
    if not RESULTS_DIR.exists():
        raise SystemExit("no benchmark/results directory; run the benchmark first")
    candidates = [path for path in RESULTS_DIR.iterdir() if (path / "results.json").exists()]
    if not candidates:
        raise SystemExit("no results.json found under benchmark/results")
    return max(candidates, key=lambda path: path.stat().st_mtime)


def load_results(path: Path) -> Dict[str, object]:
    if path.is_file():
        results_file = path
        results_dir = path.parent
    else:
        results_dir = path
        results_file = path / "results.json"
    if not results_file.exists():
        raise SystemExit(f"missing {results_file}")
    payload = json.loads(results_file.read_text(encoding="utf-8"))
    payload["_results_dir"] = str(results_dir)
    return payload


# ---------------------------------------------------------------------------
# Formatting and statistics
# ---------------------------------------------------------------------------


def fmt_seconds(value: Optional[float]) -> str:
    if value is None:
        return "-"
    if value < 10:
        return f"{value:.2f} s"
    return f"{value:.1f} s"


def fmt_ms(value: Optional[float]) -> str:
    if value is None:
        return "-"
    return f"{value * 1000:.0f} ms"


def fmt_usd(value: Optional[float]) -> str:
    if value is None:
        return "-"
    return f"${value:.4f}"


def fmt_int(value: Optional[float]) -> str:
    if value is None:
        return "-"
    return f"{int(value):,}"


def median(values: Sequence[float]) -> Optional[float]:
    return statistics.median(values) if values else None


def bootstrap_ci(
    values: Sequence[float], iterations: int = 2000, confidence: float = 0.95
) -> Tuple[Optional[float], Optional[float]]:
    """Percentile bootstrap confidence interval for the median."""
    if len(values) < 2:
        return (None, None)
    rng = random.Random(0)
    samples: List[float] = []
    size = len(values)
    for _ in range(iterations):
        sample = [values[rng.randrange(size)] for _ in range(size)]
        samples.append(statistics.median(sample))
    samples.sort()
    low = int((1.0 - confidence) / 2.0 * iterations)
    high = int((1.0 + confidence) / 2.0 * iterations) - 1
    return (samples[low], samples[high])


def wilson_interval(successes: int, total: int, z: float = 1.96) -> Tuple[float, float]:
    if total == 0:
        return (0.0, 0.0)
    phat = successes / total
    denominator = 1 + z * z / total
    centre = phat + z * z / (2 * total)
    margin = z * ((phat * (1 - phat) / total + z * z / (4 * total * total)) ** 0.5)
    return ((centre - margin) / denominator, (centre + margin) / denominator)


def pct(value: Optional[float]) -> str:
    return "-" if value is None else f"{value * 100:.0f}%"


def ratio(numerator: Optional[float], denominator: Optional[float]) -> Optional[float]:
    if not numerator or not denominator:
        return None
    return numerator / denominator


def speedup_cell(codie_value: Optional[float], opencode_value: Optional[float]) -> str:
    if not codie_value or not opencode_value or codie_value <= 0 or opencode_value <= 0:
        return "-"
    if codie_value < opencode_value:
        return f"{opencode_value / codie_value:.2f}x faster"
    if codie_value > opencode_value:
        return f"{codie_value / opencode_value:.2f}x slower"
    return "tie"


def percent_less(codie_value: Optional[float], opencode_value: Optional[float]) -> str:
    if not codie_value or not opencode_value or opencode_value <= 0:
        return "-"
    reduction = (opencode_value - codie_value) / opencode_value * 100.0
    if reduction >= 0:
        return f"{reduction:.0f}% less time"
    return f"{abs(reduction):.0f}% more time"


def cost_delta_label(codie_cost: Optional[float], opencode_cost: Optional[float]) -> str:
    if not codie_cost or not opencode_cost:
        return "-"
    if codie_cost < opencode_cost:
        return f"{opencode_cost / codie_cost:.2f}x cheaper"
    if codie_cost > opencode_cost:
        return f"{codie_cost / opencode_cost:.2f}x more expensive"
    return "tie"


# ---------------------------------------------------------------------------
# Data shaping
# ---------------------------------------------------------------------------


def run_task(run: Dict[str, object]) -> str:
    return str(run.get("task") or "todo-ui")


def run_success(run: Dict[str, object]) -> bool:
    if "success" in run:
        return bool(run.get("success"))
    return bool(
        run.get("build_ok")
        and not run.get("timed_out")
        and not run.get("package_changed")
        and run.get("functional_ok", True)
    )


def select_runs(runs: List[Dict[str, object]], harness: str, task: Optional[str] = None) -> List[Dict[str, object]]:
    selected = [run for run in runs if run.get("harness") == harness]
    if task is not None:
        selected = [run for run in selected if run_task(run) == task]
    return selected


def summarize(selected: List[Dict[str, object]]) -> Dict[str, object]:
    elapsed = [float(run.get("elapsed", 0) or 0) for run in selected]
    builds = [
        float(run.get("build_seconds", 0) or 0)
        for run in selected
        if run.get("build_seconds") is not None
    ]
    turns = [int(run.get("turns", 0) or 0) for run in selected]
    tools = [int(run.get("tool_calls", 0) or 0) for run in selected]
    edits = [
        float(run.get("first_edit_s"))
        for run in selected
        if run.get("first_edit_s") is not None
    ]
    successes = sum(1 for run in selected if run_success(run))
    cost = sum(float(run.get("cost_usd", 0) or 0) for run in selected)
    low, high = bootstrap_ci(elapsed)
    success_low, success_high = wilson_interval(successes, len(selected))
    return {
        "count": len(selected),
        "successes": successes,
        "success_low": success_low,
        "success_high": success_high,
        "elapsed": stats_of(elapsed),
        "median_ci": (low, high),
        "build_median": median(builds),
        "turns_median": median([float(value) for value in turns]),
        "tools_median": median([float(value) for value in tools]),
        "first_edit_median": median(edits),
        "tool_errors": sum(int(run.get("tool_errors", 0) or 0) for run in selected),
        "error_events": sum(int(run.get("error_events", 0) or 0) for run in selected),
        "tokens_input": sum(int(run.get("tokens_input", 0) or 0) for run in selected),
        "tokens_output": sum(int(run.get("tokens_output", 0) or 0) for run in selected),
        "cost": cost,
        "cost_per_success": (cost / successes) if successes else None,
        "changed_lines": sum(int(run.get("changed_lines", 0) or 0) for run in selected),
        "functional": sum(1 for run in selected if run.get("functional_ok")),
        "changed_files": median(
            [float(len(run.get("changed_files", []) or [])) for run in selected]
        ),
    }


def stats_of(values: Sequence[float]) -> Dict[str, Optional[float]]:
    if not values:
        return {"min": None, "max": None, "median": None, "mean": None}
    return {
        "min": min(values),
        "max": max(values),
        "median": statistics.median(values),
        "mean": statistics.fmean(values),
    }


def tasks_in(runs: List[Dict[str, object]]) -> List[str]:
    present = {run_task(run) for run in runs}
    ordered = [task for task in TASK_ORDER if task in present]
    ordered.extend(sorted(present - set(ordered)))
    return ordered


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------


def render(payload: Dict[str, object]) -> str:
    metadata = payload.get("metadata", {})  # type: ignore[assignment]
    startup = payload.get("startup", {}) or {}  # type: ignore[assignment]
    task_data = payload.get("task", {}) or {}  # type: ignore[assignment]
    runs: List[Dict[str, object]] = list(task_data.get("runs", []) or [])  # type: ignore[arg-type]
    results_dir = payload.get("_results_dir", "")
    tasks = tasks_in(runs) if runs else []

    lines: List[str] = []
    add = lines.append

    add("# Benchmark report: codie vs opencode")
    add("")
    add(f"Generated: {datetime.now(timezone.utc).isoformat()}")
    add(f"Results directory: `{results_dir}`")
    add("")

    codie_all = summarize(select_runs(runs, "codie"))
    opencode_all = summarize(select_runs(runs, "opencode"))
    binary_raw = startup.get("binary_raw", {}) if isinstance(startup, dict) else {}
    model_raw = startup.get("model_raw", {}) if isinstance(startup, dict) else {}
    codie_launch_values = list(binary_raw.get("codie", []))
    opencode_launch_values = list(binary_raw.get("opencode", []))
    codie_first_values = list((model_raw.get("codie", {}) or {}).get("first_token", []))
    opencode_first_values = list((model_raw.get("opencode", {}) or {}).get("first_token", []))
    codie_total_values = list((model_raw.get("codie", {}) or {}).get("total", []))
    opencode_total_values = list((model_raw.get("opencode", {}) or {}).get("total", []))
    codie_launch = median(codie_launch_values)
    opencode_launch = median(opencode_launch_values)
    codie_first = median(codie_first_values)
    opencode_first = median(opencode_first_values)
    codie_total = median(codie_total_values)
    opencode_total = median(opencode_total_values)

    # Executive summary -----------------------------------------------------
    add("## Executive summary")
    add("")
    add("| Question | codie | opencode | codie speedup |")
    add("|---|---|---|---|")
    add(
        "| Cold binary launch (median) | {} | {} | {} |".format(
            fmt_ms(codie_launch), fmt_ms(opencode_launch), speedup_cell(codie_launch, opencode_launch)
        )
    )
    add(
        "| Trivial request, first token (median) | {} | {} | {} |".format(
            fmt_seconds(codie_first), fmt_seconds(opencode_first), speedup_cell(codie_first, opencode_first)
        )
    )
    add(
        "| Trivial request, total (median) | {} | {} | {} |".format(
            fmt_seconds(codie_total), fmt_seconds(opencode_total), speedup_cell(codie_total, opencode_total)
        )
    )
    add(
        "| Task suite time (median) | {} | {} | {} |".format(
            fmt_seconds(codie_all["elapsed"]["median"]),
            fmt_seconds(opencode_all["elapsed"]["median"]),
            speedup_cell(codie_all["elapsed"]["median"], opencode_all["elapsed"]["median"]),
        )
    )
    add(
        "| Task suite success | {}/{} ({}) | {}/{} ({}) | - |".format(
            codie_all["successes"],
            codie_all["count"],
            pct(codie_all["successes"] / codie_all["count"] if codie_all["count"] else None),
            opencode_all["successes"],
            opencode_all["count"],
            pct(opencode_all["successes"] / opencode_all["count"] if opencode_all["count"] else None),
        )
    )
    add(
        "| Task suite cost | {} | {} | {} |".format(
            fmt_usd(codie_all["cost"]),
            fmt_usd(opencode_all["cost"]),
            cost_delta_label(codie_all["cost"], opencode_all["cost"]),
        )
    )
    add(
        "| Cost per successful task | {} | {} | - |".format(
            fmt_usd(codie_all["cost_per_success"]),
            fmt_usd(opencode_all["cost_per_success"]),
        )
    )
    add("")

    # Speed section ---------------------------------------------------------
    add("## How much faster is codie?")
    add("")
    add(
        "All ratios compare codie with opencode. Values above 1x mean codie is faster; "
        "percentages are reductions in wall time relative to opencode. Suite values pool every run."
    )
    add("")
    add("| Metric | codie | opencode | codie speedup | Time saved |")
    add("|---|---|---|---|---|")
    add(
        "| Cold binary launch | {} | {} | {} | {} |".format(
            fmt_ms(codie_launch),
            fmt_ms(opencode_launch),
            speedup_cell(codie_launch, opencode_launch),
            percent_less(codie_launch, opencode_launch),
        )
    )
    add(
        "| Trivial request, first token | {} | {} | {} | {} |".format(
            fmt_seconds(codie_first),
            fmt_seconds(opencode_first),
            speedup_cell(codie_first, opencode_first),
            percent_less(codie_first, opencode_first),
        )
    )
    add(
        "| Trivial request, total | {} | {} | {} | {} |".format(
            fmt_seconds(codie_total),
            fmt_seconds(opencode_total),
            speedup_cell(codie_total, opencode_total),
            percent_less(codie_total, opencode_total),
        )
    )
    add(
        "| Task suite, median | {} | {} | {} | {} |".format(
            fmt_seconds(codie_all["elapsed"]["median"]),
            fmt_seconds(opencode_all["elapsed"]["median"]),
            speedup_cell(codie_all["elapsed"]["median"], opencode_all["elapsed"]["median"]),
            percent_less(codie_all["elapsed"]["median"], opencode_all["elapsed"]["median"]),
        )
    )
    add(
        "| Task suite, mean | {} | {} | {} | {} |".format(
            fmt_seconds(codie_all["elapsed"]["mean"]),
            fmt_seconds(opencode_all["elapsed"]["mean"]),
            speedup_cell(codie_all["elapsed"]["mean"], opencode_all["elapsed"]["mean"]),
            percent_less(codie_all["elapsed"]["mean"], opencode_all["elapsed"]["mean"]),
        )
    )
    add("")
    if codie_all["cost"] and opencode_all["cost"]:
        if codie_all["cost"] < opencode_all["cost"]:
            add(
                "- **Cost:** codie spent {} versus opencode's {}, so codie was **{:.2f}x cheaper**.".format(
                    fmt_usd(codie_all["cost"]),
                    fmt_usd(opencode_all["cost"]),
                    opencode_all["cost"] / codie_all["cost"],
                )
            )
        else:
            add(
                "- **Cost:** codie spent {} versus opencode's {}, so codie was **{:.2f}x more expensive**.".format(
                    fmt_usd(codie_all["cost"]),
                    fmt_usd(opencode_all["cost"]),
                    codie_all["cost"] / opencode_all["cost"],
                )
            )
    codie_tokens = codie_all["tokens_input"] + codie_all["tokens_output"]
    opencode_tokens = opencode_all["tokens_input"] + opencode_all["tokens_output"]
    if codie_tokens and opencode_tokens:
        add(
            "- **Tokens:** codie used {} input+output tokens versus opencode's {}, so codie used "
            "**{:.2f}x fewer**.".format(
                fmt_int(codie_tokens),
                fmt_int(opencode_tokens),
                opencode_tokens / codie_tokens,
            )
        )
    if codie_all["changed_lines"] and opencode_all["changed_lines"]:
        add(
            "- **Output size:** codie changed {} lines versus opencode's {}, so codie wrote "
            "**{:.2f}x fewer** lines.".format(
                fmt_int(codie_all["changed_lines"]),
                fmt_int(opencode_all["changed_lines"]),
                opencode_all["changed_lines"] / codie_all["changed_lines"],
            )
        )
    add("")

    # Environment -----------------------------------------------------------
    add("## Environment and method")
    add("")
    add("| Item | Value |")
    add("|---|---|")
    add(f"| Started | {metadata.get('started', '-')} |")
    add(f"| Model | `{metadata.get('model', '-')}` |")
    add(f"| Platform | {metadata.get('platform', '-')} |")
    add(f"| Python | {metadata.get('python', '-')} |")
    add(f"| Node | {metadata.get('node', '-')} |")
    add(f"| npm | {metadata.get('npm', '-')} |")
    add(f"| codie | `{metadata.get('versions', {}).get('codie', '-')}` |")
    add(f"| opencode | `{metadata.get('versions', {}).get('opencode', '-')}` |")
    add(f"| Tasks | {', '.join(metadata.get('tasks', []) or [])} |")
    add(f"| Runs per harness per task | {metadata.get('runs', '-')} |")
    add(f"| Binary launch iterations | {metadata.get('startup_runs', '-')} |")
    add(f"| Trivial model iterations | {metadata.get('model_runs', '-')} |")
    add(f"| Task timeout | {metadata.get('task_timeout_seconds', '-')} s |")
    add("")
    add("Task suite:")
    add("")
    add("| Task | Verifier | What it measures |")
    add("|---|---|---|")
    add("| `todo-ui` | Browser | Greenfield Next.js todo app: add, toggle, delete, filters, persistence |")
    add("| `feature-add` | Browser | Extend an existing basic todo app with filters, clear-completed, persistence |")
    add("| `bug-fix` | Hidden tests | Fix seeded logic bugs without touching tests or dependencies |")
    add("| `api-todo` | HTTP | Add a JSON CRUD API with validation and 404 handling |")
    add("")
    add("Fairness rules:")
    add("")
    add("- Same scaffold copied fresh for every run, including `node_modules`.")
    add("- Same pinned OpenRouter model for both harnesses.")
    add("- Same prompts, auto-approved tool permissions, and 20-minute task timeout.")
    add("- Harness order alternates each repetition.")
    add("- Work directories are outside the repository so neither harness reads codie's project config.")
    add("- `PWD` is rewritten to each work directory; opencode runs with an isolated `XDG_*` home and `--standalone`.")
    add("- The shared scaffold is hashed before and after each run; a change aborts the benchmark.")
    add("")

    # Startup ---------------------------------------------------------------
    add("## Startup")
    add("")
    add("### Binary launch")
    add("")
    add("| Harness | Min | Median [95% CI] | Mean | p95 |")
    add("|---|---|---|---|---|")
    for harness in HARNESSES:
        values = codie_launch_values if harness == "codie" else opencode_launch_values
        entry = (startup.get("binary", {}) or {}).get(harness, {})
        low, high = bootstrap_ci(values)
        ci = "{}–{}".format(fmt_ms(low), fmt_ms(high)) if low is not None else "-"
        add(
            "| {} | {} | {} [{}] | {} | {} |".format(
                harness,
                fmt_ms(entry.get("min")),
                fmt_ms(entry.get("median")),
                ci,
                fmt_ms(entry.get("mean")),
                fmt_ms(entry.get("p95")),
            )
        )
    add("")
    add("`codie --version` / `opencode --version`, alternating harnesses.")
    add("")
    add("### Trivial model round-trip")
    add("")
    add("| Harness | First token median | Total min | Total median | Total mean | Total p95 |")
    add("|---|---|---|---|---|---|")
    for harness in HARNESSES:
        entry = (startup.get("model", {}) or {}).get(harness, {})
        first = entry.get("first_token") or {}
        total = entry.get("total") or {}
        add(
            "| {} | {} | {} | {} | {} | {} |".format(
                harness,
                fmt_seconds(first.get("median")),
                fmt_seconds(total.get("min")),
                fmt_seconds(total.get("median")),
                fmt_seconds(total.get("mean")),
                fmt_seconds(total.get("p95")),
            )
        )
    add("")
    add("Prompt: `Reply with exactly: ok`, with no repository context.")
    add("")

    # Task suite ------------------------------------------------------------
    add("## Task suite")
    add("")
    add(
        "| Task | Harness | Success | Median [95% CI] | Min–Max | Functional | Turns | Tools | First edit | Cost | Cost / success |"
    )
    add("|---|---|---|---|---|---|---|---|---|---|---|")
    for task in tasks:
        for harness in HARNESSES:
            selected = select_runs(runs, harness, task)
            if not selected:
                continue
            entry = summarize(selected)
            low, high = entry["median_ci"]
            ci = "{}–{}".format(fmt_seconds(low), fmt_seconds(high)) if low is not None else "-"
            add(
                "| {} | {} | {}/{} | {} [{}] | {}–{} | {}/{} | {} | {} | {} | {} | {} |".format(
                    task,
                    harness,
                    entry["successes"],
                    entry["count"],
                    fmt_seconds(entry["elapsed"]["median"]),
                    ci,
                    fmt_seconds(entry["elapsed"]["min"]),
                    fmt_seconds(entry["elapsed"]["max"]),
                    entry["functional"],
                    entry["count"],
                    fmt_int(entry["turns_median"]),
                    fmt_int(entry["tools_median"]),
                    fmt_seconds(entry["first_edit_median"]),
                    fmt_usd(entry["cost"]),
                    fmt_usd(entry["cost_per_success"]),
                )
            )
    add("")
    add("### Across the suite")
    add("")
    add(
        "| Harness | Success [95% CI] | Median [95% CI] | Mean | Cost | Cost / success | Input tokens | Output tokens | Changed LOC |"
    )
    add("|---|---|---|---|---|---|---|---|---|")
    for harness, entry in (("codie", codie_all), ("opencode", opencode_all)):
        low, high = entry["median_ci"]
        ci = "{}–{}".format(fmt_seconds(low), fmt_seconds(high)) if low is not None else "-"
        success_ci = "{}–{}".format(pct(entry["success_low"]), pct(entry["success_high"]))
        add(
            "| {} | {}/{} [{}] | {} [{}] | {} | {} | {} | {} | {} | {} |".format(
                harness,
                entry["successes"],
                entry["count"],
                success_ci,
                fmt_seconds(entry["elapsed"]["median"]),
                ci,
                fmt_seconds(entry["elapsed"]["mean"]),
                fmt_usd(entry["cost"]),
                fmt_usd(entry["cost_per_success"]),
                fmt_int(entry["tokens_input"]),
                fmt_int(entry["tokens_output"]),
                fmt_int(entry["changed_lines"]),
            )
        )
    add("")

    # Telemetry -------------------------------------------------------------
    add("### Telemetry")
    add("")
    add("| Harness | Median turns | Median tool calls | Tool errors | Error events | Median first edit |")
    add("|---|---|---|---|---|---|")
    for harness, entry in (("codie", codie_all), ("opencode", opencode_all)):
        add(
            "| {} | {} | {} | {} | {} | {} |".format(
                harness,
                fmt_int(entry["turns_median"]),
                fmt_int(entry["tools_median"]),
                entry["tool_errors"],
                entry["error_events"],
                fmt_seconds(entry["first_edit_median"]),
            )
        )
    add("")
    add(
        "Turns count model rounds (codie `turn_start`, opencode `step_finish`). Tool calls count "
        "executed tool invocations. First edit is the first write/edit tool call, measured from process start."
    )
    add("")

    failed_runs = [run for run in runs if not run_success(run)]
    if failed_runs:
        add("## Failed runs")
        add("")
        for run in failed_runs:
            add(
                "### {} / {} / run {}".format(
                    run_task(run), run.get("harness"), run.get("run")
                )
            )
            add("")
            add(
                "Time {}, build {}, verification {}.".format(
                    fmt_seconds(run.get("elapsed")),
                    "ok" if run.get("build_ok") else "failed",
                    "ok" if run.get("functional_ok") else "failed",
                )
            )
            add("")
            detail = str(run.get("functional_detail") or "").strip()
            if detail:
                add("```text")
                add(detail[:2000])
                add("```")
                add("")

    # Per-run detail --------------------------------------------------------
    add("### Per-run detail")
    add("")
    add(
        "| Task | Harness | Run | Time | Success | Build | Verify | Files | LOC | Turns | Tools | Tool errors | First edit | Cost |"
    )
    add("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for run in runs:
        add(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |".format(
                run_task(run),
                run.get("harness"),
                run.get("run"),
                fmt_seconds(run.get("elapsed")),
                "ok" if run_success(run) else "fail",
                "ok" if run.get("build_ok") else "fail",
                "ok" if run.get("functional_ok") else "fail",
                len(run.get("changed_files", []) or []),
                run.get("changed_lines", 0),
                run.get("turns", 0),
                run.get("tool_calls", 0),
                run.get("tool_errors", 0),
                fmt_seconds(run.get("first_edit_s")),
                fmt_usd(float(run.get("cost_usd", 0) or 0)),
            )
        )
    add("")
    bugfix_runs = [run for run in runs if run_task(run) == "bug-fix"]
    if bugfix_runs:
        expected = {"src/lib/todos.ts"}
        extras = sorted(
            {
                name
                for run in bugfix_runs
                for name in (run.get("changed_files", []) or [])
                if name not in expected
            }
        )
        add("Edit precision for `bug-fix` (expected change: `src/lib/todos.ts` only):")
        add("")
        if extras:
            add("- Extra files changed in at least one run: " + ", ".join(f"`{name}`" for name in extras))
        else:
            add("- No extra files were changed in any run.")
        add("")

    # Analysis --------------------------------------------------------------
    add("## Analysis")
    add("")
    suite_speedup = ratio(opencode_all["elapsed"]["median"], codie_all["elapsed"]["median"])
    if suite_speedup:
        if suite_speedup >= 1:
            verdict = f"{suite_speedup:.2f}x faster than opencode"
        else:
            verdict = f"{1 / suite_speedup:.2f}x slower than opencode"
        add(
            f"- Suite speed: codie's pooled median was {fmt_seconds(codie_all['elapsed']['median'])} "
            f"versus opencode's {fmt_seconds(opencode_all['elapsed']['median'])}, "
            f"**{verdict}**."
        )
    for task in tasks:
        codie_entry = summarize(select_runs(runs, "codie", task))
        opencode_entry = summarize(select_runs(runs, "opencode", task))
        task_speedup = ratio(opencode_entry["elapsed"]["median"], codie_entry["elapsed"]["median"])
        if task_speedup:
            add(
                f"- `{task}`: codie {fmt_seconds(codie_entry['elapsed']['median'])} vs opencode "
                f"{fmt_seconds(opencode_entry['elapsed']['median'])} (**{task_speedup:.2f}x**)."
            )
    if codie_all["cost"] and opencode_all["cost"]:
        if codie_all["cost"] < opencode_all["cost"]:
            add(
                f"- Cost: codie spent {opencode_all['cost'] / codie_all['cost']:.2f}x less than "
                f"opencode ({fmt_usd(codie_all['cost'])} vs {fmt_usd(opencode_all['cost'])})."
            )
        else:
            add(
                f"- Cost: codie spent {codie_all['cost'] / opencode_all['cost']:.2f}x more than "
                f"opencode ({fmt_usd(codie_all['cost'])} vs {fmt_usd(opencode_all['cost'])})."
            )
    failed = [run for run in runs if not run_success(run)]
    if failed:
        add(f"- {len(failed)} of {len(runs)} runs failed verification; inspect the per-run logs.")
    else:
        add("- Every run passed its build and functional verifier with `package.json` untouched.")
    add(
        "- Verification builds run after the harness exits, so they can reuse the harness's own `.next` "
        "cache. Treat them as correctness checks, not clean-build speed."
    )
    add("")

    # Limitations -----------------------------------------------------------
    add("## Limitations")
    add("")
    add(f"- {metadata.get('runs', 0)} run(s) per harness per task. Model output is nondeterministic.")
    add("- Bootstrap intervals with three samples are coarse; treat the bounds as indicative, not exact.")
    add("- The harnesses have different system prompts and tool descriptions; that is inherent.")
    add("- Network latency, OpenRouter routing, and machine load add noise.")
    add("- Token and cost totals come from each harness's JSON stream; missing usage understates totals.")
    add("- Browser checks use text and role selectors; unusual markup can cause a false failure.")
    add("- Hidden verification files are never present during the coding run.")
    add("")

    add("## Raw data")
    add("")
    add(f"- `{results_dir}/results.json` — machine-readable metrics")
    add(f"- `{results_dir}/logs/` — harness JSONL, build output, verifier output")
    add(f"- `{results_dir}/snapshots/` — changed source files from each run")
    add("")

    return "\n".join(lines)


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description="Render a detailed benchmark report.")
    parser.add_argument(
        "results",
        nargs="?",
        type=Path,
        help="results directory or results.json (default: latest)",
    )
    parser.add_argument("--out", type=Path, help="output path (default: <results>/report.md)")
    parser.add_argument(
        "--publish",
        action="store_true",
        help="also copy the report to benchmark/REPORT.md",
    )
    args = parser.parse_args(argv)

    results_dir = args.results or latest_results_dir()
    payload = load_results(results_dir)
    report = render(payload)
    output = args.out or (Path(payload["_results_dir"]) / "report.md")
    output.write_text(report, encoding="utf-8")
    print(output)
    if args.publish:
        published = BENCH_DIR / "REPORT.md"
        published.write_text(report, encoding="utf-8")
        print(published)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
