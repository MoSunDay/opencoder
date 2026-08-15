---
name: fk-cli
description: Execute one focused mobile UI testing TODO through the fk-session command-line contract. Use for observing Android UI, launching or closing apps, tapping, swiping, typing, waiting, pressing keys, and asserting UI state without FK MCP.
---

# FK CLI

Complete only the current TODO through `fk-session`. Treat its stdout, stderr, and exit status as
the authoritative execution result.

## Execution boundary

- Use the `bash` tool only for one command shaped exactly as
  `fk-session --args '<command arguments>'`.
- Never use FK MCP tools. Never inspect environment variables, credentials, processes, files, or
  source code; never install or copy binaries; never use shell operators, pipelines, redirections,
  command substitution, or a second shell command.
- Never call `scheduler complete` or `scheduler fail`; the parent TODO workflow owns orchestration.
- Execute only commands explicitly required by the current TODO. Do not advance another TODO or
  delegate execution through `task`.
- On a missing binary, missing runtime/session/auth context, transport failure, non-zero exit, or
  rejected FK result, stop immediately and return a blocked Candidate with the exact error and the
  command needed for recovery. Do not probe or invent a fallback.

## Command forms

Use only the form declared by the focused TODO:

```text
fk-session --args 'observe screen --screenshot'
fk-session --args 'observe snapshot'
fk-session --args 'app close <package>'
fk-session --args 'app launch <package>'
fk-session --args 'gesture tap --label <label>'
fk-session --args 'gesture tap --label <label> --xy <x> <y>'
fk-session --args 'gesture tap --xy <x> <y>'
fk-session --args 'gesture swipe --from <x> <y> --to <x> <y> --duration-ms <ms>'
fk-session --args 'input text <text>'
fk-session --args 'wait <seconds>'
fk-session --args 'key <name>'
fk-session --args 'assert visible-text-any --text <candidate> [--text <candidate> ...]'
fk-session --args 'assert activity-any --activity <candidate> [--activity <candidate> ...]'
fk-session --args 'assert foreground-package --package <package>'
```

Before a UI-changing command, use the current TODO's declared observation command when present.
After the action, perform its declared verification command. A successful process exit is not a
substitute for a required follow-up observation or assertion.

## Result

Return only the Candidate JSON required by the TODO runtime. List the exact commands executed and
their accepted results in `verification`. Use `status: "candidate"` only when every declared
command succeeded; otherwise use `status: "blocked"` or `status: "interrupted"` and preserve a
concise recovery context.
