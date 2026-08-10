# 🚀 Quick Start

For new users: submitting tasks, browsing/editing files with notepad, and attaching a note to a task. Commands stay in English; UI keys are as shown inside the TUI.

## 📝 Submitting Tasks

### Interactive TUI

Run `opencoder` to enter the TUI:

- **`Enter`** — submit the composer input
- **`Tab`** — while running, submits a follow-up that is queued and injected at the next turn boundary (same as a normal submit when idle)
- **`/plan <content>`** — compound submit: switch to plan mode with content attached, e.g. `/plan implement an LRU cache`
- **`/task`** — open the session picker to switch/resume sessions

### Headless

```bash
# One-shot run, streaming to stdout (both forms are equivalent)
opencoder "implement an LRU cache in Rust with tests"
opencoder run "implement an LRU cache in Rust with tests"
```

### Global flags (TUI and headless)

```bash
opencoder --continue              # resume the most recent session for this workdir
opencoder --session <id> "continue"   # resume a specific session
opencoder --fork "continue"       # fork the session before resuming; the original is untouched
opencoder --model anthropic/claude-3 "..."   # override the model ({provider}/{model_id})
opencoder --image screenshot.png "look at this screenshot"  # attach an image (vision model required)
```

### Remote and tmux

```bash
# Remote: one machine runs the server, another connects with the client
opencoder server --host 0.0.0.0 --port 8080
opencoder client --remote http://127.0.0.1:8080 "summarize this repo's architecture"

# tmux: the session survives SSH disconnects and reattaches on reconnect
opencoder ts          # create/resume a tmux session
opencoder ts -l       # list managed sessions (tmux name + task info)
opencoder ts -r <id>  # reattach a specific session
```

## 📓 Notepad: File Browsing & Editing

`/notepad` (alias `/note`) opens a fullscreen IDE: file tree on the left + vim editor on the right, hiding the chat. **Notepad is a pure in-memory file viewer/editor — file contents are never injected into the model context.**

### File tree

| Key | Action |
| --- | --- |
| `j` / `k` | move up/down |
| `Enter` / `l` / `→` | open file (or expand directory) |
| `n` | create file |
| `d` | delete file (with confirmation) |
| `H` | toggle tree visibility |
| `/` | content search |

### vim editor

| Key | Action |
| --- | --- |
| `i` / `a` | enter insert mode |
| `Esc` | back to normal mode |
| `:w` | save (stay) |
| `:wq` / `:x` | save & exit back to the file tree |
| `:e <path>` | open a file (relative to the workdir) |
| `u` / `Ctrl+R` | undo / redo |
| `Ctrl+D` / `Ctrl+U` | half page down / up |
| `Ctrl+F` / `Ctrl+B` | page down / up |
| `/` | content search |

### Search

`/` opens the search panel: type a query, press `Enter` to run it, move with `j`/`k`, and `Enter` opens the hit in the editor. The backend prefers **ripgrep (`rg`)** and falls back to `grep -rn` when unavailable.

### Switching panels and exiting

`Tab` cycles focus between the tree and the editor; `Esc` exits notepad back to the chat view. Unsaved editor changes are auto-saved on exit.

## 🏷️ Annotation: Task Notes

`/annotation` (alias `/ann`) opens a vim editor for the task note. The note is stored in the session record and **survives `--continue` / `--session` resumes**.

| Key | Action |
| --- | --- |
| `:wq` | save & exit |
| `:q!` / `:q` | discard changes & exit |

> Note: in insert mode `Enter` inserts a newline and `Ctrl+C` returns to normal mode; they are no longer exit paths — use `:wq` or `:q!`/`:q` to leave.

View a saved note:

```bash
opencoder session show <id> --json
```
