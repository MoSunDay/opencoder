# Bash 进程注册测试使用主动放行

`bash_registers_while_running_unregisters_after` 先让子进程等待放行文件，确认运行中的 PID 已登记后再放行并验证退出后移除。去掉固定 0.5 秒等待，避免繁忙主机上耗尽原有 1 秒前台超时；产品超时和断言保持原样。

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 运行中的进程登记，正常退出后移除 | `bash_registers_while_running_unregisters_after` | `crates/session/src/tools/bash/tests/lifecycle.rs` |
