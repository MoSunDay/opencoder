Commit: 3e870567ca1d67d0d27ecab4c8550f64a9c2aca9

# 大脑画布与项目 Tag 修复发布

画布连续修改、同层并行节点删除与草稿重开修复，以及 Overview／独立 Tag 菜单，已随综合版本 `3e870567` 发布。线上公共入口返回该版本，前端资源摘要与发布源码构建产物一致。

## 行为与规则

- 连续修改与关闭重开保留最新浏览器草稿和手动布局；保存版本提交最新计划内容。删除支持确认和多选，同层其他节点及布局保持一致。
- 项目菜单提供 Overview 和 Tag；Overview 内保留项目、专项、TODO 三个标签。
- [AGENTS.md](../../../AGENTS.md)、[回归规则](../../../rules/02-regression-gate.md)及 [UI 验收规则](../../../rules/05-ui-acceptance.md)统一为“改到哪里测哪里”。发布也不默认执行全量测试；仅用户明确要求时执行全量。

## 测试覆盖

| 功能 | 验收名 | 入口 |
| --- | --- | --- |
| 连续编辑、删除确认、同层与多选删除、布局、保存重开 | `brain-editor: Backspace / Delete` | [画布验收](../../../scripts/acceptance/brain/ui/editor.js) |
| Overview／Tag 导航、Tag 增删改、草稿及四种宽度 | `project-navigation-tags` | [项目页面验收](../../../scripts/acceptance/project_workbench_ui.js) |
| 线上资源版本、三标签、独立菜单、移动端及接口 | `public-ui` | 发布证据中的 `check_public_ui.cjs` |
| 发布后的健康状态与原生执行探测 | 15 分钟线上观察 | 同版本生产观察回执所记录的真实任务与健康采样 |

本次 Rust 全量测试按用户要求停止，不作为通过证据；未执行全站 UI 回归。复用已通过且实现未变化的定向验收，上线页面检查无浏览器异常，项目接口返回 200。

线上观察完成 901.8 秒，169 次原生执行探测通过（重复运行同一个容器检查）；最终入口开放，4 个节点就绪。发布阶段已等待原执行完成；发布备份和未完成的备份证据均保留。首次观察的最大调度间隔为 30.808 秒且有 3 次健康探测超时，未记为通过；原始失败证据保留，复验沿用最大值 30 秒门槛。

证据：`/data00/workspace/artifacts/opencoder-brain-tag-release-20261010-np58hs4n/delivery-result.json`。综合发布、备份和安装核验位于 `/var/tmp/opencoder-deploy-20261010`；采用的线上观察原始回执为 `/data00/workspace/artifacts/opencoder-mvp-production-20261010-vqqwfyh3/production-observation.json`。

相关：[大脑计划](../../brain/index.md)、[项目模块](../../project/index.md)。
