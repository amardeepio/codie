---
id: SPEC-TOOL-003
title: Web fetch tool
status: implemented
version: 1.0.0
owners: ["@codie-maintainers"]
requirements: ["TOOL-020", "TOOL-021", "TOOL-022"]
---

# Context

The agent can read local files and run commands, but it cannot consult documentation, changelogs, or issue pages
without shelling out through `bash` and parsing `curl` output. `web_fetch` adds one narrow, read-only network
capability with explicit egress controls. It is not a browser and does not execute JavaScript.

# Normative behavior

**TOOL-020 (MUST)** `web_fetch` accepts an `http` or `https` URL and returns the final status, content type, final
URL, and body text. HTML is converted to readable text by removing scripts, styles, and tags and decoding common
entities; other textual content is returned as-is. The result states when output was truncated.
Verification: `crates/codie-tools/tests/web_fetch.rs::spec_tool_020_fetches_and_converts_html`

**TOOL-021 (MUST)** `web_fetch` rejects non-http(s) schemes, malformed URLs, and hosts that resolve to loopback,
private, link-local, or unspecified addresses. It follows at most five redirects, re-validating each hop, and caps
the response at `max_bytes` (default 2 MiB, maximum 10 MiB) and the request at the timeout (default 30 seconds,
maximum 120 seconds).
Verification: `crates/codie-tools/tests/web_fetch.rs::spec_tool_021_rejects_unsafe_targets_and_caps_output`

**TOOL-022 (MUST)** `web_fetch` is not read-only: it requires an entry in the headless allow list (`--allow
web_fetch` or `--allow-all`) and prompts in the interactive app. It is never permitted by an empty policy.
Verification: `crates/codie-tools/tests/web_fetch.rs::spec_tool_022_requires_network_permission`

# Data shapes

```json
{"url": "https://example.com/docs", "max_bytes": 512000, "timeout": 20}
```

Host-only details report `status`, `content_type`, `final_url`, `bytes`, and `truncated`.

# Edge cases

- A redirect to a blocked address fails the whole fetch.
- Non-UTF-8 bodies are lossily decoded and marked.
- An empty body returns the status with no text.
- The tool never sends credentials, cookies, or a session identifier.
- `localhost` is blocked except through the test-only canonical override used by the test suite.

# Non-goals

No JavaScript execution, no form submission, no cookies, no authentication, no web search. Search is a separate
extension surface.

# Security and performance

Network egress is the risk, not mutation: the permission policy gates the tool, the URL guard blocks local
services, and size and time caps bound resource use. Page content is untrusted data and must never be treated as
instructions.
