#!/usr/bin/env python3
"""Example subagent-lite extension: a command that expands a scoped prompt."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "_sdk" / "python"))
from codie_ext import Extension  # noqa: E402

extension = Extension("subagent-lite")


@extension.on("initialize")
def initialize(_params: dict[str, Any]) -> dict[str, Any]:
    return {
        "protocol": 1,
        "name": "subagent-lite",
        "version": "0.1.0",
        "capabilities": ["commands"],
        "hooks": [],
        "tools": [],
        "commands": [
            {
                "name": "subagent",
                "description": "Expand a focused subagent-style prompt",
                "usage": "<task>",
            }
        ],
        "flags": [],
        "renderers": ["text"],
    }


@extension.on("command.invoke")
def command(params: dict[str, Any]) -> dict[str, Any]:
    task = str(params.get("arguments", "")).strip()
    if not task:
        return {"display": {"kind": "text", "text": "Usage: /ext:subagent-lite:subagent <task>"}}
    prompt = (
        "Act as a focused subagent for exactly one task. Work only inside the "
        "current repository. Do not start unrelated work.\n\n"
        f"Task: {task}\n\n"
        "Return a concise summary of what you inspected or changed, and list "
        "any follow-up work that the parent agent should consider."
    )
    return {"prompt": prompt}


if __name__ == "__main__":
    raise SystemExit(extension.run())
