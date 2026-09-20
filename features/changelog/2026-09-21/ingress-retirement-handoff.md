# 保护切换前已被入口接收的请求

Nginx reload 后，新连接读到新版本并不能证明旧 worker 已排空。旧 worker 可以先接收部分请求头，等旧 Server 退场后再连接旧端口，产生 502。隔离验收已用真实 Nginx 在旧版二进制上稳定复现这一边界。

发布控制器在切换前持久记录 Nginx worker 的 PID 和启动时间。退场协议 2 将这组进程身份与新 Server 端口交给旧 Server：旧 Server 将后续普通请求转交给新实例，完成已开始的本地请求，再迁移 Node 通道；直到原 worker 全部退出才关闭监听。PID 重用不会延长等待。读取进程身份失败会保留监听并记录错误。

请求转交保留原调用者凭证、路径查询参数、响应状态、重复响应头及流式 body/trailers；禁止自动重试、重定向和解压。原调用者在新实例重新鉴权。非法退场参数返回 400，不能意外触发无保护退场。

重复发布和回滚保留每个旧 Server 实例原有的退场记录。控制器恢复时沿用切换前记录，且在发送退场请求前先落盘。旧协议 Server 在入口仍拥有请求时继续服务。冻结控制器递归打包新增 Python 子包，并持久化目录项。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 真实入口先收部分请求头，切换后才连后端 | `accepted-ingress-request-before-reload` | `scripts/acceptance/smooth_release/main.py` |
| Node 正常关闭并释放旧 worker | `node-channel-releases-ingress-worker` | `scripts/acceptance/smooth_release/main.py` |
| 原进程身份、PID 重用、畸形记录 | `process_identity_requires_the_original_live_process` | `crates/control/src/release/ingress.rs` |
| 保留原身份、查询、流式帧和 trailers | `relay_preserves_caller_query_stream_frames_and_trailers` | `crates/control/src/release/relay.rs` |
| 不跟随重定向、不解压响应 | `relay_returns_redirect_and_encoded_body_without_following_or_decoding` | `crates/control/src/release/relay.rs` |
| 实际 Server 转交新请求、拒绝非法退场、入口排空后退出 | `retiring_server_accepts_late_ingress_connections_until_workers_exit` | `crates/control/tests/release_handoff.rs` |
| SSE 恢复游标且接收保持开放 | `retirement_interrupts_a_slow_sse_poll_and_preserves_cursor_and_admission` | `crates/control/tests/release_handoff.rs` |
| 切换恢复保留原入口记录 | `test_interrupted_switch_reuses_the_recorded_ingress_frontier` | `scripts/platform/rolling_tests/test_deployment.py` |
| 旧协议等候入口排空 | `test_legacy_retirement_waits_for_its_original_frontier_on_later_passes` | `scripts/platform/rolling_tests/test_deployment.py` |

定向 Rust 测试已通过；真实 Nginx 总体验收、最终全量回归、Clippy、构建和生产发布结果仍待完成，不能据此文件认定已上线。
