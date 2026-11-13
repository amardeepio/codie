/**
 * Minimal TypeScript SDK for codie out-of-process extensions.
 *
 * Requires Node 22.6+ and `--experimental-strip-types`. The SDK deliberately
 * has no runtime dependencies.
 */
import * as readline from "node:readline";

export type Json = Record<string, unknown>;
export type Handler = (params: Json) => Json | void;

export class Extension {
  private handlers = new Map<string, Handler>();
  readonly name: string;
  readonly version: string;

  constructor(name: string, version = "0.1.0") {
    this.name = name;
    this.version = version;
    this.on("initialize", () => ({
      protocol: 1,
      name: this.name,
      version: this.version,
      capabilities: ["hooks", "tools", "commands"],
      hooks: [],
      tools: [],
      commands: [],
      flags: [],
      renderers: ["text", "markdown", "code", "diff"],
    }));
  }

  on(method: string, handler: Handler): this {
    this.handlers.set(method, handler);
    return this;
  }

  notify(method: string, params: Json = {}): void {
    process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
  }

  log(message: string, level = "info"): void {
    this.notify("log", { level, message });
  }

  async run(): Promise<number> {
    const lines = readline.createInterface({ input: process.stdin });
    for await (const line of lines) {
      if (!line.trim()) continue;
      let request: Json;
      try {
        request = JSON.parse(line) as Json;
      } catch {
        continue;
      }
      if (!("id" in request)) continue;
      const id = request.id;
      const method = String(request.method ?? "");
      const params = (request.params ?? {}) as Json;
      const handler = this.handlers.get(method);
      if (!handler) {
        this.error(id, -32601, `unknown method \`${method}\``);
        continue;
      }
      try {
        const result = handler(params) ?? {};
        process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
      } catch (error) {
        this.error(id, -32000, error instanceof Error ? error.message : String(error));
      }
    }
    return 0;
  }

  private error(id: unknown, code: number, message: string): void {
    process.stdout.write(
      `${JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } })}\n`,
    );
  }
}

export function textResult(
  text: string,
  extra: { render?: Json; details?: unknown } = {},
): Json {
  return {
    content: [{ type: "text", text }],
    ...(extra.render ? { render: extra.render } : {}),
    ...(extra.details !== undefined ? { details: extra.details } : {}),
  };
}
