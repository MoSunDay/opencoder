# UI 设备受控合同候选

本次候选把设备管理服务源码纳入 `deploy/device-manager/`，并提供从 Git 提交构建可追溯源码包的脚本。候选已合并线上同期新增的设备准入转换与测试，预约上限及隔离核查按现有设备账本计算，保留新增设备和已有预约。Server 为 `uicase-regression` 生成严格的受控定义；Host 按冻结 case、spec SHA-256 和输入版本签发私有设备上下文。设备服务区分 Native 与 UI 预约，按预约绑定 case，先登记 UI 工作再签名代理到所属 Windows NodeAPI。操作 ID 在远程调用前落盘；未确认的操作不自动重发。未完成恢复的 UI 工作不能释放设备。Server 要求 `ui_device_v1`，当前 Worker 不声明该能力。

候选继续加入 UI 完成端点：设备管理服务独立重读固定目录中的冻结规格、逐项断言证据、截图、完整原始抓包清单和恢复回执，并在归还设备前复验。受控截图端点从所属 Windows NodeAPI 获取 PNG 并保存哈希；受控抓包端点以管理服务身份启动和停止现有 Windows 采集器，业务操作先要求抓包已启动。完成校验要求截图和抓包对应服务端登记且成功的操作，并核对采集器内部完整性；Windows 动作收到 HTTP 200 但业务响应未确认也记为不确定。管理服务可从已落盘的所属证据对账不确定的截图或抓包操作，不重复远程调用；不确定业务动作仍阻止释放。客户端增加 `ui-screenshot`、`ui-capture`、`ui-reconcile` 和 `ui-complete`。这些文件仍缺真实执行器、业务动作对账和现场恢复探针，不能据此启用 Worker 能力。

验证：设备服务 Native/UI/准入测试 43/43（新增 `UI completion independently checks frozen assertions, capture, restoration, and hashes`，受控截图与抓包并入 `scoped UI action is signed, persisted once, and never replayed after a lost outcome`）；`opencoder-dag` UI 定义测试通过；`opencoder-dag-runtime` 设备相关 11 项测试通过；Python 私有客户端 7 项测试通过（UI 完成、截图、抓包和证据对账作用域检查）；`opencoder-control` 常规编译检查通过。共享 Cargo target 的 `opencoder-control --lib` 测试曾解析到旧 `opencoder-dag` 产物；改用独立 Cargo target 后 UI 能力门禁测试通过。

尚未实现 UI 原始 case 执行器、截图与抓包证据生产、环境恢复的现场独立探针和 CLI 查询闭环。此候选未发布，不能用于真实 UI case 验收。Native 线上服务仍使用原目录源码；发布前须将候选、其他并发改动一起验证并切到可追溯制品。
