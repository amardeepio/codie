#!/usr/bin/env node
/** Example plan-mode extension: blocks mutating tools and injects context. */
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { Extension } from "../_sdk/typescript/codie-ext.ts";

const extension = new Extension("plan-mode");

function marker(params: Record<string, unknown>): string {
  const session = (params.session ?? {}) as Record<string, unknown>;
  const cwd = String(session.cwd ?? process.cwd());
  return join(cwd, ".codie", "plan-mode");
}

function enabled(params: Record<string, unknown>): boolean {
  return existsSync(marker(params));
}

function setEnabled(params: Record<string, unknown>, value: boolean): void {
  const path = marker(params);
  if (value) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, "on\n");
  } else if (existsSync(path)) {
    rmSync(path);
  }
}

extension.on("initialize", () => ({
  protocol: 1,
  name: "plan-mode",
  version: "0.1.0",
  capabilities: ["hooks", "commands", "renderers"],
  hooks: ["before_tool_call", "context", "session_start"],
  tools: [],
  commands: [
    {
      name: "plan",
      description: "Turn plan mode on or off",
      usage: "[on|off]",
    },
  ],
  flags: [],
  renderers: ["markdown"],
}));

extension.on("before_tool_call", (params) => {
  if (!enabled(params)) return {};
  const call = (params.call ?? {}) as Record<string, unknown>;
  const name = String(call.name ?? "");
  if (name === "write" || name === "edit" || name === "bash") {
    return {
      block: true,
      reason: "Plan mode is on. Ask the user to approve the plan or run /plan off.",
    };
  }
  return {};
});

extension.on("context", (params) => {
  if (!enabled(params)) return {};
  return {
    context:
      "Plan mode is ON. Do not edit files or run shell commands. Inspect the repository, then present a concrete plan.",
  };
});

extension.on("session_start", (params) => {
  const state = enabled(params) ? "on" : "off";
  extension.log(`plan mode ${state}`);
  return {};
});

extension.on("command.invoke", (params) => {
  const argument = String(params.arguments ?? "").trim();
  if (argument === "on") setEnabled(params, true);
  else if (argument === "off") setEnabled(params, false);
  else setEnabled(params, !enabled(params));
  const state = enabled(params);
  return {
    display: {
      kind: "markdown",
      title: "Plan mode",
      text: state
        ? "Plan mode is **on**. Mutating tools are blocked until `/plan off`."
        : "Plan mode is **off**. Mutating tools follow the normal permission policy.",
    },
  };
});

process.exit(await extension.run());
