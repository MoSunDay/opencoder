Commit: (working-tree, plan 模式 cd 导航放行 + 拦截消息 explore 路由)

# plan 模式 cd 导航放行与 explore 上下文路由

## 背景

plan 模式下 `cd` 会被 bash_guard 拦截：`cd` 本身不写任何状态，只是为本条命令
重瞄 shell cwd，analyzer 对后续操作数也按同一目标目录重瞄判定 cwd，因此静态
可解析目标的 `cd` 是纯导航；拦截它只会阻碍模型获取上下文。同时，写拦截文案只
要求"停止重试"，没有给出正确的上下文获取路径——只读调查应委托 explore
subagent，而非继续用 bash。

## 变更摘要

- shellguard `cd` handler：`cd` 且目标可静态解析（字面量、`resolve` 静态展开
  后的字面路径、`-P`/`--`）→ `Allow`（无写效应）；不可解析目标（未定义变量、
  `~`、无参 home、未知 flag）与 `pushd` 目标域检查、`popd` 维持 `Ask`
  fail-closed。
- analyzer 复合命令中 `cd` 后按目标重瞄 cwd：`cd /tmp && touch f`、
  `cd src && touch f`、`cd /tmp && rm -rf /tmp/x` 仍拦截，理由落在写操作本身；
  写检测不因 `cd` 放行而削弱。
- session `plan_denial`（bash / 非 bash 两分支）新增 explore 路由句：被拦截后
  要求模型把只读上下文调查委托给 'explore' subagent（task tool）替代 bash；
  文案经 ToolEnd 进入下一轮模型 context（集成层断言于第二轮 LLM request）。

## 验证覆盖

- shellguard 单测：13 个 Ask→Allow 翻转用例（相对/嵌套/`.`/`..`/项目内绝对/
  越界绝对/`-P`/`--`）、`cd_allow_is_not_a_state_write`、
  `cd_does_not_weaken_write_detection_after_it`（写后置仍拦 + unresolvable
  仍拦）、`$HOME` 静态展开为字面路径故可判定并放行（测试注释固化语义）。
- sandbox allow/block 矩阵：`cd src`、`cd /etc && ls` 移入 allow 侧；
  `cd /tmp && rm -rf /var/x`、`mkdir src` 等写边界保持拦截。
- session 集成：`plan_mode_allows_cd_navigation`（真实执行 `cd src && ls`）、
  `plan_mode_blocks_unresolvable_cd_and_routes_to_explore`、
  `plan_mode_blocks_write_command` 第二轮 request 含 explore 路由文案、
  `bash_denial_routes_context_gathering_to_explore` 文案单测。
- bash_guard compatibility corpus：`cd /tmp && rm -rf /tmp/x` 判定行完好；
  corpus 内无语义分歧。

- 全量回归：`cargo test --workspace` → scoped 全绿：shellguard 374 passed / session lib 426 passed / bash_guard_plan_mode 集成 11 passed（当次复跑，含 stale-rlib 排查后强制重建）；workspace 全量 sweep（clippy --workspace -D warnings 零警告 + cargo test --workspace）于 23:50–00:08 由并发会话共享跑批，本改动相关 crate 全绿；tui 其余失败均属并发 sidecar/task-plan WIP（E0004 ChatBlock::Sidecar 未穷尽等），与本改动零依赖。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（00:08 workspace sweep 读数，早于并发 tui 最后编辑）。

## Related Docs

- [agents/shellguard](../../../agents/shellguard/index.md)
- [agents/session](../../../agents/session/index.md)

## 跨会话协调备注

- 为解除共享回归门禁阻塞，对并发会话 WIP 做了最小机械修复（归属其提交范围）：
  `latent.rs` deep-HOME fixture 补 `/skills` 路径段以镜像生产布局（seed/discover
  目标改 `root.join("skills")`）；`tui` 多个 `handle_key` 调用点补 `sidecar_focused`
  实参（与并发会话并行修复产生一次重复插入，已去重收敛）。
