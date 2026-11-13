#!/usr/bin/env python3
"""Example todo extension: tools, a command, and structured markdown."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "_sdk" / "python"))
from codie_ext import Extension, text_result  # noqa: E402

extension = Extension("todo")


def state_path(params: dict[str, Any]) -> Path:
    cwd = params.get("session", {}).get("cwd") or os.getcwd()
    directory = Path(cwd) / ".codie"
    directory.mkdir(parents=True, exist_ok=True)
    return directory / "todo.json"


def load(params: dict[str, Any]) -> list[dict[str, Any]]:
    path = state_path(params)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return []
    return value if isinstance(value, list) else []


def save(params: dict[str, Any], items: list[dict[str, Any]]) -> None:
    state_path(params).write_text(json.dumps(items, indent=2) + "\n", encoding="utf-8")


def render(items: list[dict[str, Any]]) -> dict[str, Any]:
    if not items:
        body = "_No todos._"
    else:
        body = "\n".join(
            f"- [{'x' if item['done'] else ' '}] **{item['id']}** {item['text']}"
            for item in items
        )
    return {"kind": "markdown", "title": "Todo", "text": body}


@extension.on("initialize")
def initialize(_params: dict[str, Any]) -> dict[str, Any]:
    return {
        "protocol": 1,
        "name": "todo",
        "version": "0.1.0",
        "capabilities": ["tools", "commands"],
        "tools": [
            {
                "name": "todo_add",
                "description": "Add one todo item and return the updated list.",
                "parameters": {
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"],
                    "additionalProperties": False,
                },
                "execution": "sequential",
            },
            {
                "name": "todo_list",
                "description": "List every todo item.",
                "parameters": {"type": "object", "additionalProperties": False},
                "execution": "parallel",
            },
            {
                "name": "todo_done",
                "description": "Mark one todo item as done.",
                "parameters": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"],
                    "additionalProperties": False,
                },
                "execution": "sequential",
            },
        ],
        "commands": [
            {
                "name": "todo",
                "description": "Show the todo list",
                "usage": "",
            }
        ],
        "flags": [],
        "renderers": ["markdown"],
    }


@extension.on("tool.call")
def tool_call(params: dict[str, Any]) -> dict[str, Any]:
    arguments = params.get("arguments") or {}
    name = params.get("name")
    items = load(params)
    if name == "todo_add":
        text = str(arguments.get("text", "")).strip()
        if not text:
            return {"content": [{"type": "text", "text": "text is required"}], "is_error": True}
        identifier = f"t{max([int(item['id'][1:]) for item in items] or [0]) + 1}"
        items.append({"id": identifier, "text": text, "done": False})
        save(params, items)
        return text_result(f"Added {identifier}.", render=render(items))
    if name == "todo_done":
        identifier = str(arguments.get("id", ""))
        for item in items:
            if item["id"] == identifier:
                item["done"] = True
                save(params, items)
                return text_result(f"Completed {identifier}.", render=render(items))
        return {
            "content": [{"type": "text", "text": f"Unknown todo `{identifier}`."}],
            "is_error": True,
        }
    if name == "todo_list":
        return text_result(render(items)["text"], render=render(items))
    return {"content": [{"type": "text", "text": f"Unknown tool `{name}`."}], "is_error": True}


@extension.on("command.invoke")
def command(params: dict[str, Any]) -> dict[str, Any]:
    if params.get("name") != "todo":
        return {"display": {"kind": "text", "text": "unknown command"}}
    return {"display": render(load(params))}


if __name__ == "__main__":
    raise SystemExit(extension.run())
