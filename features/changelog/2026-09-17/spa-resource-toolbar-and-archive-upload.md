Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Agent 配置工具行整理与「上传压缩包」覆盖导入

Agent 配置抽屉的两处 UI 收敛：

1. Prompt 页签工具行重排：版本文本、保存按钮、「预览」开关收进同一个 `Space`（wrap），保存与预览开关之间由 antd Space 统一 8px 间距，修复原先预览行作为兄弟 inline-flex 元素挤到保存按钮旁边零间距的问题。
2. Skills / Memory / Tools 三个文件页签的结构操作全部收敛到目录树右键菜单（新增文件 / 新增目录 / 重命名 / 删除），删除顶部一排工具按钮（新增文件 / 上传文件 / 上传技能目录 / 上传替换 / 重命名 / 移除）；保存按钮右侧新增「上传压缩包」，只接受 .zip，对话框明示覆盖语义：同名文件直接覆盖（只覆盖文件、不动目录），目录中原有其他文件全部保留，确认后合并进草稿，仍由「保存」统一走 files-only PUT，后端 `merge_files` 契约不变。

## 变更

- `crates/web/spa/src/agents/archiveUpload.jsx`（新增）：`ArchiveUpload` 组件——antd Upload（accept=".zip"，单个压缩包上限 8 MiB）→ `unzipArchive` 解包 → `mergeArchive` 合并 → Modal 展示新增/覆盖同名计数、覆盖语义说明、路径预览列表与被忽略条目数；`beforeUpload` 恒 resolve false，阻断 antd 在无 `action` 时的默认后台请求（上传/解包/合并全部组件自管）；错误经 `onError` 冒泡为工具行下方可关闭 Alert。
- `crates/web/spa/src/agents/resourceModel.js`：新增 `bytesToB64` / `unzipArchive`（跳过 zip 目录标记条目、拒绝隐藏段与 `__MACOSX`/`._` 元数据、>64 段拒绝）/ `mergeArchive`（putFile replace 覆盖同名文件并沿用其权限位，新文件 0o600；skills 校验合并集含 SKILL.md；总量 ≤1.5 MiB / ≤4096 文件，与后端 `merge_files` 同规）。
- `crates/web/spa/src/agents/resourceTab.jsx`：单一工具行 Space 依次为版本文本、保存按钮、ArchiveUpload（仅非 prompts 且可写）、未保存/已保存状态、「预览」+ Switch；prompts 卡片仅预览开关开启时渲染 Markdown 视图。
- `crates/web/spa/src/agents/resourceFiles.jsx`：移除全部工具栏结构按钮，结构操作只走 FileWorkspace 树右键（创建/重命名内联草稿、删除确认弹窗）；空态与二进制占位文案改为指向树右键与「上传压缩包」。
- `crates/web/spa/src/agents/useResources.js` / `crates/web/spa/src/agentDetail.jsx`：随单文件上传一起删除失效的 `working`/`uploading` 状态链路（`ResourceTab` 不再接收 `onWorking`）。
- `crates/web/spa/package.json`：新增 `fflate@^0.8.3`（零依赖 zip 解包，≈8 KiB gzip）。
- `crates/web/spa/dist/`：`npm run build` 产物刷新，`scripts/check-spa-drift.sh` 无漂移。

## 边界

- 覆盖上传只作用于前端草稿：后端仍只接收变更文件 + removed，未提及文件原样保留，无需后端改动。
- zip 内目录标记条目静默丢弃不计入忽略数；隐藏文件与系统元数据条目计入「已忽略」。
- prompts 页签不提供上传入口；只读（内置）Agent 工具行不含保存与上传。

## 验证

- `crates/web/spa` vitest 全量：111 文件 / 811 用例通过；其中新增 `src/agents/archiveUpload.dom.test.jsx` 5 例（同名覆盖且保留其余文件、取消不合并、非 zip 与缺 SKILL.md 拒绝、二进制字节往返与权限位沿用、beforeUpload 恒 false 无任何后台请求），`resourceModel.test.js` 增 2 例（mergeArchive 覆盖语义、unzipArchive 元数据跳过），`resourceFiles.dom.test.jsx` 改写为「结构操作只留树右键」，`agentDetail.dom.test.jsx` 三例改为树右键新增/删除与压缩包导入保留可执行位。
- 浏览器实测（fixture 页 + Playwright chromium）：Prompt 工具行保存右缘→预览标签左缘 8px、同行对齐；Skills/Memory/Tools 上传按钮与保存同行 8px 间距；zip 导入对话框计数/忽略数正确，确认后目录树新增 `extra/note.md`、保留 `topics/rust.md`、覆盖 `memory.md`（18 字节 · 权限 600）并出现「未保存」；Prompt 预览开关开启后 3 张 Markdown 卡片替换输入框。

## 发布：rel-9e66930a 本机平滑上线（同日 10:44）

按「新逻辑生效」要求，将 HEAD（`9e66930a`，含本特性与 review 两处一行修复）构建为发布包并本机平滑发布，无停机：

- 构建前置：`crates/web/spa` vitest 全量 811 passed；干净 worktree 上 `scripts/platform/release/build.sh --output /srv/releases/opencoder-9e66930a` 通过 SPA 漂移检查与编译期 SPA 摘要核验（manifest `spa_sha256 5ce3978fef8a557c03707b7e28d3f80e1119a1ac9d21c0c524750e2e4e75de24`，协议 10）；同树 `cargo test --workspace` 全绿。
- 发布：`scripts/platform/deploy.sh --bundle /srv/releases/opencoder-9e66930a --wait-seconds 300`（普通平滑通道，候选预热 + WASM 探针 + nginx reload），phase complete，current 切至 `rel-9e66930a59e7154bd6fa2548265071ba46a7c9d6`，旧 rel-653c7162 / rel-2dc1323d / rel-a51016ca 按既有执行进入退役回收。
- 线上核验：`opencoder-server 0.1.0 (9e66930a) listening on http://127.0.0.1:3048`；公共入口（18081）`/static/app.js` 含「上传压缩包」逻辑且与提交 dist 字节一致（2,565,079 字节，HTTP 200），server / runtime / host 三件套 systemd unit active。
