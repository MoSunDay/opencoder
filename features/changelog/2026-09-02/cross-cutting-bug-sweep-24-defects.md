Commit: (working-tree, 近两日大变更全面诊断——24 项缺陷扫除 + 全量门禁 3908/0)

# 近两日变更全面缺陷扫除（TUI / shellguard / node / session，24 项）

## 背景

近两日落库 35+ commits（shellguard 新 crate、skill 正文瞬时化、plan/act
模式切换、slash 零回显、sidecar actor、TUI 键位改造等），用户要求全面诊断
扫除 bugs。三路独立只读审查（session 核心 / TUI / shellguard+node+store）
+ 主会话逐点抽查确证后，并行三路修复。所有发现先核实再修，无凭空项。

## 修复清单

### critical（shellguard）
- `find -delete` 早退短路：命中释放集即 return Allow，不再扫描同命令后续
  `-exec/-ok/-f*` 输出——`find /tmp -delete -exec rm -rf / ;` 被放行。改为
  全 action 归并（most_restrictive）后才判定（handlers/find.rs）。

### major（shellguard 逃逸面 ×8）
- find `-fprint/-fprint0/-fprintf/-fls` 只查第一个输出旗标 → 收集全部逐一过释放集。
- `sed -f/--file` 脚本文件内容不扫描（对照 perl/ruby 同源扫描）→ 读入走
  check_sed_expression，读不到 Ask；附带 GNU sed `10w/tmp/x` 紧连写漏检。
- SIMPLE_SAFE 盲 Allow：hyperfine（参数串交 shell 执行）移出并加专用
  handler；shuf/iconv `-o` 写路径走 redirect 检查。
- 值旗标提取只认空格分隔+首个出现（args.rs get_flag_value）→ 新增
  collect_flag_values，curl `-o/--output` 全拼写全出现、docker
  save/load `--output/--input`、gh api `--method=/--input=` 全收口。
- git archive/format-patch 短旗标粘连 `-oPATH` 绕过 → flag_path_value 补粘连形式。
- `ip netns/vrf exec <cmd>` 任意命令执行 → Recurse 内层（越界 fail-closed Ask）。
- sqlite3 只分类第一条 SQL → 全 positional 逐条 least_safe 归并。
- `git branch --edit-description` 拉 $GIT_EDITOR → Ask。

### major（TUI 键位交互 ×3）
- BackTab arm 无焦点守卫：聚焦子代理/侧车时 Shift+Tab 照常 arm 父会话
  clear 守卫，后续 Enter 把子代理 steer 文本 merge_typed 进
  `/act_clear_context` 复合尾部执行破坏性 clear → 焦点命中即不 arm。
- 修饰键过滤缺失：retire 掉的 Ctrl+Shift+Tab（BackTab+CONTROL|SHIFT 报法）
  双按从无害模式切换变成 arm→Fire → intercept 两个 Fire 臂与 arm 路径统一
  排除 CONTROL|ALT|SUPER（不动 SHIFT；用户在途的 BackTab Fire 行为保留）。
- Shift+Tab 拼写不对称：intercept 已收 `(Tab, SHIFT)` 拼写而 arm 路径只认
  BackTab——该类终端上 plan 模式 Shift+Tab 落进普通 Tab 分支**直接提交草稿**
  → 提取 shift_tab_action 共用入口，两种拼写同路。

### major（node）
- 事件上传被推迟到 run 结束：`uploader` 在 run_with_cancel 与 flusher 之后
  才消费 batch_rx——任务执行期 server 侧 SSE 全程空白、事件无上界堆积。改
  run 前 spawn 专职 uploader，run 结束 drop batch_tx 后 join，上传收尾与
  本地 flush 均先于终态上报，批序不变。
- idle claim 无短超时：`claim_next` 只受控制面 `READ_TIMEOUT=120s` 约束，
  而 idle runner 的 heartbeat 与 claim 同处一个 `select!` 循环——server
  接受请求后挂起时，单次 wedged claim 可让存活节点静默最长 ~2 分钟，远超
  `STALE_AFTER_MS=20s`，节点被误判 lost。新增 `CLAIM_TIMEOUT=5s`（与
  `HEARTBEAT_TIMEOUT` 同族预算），claim 全程套超时；最坏静默间隙维持
  `timeout + tick ≈ 10s < 20s` 的既有算术。测试注入毫秒级预算证明
  timeout-then-recovery（tests/claim_budget.rs ×2，stub 增 claim 挂起注入）。

### minor（×10）
- sidecar actor 的 conv 在 TranscriptReset 重建视图后悬空：追问的
  Child/Turn 帧因块 id 未知被静默吞弃（界面永远看不到问答）→ ask 通道改
  `SidecarAsk{Question,Reset}` enum，rebuild 后发 Reset 重开快照。
- skill_context body_and_pointer 对无 `\n\n` 的 `> Source: p` prompt 整段
  当 body → fallback 改 `(None, Some(paths[0]))` 走 pointer 路径。
- compaction 空折叠循环：compact() Ok(None)（无可折叠）后每轮重试 + warn
  → run 内 compaction_unproductive 标记短路（hard-limit gate 不变）。
- /ap 弹窗打开时括号粘贴穿透到背后 composer → route_paste 补 ap_menu swallow。
- SidecarTurn 终帧 try_send 可被甩 → 块永远转圈 → 终帧缓冲后 send().await。
- idle Fire 丢 pending_images 且不清理（残留图片搭下一次提交）→ 与
  app_submit 对齐 snapshot+clear。
- smoke watchdog kill_tree 组杀依赖进程组但 spawn 未设 → `.process_group(0)`。
- e2e 测试落库真实 ~/.local/share → spawn_server 钉 TMP+XDG_DATA_HOME
  （scratch 跨 restart-resume 共享）。
- shellguard analyzer 的 cd 目标解析与 handler 双实现漂移（不跳 `-P/-L` 等
  旗标，cwd re-aim 偏差）→ 委托 resolve_target 单一实现。
- mkdir 仅逻辑路径判定，未做 scope.rs 要求的 canonicalize symlink 复查 →
  双过 is_within_safe_dir。

## 有意保留（已评估非缺陷或低价值改动）
- armed 5s 窗口内 Ctrl+T/退出等全局键 inert：Esc 回撤 + Fire/Esc 契约已
  覆盖主路径，放行 switch_mode 引入的新分发序风险大于收益，记 backlog。
- arm 时 `$skill` 已持久化、Esc 只还原文本：resolve 顺序改造风险大，
  记 backlog。
- auth_sig 同毫秒同请求伪 409：无 nonce 协议固有权衡，React 双发缓解在位。
- `uv sync`、`git fetch` 等写 cwd/.git 的 Allow 与「释放集仅 /tmp」叙事的
  策略级张力：rippy 上游遗留，需单独拍板，不在本轮擅改。
- `estimated_tokens` 每轮经 `skills_dir` 现读 skill 目录（每 turn 一次
  文件 IO 放大）：量级小且 gate 语义要求反映瞬时目录，缓存会引入失效一致
  性问题，收益不抵风险，记 backlog（性能项，非正确性缺陷）。

## 门禁读数（rules/02）

- 全量回归：`cargo test --workspace` → **3908 passed / 0 failed**（249
  suites；基线 3856/0 + 本轮新增测试 ~52）
- `cargo clippy --workspace --all-targets -- -D warnings` → 0 warning
- 分组：tui 1648/0；shellguard 407/0（新增 33）；session bash_guard 兼容
  corpus 40/0、compaction 22/0、skill_context 17/0；node 27/0（新增
  executor_streams_midrun 1、claim_budget 2）；running_mode_switch_e2e 2/0；nodes_smoke 2/0
- 行数门禁：无文件超 800 行，无新增超 400 行文件
- 本条目测试清单：find 短路/多输出 ×7、sed script/w 粘连 ×3、
  hyperfine/shuf/iconv ×6、值旗标三拼写 ×5、git 粘连/editor ×3、ip exec ×2、
  sqlite3 ×1、cd 旗标 ×2、mkdir symlink ×1、TUI 焦点/修饰键/拼写 ×7、
  sidecar Reset ×2、终帧 ×1、粘贴 ×1、idle 图片 ×1、uploader 并发 ×1、
  compaction 短路 ×1、body fallback ×1、kill_tree ×1、claim 挂起超时/恢复 ×2、
  e2e 环境钉扎（改既有）
