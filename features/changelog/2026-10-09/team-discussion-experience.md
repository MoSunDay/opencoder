Commit: 6bf573e09693caf9a00e37429f9a8f2d796dc219

# Team 对话体验

Team 执行明细依照队长规划、成员发言、小结、补充澄清和收尾的实际流程组织展示。上方集中呈现讨论目标与结论；讨论过程按轮次折叠，默认展开当前页最新轮次的最新讨论。成员发言和队长小结使用 Markdown，澄清只读取上一份小结中被追问成员的回答，并展示对应问题。

达到讨论或澄清上限、取消和错误有明确提示，已有总结标为阶段小结。大记录继续分段读取，读取错误可重试；切换记录取消旧请求，历史轮次分页，长文本在容器内滚动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 结论优先、最新轮次、澄清成员与首次讨论切换 | `shows the conclusion first and opens the latest clarification with only the requested member` | `crates/web/spa/src/fleet/detail/team/conversation.dom.test.jsx` |
| 非正常结束的阶段小结 | `does not describe %s as a completed discussion` | 同上 |
| 历史轮次与失败重试 | `paginates round history and retries failures without showing an empty history as success` | 同上 |
| 切换执行时取消旧请求 | `aborts an old record request and ignores its late response after switching execution` | 同上 |
| 大记录只自动读取一个窗口 | `only fetches one bounded window automatically for a large record` | 同上 |
| 上次澄清内容过长时仍能读取本次小结 | `keeps the current summary available when the previous clarification list is too large` | 同上 |
| 分段长度与格式校验 | `rejects malformed, oversized and inconsistent record windows` | `crates/web/spa/src/fleet/detail/team/model.test.js` |
| 执行明细接入成员发言和队长小结 | `renders readable team statements and captain summaries through bounded record requests` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |

验证：SPA 全量 154 个文件、1091 项测试通过，最新 Team 专项 16 项通过；成员表格、对话和超长轮次标题在四种屏宽下验证通过，前端构建通过。仓库级检查未全部通过：Rust 全量测试及 Clippy 在 180 秒内未完成编译，发布脚本回归在 120 秒内未完成；格式检查发现 `crates/worker/src/workloads/agent_runc.rs` 的既有格式差异。
