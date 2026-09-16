Commit: eb6ca791cd8b766a4e51d8f91e26b9b1289775c7

# TODO 编辑器目录右键操作

## 交互

- 编辑页顶部只保留「返回」「保存」，移除父/执行 Agent 说明、任务操作工具栏、独立校验按钮和抽屉标题栏；环境绑定通过 `env.json` 修改。
- 在文件、目录及目录树空白处右键新增文件或目录，按目标类型改名、删除。对文件新增时创建同级条目；删除目录包含全部子项，重名和非法路径拒绝覆盖。
- `todos` 下新增目录自动生成完整任务文件；任务目录改名维护依赖引用，仍被其他任务依赖时禁止删除。未选中的右键目录也按自身路径操作。
- 保存先自动校验，再创建完整新版本。错误保留草稿并支持定位；空目录单独跟踪，不能在保存时静默丢失。加载期间可返回，保存期间禁止修改和重复提交；只读 Review 不显示修改菜单。

## 验证

- TODO 编辑器、目录模型、模板面板、运行与只读文件视图专项回归：8 个测试文件、77 项通过。
- SPA 全量回归：`npm test`，97 个测试文件、695 项通过；日志 `/tmp/opencoder-todo-editor-spa-full.log`。
- `cargo clippy --workspace --all-targets -- -D warnings` 通过、零警告；日志 `/tmp/opencoder-todo-editor-clippy.log`。
- Rust 全量回归：`cargo test --workspace`，5,238 项通过、0 失败，6 项既有忽略项保持不变；日志 `/tmp/opencoder-todo-editor-cargo-tests.log`。
- `cargo build --workspace` 通过；日志 `/tmp/opencoder-todo-editor-cargo-build.log`。
- `npm run build` 通过，提交追踪的 SPA 产物与当前源码同步。
- Chromium 使用构建后的完整 SPA 和隔离 API 回执完成右键增删改名、错误修正、保存顺序与 390px 窄屏检查；浏览器无运行时错误。验证与新版本写入依次调用，写入内容仅包含保留的任务目录。
- 浏览器结果：`/tmp/opencoder-todo-editor-browser/result.json`；专项日志：`/tmp/opencoder-todo-context-regression.log`。
- 合并候选 `eb6ca791` 已补齐真实 Server/Agent 接口验收：右键文件与目录增删改名、错误修正后自动校验保存、不可变版本及页面刷新回读全部通过；前端全量 712 项、Rust 全量 5,250 项通过。脚本 `scripts/acceptance/todo_workbench/editor.js::verifyEditor`，完整范围与回执见[执行记录收口验证](./todo-run-conversation.md#收口验证)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 工具栏精简 | `keeps only back and save in the page toolbar` | `crates/web/spa/src/todoEditor.dom.test.jsx` |
| 右键目录增删改名与保存 | `creates, renames and deletes task directories from the right-clicked row` | 同上 |
| 文件操作与同级文件保护 | `distinguishes file operations and keeps sibling files when a file is deleted` | 同上 |
| 空目录校验和脏状态 | `supports empty directories at the root and validates them on save without losing the draft` | 同上 |
| 加载与保存期间的交互 | `allows returning during loading and blocks duplicate saves and file operations while saving` | 同上 |
| 任务改名同步依赖 | `creates a task directory with complete files, then keeps dependencies in sync when renaming` | `crates/web/spa/src/todo/directory/operations.test.js` |
| 重名与路径保护 | `creates siblings when invoked on a file and never overwrites existing files or directories` | 同上 |
| 只读视图不提供修改菜单 | `read-only workspaces keep file selection and hide mutation menus` | `crates/web/spa/src/ui/files/editor.dom.test.jsx` |

相关：[TODO 功能](../../todos/index.md)、[Web 模块](../../../agents/web/index.md)、[操作说明](../../../docs/todo-workbench.md)。
