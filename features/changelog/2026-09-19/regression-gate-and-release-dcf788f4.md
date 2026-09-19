# 回归门修复与 rel-dcf788f4 发布（真模型验收）

## 背景
rel-6c6ad744 发布缺少 live.py 真模型验收。本次补齐 main（squash `428dd942` + 两个测试修复）的全量回归门与完整发布链。

## 变更（均为测试/构建面，发布二进制行为与 428dd942 一致）
- `crates/worker/tests/brain_scheduler_v3.rs`：删除死代码 `if snapshot.is_err() {}`，修复 clippy `needless_ifs`（`ae6f6a3d`）。
- `crates/session/src/tools/bash.rs`：后台输出溢出测试通过 `ToolContext.extra_env` 注入独立空 `HOME`，不再依赖宿主登录环境——`bash -lc` 在空 HOME 下 source `/root/.profile`+`/root/.bashrc` 产生 ~345B stderr 噪声，会挤爆 8MiB 流预算断言（`dcf788f4`）。
- 记录测试套件隐含契约：需具备 `HOME`/`SHELL` 的登录式环境（`crates/session/tests/bash_guard_plan_mode.rs:36` 显式 `expect("$HOME set")`）；回归门 systemd-run 单元须导出 `HOME SHELL USER LOGNAME TERM`。

## 验证
- 全量回归门 @dcf788f4：build ✓ + clippy `-D warnings` ✓ + `cargo test --workspace` ✓（5379 通过 / 0 失败；log `/tmp/opencoder-regression/gate-main.log`）。
- 平台 Python 单测 4 组 57 例 @428dd942：signal 12 + rolling 20 + platform 19 + smooth_release 6 全绿。
- bash 溢出测试最小环境 A/B：修复前 3/3 fail → 修复后 3/3 pass。
- build.sh bundle `/tmp/opencoder-release-rel-dcf788f4`（4 二进制，protocol 10）；备份 `/tmp/opencoder-release-backups/backup-dcf788f4-20260919-2324`。

## 发布
- live.py `--signal --observe-seconds 900`：**PASS**；current `rel-dcf788f42eac069d518436accb649238a23780aa`，previous `rel-6c6ad744`。
- 接收/调度间隔：max_accept 0.10s、max_accept_gap 0.20s、max_scheduling_delay 0.051s、max_scheduling_gap 0.201s（均 <1s）。
- 观察 900s / 165 采样；长任务跨发布存活（model_shell + runtime 进程被跟踪）；SSE 游标恢复（246/247/258/449/450/451）；旧 Runtime rel-6c6ad744 已退役。
- 公共入口 18081：`{"commit":"0.1.0 (dcf788f4)","ok":true,"protocol_version":10}`。
- 证据：`/tmp/opencoder-acceptance/release-live-5d737bc8c7791960/result.json`。

## 后续
- 并行分支（brain v3 admission + lane closure，@9513d000）尚未合入 main；其发布需包含 `dcf788f4` 之后的 main 或经合并，避免覆盖本次测试修复。
