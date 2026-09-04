Commit: (working-tree, 待提交)

# SPA 传输去重误读 payload `seq`：live steer/queue 回显被当重放重复丢弃

## Context

saypairs 真机验收 c2 失败：steer 后回显气泡直到下一轮 done 快照重建才
出现（~10s），期间 `❯ Say(N steps)` running 标签无从观察。raw SSE 抓帧
（`/tmp/dump-steer-frames.js` 思路：先连流再发 steer）证明 daemon 在
steer POST 同一毫秒就广播了 `steer_consumed`——帧早到了 10 秒，丢失点
在 SPA 折叠层之外：`sse.js::parseBlock` 把 payload 内的 `data.seq` 当作
事件行 seq 提升到 `frame.seq`。

两个 seq 命名空间在线上共用一个字段名：

- SSE `id:` 行（api_events.rs，落库后才有）= 事件行 seq，重连游标 /
  applySeq 水位 / 传输层 tier-1 去重的唯一依据；
- `steer_consumed` / `queue_consumed` 的 `data.seq` = **session_inputs
  行 seq**（TUI 队列行按身份删除用），每会话从 1 重新计数。

后果：run 中途事件水位早已 > 输入 seq（如 5 > 2），`handleBlock` 的
`seq <= lastSeq` 判定把 live steer/queue 回显整帧当「重放重复」静默丢弃
（b1 因下一轮无工具 ~1s 即 done 而侥幸通过）；若侥幸未被传输层丢弃，
reduce.js applySeq 还会把水位**倒退**成输入 seq，污染 resync 游标。

## Change Summary

- `crates/web/spa/src/sse.js::parseBlock`：删除 `dataSeq` 回退，仅 `id:`
  行提升为 `frame.seq`；未落库的 live 帧保持 `seq: null`（不做去重、不
  碰水位）。payload 字段原样保留在 `frame.data` 上（TUI 侧身份消费不受
  影响，服务端协议零改动）。

## 测试

- 新增 `crates/web/spa/src/sse.test.js` 用例：水位 5 之后到达无 `id:`
  行、`data.seq=2` 的 live `steer_consumed` 必须正常投递（`frame.seq ===
  null`、payload 字段原样），后续带 `id: 6` 的帧不受影响。
- 真机验收 `scripts/browser-acceptance-saypairs.js` 连续两轮 9 步全绿
  （c1 echo 在 steer 后即时渲染、c2 可观察 ladder-B running 窗口），
  零 console 错误。
- 回归：`npm test`（spa）213 用例全绿；`cargo test -p opencoder-web
  -p opencoder-tui` 1966 通过 0 失败。
