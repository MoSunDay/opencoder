Commit: e7685686ee338523547283309670a13a4560dd00

# 真实业务正例、runc 调度及副本回收闭环

验收工具修复私有挂载命名空间向 systemd 服务传递时遗漏根目录的问题，兼容宿主 Python 3.7，支持重复收集证据。运行目录保留归属标识，测试输入服务保留进程启动身份；显式授权回收后，检查进程、文件及挂载占用再销毁本轮副本。

受控回归将实际 Git 分支头与祖先验证纳入固定上下文，避免复核因快照缺少分支引用而返回证据不足。平台 bundle `0103d76b` 上的真实 Codex 评测为 `clean`、回归为 `pass`，两次实际测试命令成功；FIFO、完整 NFS 资源固定、三层折叠、刷新回放和下载哈希全部通过。早期失败证据保留，业务历史失败未改写。

新增 runc 双节点验收：轮流占满节点，自动任务在另一个就绪节点执行，固定任务留在指定节点排队。保存实际 OCI 状态和产物，结束后清理临时服务与副本。rootfs 准备脚本遵循 Cargo 的实际 target 目录。

验证为全仓 4958 通过、0 失败；全仓默认忽略的 3 项 runc 和 2 项 NFS 测试已分别执行并通过。Clippy、构建、17 项宿主 Python 测试及相关回放测试通过。完整证据位于 `/root/.cache/opencoder-e2e/20260909-positive-1720/evidence/REPORT.md`。

历史评测补取 45 条真实 trace span，仍没有历史部署 Git revision；历史回归固定提交在解决 Go 链接与日志配置后，指标 SDK 仍在包初始化中依赖 TCC/BCC。该样本不能据此准出，原报告与补充预检分开保留。

相关语义：[Worker](../../../agents/worker/index.md)、[Agent 平台](../../agent-platform/index.md)、[验收入口](../../../scripts/acceptance/business/README.md)。
