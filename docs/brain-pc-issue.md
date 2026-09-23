# PC 问题诊断与修复

Brain 工作台的「PC 问题诊断与修复」入口首次显式注册 `pc-issue@1`，已有计划则使用最新版本（当前生产为 `pc-issue@2`）。输入问题文本和最多四张图片，选择 Brain、设备和构建节点后运行。原始图片按内容摘要冻结，通过鉴权接口读取。

五个里程碑依次为影响面定位、Windows 复现、隔离修复、构建复测和证据验收。影响面从 `/data00/workspace/index.md` 定位到实际源码及提交；设备执行沿 `device-cases` DAG 的 Node API 预约、工作注册、抓包、恢复与归还流程。修改保留在独立 worktree，由现有 `jy-builder` Team 构建，不自动合并或发布产品。

设备候选包采用完整便携 ZIP。构建输入必须绑定源码提交及实际 diff 摘要，制品须独立下载验哈。候选适配器核对包、应用及 manifest，在当前预约所属的工作目录安装，使用本次工作的 profile；原客户端由既有 supervisor 恢复。不会改写注册的机器 profile。

运行最多两轮，只有复测失败且存在可执行修复时才从 verify 反思回 repair。取消以阶段归属和确定任务 ID 约束子任务；停止后的阶段不能提交新的设备或构建任务。恢复未完成时保留设备所有权，不能按超时强制归还。

阶段输出采用单个 JSON 对象，或唯一明确标记的 JSON 代码块；代码块外的说明不进入机器报告。多块、损坏格式及无标记的混合文本均拒收。有效的受阻报告完成当前里程碑并继续传递缺口，最终明确报告未解决。

阶段完成前，Worker 核对证据文件大小和 SHA-256，并复制到执行目录，工作台可鉴权下载。产品结论单独显示：流程完成、已诊断、未复现、未解决均不等于修复成功。`fixed` 必须匹配权威前序结果中的原始复现、修复包复测及实际执行 ID。抓包报告须说明覆盖范围，不能默认所有原生 TLS 都已解密。

## 接入依赖

- Server 和 Brain 节点支持 schema 5 与 `brain_pc_issue_v1`；候选设备节点支持 `pc_candidate_v1`。
- 在设备控制宿主执行 `python3 scripts/pc_issue/install.py`，安装不可变内容摘要目录；运行的 settings.helper 可固定到返回的完整路径。
- 设备节点使用 `deploy/device-cases` 的资源 v2 和私有运行配置；Native harness 入口为 `/opt/device-cases/harness`，设备管理工作目录为 `/data00/device_mananger/runtime/native-harness`。
- helper 使用宿主已有凭证文件访问控制面和设备 API，凭证不进入问题、产物或 Git。设备和构建节点必须能访问冻结源码、用例和候选包路径。

API：`POST /api/brain/pc-issue/plan` 注册计划；`POST /api/brain/attachments` 上传图片；`GET /api/brain/attachments/:id` 读取；运行沿用 `/api/brain/runs`。实现契约在 `crates/core/src/brain/pc_issue`，受控执行工具在 `scripts/pc_issue`。
