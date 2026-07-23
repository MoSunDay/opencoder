# Fix: TUI subagent back-hint keybind + stale `.opencode` path references

**Date:** 2026-07-23
**Scope:** `opencoder-tui`, `opencoder-core`, `opencoder-cli`, `opencoder-session`, `scripts`

## Problem

Two user-facing inconsistencies accumulated from the `.opencode` → `.opencoder`
config-directory rename and the subagent keybind change:

1. **Wrong back-hint keybind (P1):** The TUI subagent body title showed
   `[Esc] back`, but the actual key that returns from a subagent view is
   `Ctrl+L` (see `keybind.rs` `pre_key_intercept` + help text). Users following
   the on-screen hint pressed a no-op key.

2. **Stale `~/.opencode` path references (P1):** Four locations still referenced
   the pre-rename directory `~/.opencode/` while the rest of the codebase
   canonically uses `~/.opencoder/` (`config.rs`, `skill.rs:205`,
   `skill.rs:45`). Concretely:
   - `install-skills-dep.sh` set `OP_DIR="${HOME}/.opencode"`, so the
     `.skills-deps` sentinel was written to the **wrong** directory — the dep-
     gated skills (ssh-pty, chrome-headless) would never seed on next startup.
   - `write_install_script()` computed `~/.opencode` as the destination, writing
     the install script outside the config home.
   - `exit_tips.rs` and `chrome_headless.rs` printed `~/.opencode/...` in
     user-facing strings, directing users to a path that does not exist.

## Fixes

| Fix | File | Change |
|-----|------|--------|
| 1 — Correct back hint | `crates/tui/src/app_loop.rs` | Subagent body title `[Esc] back` → `[Ctrl+L] back`; matching comment updated. Aligns with the `Ctrl+L` binding in `keybind.rs`. |
| 2 — Install script OP_DIR | `scripts/install-skills-dep.sh` | `OP_DIR="${HOME}/.opencode"` → `"${HOME}/.opencoder"` (+ comment). Sentinel now lands where `seed_dep_gated_skills_in` looks. |
| 3 — write_install_script dest | `crates/core/src/skill.rs` | `write_install_script()`: `h.join(".opencode")` → `h.join(".opencoder")`; doc comments corrected. |
| 4 — exit tip path | `crates/cli/src/exit_tips.rs` | `~/.opencode/install-skills-dep.sh` → `~/.opencoder/...` |
| 5 — chrome-headless error path | `crates/session/src/tools/chrome_headless.rs` | `not_found_msg()`: `~/.opencode/...` → `~/.opencoder/...` |

## 测试覆盖

No behavioral logic changed — all edits are display strings, doc comments, and
path literals. No new tests are warranted (rules/01: no new `pub fn`, behavior,
or contract). Existing coverage guards the surrounding code.

| Area | Existing guard | File |
|------|----------------|------|
| Subagent view rendering | `subagent_tests` suite (render/flush paths) | `crates/tui/src/chat.rs` (+ `app_tests.rs`) |
| Skills discovery path | `skills_resolve_to_config_home` | `crates/core/tests/skill_contract.rs` |
| Config dir resolution | `config_contract` suite | `crates/core/tests/config_contract.rs` |

- 全量回归：`cargo test --workspace` → **826 passed / 0 failed / 0 ignored**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → clean
