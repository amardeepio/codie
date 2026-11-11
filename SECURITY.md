# Security policy

codie is a coding agent that can read, modify, and execute code on your machine.
We take the trust boundary that creates seriously, and we document it explicitly
so users know what the project does and does not promise.

## Supported versions

Security fixes land on the newest release line. Before 1.0, `0.MINOR` is the
compatibility unit and only the latest minor receives fixes.

| Version | Supported |
|---|---|
| 1.x (latest minor) | :white_check_mark: after 1.0 GA |
| 0.x (latest minor) | :white_check_mark: security and critical fixes |
| Older 0.x minors | :x: upgrade to the latest minor |

## Reporting a vulnerability

**Do not open a public issue, discussion, or pull request for a vulnerability.**

Preferred channel: [GitHub Security Advisories](https://github.com/amardeepio/codie/security/advisories/new)
(private by default, gives us a shared workspace for triage and a CVE if needed).

Backup channel: the maintainer's private contact listed on
[github.com/amardeepio](https://github.com/amardeepio). Do not open a public issue for a suspected
vulnerability.
Encrypt sensitive material if you can; ask for a key in your first message.

Please include:

- Affected version(s) and commit, plus platform and terminal.
- A description of the impact and the trust boundary that is crossed.
- Reproduction steps or a minimal proof of concept, with secrets redacted.
- Whether the issue is already public and any planned disclosure date.

### What to expect

- **Acknowledgement** within 3 business days.
- **Triage and severity assessment** within 10 business days, using CVSS v4.0
  as a starting point.
- **Coordinated disclosure window of 90 days** from acknowledgement. We will
  ship a fix and publish an advisory within that window when feasible. If a fix
  is not ready, we will agree on a new date with you rather than publish
  silently. Earlier disclosure is welcome once a patched release is available.
- **Credit** in the advisory and release notes unless you ask to stay anonymous.

## Threat model

The model is documented in `PLAN.MD` §11 and enforced by specs under
`specs/security/` (`SEC-001..007`).

### In scope

We treat these as security bugs:

1. **Accidental destructive tool execution.** The agent or a bug causes data
   loss outside the intended scope: writes outside the workspace, broad deletes,
   process-tree escapes, or a timeout that leaves orphaned children.
2. **Secret leakage.** Credentials, tokens, or private file contents escaping
   through logs, error messages, child processes, session files, JSON events,
   or exports despite the redaction filter.
3. **Untrusted project resources.** A malicious `.codie/` directory,
   `AGENTS.md`, skill, or other project-supplied configuration influencing the
   agent without the project trust gate, or executing code the user did not
   approve.
4. **Supply-chain compromise.** A malicious dependency, build script, release
   artifact, or update path that compromises users. Signing, checksums, SBOMs,
   and locked dependencies are the controls.
5. **Malicious extension processes.** An extension crossing the declared
   capability boundary, reading data it was not granted, or spoofing agent
   output in a way that triggers unapproved actions.

### Out of scope

These are known, documented limitations. Reports about them are answered with
guidance, not advisories:

- **OS-level sandboxing.** codie is not a sandbox. Use a container, VM, or a
  restricted user account for untrusted work. We publish recipes rather than
  claim isolation we cannot guarantee.
- **Prompt injection from repository content.** Any agent that reads untrusted
  text can be influenced by it. We mitigate with permission gates and clear
  provenance, but we do not guarantee that a prompt injection cannot change
  model behavior. Reports should show a concrete bypass of a control, not only
  that the model was persuaded.
- **User-installed extensions.** Extensions are code you chose to run, with the
  permissions you granted. Bugs in a third-party extension are the author's
  responsibility; report them upstream. A flaw in the extension *isolation
  mechanism* is in scope.
- **`--allow` / `--allow-all` on an untrusted repository.** Deliberately
  disabling approval prompts on content you do not trust is user error. The
  flags exist for automation and are documented as such.

Also out of scope: vulnerabilities in third-party model providers, issues that
require an already-compromised local account, and missing hardening headers in
documentation-only web properties.

## Safe harbor

We support good-faith security research. If you make a reasonable effort to
follow this policy, avoid privacy violations, data destruction, and service
disruption, and give us a chance to fix the issue before disclosure, we will:

- Not pursue or support legal action against you.
- Treat your research as authorized under applicable anti-hacking laws.
- Work with you on disclosure timing and credit.

This safe harbor does not cover extortion, intentional data exfiltration, or
attacks on systems and users outside this project.

## Hardening guidance for users

- Run `codie` in a container or VM when working in an untrusted repository.
- Keep `--allow` and `--allow-all` for trusted automation only; prefer
  interactive approval on unfamiliar code.
- Never commit `auth.json`, session files, or `.codie/` state. The default
  `.gitignore` covers them; verify before pushing.
- Run `codie doctor` to confirm the effective provider endpoints.
- Keep dependencies and the binary up to date; releases are signed and
  checksummed.
