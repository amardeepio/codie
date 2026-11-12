#!/usr/bin/env python3
"""Minimal codie RPC client.

Usage:
    python3 rpc_client.py "Summarize this repository"

The codie binary is taken from CODIE_BIN, then ./target/debug/codie, then
`codie` on PATH. Extra codie arguments can be provided with CODIE_ARGS.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys


def find_binary() -> str:
    configured = os.environ.get("CODIE_BIN")
    if configured:
        return configured
    local = os.path.join("target", "debug", "codie")
    if os.path.isfile(local):
        return local
    return "codie"


class Client:
    def __init__(self) -> None:
        self.process = subprocess.Popen(
            [find_binary(), "--mode", "rpc", *shlex.split(os.environ.get("CODIE_ARGS", ""))],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )

    def send(self, record: dict) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(record) + "\n")
        self.process.stdin.flush()

    def records(self):
        assert self.process.stdout is not None
        for line in self.process.stdout:
            line = line.strip()
            if line:
                yield json.loads(line)

    def close(self) -> int:
        assert self.process.stdin is not None
        self.process.stdin.close()
        return self.process.wait(timeout=10)


def main() -> int:
    prompt = " ".join(sys.argv[1:]) or "Say hello and name the model you are using."
    client = Client()
    client.send({"id": "1", "command": "prompt", "message": prompt})

    aborted = False
    try:
        for record in client.records():
            kind = record.get("type")
            if kind == "response" and record.get("id") == "1":
                if not record.get("success"):
                    print("error:", record.get("error"), file=sys.stderr)
                    return 1
                print(f"session: {record['data'].get('session')}", file=sys.stderr)

            elif kind == "message_end":
                message = record.get("message", {})
                if message.get("role") == "assistant":
                    for block in message.get("content", []):
                        if block.get("type") == "text":
                            print(block["text"])

            elif kind == "agent_settled":
                aborted = record.get("aborted", False)
                break
    except KeyboardInterrupt:
        client.send({"id": "abort", "command": "abort"})

    client.send({"id": "2", "command": "shutdown"})
    status = client.close()
    return 0 if status == 0 and not aborted else 1


if __name__ == "__main__":
    raise SystemExit(main())
