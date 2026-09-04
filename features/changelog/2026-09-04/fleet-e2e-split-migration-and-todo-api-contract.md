Commit: (working-tree, 待提交)

# Fleet e2e 迁移三二进制 + TODO workflows API 契约打磨

## Context

两个收尾动作，均来自 todo-web-management 评审（同日）的 TODO 清单：

- **P0（workspace 收口）**：daemon→三二进制拆分已收敛（`daemon --server/--client` 仅打印迁移指引即退出；server 实体为 `crates/server` 的 `opencoder-server`，worker 为 `crates/agent` 的 `opencoder-agent`），但根包 3 个 e2e 与 `scripts/smoke_nodes.sh` 仍 spawn `daemon --server`，全部卡在 "server did not start"，阻塞 `cargo test --workspace` 全量回归。
- **P1（API 契约）**：`POST /api/todo/workflows/:id/interrupt` 对终态工作流经 anyhow 错误落 500（语义应为 409）；`resume` 对终态返回裸 `200 {ok:true}`，与真实接管不可区分。

## Changes

- `tests/support/mod.rs`（新增，52 行）：`sibling_bin(candidates)` —— 从本测试二进制同目录解析 workspace 兄弟二进制。集成测试只能拿到本包目标的 `CARGO_BIN_EXE_*`，而 fleet e2e 刻意住在根包、server/agent 二进制住在各自 crate；`cargo test --workspace`（rules/02 规定的回归门）先构建全部成员二进制再跑任何测试，故同目录解析是确定性的。候选名列表兼容 `opencode-server`/`opencode-agent` 与 `opencoder-server`/`opencoder-agent` 两种拼写（拆分迭代的命名仍在振荡，bin 名为包拼写、daemon 迁移指引与部分文档为无 r 拼写——以存在性探测优先包拼写，两种收敛方向都绿）。
- `tests/daemon_smoke.rs`：server→`opencoder-server`（`--workdir` 变普通旗标）、worker→`opencoder-agent`（`--workflow-root` 收进临时目录，不碰 `/workflow`）；断言消息与文档同步改指新二进制。
- `tests/running_mode_switch_e2e.rs`：`spawn_server` 改 spawn `opencoder-server`（一处函数、两处调用点）。
- `tests/nodes_smoke_proc.rs`：注入 `OPENCODER_SMOKE_SERVER_BIN`/`OPENCODER_SMOKE_AGENT_BIN`（原 `OPENCODER_SMOKE_BIN` 单值不再适用）。
- `scripts/smoke_nodes.sh`：`SERVER_BIN`/`AGENT_BIN` 双注入点 + 缺省 release 构建；worker 侧播种确定性回环 LLM stub 配置（`http://127.0.0.1:9`，checkpoint 3 本就接受 `error` 终态）——启动不再依赖机器全局配置，零凭证可复跑。
- `crates/web/src/api_todo_runs.rs`（P1）：
  - `interrupt_workflow`：前置终态判定（`completed`/`failed`）→ 409（带状态文案），不再落到 runtime 的 anyhow→500。
  - `resume_workflow`：终态 → 显式 `200 {ok:true, terminal:<status>}` 且**不**再 spawn Runtime（原实现 spawn 后 runner 返回存储态原样，调用方无从区分真实接管）；running 409 与真实接管路径不变。
- `crates/web/tests/web_todo_runs.rs`（P1 断言）：R-1 中 interrupt 终态 500 断言 → 409 + 文案；resume 终态断言补 `ok==true`、`terminal=="failed"`。

## 关键决策

- bin 命名已由归属迭代收敛为包拼写：`daemon` 迁移指引与实际 bin（`opencoder-server`/`opencoder-agent`）一致，copy-paste 可用；本轮候选列表中的无 r 拼写仅作过渡兼容，命名冻结后可删（评审 TODO P1）。
- 兄弟二进制解析失败时 panic 带可操作指引（`cargo build --workspace --bins`），而不是静默 skip——回归门必须真绿。

## Tests

- `tests/daemon_smoke.rs`：`daemon_server_and_client_end_to_end`（HMAC 矩阵 + fleet 注册/心跳/来源 IP，改经新二进制）。
- `tests/nodes_smoke_proc.rs`：`smoke_script_two_process_nodes_flow_passes`（脚本 4 checkpoint）。
- `tests/running_mode_switch_e2e.rs`：2 用例（运行中 409 / 恢复边界持久化）。
- `crates/web/tests/web_todo_runs.rs`：2 用例（R-1 增补终态 409 与 `{ok,terminal}` 断言；R-2 不变）。
- 回归：`cargo test --workspace` **4382 passed / 0 failed / exit 0**（含上述全部；评审期 4 个失败项全数收口）；`cd crates/web/spa && npm test` 328 用例全绿；`scripts/check-spa-drift.sh` 无漂移。

## 协作说明

与 agent-nfs 归属迭代共享工作树：本轮曾撞上其 `api_agent_nfs.rs` 中途态（`opencode_core` 笔误 crate 名）导致的 workspace 编译红窗，等待其自行收敛后复绿，未代改（避免与其在途编辑冲突）。
