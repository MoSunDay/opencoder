Commit: 1a1255c029fed0162469ac1c2ed34f6c2160c654

# 验收投递接口、历史输入与离线 metrics 夹具

验收工具兼容旧群投递和新版 Viking 工单创建，外部投递使用本地接收器，分析始终经过真实 API、Node、Runner 和 Codex。未知投递接口拒绝启动；无需建单和本地投递回执都必须属于已完成的同一任务。

历史场景支持显式提供核实过的 trace 请求，记录复制仓库的实际分支头和祖先证明；回归分支证明不冒充历史评测部署版本。私有 Runner 继承已有 SSO 身份，宿主登录状态不变。无效输入在启动夹具或复制平台数据前拒绝。

`metricw-offline` 是显式测试环境：固定目标依赖及精确 Go 工具链，保留断言和依赖准备命令，只在副本补充缺失日志配置；测试网络仅 loopback，配置响应限于 metrics sampler/clip。记录工具链、包装器哈希、兼容链接参数和真实请求，未知配置请求导致失败。该结果不替换原始无夹具业务结论。

## 测试覆盖

| 功能 | 测试或证据 |
| --- | --- |
| 投递身份、完成状态与无需建单 | `test_not_required_does_not_bypass_job_identity_or_completion`、`test_matching_receipt_is_required_for_both_supported_delivery_types` |
| 新旧投递隔离、未知接口拒绝 | [delivery.test.mjs](../../../scripts/acceptance/business/tests/delivery.test.mjs)，3 项通过 |
| 输入拒绝与真实分支祖先关系 | [test_inputs.py](../../../scripts/acceptance/business/tests/test_inputs.py)，另核验固定历史提交的真实分支关系 |
| 私有 SSO 与无关凭证隔离 | `test_private_runner_preserves_existing_sso_without_forwarding_unrelated_secrets` |
| metrics 夹具边界和原断言保留 | [test_metricw.py](../../../scripts/acceptance/business/tests/test_metricw.py)；实际 Go 1.24.0 执行 4 个包、13 项测试通过 |
| 真实业务、报告、回放与下载 | 业务版本 `74755643`、`62648d89` 各完成一组评测及回归 E2E；均 `clean` / `pass`，各 2 条实际回归命令成功，浏览器验证通过 |

工具 Python 3.7 回归 29 项通过，Node 回归 3 项通过。生产运行代码与 `6e4f2bf7` 一致，沿用该版本已通过的全量 Rust、SPA 和平台 E2E 证据；本次没有重建或更换运行时二进制。

2026-09-09 18:11 即时复核：Server/Node 实际运行文件与 bundle 一致，服务就绪、1 个节点在线且空闲，重启次数为 0，发布后运行错误为 0。103 个 NFS 资源文件与源一致，Server 明细为空、执行索引五字段；历史记录与 56,422 个历史产物文件校验通过。工作区修改全部提交，测试服务和挂载均退出，运行数据库和证据保留，无额外观察期。

完整回执：`/var/tmp/opencoder-publish-closure-twf_de3i/release-receipt.json`。验收工具归档 `acceptance-tools-1a1255c0.tar.gz` 与同目录源码清单逐文件核对。

相关语义：[Worker](../../../agents/worker/index.md)、[验收入口](../../../scripts/acceptance/business/README.md)、[平台发布](platform-resource-release-closure.md)。
