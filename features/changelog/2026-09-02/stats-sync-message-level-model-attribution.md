Commit: 0834e85

# stats-sync 消息级 model 归属：glm-5.3-flash 被记入 unknown/glm-5.3 修复

## 背景

kaboo 1D 口径 glm-5.3-flash 仅 445.4M，与实际消耗明显偏少。服务端数字与
`~/.local/share/opencode/opencode.db` 本地一致，缺口在同步脚本归属逻辑：
`compute_session_payload` 用**会话级** `sessions.model` 给整段会话所有消息
打 `modelID`，而源库 `messages.model` 每条消息都有权威值。

## 三类错账（2026-09-02 日历日实测，750 个源库交叉表）

| 会话级 model（旧口径） | 消息级真实 model | 量 | 后果 |
|---|---|---|---|
| `NULL`（子代理会话） | glm-5.3-flash | 155.6M | 记成 `unknown` |
| `glm/glm-5.3` | glm-5.3-flash | 50.0M | 错记成 glm-5.3 |
| `cm-deepseek-flash/glm-5.3-flash` | qwen3.8-max | 24.7M | 反向多算给 flash |

## 修复

`parse_model(msg.get("model") or session.get("model"))`：消息级优先、会话级
兜底。会话行的 `model_json` 仍用会话级（描述会话主模型）。

验证：重置水位重刷当日会话后，opencode.db 日历日口径
glm-5.3-flash 408.3M → **604.6M**、unknown 175.8M → 24.6M；kaboo 18:30
tick 上传后服务端 1D glm-5.3-flash 445.4M → **668.3M**（滚动窗口含昨晚
旧数据，unknown 残余 211M 会随窗口滑出；后又把水位回拨 7 天重刷 432 会话
/38,438 消息，19:00 tick 起全窗口生效）。

## 运维事实

- 水位文件 `~/.local/share/opencode/.opencoder-sync.json` 的 `last_offset`
  可回拨强制重刷（写入幂等：enc_<id> 前缀 delete+reinsert）。
- kaboo 增量解析 opencode.db 依赖 mtime 变化，重刷后下一 tick 自动生效；
  单轮 report 全程 ~20min（parse.codex 为长尾）。
