Commit: (working-tree, 基于 9f39c6cb36fb511c1e3a921338f4f1c7dc36efa1)

# 独立副本上的真实业务验收与 NFS 完整资源包

新增 `scripts/acceptance/business/`，使用真实 case 5、固定提交的代码回归和真实 Codex，在临时 API、Server、单并发 FIFO Node 上完成业务请求、排队、Runner、发布、Web 消息展开、刷新与产物下载。仅消息投递使用本地接收器。

本轮采用固定节点和 workspace 调度。原 `/root/workspace` 在执行命名空间中只读；Git 对象、模型账号副本、缓存和数据库归本次临时目录所有。评测通过显式只读挂载保护原目录，回归沿用 OverlayFS/chroot 的独立工作区，并只读映射 Go 实际安装目录。结束前保留回执与前后审计，停止服务、卸载 NFS 后销毁运行副本；并发任务产生的原目录差异只记录，不回滚。

真实挂载验收发现 NFS 把超过 60 字节的相对路径当成不存在，导致完整上传的 skill 包缺少 `references/service-contract.md`。现在短路径沿用旧句柄，长路径使用有界摘要句柄；导出重启后可从资源树恢复，恢复过程不遍历符号链接。上传后经实际 NFS 挂载校验全部文件的 SHA256，并验证 Node 固定的资源副本。

验证覆盖：

- `opencoder-agents`：28 项通过，1 项既有手动挂载用例忽略；新增长路径查找、列举、读取、重启恢复、失效句柄及链接边界用例。
- Worker 资源快照：3 项通过；Web 资源包与 NFS 接口：7 项通过。
- Agents 全目标 Clippy、全仓格式检查、Server release 构建通过。
- 验收脚本 4 项测试覆盖空子模块目录、只读挂载边界、消息字节分页及批处理标准输入隔离；JavaScript 语法检查通过。

收尾全仓检查：`cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo test --workspace --no-fail-fast -- --test-threads=1` 为 4,953 passed / 0 failed / 5 项既有 ignored，覆盖 360 个测试目标；`cargo build --workspace` 通过。完整输出在验收证据的 `validation/` 目录。

上述检查覆盖 `b29cc9776082f7da8d089e6b4ae43c4ea43bc524` 加本轮补丁；该基线相对实际业务 E2E 基线仅有文档提交。全量回归启动后，共享工作区出现后续 NFS、Worker 和前端修改，构建曾与前端产物重建冲突。因此最终构建在本轮源码的独立快照中执行，快照清单与完整补丁随证据保存，后续并行修改全部保留，不计入本轮已验收范围。

早期全仓检查的失败日志同样保留：修正旧配套二进制与标准输入继承后，三个节点 RPC 超时用例在单独及最终全仓运行均通过；超时根因尚未确证，没有修改断言或延长 RPC 时限来通过检查。

验收入口、依赖和清理命令见 [运行说明](../../../scripts/acceptance/business/README.md)。本轮修复仅安装到临时 Server，现有服务未更新。

真实业务验收两项均完成，Web 折叠、刷新回放及下载哈希通过。评测保留 trace/历史版本缺口；回归为 `inconclusive`，5 条尝试均在依赖准备阶段失败，不能据此准出。补充预检成功补齐依赖，但发现目标项目的 Go 链接兼容及离线 SDK 初始化问题；业务失败回执保持原样。

临时服务、进程、挂载和约 40.9 GB 的运行数据已清理。原目录前后审计记录了并发变化，没有回滚其他任务。完整记录与报告：`/root/.cache/opencoder-e2e/20260909-135609/evidence/REPORT.md`。

## 测试覆盖

| 功能 | 测试名或入口 | 文件 |
| --- | --- | --- |
| 深层资源列举、查找、读取、导出重启与失效句柄 | `complete_skill_package_survives_long_paths_and_export_restart` | `crates/agents/src/nfs/tests.rs` |
| 目录链接及伪造路径边界 | `opaque_handle_recovery_cannot_follow_a_link_outside_the_export` | `crates/agents/src/nfs/tests.rs` |
| 空子模块目录的独立 Git 对象副本 | `test_empty_submodule_directory_is_not_its_parent_repository` | `scripts/acceptance/business/tests/test_harness.py` |
| 嵌套挂载的实际只读状态 | `test_nested_writable_home_overrides_strict_root` | 同上 |
| 消息分页在 UTF8 字符中间分片 | `test_message_byte_cursor_preserves_split_utf8` | 同上 |
| 批处理不读取调用方脚本输入 | `test_batch_command_does_not_consume_its_callers_input` | 同上 |
| 真实业务发布、FIFO、配置与资源固定、模型隔离 | `main.py` / `verify.py` | `scripts/acceptance/business/` |
| 折叠、刷新回放与实际下载哈希 | `browser.mjs` | `scripts/acceptance/business/` |

相关语义：[Agent 资源逻辑](../../../agents/agents/index.md)、[Agent 平台](../../agent-platform/index.md)。
