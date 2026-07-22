---
name: ssh-pty
description: Persistent SSH sessions via local tmux. Connect once, then send commands and read output across multiple tool calls without losing shell state (cwd, exported vars, background processes). Non-interactive only — vim/nano/top are rejected. Requires tmux + an SSH key or passwordless auth.
---

# ssh-pty skill

You have a persistent SSH tool backed by a local tmux session. This lets you
run commands on a remote host while preserving shell state (working directory,
exported variables, background jobs) across multiple tool calls.

## Workflow

1. **connect** — create the persistent session:
   `ssh_pty(action="connect", host="user@host")`
   Optional: `port`, `key_path`. The tmux session is named `oc-ssh-<host>`.

2. **send** — run a command and get the new output:
   `ssh_pty(action="send", command="ls -la /var/log")`
   Output is captured after the command completes (up to 30 s timeout).

3. **read** — snapshot the current pane without sending a command:
   `ssh_pty(action="read", lines=200)`

4. **status** — check if the session is alive:
   `ssh_pty(action="status")`

5. **disconnect** — tear down the session:
   `ssh_pty(action="disconnect")`

## Rules

- **Non-interactive only.** Interactive programs (vim, nano, top, htop, less,
  man, python REPL, mysql shell) are rejected. Use their non-interactive
  equivalents: `sed`/`awk` for editing, `head -n` for paging, `python3 -c`,
  `mysql -e`.
- **One command at a time.** `send` waits for the command to finish before
  returning output. Do not chain with `&&` unless each segment is non-interactive.
- **Output is truncated** to 800 lines / 4 KB. If you need more, use `read`
  with a larger `lines` parameter or pipe through `grep`/`head`.
- **Environment is persistent.** `cd` and `export` from one `send` survive to
  the next. Leverage this instead of absolute paths everywhere.
