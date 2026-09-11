Commit: 0bc5b867

# 会话页布局协调与收起菜单钉底（含 0bc5b867 平台发布验证）

## Context

fleet 控制台侧边栏的“收起菜单”按钮原先位于菜单顶部，与分类切换、菜单项挤在一起；会话（chat）页内部卡片圆角（8px）与全局 Card token（borderRadiusLG=10）不一致，侧栏与主列之间无间隔，工具条/Alert 的 8px 间距与其余 12px 节奏不统一，整体观感不协调。

## Change Summary

- `crates/web/spa/src/main.jsx`：`fleet-nav-toggle` 按钮移至 Sider 末尾（Menu 之后），props 与无障碍属性不变。
- `crates/web/spa/src/app.css`：Sider children 改 flex 列布局，Menu `flex:1` 可滚动，按钮 `margin:auto 8px 8px` 钉底并加 `border-top` 发丝分隔；新增 collapsed（64px）状态防溢出规则。
- `crates/web/spa/src/chat.jsx`：行容器加 `gap:16`；工具条与 Alert `marginBottom` 8→12；transcript 卡片 `borderRadius` 8→10、padding `12px 16px`。
- `crates/web/spa/src/chatSidebar.jsx`：`paddingRight` 12→16 与容器 gap 对称。
- `crates/web/spa/src/queuePanel.jsx`：卡片 `borderRadius` 8→10、padding `8px 12px`。
- 发布：改动随 commit `0bc5b867` 进入发布包 `dist/opencoder-platform-0bc5b867`（manifest/SHA256SUMS 校验通过，`protocol_version: 9`），经 `scripts/platform/install_bundle.py` 激活到 `/usr/local/bin`，systemd 滚动重启 server→agent。

## Validation

- `npm test`（crates/web/spa）：80 文件 / 652 passed。
- `npm run build` + `scripts/check-spa-drift.sh`：无漂移。
- 发布后验证：`GET /api/health` → `0.1.0 (0bc5b867)` / protocol 9；served `/static/app.css` 含 `margin:auto 8px 8px`、`/static/app.js` 含 `borderRadius:10`；节点 human-os-02 online/idle。
- `cargo test --workspace`（发布轮全量回归，同一工作树 @ 0bc5b867）：375 个测试套件，5051 passed / 0 failed。

## Related Docs

- [web 模块](../../../agents/web/index.md)
