Commit: 1ff961846496dd24f0a32f7e5106bbee0e234a5a

# Team 启动超时后的自动确认

启动请求等待节点准备资源时，原表单一直处于提交中；HTTP 超时后，即使服务端后来完成受理，页面也不会自动进入讨论过程。

启动即显示执行编号。提交等待超过 15 秒、网络断开或返回不确定结果后，页面解除提交锁定，转为自动查询已有受理回执；确认成功后打开原执行，明确拒绝则恢复编辑并显示原因。等待期间可以关闭或查看历史，重试保留原请求和编号。回执读取也有等待上限，关闭和切换时取消旧读取，迟到响应不会重复打开详情。

实现分为 [传输与回执校验](../../../crates/web/spa/src/fleet/teams/submission.js)、[提交状态](../../../crates/web/spa/src/fleet/teams/useSubmission.js) 和 [启动表单](../../../crates/web/spa/src/fleet/teams/launch.jsx)，复用已有受理 API。此次修复不改变节点准备资源的耗时或执行语义。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 提交超时后自动确认，迟到响应不重复打开 | `leaves a hung POST after 15 seconds and automatically opens the matching accepted receipt without another POST` | `crates/web/spa/src/fleet/teams/submission.dom.test.jsx` |
| 关闭、恢复原输入与同编号重试 | `keeps an uncertain request and its input when closed and resumed, and aborts stale reads` | 同上 |
| 回执暂缺或读取失败后继续确认 | `keeps polling after a missing or failed receipt and never treats either as success` | 同上 |
| 明确拒绝后允许修改并再次提交 | `releases a definitively rejected ID so a corrected request can be submitted` | 同上 |
| 回执读取超时及卸载取消 | `bounds a hung receipt read and cancels all work when unmounted` | 同上 |
| 拒绝执行身份不匹配的回执 | `rejects an accepted receipt for a different ID or execution kind` | 同上 |
| 界面反馈、原输入和重试入口 | `shows an uncertain submission inside the drawer and retries the same ID with the draft intact` | `crates/web/spa/src/fleet/teams/teams.dom.test.jsx` |

验证：相关 47 项测试、SPA 全量 156 个文件共 1113 项测试、前端及 Rust 工作区构建、格式检查通过。浏览器使用真实提交和回执 API，验证慢提交转自动确认、仅创建一次执行、自动打开过程、历史重开以及四种屏宽。Rust 全量测试和 Clippy 在编译阶段超过检查时限，发布脚本回归也超时，均未计为通过。
