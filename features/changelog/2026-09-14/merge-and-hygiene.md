Commit: 39089267

# 合并与卫生收尾：brain plan flows 入 main（08e5f006 + 6fd9e78e）+ rustfmt 清扫 + dist 重建

覆盖 2026-09-12 ~ 09-14 的非功能落地：两次合并把特性线并入 main，外加纯格式清扫与发布产物重建，最后把 brain/control 记忆锚定到合并基线。

## 合并裁决

- **08e5f006**（merge `104c6b26` + `2280fb3c`）：特性线 brain plan flows 与 live execution logs（104c6b26，208 文件 +6362/−4797：本体计划编辑、条件动作执行、DAG step records、TODO 运行画布、TODO env 收敛与 node ENV 配置移除）合入 main@2280fb3c（该侧 playbook 双轨、成员即 agent、版本化 wasm 模块池及评审 fast-follow 已先行落库）。合并时重建 SPA、调和集成 fixture 与 Rust 格式（对第一父净入 158 文件）。
- **6fd9e78e**（merge `08e5f006` + `5fc3b5ca`）：树与 08e5f006 完全一致（`git diff 08e5f006 6fd9e78e` 为空）——格式改动已包含在合并结果里，本合并仅统一历史，保留同时含两支的重建 SPA 产物。
- **39089267**：agents/brain、agents/control 记忆戳从 `104c6b26 + 2280fb3c (merge working-tree)` 锚定为 6fd9e78e。

## 卫生提交

- **5fc3b5ca**（style(store)）：libsql_store rustfmt 清扫，19 文件 +35/−48——import 统一为小写在前（`params, Connection, Row`），过宽解构/断言/宏调用压缩单行。纯格式零语义，提交注明 `cargo fmt --check` 通过。
- **a46fff84**（chore(spa)）：为 release 2280fb3c 重建 dist——仅 `crates/web/spa/dist/static/app.js` 变更（压缩单行产物，1,822,683 → 1,822,749 B），dist 其余三文件（index.html / app.css / download-sw.js）不动。该提交未附带测试证据；其产物随后被 08e5f006 合并轮的重建（app.js/app.css 均重写）取代，验证留待发布轮记录。

## Impact Surface

- 非功能轮：无 API/协议形状变更；rustfmt 零语义、a46fff84 零源码变更。
- 记忆回填：2026-09-11 的能力页签直连 / 模板全宽抽屉 / 运行画布三 SPA 条目随 104c6b26 落库，working-tree 戳同轮回填。

## Related Docs

- [agents/brain](../../../agents/brain/index.md)、[agents/control](../../../agents/control/index.md)（锚定 6fd9e78e）
- [TODO 运行画布](../2026-09-11/todo-run-canvas.md)、[模板全宽抽屉](../2026-09-11/todo-template-drawer.md)、[能力页签直连](../2026-09-11/brain-capability-tab-headerless.md)
