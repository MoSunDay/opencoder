Commit: (working-tree, pre-initial-commit)

# read/edit/search 与 bash 同享 130s 超时（到期仅取消，不后台化）

## 背景
runner 的 leaf-tool 安全网（`execute_call`）此前对所有非 bash 工具一律套用
`DEFAULT_TOOL_TIMEOUT`（600s）。`read`/`edit`/`search` 本是本地快速工具，一旦卡死
（如巨型目录 ripgrep、NFS 挂起）会占用整个 turn 长达 10 分钟。

本次给三者与 bash 相同的墙钟预算（`BASH_TIMEOUT_SECS` = 130s，`cfg(test)` 下 1s），
但到期语义不同于 bash：bash 到期会把进程移交后台继续运行（handoff），而
read/edit/search 走通用 leaf-tool 路径——直接丢弃 future 并返回 `"timed out"` 消息，
不做任何后台延续。

## 变更

### `crates/session/src/runner/execute.rs`（+77 行，705 → 782，< 800 限）
- **提取纯函数 `leaf_tool_timeout(name: &str) -> Option<Duration>`**：替代 `execute_call`
  内联的 `if name == "bash"` 判定。路由表：
  - `"bash"` → `None`（豁免，自有内部前台超时 + 后台 handoff，两个 deadline 不竞争）。
  - `"read" | "edit" | "search"` → `Some(BASH_TIMEOUT_SECS)`（与 bash 同预算，到期走通用
    取消路径——无 handoff）。
  - 其它 → `Some(DEFAULT_TOOL_TIMEOUT)`（600s 兜底；`task` 在此之前 early-return）。
- 复用 `crate::tools::bash::BASH_TIMEOUT_SECS`（非新常量）：需求即「与 bash 同」，耦合是
  有意的，且自动继承 `cfg(test)` → 1s 的测试加速。

### 为什么复用而非新建常量
需求字面是「与 bash 相同的超时」。直接引用 `BASH_TIMEOUT_SECS` 让二者永远同步，且自动
继承 `cfg(test)` 缩短（1s），无需再维护第二个测试覆写。

## 测试覆盖（当次实跑）
- `cargo test -p opencoder-session --lib` → 247 passed / 0 failed（基线 243 + 新增 4）
  - `leaf_tool_timeout_routes_read_edit_search_to_bash_budget` — 纯路由：read/edit/search → `Some(BASH_TIMEOUT_SECS)`
  - `leaf_tool_timeout_exempts_bash` — bash → `None`
  - `leaf_tool_timeout_defaults_unknown_tools` — `ls`/未知 → `Some(DEFAULT_TOOL_TIMEOUT)`
  - `read_tool_times_out_via_execute_call_routing` — **经 `execute_call`（非 `_with_timeout`）**
    注册 `"read"` → HangingTool，断言在 ~1s（测试 `BASH_TIMEOUT_SECS`）内返回 `"timed out"`
    错误；外层 5s `tokio::time::timeout` 守卫确保若误用 600s 网则测试快速失败（而非挂 10 分钟）。
    实测 1.00s 完成，证明走的是路由后的 bash 预算。
- `cargo clippy -p opencoder-session --all-targets -- -D warnings` → 0 warning
- `cargo build -p opencoder-session` → Finished，0 error
- `cargo build --workspace` → Finished，0 error（dev profile：workspace 全部 crate 的 lib 编译通过）
- `cargo test --workspace` → ⚠️ 不可跑：opencoder-tui 测试目标有 51 个编译错误（scope 外的 keymap 重构未完成，与本变更无关）。session crate 的 247 passed / 0 failed 为本变更的权威结果。

## 风险
- **`search` 用 `spawn_blocking`**：deadline 触发时 future 被丢弃，阻塞的 ripgrep 任务继续
  detached 运行（结果被丢弃，非 bash 式「后台化」）。与原 600s 行为一致，仅缩短了引线，可接受。
