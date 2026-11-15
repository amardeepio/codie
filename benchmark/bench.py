#!/usr/bin/env python3
"""Benchmark codie against opencode on a suite of identical coding tasks.

Phases:
  setup    prepare the shared Next.js scaffold and the browser verifier
  startup  measure binary launch and trivial model round-trip time
  task     run the task suite with each harness
  all      setup + startup + task (default)

Tasks:
  todo-ui      greenfield todo app (UI + browser checks)
  feature-add  extend an existing basic todo app (brownfield UI)
  bug-fix      fix failing logic tests (hidden tests + edit precision)
  api-todo     add a JSON API (HTTP checks)

Results are written under benchmark/results/<timestamp>/ and are gitignored.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Dict, Iterable, List, Optional, Tuple

BENCH_DIR = Path(__file__).resolve().parent
REPO_ROOT = BENCH_DIR.parent
CACHE_DIR = BENCH_DIR / ".cache"
TEMPLATE_DIR = CACHE_DIR / "next-template"
OPENCODE_HOME = CACHE_DIR / "opencode-home"
TASKS_DIR = BENCH_DIR / "tasks"
VERIFIER_DIR = BENCH_DIR / "verifier"
RESULTS_DIR = BENCH_DIR / "results"
DEFAULT_CODIE = REPO_ROOT / "target" / "release" / "codie"

MODEL_ID = "deepseek/deepseek-v4.1-flash-1017"
CODIE_PROVIDER = "openrouter"
CODIE_MODEL = MODEL_ID
OPENCODE_MODEL = "openrouter/" + MODEL_ID

TASK_ORDER = ["todo-ui", "feature-add", "bug-fix", "api-todo"]

DEFAULT_RUNS = 3
DEFAULT_STARTUP_RUNS = 20
DEFAULT_MODEL_RUNS = 5
DEFAULT_TASK_TIMEOUT = 20 * 60
DEFAULT_BUILD_TIMEOUT = 15 * 60
DEFAULT_VERSION_TIMEOUT = 30

IGNORED_DIRS = {".git", ".next", "node_modules", "out", "dist", "coverage"}
SOURCE_SUFFIXES = {".ts", ".tsx", ".js", ".jsx", ".css", ".json", ".md", ".mjs"}
STATIC_EXCLUDED = {"package-lock.json", "next-env.d.ts"}


def log(message: str) -> None:
    print("[bench] " + message, flush=True)


def die(message: str) -> "None":
    print("[bench] error: " + message, file=sys.stderr, flush=True)
    raise SystemExit(1)


# ---------------------------------------------------------------------------
# Environment and process helpers
# ---------------------------------------------------------------------------


def load_dotenv(path: Path) -> None:
    """Load KEY=VALUE pairs into the process environment without overriding it."""
    if not path.exists():
        return
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if key.startswith("export "):
            key = key[len("export ") :].strip()
        value = value.strip().strip('"').strip("'")
        if key:
            os.environ.setdefault(key, value)


def which(name: str) -> Optional[str]:
    return shutil.which(name)


def env_for(cwd: Path, env: Dict[str, str]) -> Dict[str, str]:
    """Return env with PWD matching cwd.

    opencode resolves its project root from PWD rather than the spawn cwd, so a
    stale value would make it edit the caller's repository.
    """
    result = dict(env)
    result["PWD"] = str(cwd)
    result["OLDPWD"] = str(cwd)
    return result


def run_capture(
    cmd: List[str],
    cwd: Optional[Path] = None,
    env: Optional[Dict[str, str]] = None,
    timeout: int = DEFAULT_VERSION_TIMEOUT,
) -> Tuple[int, str, float]:
    started = time.perf_counter()
    command_env = env_for(cwd, env) if env is not None and cwd is not None else env
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(cwd) if cwd else None,
            env=command_env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
        )
        elapsed = time.perf_counter() - started
        return proc.returncode, (proc.stdout or "").strip(), elapsed
    except FileNotFoundError:
        return 127, "command not found: " + cmd[0], time.perf_counter() - started
    except subprocess.TimeoutExpired:
        return 124, "timed out", time.perf_counter() - started


def kill_tree(proc: subprocess.Popen) -> None:
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except Exception:
        try:
            proc.kill()
        except Exception:
            pass


def run_streaming(
    cmd: List[str],
    cwd: Path,
    env: Dict[str, str],
    log_path: Path,
    timeout: int,
    detect_first_token: bool = False,
) -> Dict[str, object]:
    """Run a command, capture merged output, and optionally time the first token."""
    started = time.perf_counter()
    first_token: Optional[float] = None
    timed_out = False
    lines: List[str] = []
    line_times: List[float] = []

    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "w", encoding="utf-8") as log_file:
        proc = subprocess.Popen(
            cmd,
            cwd=str(cwd),
            env=env_for(cwd, env),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
            start_new_session=True,
        )
        timer = threading.Timer(timeout, lambda: kill_tree(proc))
        timer.daemon = True
        timer.start()
        try:
            assert proc.stdout is not None
            for line in proc.stdout:
                log_file.write(line)
                log_file.flush()
                elapsed = time.perf_counter() - started
                lines.append(line)
                line_times.append(elapsed)
                if detect_first_token and first_token is None and is_token_line(line):
                    first_token = elapsed
            proc.wait()
        except Exception:
            kill_tree(proc)
            proc.wait()
            raise
        finally:
            timer.cancel()

    elapsed = time.perf_counter() - started
    if proc.returncode is not None and proc.returncode < 0:
        timed_out = True
    return {
        "elapsed": elapsed,
        "exit_code": proc.returncode if proc.returncode is not None else -9,
        "timed_out": timed_out,
        "first_token": first_token,
        "lines": lines,
        "line_times": line_times,
    }


def is_token_line(line: str) -> bool:
    """True when a JSONL record carries the first visible model output."""
    line = line.strip()
    if not line.startswith("{"):
        return False
    try:
        record = json.loads(line)
    except json.JSONDecodeError:
        return False
    kind = record.get("type")
    if kind == "message_update":
        delta = record.get("delta") or {}
        return delta.get("type") in ("text_delta", "thinking_delta") and bool(
            delta.get("delta")
        )
    if kind in ("text", "reasoning"):
        part = record.get("part") or {}
        return bool(part.get("text") or part.get("reasoning"))
    return False


def stats(values: Iterable[float]) -> Dict[str, Optional[float]]:
    items = sorted(float(v) for v in values)
    if not items:
        return {"min": None, "max": None, "median": None, "mean": None, "p95": None}
    index = min(len(items) - 1, max(0, int(round(0.95 * (len(items) - 1)))))
    return {
        "min": items[0],
        "max": items[-1],
        "median": statistics.median(items),
        "mean": statistics.fmean(items),
        "p95": items[index],
    }


def fmt_seconds(value: Optional[float]) -> str:
    return "-" if value is None else f"{value:.2f}s"


def fmt_ms(value: Optional[float]) -> str:
    return "-" if value is None else f"{value * 1000:.0f}ms"


def fmt_count(value: Optional[float]) -> str:
    return "-" if value is None else f"{int(value)}"


# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------


def harness_env() -> Dict[str, str]:
    env = os.environ.copy()
    env.setdefault("NO_COLOR", "1")
    env.setdefault("CI", "1")
    env["XDG_DATA_HOME"] = str(OPENCODE_HOME / "data")
    env["XDG_CONFIG_HOME"] = str(OPENCODE_HOME / "config")
    env["XDG_CACHE_HOME"] = str(OPENCODE_HOME / "cache")
    return env


def ensure_verifier() -> None:
    VERIFIER_DIR.mkdir(parents=True, exist_ok=True)
    marker = VERIFIER_DIR / "node_modules" / "playwright-core"
    if marker.exists():
        return
    log("installing browser verifier dependency (playwright-core)")
    code, output, _ = run_capture(
        ["npm", "install", "--no-audit", "--no-fund"],
        cwd=VERIFIER_DIR,
        env=os.environ.copy(),
        timeout=10 * 60,
    )
    if code != 0:
        die("installing playwright-core failed:\n" + output)


def chrome_available() -> bool:
    if os.environ.get("CHROME_PATH"):
        return Path(os.environ["CHROME_PATH"]).exists()
    mac = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
    if mac.exists():
        return True
    return any(which(name) for name in ("google-chrome", "google-chrome-stable", "chromium", "chrome"))


def check_binaries(codie: Path, args: argparse.Namespace) -> None:
    if args.build:
        log("building codie (release)")
        code, output, _ = run_capture(
            ["cargo", "build", "--release"],
            cwd=REPO_ROOT,
            env=os.environ.copy(),
            timeout=30 * 60,
        )
        if code != 0:
            die("cargo build --release failed:\n" + output)
    if not codie.exists():
        die(f"codie binary not found at {codie}; run with --build")
    if not os.access(codie, os.X_OK):
        die(f"codie binary is not executable: {codie}")
    if not which("opencode"):
        die("opencode is not on PATH")
    if not which("npm"):
        die("npm is not on PATH")
    if not which("node"):
        die("node is not on PATH")
    if not os.environ.get("OPENROUTER_API_KEY"):
        die("OPENROUTER_API_KEY is not set (add it to .env or the environment)")
    for name in TASK_ORDER:
        if not (TASKS_DIR / name / "prompt.md").exists():
            die(f"task prompt missing: {TASKS_DIR / name / 'prompt.md'}")
    if not args.skip_browser and not chrome_available():
        log("warning: Chrome was not found; browser tasks will fail (set CHROME_PATH)")


def scaffold_exists() -> bool:
    package = TEMPLATE_DIR / "package.json"
    if not package.exists():
        return False
    try:
        data = json.loads(package.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return False
    return "next" in data.get("dependencies", {}) and (
        TEMPLATE_DIR / "src" / "app" / "page.tsx"
    ).exists()


def setup_scaffold(args: argparse.Namespace) -> None:
    TEMPLATE_DIR.parent.mkdir(parents=True, exist_ok=True)
    if scaffold_exists() and not args.reset:
        log("reusing Next.js scaffold at " + str(TEMPLATE_DIR))
        return

    if TEMPLATE_DIR.exists():
        log("removing previous scaffold")
        shutil.rmtree(TEMPLATE_DIR)

    cmd = [
        "npx",
        "--yes",
        "create-next-app@latest",
        str(TEMPLATE_DIR),
        "--ts",
        "--app",
        "--no-tailwind",
        "--eslint",
        "--src-dir",
        "--import-alias",
        "@/*",
        "--use-npm",
        "--yes",
    ]
    log("creating Next.js scaffold (first run downloads packages)")
    code, output, _ = run_capture(cmd, env=os.environ.copy(), timeout=30 * 60)
    if code != 0:
        die("create-next-app failed:\n" + output)
    if not scaffold_exists():
        die("create-next-app finished but the scaffold is incomplete")
    log("scaffold ready")


# ---------------------------------------------------------------------------
# Task definitions
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class TaskSpec:
    name: str
    prompt: Path
    overlay: Optional[Path]
    verifier: str
    forbidden: Tuple[str, ...] = ()


def load_tasks(selection: str) -> List[TaskSpec]:
    if selection.strip() == "all":
        names = TASK_ORDER
    else:
        names = [name.strip() for name in selection.split(",") if name.strip()]
    unknown = [name for name in names if name not in TASK_ORDER]
    if unknown:
        die(f"unknown task(s): {', '.join(unknown)}; expected {', '.join(TASK_ORDER)} or all")
    specs: List[TaskSpec] = []
    for name in names:
        base = TASKS_DIR / name
        overlay = base / "files"
        verify = {
            "todo-ui": "browser",
            "feature-add": "browser",
            "bug-fix": "hidden-test",
            "api-todo": "http-api",
        }[name]
        specs.append(
            TaskSpec(
                name=name,
                prompt=base / "prompt.md",
                overlay=overlay if overlay.exists() else None,
                verifier=verify,
                forbidden=(
                    ("src/lib/todos.test.ts",) if name == "bug-fix" else ()
                ),
            )
        )
    return specs


def overlay_tree(source: Path, destination: Path) -> None:
    for path in source.rglob("*"):
        if path.is_dir():
            continue
        target = destination / path.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)


def prepare_task_workdir(spec: TaskSpec, workdir: Path) -> None:
    clone_tree(TEMPLATE_DIR, workdir)
    if spec.overlay:
        overlay_tree(spec.overlay, workdir)


# ---------------------------------------------------------------------------
# File snapshots
# ---------------------------------------------------------------------------


def iter_source_files(root: Path) -> Iterable[Path]:
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        if any(part in IGNORED_DIRS for part in path.parts):
            continue
        rel = path.relative_to(root)
        if rel.parts and rel.parts[0] == "public":
            continue
        if path.name in STATIC_EXCLUDED:
            continue
        if path.suffix.lower() in SOURCE_SUFFIXES:
            yield path


def hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    try:
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(65536), b""):
                digest.update(chunk)
    except OSError:
        return ""
    return digest.hexdigest()


def snapshot_tree(root: Path) -> Dict[str, str]:
    result: Dict[str, str] = {}
    for path in iter_source_files(root):
        result[str(path.relative_to(root))] = hash_file(path)
    return result


def diff_snapshot(before: Dict[str, str], after_dir: Path) -> List[str]:
    changed: List[str] = []
    for path in iter_source_files(after_dir):
        rel = str(path.relative_to(after_dir))
        if before.get(rel) != hash_file(path):
            changed.append(rel)
    return sorted(changed)


def changed_source_text(root: Path, changed: List[str]) -> str:
    chunks: List[str] = []
    for rel in changed:
        path = root / rel
        try:
            chunks.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            pass
    return "\n".join(chunks)


def count_lines(root: Path, changed: List[str]) -> int:
    total = 0
    for rel in changed:
        try:
            total += len((root / rel).read_text(encoding="utf-8", errors="replace").splitlines())
        except OSError:
            pass
    return total


def copy_changed(root: Path, changed: List[str], destination: Path) -> None:
    for rel in changed:
        source = root / rel
        target = destination / rel
        try:
            if source.stat().st_size > 2 * 1024 * 1024:
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        except OSError:
            pass


def clone_tree(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if sys.platform == "darwin":
        result = subprocess.run(
            ["cp", "-cR", str(source), str(destination)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode == 0:
            return
        if destination.exists():
            shutil.rmtree(destination, ignore_errors=True)
    shutil.copytree(source, destination, symlinks=True)


# ---------------------------------------------------------------------------
# Commands
# ---------------------------------------------------------------------------


def codie_version_cmd(codie: Path) -> List[str]:
    return [str(codie), "--version"]


def opencode_version_cmd() -> List[str]:
    return ["opencode", "--version"]


def codie_task_cmd(codie: Path, prompt: str) -> List[str]:
    return [
        str(codie),
        "-p",
        "--mode",
        "json",
        "--provider",
        CODIE_PROVIDER,
        "--model",
        CODIE_MODEL,
        "--allow-all",
        "--no-session",
        prompt,
    ]


def opencode_task_cmd(prompt: str) -> List[str]:
    return [
        "opencode",
        "run",
        "--model",
        OPENCODE_MODEL,
        "--auto",
        "--archived",
        "--standalone",
        "--format",
        "json",
        prompt,
    ]


def codie_warmup_cmd(codie: Path) -> List[str]:
    return [
        str(codie),
        "-p",
        "--mode",
        "json",
        "--no-tools",
        "--no-session",
        "--provider",
        CODIE_PROVIDER,
        "--model",
        CODIE_MODEL,
        "Reply with exactly: ok",
    ]


def opencode_warmup_cmd() -> List[str]:
    return [
        "opencode",
        "run",
        "--model",
        OPENCODE_MODEL,
        "--auto",
        "--archived",
        "--standalone",
        "--format",
        "json",
        "Reply with exactly: ok",
    ]


# ---------------------------------------------------------------------------
# Startup phase
# ---------------------------------------------------------------------------


@dataclass
class StartupResult:
    binary: Dict[str, Dict[str, Optional[float]]]
    model: Dict[str, Dict[str, object]]
    version_text: Dict[str, str]
    binary_raw: Dict[str, List[float]] = field(default_factory=dict)
    model_raw: Dict[str, Dict[str, List[float]]] = field(default_factory=dict)


def measure_binary_startup(
    codie: Path,
    runs: int,
    env: Dict[str, str],
    scratch: Path,
) -> Tuple[Dict[str, List[float]], Dict[str, str]]:
    timings: Dict[str, List[float]] = {"codie": [], "opencode": []}
    versions: Dict[str, str] = {}
    for index in range(runs):
        order = ["codie", "opencode"] if index % 2 == 0 else ["opencode", "codie"]
        for harness in order:
            cmd = codie_version_cmd(codie) if harness == "codie" else opencode_version_cmd()
            code, output, elapsed = run_capture(cmd, cwd=scratch, env=env)
            if code != 0:
                die(f"{harness} --version failed: {output}")
            timings[harness].append(elapsed)
            versions[harness] = output.splitlines()[0] if output else ""
    return timings, versions


def measure_model_startup(
    codie: Path,
    runs: int,
    env: Dict[str, str],
    scratch: Path,
    log_dir: Path,
) -> Tuple[Dict[str, Dict[str, object]], Dict[str, Dict[str, List[float]]]]:
    results: Dict[str, Dict[str, object]] = {}
    raw: Dict[str, Dict[str, List[float]]] = {}
    for harness in ("codie", "opencode"):
        totals: List[float] = []
        firsts: List[float] = []
        for index in range(runs):
            cmd = codie_warmup_cmd(codie) if harness == "codie" else opencode_warmup_cmd()
            outcome = run_streaming(
                cmd,
                scratch,
                env,
                log_dir / f"startup-{harness}-{index + 1}.log",
                timeout=180,
                detect_first_token=True,
            )
            if outcome["exit_code"] != 0:
                log(f"warning: {harness} warm-up run {index + 1} exited {outcome['exit_code']}")
            totals.append(float(outcome["elapsed"]))
            if outcome["first_token"] is not None:
                firsts.append(float(outcome["first_token"]))
        results[harness] = {
            "first_token": stats(firsts),
            "total": stats(totals),
            "runs": runs,
        }
        raw[harness] = {"first_token": firsts, "total": totals}
    return results, raw


def run_startup_phase(
    codie: Path,
    args: argparse.Namespace,
    env: Dict[str, str],
    results_root: Path,
) -> StartupResult:
    log("measuring binary startup")
    scratch = Path(tempfile.mkdtemp(prefix="codie-bench-startup-"))
    try:
        binary_timings, versions = measure_binary_startup(
            codie, args.startup_runs, env, scratch
        )
        binary_stats = {harness: stats(values) for harness, values in binary_timings.items()}
        if args.model_runs > 0:
            log(f"measuring trivial model round-trip ({args.model_runs} runs)")
            model_stats, model_raw = measure_model_startup(
                codie, args.model_runs, env, scratch, results_root / "logs"
            )
        else:
            model_stats, model_raw = {}, {}
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    return StartupResult(
        binary=binary_stats,
        model=model_stats,
        version_text=versions,
        binary_raw=binary_timings,
        model_raw=model_raw,
    )


# ---------------------------------------------------------------------------
# Server and verification helpers
# ---------------------------------------------------------------------------


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_for_url(url: str, timeout: int = 90) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                response.read(64)
                return True
        except urllib.error.HTTPError:
            return True
        except Exception:
            time.sleep(0.5)
    return False


def run_with_server(
    workdir: Path,
    env: Dict[str, str],
    log_path: Path,
    probe_path: str,
    callback: Callable[[int], Tuple[bool, str]],
) -> Tuple[bool, str]:
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "w", encoding="utf-8") as server_log:
        proc = subprocess.Popen(
            ["npm", "run", "start", "--", "-p", str(port)],
            cwd=str(workdir),
            env=env_for(workdir, env),
            stdout=server_log,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        try:
            if not wait_for_url(base + probe_path):
                return False, "server did not become ready"
            return callback(port)
        finally:
            kill_tree(proc)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass


def verify_browser(
    workdir: Path,
    env: Dict[str, str],
    spec: TaskSpec,
    log_path: Path,
) -> Tuple[bool, str]:
    def callback(port: int) -> Tuple[bool, str]:
        node = which("node") or "node"
        code, output, _ = run_capture(
            [
                node,
                str(VERIFIER_DIR / "browser_check.mjs"),
                f"http://127.0.0.1:{port}",
                spec.name,
            ],
            cwd=VERIFIER_DIR,
            env=env,
            timeout=180,
        )
        payload = ""
        for line in output.splitlines():
            if line.startswith("BENCH_RESULT "):
                payload = line[len("BENCH_RESULT ") :]
                break
        ok = code == 0
        if payload:
            try:
                ok = ok and bool(json.loads(payload).get("ok"))
            except json.JSONDecodeError:
                ok = False
        return ok, (payload or output)[:4000]

    return run_with_server(workdir, env, log_path, "/", callback)


def verify_hidden_test(
    workdir: Path,
    env: Dict[str, str],
    spec: TaskSpec,
) -> Tuple[bool, str]:
    hidden = TASKS_DIR / spec.name / "verify" / "todos.hidden.test.ts"
    if not hidden.exists():
        return False, "hidden test missing"
    target = workdir / "__hidden_todos.test.ts"
    shutil.copy2(hidden, target)
    try:
        node = which("node") or "node"
        code, output, _ = run_capture(
            [node, "--experimental-strip-types", "--test", target.name],
            cwd=workdir,
            env=env,
            timeout=120,
        )
        return code == 0, output[-4000:]
    finally:
        try:
            target.unlink()
        except OSError:
            pass


def http_json(
    url: str,
    method: str = "GET",
    payload: Optional[Dict[str, object]] = None,
    timeout: int = 10,
) -> Tuple[int, object]:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={"content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
            return response.status, json.loads(body) if body else {}
    except urllib.error.HTTPError as error:
        body = error.read()
        try:
            parsed = json.loads(body) if body else {}
        except json.JSONDecodeError:
            parsed = {}
        return error.code, parsed


def todos_from(payload: object) -> List[Dict[str, object]]:
    if isinstance(payload, list):
        return [item for item in payload if isinstance(item, dict)]
    if isinstance(payload, dict):
        for key in ("todos", "data", "items"):
            value = payload.get(key)
            if isinstance(value, list):
                return [item for item in value if isinstance(item, dict)]
    return []


def todo_from(payload: object) -> Dict[str, object]:
    if isinstance(payload, dict) and isinstance(payload.get("todo"), dict):
        return payload["todo"]  # type: ignore[return-value]
    if isinstance(payload, dict):
        return payload
    return {}


def verify_api(
    workdir: Path,
    env: Dict[str, str],
    spec: TaskSpec,
    log_path: Path,
) -> Tuple[bool, str]:
    steps: List[Dict[str, object]] = []

    def step(name: str, ok: bool, detail: str = "") -> None:
        steps.append({"name": name, "ok": bool(ok), "detail": detail})

    def callback(port: int) -> Tuple[bool, str]:
        base = f"http://127.0.0.1:{port}/api/todos"

        status, payload = http_json(base)
        step("GET initial is 200", status == 200, f"status={status}")
        step("GET initial returns a list", isinstance(todos_from(payload), list))

        status, payload = http_json(base, "POST", {"text": "Buy milk"})
        todo = todo_from(payload)
        todo_id = str(todo.get("id", ""))
        step("POST creates a todo", status in (200, 201), f"status={status}")
        step("POST returns an id", bool(todo_id))
        step("POST preserves text", todo.get("text") == "Buy milk", str(todo.get("text")))

        status, payload = http_json(base)
        items = todos_from(payload)
        step(
            "GET lists the created todo",
            status == 200 and any(str(item.get("id")) == todo_id for item in items),
            f"status={status} count={len(items)}",
        )

        status, payload = http_json(base, "PATCH", {"id": todo_id, "completed": True})
        updated = todo_from(payload)
        step("PATCH completes the todo", status == 200, f"status={status}")
        step("PATCH returns completed=true", updated.get("completed") is True, str(updated.get("completed")))

        status, _ = http_json(base, "POST", {"text": "   "})
        step("POST rejects empty text with 400", status == 400, f"status={status}")

        status, _ = http_json(base, "PATCH", {"id": "missing-id", "completed": True})
        step("PATCH rejects an unknown id with 404", status == 404, f"status={status}")

        delete_url = base + "?id=" + urllib.parse.quote(todo_id)
        status, payload = http_json(delete_url, "DELETE")
        step("DELETE removes the todo", status == 200, f"status={status}")
        if isinstance(payload, dict):
            step("DELETE confirms deletion", payload.get("deleted") is True, str(payload))

        status, payload = http_json(base)
        items = todos_from(payload)
        step(
            "GET no longer lists the todo",
            status == 200 and all(str(item.get("id")) != todo_id for item in items),
            f"count={len(items)}",
        )

        ok = all(entry["ok"] for entry in steps)
        return ok, json.dumps(steps)[:4000]

    return run_with_server(workdir, env, log_path, "/api/todos", callback)


# ---------------------------------------------------------------------------
# Telemetry
# ---------------------------------------------------------------------------


@dataclass
class Telemetry:
    turns: int = 0
    tool_calls: int = 0
    tool_errors: int = 0
    error_events: int = 0
    first_edit_s: Optional[float] = None


def parse_telemetry(
    harness: str,
    lines: List[str],
    line_times: List[float],
    elapsed: float,
) -> Telemetry:
    telemetry = Telemetry()
    seen_parts = set()
    for line, at in zip(lines, line_times):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        kind = record.get("type")
        if harness == "codie":
            if kind == "turn_start":
                telemetry.turns += 1
            elif kind == "tool_execution_start":
                telemetry.tool_calls += 1
                if record.get("tool_name") in ("write", "edit"):
                    telemetry.first_edit_s = (
                        at if telemetry.first_edit_s is None else min(telemetry.first_edit_s, at)
                    )
            elif kind == "tool_execution_end":
                if record.get("is_error"):
                    telemetry.tool_errors += 1
            elif kind == "error":
                telemetry.error_events += 1
            elif kind == "message_end":
                message = record.get("message") or {}
                if message.get("role") == "assistant" and message.get("stop_reason") == "error":
                    telemetry.error_events += 1
        else:
            if kind == "step_finish":
                telemetry.turns += 1
            elif kind == "tool_use":
                part = record.get("part") or {}
                part_id = part.get("id")
                if part_id not in seen_parts:
                    seen_parts.add(part_id)
                    telemetry.tool_calls += 1
                state = part.get("state") or {}
                if state.get("status") == "error" or state.get("error"):
                    telemetry.tool_errors += 1
                if part.get("tool") in ("write", "edit", "patch", "multiedit"):
                    telemetry.first_edit_s = (
                        at if telemetry.first_edit_s is None else min(telemetry.first_edit_s, at)
                    )
            elif kind == "error":
                telemetry.error_events += 1
    if telemetry.turns == 0 and elapsed > 0:
        telemetry.turns = 1
    return telemetry


# ---------------------------------------------------------------------------
# Task phase
# ---------------------------------------------------------------------------


@dataclass
class TaskRun:
    task: str
    harness: str
    run: int
    elapsed: float
    exit_code: int
    timed_out: bool
    success: bool
    build_ok: bool
    build_seconds: Optional[float]
    functional_ok: bool
    functional_detail: str
    package_changed: bool
    page_exists: bool
    uses_local_storage: bool
    has_input: bool
    has_filters: bool
    changed_files: List[str] = field(default_factory=list)
    changed_lines: int = 0
    turns: int = 0
    tool_calls: int = 0
    tool_errors: int = 0
    error_events: int = 0
    first_edit_s: Optional[float] = None
    tokens_input: int = 0
    tokens_output: int = 0
    tokens_reasoning: int = 0
    tokens_cache_read: int = 0
    cost_usd: float = 0.0
    log: str = ""
    workdir: str = ""


@dataclass
class TaskSummary:
    runs: List[TaskRun]
    scaffold_package_hash: str


def is_success(run: TaskRun) -> bool:
    return (
        run.build_ok
        and run.functional_ok
        and not run.timed_out
        and not run.package_changed
    )


def verify_task(
    workdir: Path,
    baseline: Dict[str, str],
    baseline_package_hash: str,
    spec: TaskSpec,
    env: Dict[str, str],
    results_root: Path,
    run_label: str,
) -> Dict[str, object]:
    package_path = workdir / "package.json"
    package_changed = hash_file(package_path) != baseline_package_hash
    page_exists = (workdir / "src" / "app" / "page.tsx").exists() or (
        workdir / "app" / "page.tsx"
    ).exists()
    changed = diff_snapshot(baseline, workdir)
    text = changed_source_text(workdir, changed)

    build_started = time.perf_counter()
    build_code = 124
    build_output = "build did not run"
    try:
        proc = subprocess.run(
            ["npm", "run", "build"],
            cwd=str(workdir),
            env=env_for(workdir, env),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=DEFAULT_BUILD_TIMEOUT,
        )
        build_code = proc.returncode
        build_output = proc.stdout or ""
    except subprocess.TimeoutExpired:
        build_output = "npm run build timed out"
    build_seconds = time.perf_counter() - build_started
    build_ok = build_code == 0 and (workdir / ".next" / "BUILD_ID").exists()

    forbidden_changed = [name for name in spec.forbidden if name in changed]

    functional_ok = True
    functional_detail = ""
    if build_ok:
        if spec.verifier == "browser":
            functional_ok, functional_detail = verify_browser(
                workdir, env, spec, results_root / "logs" / f"verify-{run_label}.log"
            )
        elif spec.verifier == "hidden-test":
            functional_ok, functional_detail = verify_hidden_test(workdir, env, spec)
        elif spec.verifier == "http-api":
            functional_ok, functional_detail = verify_api(
                workdir, env, spec, results_root / "logs" / f"verify-{run_label}.log"
            )
    else:
        functional_ok = False
        functional_detail = "build failed"

    if forbidden_changed:
        functional_ok = False
        functional_detail = (
            "forbidden files changed: " + ", ".join(forbidden_changed) + "; " + functional_detail
        )[:4000]

    (results_root / "logs" / f"build-{run_label}.log").write_text(
        build_output, encoding="utf-8"
    )

    return {
        "package_changed": package_changed,
        "page_exists": page_exists,
        "changed_files": changed,
        "changed_lines": count_lines(workdir, changed),
        "build_ok": build_ok,
        "build_seconds": build_seconds,
        "build_output": build_output,
        "functional_ok": functional_ok,
        "functional_detail": functional_detail,
        "uses_local_storage": "localstorage" in text.lower(),
        "has_input": bool(re.search(r"<input|role=[\"']textbox", text, re.I)),
        "has_filters": all(
            word in text.lower() for word in ("completed", "active", "all")
        ),
    }


def parse_codie_usage(log_path: Path) -> Dict[str, float]:
    totals = {
        "input": 0,
        "output": 0,
        "reasoning": 0,
        "cache_read": 0,
        "cost_usd": 0.0,
    }
    if not log_path.exists():
        return totals
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("type") != "message_end":
            continue
        message = record.get("message") or {}
        if message.get("role") != "assistant":
            continue
        usage = message.get("usage") or {}
        totals["input"] += int(usage.get("input", 0) or 0)
        totals["output"] += int(usage.get("output", 0) or 0)
        totals["reasoning"] += int(usage.get("reasoning", 0) or 0)
        totals["cache_read"] += int(usage.get("cache_read", 0) or 0)
        cost = usage.get("cost") or {}
        totals["cost_usd"] += float(cost.get("total", 0) or 0) / 1_000_000_000.0
    return totals


def parse_opencode_usage(log_path: Path) -> Dict[str, float]:
    totals = {
        "input": 0,
        "output": 0,
        "reasoning": 0,
        "cache_read": 0,
        "cost_usd": 0.0,
    }
    if not log_path.exists():
        return totals
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("type") != "step_finish":
            continue
        part = record.get("part") or {}
        tokens = part.get("tokens") or {}
        cache = tokens.get("cache") or {}
        totals["input"] += int(tokens.get("input", 0) or 0)
        totals["output"] += int(tokens.get("output", 0) or 0)
        totals["reasoning"] += int(tokens.get("reasoning", 0) or 0)
        totals["cache_read"] += int(cache.get("read", 0) or 0)
        totals["cost_usd"] += float(part.get("cost", 0) or 0)
    return totals


def run_task_once(
    spec: TaskSpec,
    harness: str,
    index: int,
    codie: Path,
    prompt: str,
    env: Dict[str, str],
    work_root: Path,
    results_root: Path,
) -> TaskRun:
    run_label = f"{spec.name}-{harness}-{index}"
    workdir = work_root / run_label
    log(f"{spec.name} / {harness} run {index}: preparing")
    template_before = snapshot_tree(TEMPLATE_DIR)
    prepare_task_workdir(spec, workdir)
    baseline = snapshot_tree(workdir)
    baseline_package_hash = hash_file(workdir / "package.json")

    log_path = results_root / "logs" / f"task-{run_label}.log"
    cmd = (
        codie_task_cmd(codie, prompt)
        if harness == "codie"
        else opencode_task_cmd(prompt)
    )

    log(f"{spec.name} / {harness} run {index}: coding")
    outcome = run_streaming(
        cmd,
        workdir,
        env,
        log_path,
        timeout=DEFAULT_TASK_TIMEOUT,
    )
    if outcome["timed_out"]:
        log(f"{spec.name} / {harness} run {index}: timed out")

    if snapshot_tree(TEMPLATE_DIR) != template_before:
        die(
            f"{harness} run {index} modified the shared scaffold at {TEMPLATE_DIR}. "
            "Refusing to continue: the harness must run inside its copy."
        )

    log(f"{spec.name} / {harness} run {index}: verifying")
    verification = verify_task(
        workdir,
        baseline,
        baseline_package_hash,
        spec,
        env,
        results_root,
        run_label,
    )

    usage = (
        parse_codie_usage(log_path)
        if harness == "codie"
        else parse_opencode_usage(log_path)
    )
    telemetry = parse_telemetry(
        harness,
        list(outcome["lines"]),
        list(outcome["line_times"]),
        float(outcome["elapsed"]),
    )
    snapshot_dir = results_root / "snapshots" / run_label
    copy_changed(workdir, verification["changed_files"], snapshot_dir)

    run = TaskRun(
        task=spec.name,
        harness=harness,
        run=index,
        elapsed=float(outcome["elapsed"]),
        exit_code=int(outcome["exit_code"]),
        timed_out=bool(outcome["timed_out"]),
        success=False,
        build_ok=bool(verification["build_ok"]),
        build_seconds=float(verification["build_seconds"]),
        functional_ok=bool(verification["functional_ok"]),
        functional_detail=str(verification["functional_detail"]),
        package_changed=bool(verification["package_changed"]),
        page_exists=bool(verification["page_exists"]),
        uses_local_storage=bool(verification["uses_local_storage"]),
        has_input=bool(verification["has_input"]),
        has_filters=bool(verification["has_filters"]),
        changed_files=list(verification["changed_files"]),
        changed_lines=int(verification["changed_lines"]),
        turns=telemetry.turns,
        tool_calls=telemetry.tool_calls,
        tool_errors=telemetry.tool_errors,
        error_events=telemetry.error_events,
        first_edit_s=telemetry.first_edit_s,
        tokens_input=int(usage["input"]),
        tokens_output=int(usage["output"]),
        tokens_reasoning=int(usage["reasoning"]),
        tokens_cache_read=int(usage["cache_read"]),
        cost_usd=float(usage["cost_usd"]),
        log=str(log_path.relative_to(results_root)),
        workdir=str(workdir),
    )
    run.success = is_success(run)
    return run


def run_task_phase(
    codie: Path,
    args: argparse.Namespace,
    env: Dict[str, str],
    results_root: Path,
    progress: Optional[Callable[[List[TaskRun]], None]] = None,
) -> TaskSummary:
    specs = load_tasks(args.tasks)
    work_root = Path(tempfile.mkdtemp(prefix="codie-bench-work-"))
    log(f"work directory: {work_root}")
    runs: List[TaskRun] = []
    try:
        for spec in specs:
            prompt = spec.prompt.read_text(encoding="utf-8").strip()
            for index in range(1, args.runs + 1):
                order = ["codie", "opencode"] if index % 2 == 1 else ["opencode", "codie"]
                for harness in order:
                    runs.append(
                        run_task_once(
                            spec,
                            harness,
                            index,
                            codie,
                            prompt,
                            env,
                            work_root,
                            results_root,
                        )
                    )
                    if progress:
                        progress(runs)
    finally:
        if not args.keep_work:
            shutil.rmtree(work_root, ignore_errors=True)
        else:
            log(f"kept work directory: {work_root}")

    return TaskSummary(
        runs=runs,
        scaffold_package_hash=hash_file(TEMPLATE_DIR / "package.json"),
    )


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def summarize_runs(runs: List[TaskRun]) -> Dict[str, Dict[str, Dict[str, object]]]:
    summary: Dict[str, Dict[str, Dict[str, object]]] = {}
    tasks = []
    for run in runs:
        if run.task not in tasks:
            tasks.append(run.task)
    for task in tasks:
        summary[task] = {}
        for harness in ("codie", "opencode"):
            selected = [run for run in runs if run.task == task and run.harness == harness]
            if not selected:
                continue
            elapsed = [run.elapsed for run in selected]
            builds = [run.build_seconds for run in selected if run.build_seconds is not None]
            summary[task][harness] = {
                "runs": len(selected),
                "successes": sum(1 for run in selected if run.success),
                "time": stats(elapsed),
                "build": stats(builds),
                "turns": stats([run.turns for run in selected]),
                "tool_calls": stats([run.tool_calls for run in selected]),
                "tool_errors": stats([run.tool_errors for run in selected]),
                "error_events": sum(run.error_events for run in selected),
                "first_edit": stats(
                    [run.first_edit_s for run in selected if run.first_edit_s is not None]
                ),
                "tokens_input": sum(run.tokens_input for run in selected),
                "tokens_output": sum(run.tokens_output for run in selected),
                "tokens_reasoning": sum(run.tokens_reasoning for run in selected),
                "cost_usd": sum(run.cost_usd for run in selected),
                "changed_lines": sum(run.changed_lines for run in selected),
                "functional": sum(1 for run in selected if run.functional_ok),
            }
    return summary


def aggregate_runs(runs: List[TaskRun]) -> Dict[str, Dict[str, object]]:
    aggregate: Dict[str, Dict[str, object]] = {}
    for harness in ("codie", "opencode"):
        selected = [run for run in runs if run.harness == harness]
        if not selected:
            continue
        aggregate[harness] = {
            "runs": len(selected),
            "successes": sum(1 for run in selected if run.success),
            "time": stats([run.elapsed for run in selected]),
            "cost_usd": sum(run.cost_usd for run in selected),
            "tokens_input": sum(run.tokens_input for run in selected),
            "tokens_output": sum(run.tokens_output for run in selected),
            "tool_calls": sum(run.tool_calls for run in selected),
            "turns": sum(run.turns for run in selected),
        }
    return aggregate


def write_summary(
    results_root: Path,
    metadata: Dict[str, object],
    startup: Optional[StartupResult],
    tasks: Optional[TaskSummary],
) -> None:
    lines: List[str] = []
    lines.append("# codie vs opencode benchmark")
    lines.append("")
    lines.append(f"- Started: {metadata['started']}")
    lines.append(f"- Model: `{MODEL_ID}` (OpenRouter)")
    lines.append(f"- Runs per harness per task: {metadata['runs']}")
    lines.append(f"- Tasks: {', '.join(metadata['tasks'])}")
    lines.append(f"- Platform: {metadata['platform']}")
    lines.append(f"- codie: `{metadata['versions'].get('codie', '?')}`")
    lines.append(f"- opencode: `{metadata['versions'].get('opencode', '?')}`")
    lines.append("")
    lines.append("## Startup")
    lines.append("")
    if startup:
        lines.append("| Harness | Launch min | Launch median | Launch p95 | First token median | Total median |")
        lines.append("|---|---|---|---|---|---|")
        for harness in ("codie", "opencode"):
            binary = startup.binary.get(harness, {})
            model = startup.model.get(harness, {})
            first = (model.get("first_token") or {}).get("median")
            total = (model.get("total") or {}).get("median")
            lines.append(
                "| {} | {} | {} | {} | {} | {} |".format(
                    harness,
                    fmt_ms(binary.get("min")),
                    fmt_ms(binary.get("median")),
                    fmt_ms(binary.get("p95")),
                    fmt_seconds(first),
                    fmt_seconds(total),
                )
            )
    else:
        lines.append("_Not measured._")
    lines.append("")
    lines.append("## Task suite")
    lines.append("")
    if tasks:
        summary = summarize_runs(tasks.runs)
        lines.append(
            "| Task | Harness | Success | Time median | Time min | Time max | Verify | Turns | Tools | First edit | Cost |"
        )
        lines.append("|---|---|---|---|---|---|---|---|---|---|---|")
        for task in TASK_ORDER:
            if task not in summary:
                continue
            for harness in ("codie", "opencode"):
                entry = summary[task].get(harness)
                if not entry:
                    continue
                lines.append(
                    "| {} | {} | {}/{} | {} | {} | {} | {}/{} | {} | {} | {} | ${:.4f} |".format(
                        task,
                        harness,
                        entry["successes"],
                        entry["runs"],
                        fmt_seconds(entry["time"].get("median")),
                        fmt_seconds(entry["time"].get("min")),
                        fmt_seconds(entry["time"].get("max")),
                        entry["functional"],
                        entry["runs"],
                        fmt_count(entry["turns"].get("median")),
                        fmt_count(entry["tool_calls"].get("median")),
                        fmt_seconds(entry["first_edit"].get("median")),
                        entry["cost_usd"],
                    )
                )
        lines.append("")
        lines.append("### Across the suite")
        lines.append("")
        aggregate = aggregate_runs(tasks.runs)
        lines.append("| Harness | Success | Median | Cost | Cost / success | Input tokens | Output tokens |")
        lines.append("|---|---|---|---|---|---|---|")
        for harness in ("codie", "opencode"):
            entry = aggregate.get(harness)
            if not entry:
                continue
            successes = entry["successes"] or 0
            lines.append(
                "| {} | {}/{} | {} | ${:.4f} | {} | {} | {} |".format(
                    harness,
                    entry["successes"],
                    entry["runs"],
                    fmt_seconds(entry["time"].get("median")),
                    entry["cost_usd"],
                    f"${entry['cost_usd'] / successes:.4f}" if successes else "-",
                    entry["tokens_input"],
                    entry["tokens_output"],
                )
            )
        lines.append("")
        lines.append("### Per-run detail")
        lines.append("")
        lines.append(
            "| Task | Harness | Run | Time | Success | Build | Verify | Turns | Tools | Tool errors | First edit | Cost |"
        )
        lines.append("|---|---|---|---|---|---|---|---|---|---|---|---|")
        for run in tasks.runs:
            lines.append(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | ${:.4f} |".format(
                    run.task,
                    run.harness,
                    run.run,
                    fmt_seconds(run.elapsed),
                    "ok" if run.success else "fail",
                    "ok" if run.build_ok else "fail",
                    "ok" if run.functional_ok else "fail",
                    run.turns,
                    run.tool_calls,
                    run.tool_errors,
                    fmt_seconds(run.first_edit_s),
                    run.cost_usd,
                )
            )
    else:
        lines.append("_Not run._")
    lines.append("")
    lines.append("## Notes")
    lines.append("")
    lines.append("- Both harnesses received the same prompt, task scaffold, model, and auto-approval settings.")
    lines.append("- opencode ran with an isolated XDG home and a private server (`--standalone`).")
    lines.append("- Task time is wall-clock time for the harness process; build and functional verification are separate.")
    lines.append("- Success requires a passing build, passing functional/hidden/API checks, and an unchanged package.json.")
    lines.append("")

    (results_root / "summary.md").write_text("\n".join(lines), encoding="utf-8")
    print("\n".join(lines))


def write_results(
    results_root: Path,
    metadata: Dict[str, object],
    startup: Optional[StartupResult],
    tasks: Optional[TaskSummary],
) -> None:
    payload: Dict[str, object] = {"metadata": metadata}
    if startup:
        payload["startup"] = {
            "binary": startup.binary,
            "binary_raw": startup.binary_raw,
            "model": startup.model,
            "model_raw": startup.model_raw,
            "version_text": startup.version_text,
        }
    if tasks:
        payload["task"] = {
            "summary": summarize_runs(tasks.runs),
            "aggregate": aggregate_runs(tasks.runs),
            "runs": [asdict(run) for run in tasks.runs],
            "scaffold_package_hash": tasks.scaffold_package_hash,
        }
    (results_root / "results.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8"
    )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def parse_args(argv: List[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Benchmark codie against opencode on a task suite."
    )
    parser.add_argument(
        "phase",
        nargs="?",
        default="all",
        choices=["setup", "startup", "task", "all"],
        help="which phase to run (default: all)",
    )
    parser.add_argument(
        "--tasks",
        default="all",
        help="comma-separated task names or `all` (todo-ui,feature-add,bug-fix,api-todo)",
    )
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS, help="runs per harness per task")
    parser.add_argument(
        "--startup-runs",
        type=int,
        default=DEFAULT_STARTUP_RUNS,
        help="binary launch iterations per harness",
    )
    parser.add_argument(
        "--model-runs",
        type=int,
        default=DEFAULT_MODEL_RUNS,
        help="trivial model round-trip iterations per harness (0 disables)",
    )
    parser.add_argument(
        "--codie",
        type=Path,
        default=DEFAULT_CODIE,
        help="path to the codie binary",
    )
    parser.add_argument("--build", action="store_true", help="build codie before running")
    parser.add_argument(
        "--reset",
        action="store_true",
        help="delete and rebuild the scaffold and the isolated opencode home",
    )
    parser.add_argument(
        "--keep-work",
        action="store_true",
        help="keep temporary task work directories for inspection",
    )
    parser.add_argument(
        "--skip-browser",
        action="store_true",
        help="do not warn when Chrome is unavailable",
    )
    parser.add_argument(
        "--yes",
        action="store_true",
        help="confirm that the run may call the paid model API",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print the commands and exit without running them",
    )
    return parser.parse_args(argv)


def dry_run(codie: Path, args: argparse.Namespace) -> None:
    print(
        "scaffold: npx --yes create-next-app@latest "
        + str(TEMPLATE_DIR)
        + " --ts --app --no-tailwind --eslint --src-dir --import-alias '@/*' --use-npm --yes"
    )
    print("codie version:    " + " ".join(codie_version_cmd(codie)))
    print("opencode version: " + " ".join(opencode_version_cmd()))
    print("codie warmup:     " + " ".join(codie_warmup_cmd(codie)))
    print("opencode warmup:  " + " ".join(opencode_warmup_cmd()))
    print("codie task:       " + " ".join(codie_task_cmd(codie, "<prompt>")))
    print("opencode task:    " + " ".join(opencode_task_cmd("<prompt>")))
    print("tasks:")
    for spec in load_tasks(args.tasks):
        print(f"  {spec.name} ({spec.verifier})")
        print("    " + spec.prompt.read_text(encoding="utf-8").strip().replace("\n", " ")[:200])


def main(argv: List[str]) -> int:
    args = parse_args(argv)
    load_dotenv(REPO_ROOT / ".env")
    codie = args.codie.resolve()

    if args.dry_run:
        dry_run(codie, args)
        return 0

    check_binaries(codie, args)

    if args.phase != "setup" and not args.yes:
        die("this benchmark calls the paid model API; re-run with --yes")

    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S")
    results_root = RESULTS_DIR / timestamp
    results_root.mkdir(parents=True, exist_ok=True)
    (results_root / "logs").mkdir(exist_ok=True)

    if args.reset and OPENCODE_HOME.exists():
        shutil.rmtree(OPENCODE_HOME, ignore_errors=True)
    OPENCODE_HOME.mkdir(parents=True, exist_ok=True)

    env = harness_env()
    setup_scaffold(args)
    ensure_verifier()

    versions = {}
    for harness, cmd in (
        ("codie", codie_version_cmd(codie)),
        ("opencode", opencode_version_cmd()),
    ):
        code, output, _ = run_capture(cmd, cwd=REPO_ROOT, env=env)
        versions[harness] = output.splitlines()[0] if code == 0 and output else "unavailable"

    metadata: Dict[str, object] = {
        "started": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "node": run_capture(["node", "--version"], env=env)[1],
        "npm": run_capture(["npm", "--version"], env=env)[1],
        "versions": versions,
        "model": MODEL_ID,
        "tasks": load_tasks(args.tasks),
        "runs": args.runs,
        "startup_runs": args.startup_runs,
        "model_runs": args.model_runs,
        "task_timeout_seconds": DEFAULT_TASK_TIMEOUT,
    }
    metadata["tasks"] = [spec.name for spec in metadata["tasks"]]  # type: ignore[index]

    startup: Optional[StartupResult] = None
    tasks: Optional[TaskSummary] = None

    if args.phase in ("startup", "all"):
        startup = run_startup_phase(codie, args, env, results_root)
        write_results(results_root, metadata, startup, None)

    if args.phase in ("task", "all"):
        log(f"running {args.runs} run(s) per harness for tasks: {', '.join(metadata['tasks'])}")
        progress = lambda partial: write_results(results_root, metadata, startup, TaskSummary(partial, ""))
        tasks = run_task_phase(codie, args, env, results_root, progress=progress)

    write_results(results_root, metadata, startup, tasks)
    write_summary(results_root, metadata, startup, tasks)
    log("results: " + str(results_root))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
