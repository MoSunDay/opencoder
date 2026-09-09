Commit: c9bfebbb553777c4aaf16db69c30648e1fd71819

# 历史任务的 Runner 输出、登录副本与 Go 执行环境

相关模块：[节点执行](../../../agents/worker/index.md)、[独立副本验收](../../../scripts/acceptance/business/README.md)。

真实历史评测暴露了业务 Runner 向非阻塞 stdout 写入大型 NDJSON 时忽略短写的问题。修复位于独立业务仓库 `/data00/github/jy-hub-agent-opencoder-closure`：按 UTF-8 字节偏移写完整帧，对中断及暂时不可写进行有期限的处理，保留 16 MiB 帧限制及严格解析。未修改原 `/root/workspace` 中的代码。

验收账号副本增加当前 bytedcli 的 XDG `data` 目录。通用 `JWT_TOKEN` 不作为 bytedcli 身份；仅沿用 CLI 支持的显式身份变量，或独立复制已有登录数据。真实 Runner 已重新获取历史 case 5 的全部 45 个 spans，采集缺口为零，归因结果有证据地排除了已经恢复的建账漏参。

历史 Go 回归使用明确启用的离线 metrics 夹具、目标声明的 Go 1.24.0 及兼容链接参数。业务执行器为 Go 默认设置 `GOMAXPROCS=4`，避免按宿主 64 核展开编译而耗尽原有 512 个进程额度；原 8 CPU / 16 GiB 沙箱限制及测试命令不变。目标和基线的 4 次实际执行、14 次顶层测试调用通过。该结果只覆盖声明的测试环境，不等同于真实指标后端或历史提交的业务准出。

业务 API 与两个注册 Runner 已发布 `3e86f31416f9b79afe30e2eb4c058e8cf56a373c`，Runner revision 均为 5；保留 `9a64b959` 已有的 Viking 流程与历史业务记录。平台四个二进制维持已验证的 `6e4f2bf7`，本次 opencoder 改动仅涉及验收工具和文档。

验证：业务项目全量 156 项测试；验收 Python 30 项、投递隔离 Node 3 项；真实大消息、45 spans 归因、固定节点/FIFO 排队及两项任务的 Web 折叠、刷新回放、下载哈希核验。回归平台执行成功，但历史提交的业务准出仍为 `inconclusive`：生产指标采样与裁剪配置、基数预算及资源/丢失实测等证据不足。

三份本轮归属的临时运行目录已在停止服务、检查打开文件和挂载后清理，累计约 102 GB 运行数据，证据保留。原 workspace 由隔离边界保护；源目录审计同时发现其他任务的并发修改，本轮未修改或回退这些内容，不将整个目录描述为前后完全一致。

证据保留在 `/root/.cache/opencoder-e2e/20260909-closure-1733/evidence/`，包括 `business-deployment/result.json`、`actual-regression-tests/summary.json`、`release/current-source-validation.json` 及独立业务源码 patch/bundle。评测完整证据在 `20260909-historical-final-1800/evidence/identity-rerun/evidence/`，回归复跑证据在 `20260909-historical-eval-1738/evidence/fanout-rerun/evidence/`。
