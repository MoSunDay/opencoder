Commit: 6bf573e09693caf9a00e37429f9a8f2d796dc219

# Ontology 内容区移除重复标题

图谱、实体、实体类型、关系类型、环境管理五个页面不再显示内容区域顶部的页面名称和说明，环境选择器保留。页面名称由导航标签提供。

相关规则见 [Ontology](../../ontology/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 五个页面无重复标题和说明，保留环境操作及正文 | `%s omits the title and description while keeping environment controls` | `crates/web/spa/src/shell/pageShell.dom.test.jsx` |
| 浏览器验证五个页面无标题、环境操作与新增和图谱交互可用 | `headerless`、`checkGraph` | `scripts/acceptance/ontology/browser.mjs`、`scripts/acceptance/ontology/experience.mjs` |
| 真实 Ontology 页面无重复标题 | `ontology* keeps exactly one title source` | `crates/web/spa/src/shell/headerContract.dom.test.jsx` |

- 导航及 PageShell 定向回归：37 passed。
- 真实 Ontology 页面标题检查：5 passed。
- Ontology TypeScript 类型检查和 SPA 构建通过。
- Clippy 全量检查通过。
- SPA 全量回归：156 个测试文件、1114 项测试全部通过。
