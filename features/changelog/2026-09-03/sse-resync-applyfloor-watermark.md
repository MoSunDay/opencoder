# SSE resync 水位重建：重连去重 + applySeq 水位（round-2 #5 收口）

## Context

bugfix brief round-2 #5（上一轮评审遗留 open）：live 广播帧在事件 flusher 异步落库**之前**发出，
永远不携带 `id:`/seq（`handle.rs::sse_from_session_event` + `event_sink.rs` 缓冲落盘）；而重连
replay（`api_events.rs`，`?after=`）回放的落库帧**全部带 seq**。后果：客户端 `lastSeq` 只会被
回放帧推进，一次断线/lag 重连后 `after=max(lastSeq, head-400)` 会把「上次连接里已 fold 过的全部
无 id live 帧」整段重放——脏状态上二次 fold：文本翻倍、孤儿 tool 行、echo 用户轮重推。旧「整体
resync 机制」删除后此缺口一直靠 done → 快照 reload 兜底收敛，run 中途 transcript 长时间错乱。
另一伴生缺口：run 在断线窗口内结束 → 终帧 seq ≤ head 永不回放 → 客户端 'streaming' 永挂、
busy 不释放。

## Change Summary

- `spa/src/sse.js`：
  - `parseBlock` 把 SSE `id:` 行（回放帧的持久化行 seq）提取到 `frame.seq`；不再就地改 `lastSeq`。
  - `handleBlock` 传输层去重（服务端 tier-1 判定的镜像）：`seq ≤ lastSeq` 的帧整帧丢弃（不算
    onFrame、不触发 lag/terminal 分支；仍计入存活探测）。`lastSeq` 只在此处单调推进。
  - `reconnectCursor` 新增 `onResync(lastSeq) → floor` 协议：调用方以快照重建 fold 态并返回
    水位，游标取 `max(lastSeq, floor)`；异常/非数值回退旧有界尾部游标（行为不变）。
- `spa/src/reduce.js`：
  - `emptyStream()` 新增 `applySeq` 水位（最高已 fold 的持久化 seq，null=尚无）；嵌套 child
    fold 的裸状态（undefined）不受影响。
  - `reduceFrame` 包一层守卫：带 seq 且 `≤ applySeq` 的帧原样丢弃（重放重叠/重建水位以下永
    不二次 fold）；fold 带 seq 帧即推进水位；无 seq live 帧恒 fold、不动水位。
  - 新增纯函数 `resyncState({messages, draining, headSeq, pendingEcho})`：按 `/seq` 先读、
    快照后读的次序重建——seq ≤ head 的落库事件其消息效果必在快照内且永不回放；replay 只载
    未来尾部。`draining=false` 重建直接落终态 `done`（终帧缺口收口：busy 释放而非永挂）；
    在途轮的未落库部分截断于水位，由 done → 快照 reload 收敛。
- `spa/src/chat.jsx`：`startStream` 接线 `onResync`——`/seq` head → `GET /api/sessions/:id`
  快照 → `resyncState` 重建（pendingEcho 经 `ensurePendingEcho` 重推保住在途用户锚）→ 返回
  head；失败返回 null 走旧游标。远端 node 任务流 session 404 时同路径优雅回退。

## Test list (rules/02)

- `spa/src/reduce.test.js`：applySeq 水位守卫/推进 + 裸状态不误删（×2）；`resyncState` 快照
  重建/echo 重推/终态收口/无水位缺省（×3）。
- `spa/src/sse.test.js`：id 行 seq 暴露 + `≤ lastSeq` 重复帧丢弃（存活计数不变）；lag 后
  `onResync` 水位驱动 `after=42` 且不再走旧 `/seq` 抓取；`onResync` 抛错回退有界尾部游标
  （`after=head-400`）（×3）。
- `spa/src/chat.dom.test.jsx`（router 增加 `seqHead` 注入）：lag → 快照重建（脏尾部被快照真
  相替换、`after=30` 重连、水位下 `id:12` 重复帧丢弃、live 尾部继续 fold）；断线窗口内 run
  结束 → `draining=false` 重建落 `done`、composer 解锁（×2）。
- `cargo test --workspace`：全绿（数字见提交说明）；`cargo clippy --workspace --all-targets`
  0 warning；`cargo fmt --check` 通过；`npm run build` 后 dist 无漂移外改动（仅 app.js）。
