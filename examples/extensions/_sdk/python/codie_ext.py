"""Minimal Python SDK for codie out-of-process extensions.

An extension reads line-delimited JSON-RPC 2.0 on stdin and writes responses on
stdout. Only `initialize`, hooks, `tool.call`, and `command.invoke` need to be
implemented; unknown methods return a JSON-RPC error.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Callable

Handler = Callable[[dict[str, Any]], dict[str, Any] | None]


class Extension:
    def __init__(self, name: str, version: str = "0.1.0") -> None:
        self.name = name
        self.version = version
        self.handlers: dict[str, Handler] = {}
        self.on("initialize", self._initialize)

    def on(self, method: str, handler: Handler | None = None) -> Handler:
        if handler is None:
            def register(function: Handler) -> Handler:
                self.handlers[method] = function
                return function

            return register  # type: ignore[return-value]
        self.handlers[method] = handler
        return handler

    def _initialize(self, _params: dict[str, Any]) -> dict[str, Any]:
        return {
            "protocol": 1,
            "name": self.name,
            "version": self.version,
            "capabilities": ["hooks", "tools", "commands"],
            "hooks": [],
            "tools": [],
            "commands": [],
            "flags": [],
            "renderers": ["text", "markdown", "code", "diff"],
        }

    def notify(self, method: str, params: dict[str, Any] | None = None) -> None:
        record = {"jsonrpc": "2.0", "method": method, "params": params or {}}
        print(json.dumps(record), flush=True)

    def log(self, message: str, level: str = "info") -> None:
        self.notify("log", {"level": level, "message": message})

    def run(self) -> int:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                request = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" not in request:
                continue
            request_id = request["id"]
            method = request.get("method", "")
            params = request.get("params") or {}
            handler = self.handlers.get(method)
            if handler is None:
                self._respond_error(request_id, -32601, f"unknown method `{method}`")
                continue
            try:
                result = handler(params) or {}
            except Exception as error:  # noqa: BLE001 - report to the host
                self._respond_error(request_id, -32000, str(error))
                continue
            record = {"jsonrpc": "2.0", "id": request_id, "result": result}
            print(json.dumps(record), flush=True)
        return 0

    def _respond_error(self, request_id: Any, code: int, message: str) -> None:
        record = {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": code, "message": message},
        }
        print(json.dumps(record), flush=True)


def text_result(text: str, *, render: dict[str, Any] | None = None, details: Any = None) -> dict[str, Any]:
    result: dict[str, Any] = {"content": [{"type": "text", "text": text}]}
    if render is not None:
        result["render"] = render
    if details is not None:
        result["details"] = details
    return result
