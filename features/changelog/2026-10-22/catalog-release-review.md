Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 能力同步发布隔离与实体页复查修复

能力同步原先只有批次互斥，没有核验当前发布实例。候选 Server 在预热阶段即可按自己的能力快照改写共享目录，切换后旧 Server 也可能写回旧定义。此外，窄屏实体页缺少“全部实体”入口，未挂载目录的实体无法从手机端找到；删除的使用说明仍被文档链接及 Windows 打包脚本依赖。

## 修复及边界

- 托管 Server 的同步写入核验发布记录中的版本、监听端口和阶段，覆盖自动同步、手动同步、开始状态及错误记录。预热候选、同版本旧实例及切换中的实例只允许检查差异，拒绝写入不改变目录或同步状态。
- Server 从权限核验到最终提交持有 `catalog-writer.lock` 共享锁，发布日志保存持独占锁。切换等待已开始的同步结束，排队请求拿到锁后重新核验；进程退出由内核释放锁。锁或发布记录缺失时拒绝写入。独立 Server 保留原同步行为。
- 桌面与窄屏共用“全部实体”入口，覆盖未归入目录的实体。切换目录范围重置分页，原有搜索、类型和状态筛选继续生效。
- 恢复仍被使用的 17 份通用说明，补齐 123 处文档引用及 Windows 包的 README 输入。发布说明补充当前实例写入规则。
- CI 执行完整能力同步集成测试、跨语言文件锁测试和发布锁恢复测试。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| 候选启动、手动同步及来源失败均不改共享目录 | `candidate_startup_and_manual_sync_leave_shared_catalog_and_status_unchanged` |
| 阶段、实例端口、缺失记录及回滚写权限 | `only_current_activation_can_write_after_switch_or_rollback` |
| 排队旧请求在切换后拒绝写入 | `queued_old_sync_rechecks_ownership_after_switch` |
| Rust 同步持锁期间 Python 发布日志不能切换 | `release_journal_waits_for_catalog_commit_and_old_writer_cannot_resume` |
| 持锁进程退出、日志失败及重试保留锁文件 | `test_catalog_writer.py` 的 2 个用例 |
| 手机查找无目录实体、搜索和失败重试 | `EntitiesPage.test.tsx` 的 2 个用例 |
| 新页面四种宽度、目录切换、手机创建及加载/失败/重试 | `scripts/acceptance/ontology/entities.mjs` |

## 验证范围

- `cargo test -p opencoder-control --test ontology_catalog --test release_handoff`：13 个用例通过。
- `cargo test -p opencoder-control --lib ontology_catalog::`：2 个用例通过，包括实际跨进程 Rust/Python 文件锁。
- `cargo test -p opencoder-core --lib ontology::`：2 个用例通过；`cargo clippy -p opencoder-core -p opencoder-control --all-targets -- -D warnings` 通过。
- 发布脚本 `test_catalog_writer.py`、`test_deployment.py`、`test_recovery.py`：分别 2、29、2 个用例通过，覆盖切换中断后续跑和回滚。
- SPA 的实体页与目录辅助函数：共 7 个用例通过；Ontology 类型检查、SPA 构建和产物漂移检查通过。
- 本轮 Server 构建信息为当前基线的 dirty 工作区，协议 10、数据格式 11；二进制单独复制并记录 SHA256。浏览器核对实际服务的 HTML、JS、CSS 与工作区产物逐字节一致。
- 浏览器在 390、768、1280、1920 像素宽度完成 13 组实体页状态检查、16 组能力详情检查，保留截图并确认页面无横向溢出；自动字段只读、人工补充说明在同步后保留。
- 恢复文档的本地引用和 Windows 打包所需输入已检查。当前 Linux 环境未运行 Windows MSVC 构建；恢复说明文件没有改变 Windows 原生代码和打包流程。

本轮共执行 57 个定向 Rust、Python、SPA 用例，均已通过；没有运行全量测试。格式检查、改动文件行数及相关文档链接检查通过。本轮三个复查问题已闭环，未发现剩余的代码阻塞项。

生产发布未执行。正式发布仍须从干净提交构建可追溯包，使用本轮控制器脚本与对应 Server，按原发布流程完成入口切换和观察。代码及隔离验证不代表线上 CPU 已下降；CPU 优化的已有定向测试和合成基准见 [索引同步优化](cpu-index-sync.md)。

相关说明：[Ontology 功能](../../ontology/index.md)、[控制面](../../../agents/control/index.md)、[页面实现](../../../agents/web/ontology.md)、[发布约定](../../../docs/smooth-release.md)。
