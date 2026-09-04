Commit: (working-tree, 待提交)

# todos 状态机 + envs 配置层审计修复（T-1/T-2 + E-1..E-4 + 3 项 P2）

## Context

对 todos 状态机与 envs 配置层做只读审计，产出 P0/P1/P2 分级缺陷清单。本轮修复全部 P0/P1 与三项低成本 P2：

- **T-1（P0，todos）**：外部 interrupt 落 `Interrupted` 的 TODO（`attempt >= max_attempts`）在 resume 后被 `validate_dispatch` 的 max_attempts 门禁永久拒绝——中断本非 TODO 自身失败，却令工作流无法恢复。
- **T-2（P0，todos）**：父模型 `rewind` 指向与当前 TODO 无关的里程碑时 `validate_acceptance` 不设防，子树外状态被回退失效。
- **E-1（P1，envs）**：env 层保存（`save_to` / `save_domain`）经 `OpenOptions::write+create` 写已有 0644 文件时不改权限——环境快照含 API key，权限承诺只在首建时成立。
- **E-2（P1，envs）**：`set_active_env` 先写激活标记再返回，`Config::load` 是否可解析（如 env 配置损坏）无人验证——失败配置在下次进程启动才暴露，且已破坏激活语义。
- **E-3（P1，envs）**：激活 marker 为单次 truncate+write，进程崩溃/并发激活可留半截文件；web 侧无并发激活防护。
- **E-4（P2，envs）**：`save_target` 文档注释与 TUI 激活提示文案与实际语义不符（激活期间项目层已有可编辑配置时仍写项目层）。
- **P2×3（todos）**：`json_output` 关闭 fence 判定过严（带尾随 fence 的 JSON 响应被拒）；命名含混（`json_contains` 等）；`accepted_generation` 死字段——后两者仅审阅记录，本轮不动代码。

## Change Summary

- **T-1**（`crates/todos/src/transitions.rs::validate_dispatch`）：max_attempts 门禁对 `status == Interrupted && active_session_id.is_some()` 放行——与 `execution_failed(interrupted=true)` 落 Interrupted 的判据完全镜像；再 dispatch 仍 `attempt += 1`，后续普通失败照常落 `Failed`（放行不豁免计数）。
- **T-2**（`crates/todos/src/batch.rs::validate_acceptance` Rewind 支路）：milestone 必须是当前 TODO 自身或其祖先（`milestone_todo_id == todo_id || descendants(spec, milestone_todo_id).contains(todo_id)`），否则报 `cannot rewind to milestone {id}: TODO {id} is not part of its subtree`，走既有纠错重问循环。
- **json_output**（`crates/todos/src/json_output.rs`）：全文 JSON 提取改为「最后一个裸 ``` 行即闭合 fence、其后仅允许空白」；「multiple JSON fences」错误仅在解析失败且内容内嵌 ``` 时才报。新增带内嵌反引号 fence 的验收用例。
- **E-1**（`crates/core/src/config/envs.rs`）：新增 `write_config_save`（envs_home 之下 → 创建即 0o600，成功写入后对已存在文件 chmod 收敛到 0o600；`write_file_maybe_private` 共享核心，`write_private_json` 复用）；`Config::save_to` 与 `domain::save_domain` 接入。
- **E-2**（`envs.rs::set_active_env_checked`）：先写 marker → `Config::load` 干跑校验 → 失败回滚到前一 marker 并返回含 "leaves the config unresolvable" 的 io::Error；去激活（None）透传不校验。TUI `/envs` 激活与 `session_cmd.rs`（banner 先于 `Config::load` 打印）接入。
- **E-3**（`envs.rs::write_marker_atomic` + `crates/web/src/api_envs.rs`）：marker 经同级临时文件（`active.tmp-{pid}-{subsec_nanos}`）+ fsync + rename + 目录 fsync 尽力落盘；web PATCH 激活段由 `ACTIVATE_GATE: tokio::sync::Mutex`（const_new）串行化。
- **web PATCH 语义收紧**（`api_envs.rs`）：空串/纯空白 `active` → 400（仅显式 null 去激活）；重复激活 → 200 `{"unchanged": true}` 短路不扇出；激活走 preflight，env 配置损坏 → 500 携带解析错误。
- **E-4**：`save_target` doc 注释修正；TUI 激活提示改为「→激活后配置改动默认写入此处；项目层已有可编辑配置时仍写项目层」。

## Impact Surface

- `crates/todos`：`transitions.rs` / `batch.rs` / `json_output.rs`；`json_output` 对带尾随 fence 的合法响应从拒绝改为接受（放宽）。
- `crates/core`：`config/envs.rs` 新增 4 个导出符号；`save_to`/`save_domain` 落盘路径行为变化仅在 envs_home 之下（全局/项目层写法不变）。
- `crates/web`：`PATCH /api/envs` 的 400/500/短路语义为新契约。
- `crates/tui`、`crates/cli`：仅提示与调用点接缝，无存储格式变更。

## Notes / Compatibility

- 已存在的 0644 env 快照在下一次保存时被 chmod 收敛；不主动批量迁移。
- `set_active_env`（未校验版）保留为底层原语，`set_active_env_checked` 是交互面默认。
- 遗留 P2 backlog：todos 孤儿 session 泄漏、takeover 竞态、Suspended 重复 interrupt 覆写、JoinSet panic、`json_contains`/`arguments_contains` 改名、`accepted_generation` 死字段；envs 保存/捕获非事务、并发 POST 409 旁路、envs 目录 0755、macOS `Active` 大小写、测试 thread-local 隔离。

## Related Docs

- [agents/todos](../../../agents/todos/index.md)（dispatch 豁免与 rewind 子树守卫不变量）
- [agents/core](../../../agents/core/index.md)（envs.rs：私有写入 / 原子 marker / checked 激活）

## 测试清单（规则 01）

| 保证 | 测试 |
| --- | --- |
| T-1：max_attempts 耗尽的 Interrupted TODO 可再 dispatch，普通失败仍落 Failed | `crates/todos/tests/transitions_guards.rs`（2 新增）；e2e `crates/todos/tests/interrupt_retry.rs::max_attempt_one_todo_survives_external_interrupt_and_resumes` |
| T-2：rewind 指向子树外 milestone 被纠错重问 | `crates/todos/tests/boundary_guards.rs::acceptance_rewind_to_unrelated_milestone_is_corrected` |
| json_output：内嵌反引号 fence 的响应可解析 | `crates/todos/src/json_output.rs::accepts_fenced_json_embedding_backticks` |
| E-1：env 层保存 owner-only（创建 + chmod 收敛） | `crates/core/tests/config_envs_contract.rs::env_layer_saves_are_owner_only` |
| E-2：激活 preflight 失败回滚 marker | `crates/core/tests/config_envs_contract.rs`（preflight rollback） |
| E-3：连续 marker 重写原子性 | `crates/core/tests/config_envs_contract.rs`（rapid marker rewrites） |
| Web PATCH：空白 active 400 / 损坏 env 500+回滚 / 重复激活短路 / 并发激活 marker 完整 | `crates/web/tests/web_envs.rs`（4 新增） |

回归：`cargo test --workspace` 39 个测试二进制全部通过、0 失败；key-free e2e `scripts/e2e/config_scenarios.py` 18/18 通过（深度 e2e cli/web/todos 场景硬绑定智谱 GLM 端点，本环境无 `ZHIPU_API_KEY` 未跑；业务契约由上表 workspace 集成测试覆盖）。
