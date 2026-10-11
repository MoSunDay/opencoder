Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# Ontology 页面使用当前 Server 的数据空间

Ontology 移除右上角环境选择和环境管理页面，保留图谱、实体、实体类型、关系类型四个入口。页面直接使用所连接 Server 初始化的默认数据空间，浏览器旧的环境选择不再影响读写；环境列表和管理接口不再由前端调用。

服务端存储结构、已有数据及接口未变。页面直接渲染后，同步更新初始抽屉、弹窗和加载提示的 antd 参数；导航测试提供完整的 Ontology 空列表响应，图形渲染交给独立图谱测试，标题检查区分观测方式标签和页面标题。

相关索引：[Ontology](../../ontology/index.md)、[前端逻辑](../../../agents/web/ontology.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 旧环境选择不影响当前 Server 的读取与创建 | `reads and creates in the Server namespace despite a stale browser environment selection` | `crates/web/spa/src/ontology/navigation/serverEnvironment.test.tsx` |
| 请求失败保留错误并重试 | `keeps read failures visible and retries directly against the Server namespace` | 同上 |
| viewer 仍只读且无环境选择 | `preserves viewer permissions without an environment selector` | 同上 |
| 页面标题与观测方式标签 | `ontologyGraph keeps exactly one title source` | `crates/web/spa/src/shell/headerContract.dom.test.jsx` |
| 四个页面保留桌面与移动端导航名称 | `names every menu-only page in the mobile page Select` | `crates/web/spa/src/appNavigation.dom.test.jsx` |
| 验收范围覆盖导航注册页面 | `every registered page has a functional case, separate from responsive snapshots` | `scripts/acceptance/ui/scope.test.js` |

- Ontology 定向回归：22 个测试文件、100 项测试通过；页面标题 20 项通过，导航具名用例 1 项通过。
- Ontology 类型检查、SPA 构建、验收范围 4 项、Ontology 验收脚本单测 2 项、发布脚本回归 67 项通过。
- 390、768、1280 像素全站 16 页检查通过，无横向溢出；1920 像素图谱测量通过，但截图重复超时，未取得完整通过回执。
- 全仓检查未通过：Clippy 遇到工作区项目模块 `ProjectGoalPatch.current_summary` 缺字段；格式检查遇到 `project/summary.rs` 缺失与格式差异；Rust 全量测试和构建在 120 秒限制内未结束。前端全量回归在机器高负载下停止，不能记为全量通过。
- 证据日志：`/tmp/ontology-server-env-{ontology-tests,header-final,navigation-final,build-final,clippy,fmt,cargo-test,cargo-build,spa-all,release-tests}.log`；屏宽结果位于 `/tmp/ontology-server-env-responsive-*`。

## 合入现网版本的定向验证

按用户确认的范围，以现网 `6988229a` 为基础合入本次 Ontology 修改，保留现网已有功能；该修改提交为 `fc51e6cf`，没有 Rust 或数据库变更。随后核对现有统一发布源码，17 个 Ontology 变更文件全部一致，导航中的环境管理入口也已移除，复用该发布流程。

- 合并后的源码：5 个相关前端测试文件、73 项测试通过；类型检查、SPA 构建与验收范围 4 项通过。
- 四个 Ontology 页面及相关页内标签、主要弹窗，在 1920、1280、768、390 像素均通过，无页面横向溢出；1920 像素截图已补齐。
- 正式包 `rel-3e870567ca1d67d0d27ecab4c8550f64a9c2aca9` 的独立 Server 验收通过：四个页面、属性与 Action 配置、实体与关系创建、草稿保护、图谱控制、切面保存与观测恢复、旧浏览器环境选择、实际只读 NFS。验收脚本关闭提示框时改为点击画布空白处，避免继续依赖已移除的环境文字；失败记录保留，功能断言不变。
- 相关发布脚本检查通过，详见 `python-merge-results.json`。上述结果替代前述工作区检查的未完成状态，不代表全仓测试通过。
- 本次证据目录：`/data00/workspace/artifacts/opencoder-ontology-web-release-20261010-irximx_z`；线上生效结果以该目录的 `delivery-result.json` 为准。

## 线上生效

- 本次 Ontology 修改已随统一发布 `rel-3e870567ca1d67d0d27ecab4c8550f64a9c2aca9` 合入原线上服务，沿用原入口。发布包校验通过；线上前端文件与构建产物一致，实际 Server 版本与发布记录一致。
- 原入口桌面与移动端检查通过：环境选择及环境管理入口均不存在；四个页面的 18 次 Ontology 请求全部返回 200，均读取当前 Server 的默认数据空间，无浏览器异常或页面溢出。
- 切换前后对照，已有 1 个实体类型、1 个实体、6 个关系类型的标识全部保留；图谱切面仍为 0 个。检查只读取生产数据。
- 完成 900.18 秒线上观察，89 次采样无失败；Server、Host、Runtime 均保持 active。
- 回执：上述证据目录中的 `delivery-result.json`、`public-ontology-result.json`、`ontology-production-before.json`、`ontology-production-after.json`；线上截图为 `public-ontology-1280.png` 与 `public-ontology-390.png`。
