# 交付清单 · live-region-steps-only-say-in-transcript

- **commit**:`3e609c7ef`(28 files,+736/−613);前置链 `753250dd2`(调用行
  阶梯一致)→ 本轮。功能口径见
  [live-region-steps-only-say-in-transcript.md](live-region-steps-only-say-in-transcript.md)。
- **TUI**:chat_live_turn.rs(steps-only live_data/render_lines)、
  render.rs:559(可见行定位)、render_live.rs(wrapped 保尾)、
  chat_flatten.rs/chat_headers.rs(open Say 即时入转录 + 计数镜像)、
  14 个测试文件适配 + 溢出可见性回归。
- **SPA**:stepsBlock.jsx(StreamingSay + TurnLiveRegion steps-only)、
  chat.jsx(滚动容器内布局 + 近底 auto-stick)、stepsBlock.dom.test.jsx。
- **文档**:本 changelog 目录条目与 supersede 链、agents/frontend/{tui,web}/
  index.md、repo-memory.md、dist/static/app.js(无 drift)。
- **门禁事实**(本轮执行记录):`cargo test -p opencoder-tui --lib` 1812/1812;
  `cargo clippy -p opencoder-tui --all-targets` 0 warning;
  `cargo fmt -p opencoder-tui --check` clean;SPA 定向
  `stepsBlock.dom + chat.dom` 44/44;`check-spa-drift.sh` no drift。
- **部署事实**(本轮执行记录):`cargo build --release`(rc=0)→
  `/usr/local/bin/{opencoder,opencoder-server}` cmp 一致,版本
  `0.1.0 (3e609c7ef)`;`oc-server` 重启后 active、HTTP 200、
  `/static/app.js` 与 dist 字节一致(APP_JS_FRESH)。
- **范围外遗留**(明确推迟):`theme.test.js` 1 例失败归 `21389c292` 的
  brain/workbench/history/style.css,与本链路无关,未处理。
