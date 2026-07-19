# opencode vs opencoder — 贪吃蛇任务实测

- 日期：2026-07-19
- 机型：Intel Xeon E5-2673 v3 @ 2.40GHz · 24 核 · Ubuntu 22.04 (Linux 6.8)
- 模型：`zhipuai-coding-plan/glm-5.2`（reasoning_effort=high, max_tokens=16384）
- 任务：用 Rust + crossterm 实现终端贪吃蛇，`cargo build` 确认编译通过
- 负载：隔离临时工作目录，headless `run`，0.5 s 采样全程进程树
- 测量：`/usr/bin/time -v` + 自写进程树采样器（`/proc/<pid>/stat` 的 utime+stime → CPU%；`/proc/<pid>/status` VmRSS）

## 数据文件

- `opencoder_snake_cpu.csv` — opencoder 0.1.0，79.0 s，141 样本
- `opencode_snake_cpu.csv` — opencode 1.17.8，125.7 s，228 样本

字段：`ts,cpu_pct,rss_kb,npids`。`npids` 为进程树节点数（含 cargo/rustc 子进程）。
口径：Agent 进程 RSS = 仅取 `npids<=2` 的样本（排除 cargo 编译子进程）。

## 汇总（对应 README 表格）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 |
| --- | --- | --- |
| wall | 125.7 s | 79.0 s |
| CPU 全程均值 | 65.6 % | ~0 % |
| CPU 活跃期均值 | 71.4 % | 63.6 % |
| CPU 峰值 | 1954.7 % | 229.3 % |
| CPU p95 | 164.7 % | 100.1 % |
| Agent RSS 均值 | 496.5 MB | 13.7 MB |
| Agent RSS 峰值 | 656.6 MB | 14.0 MB |
| 整树 RSS 峰值 | 1017.9 MB | 635.8 MB |
| 结果 | 编译通过 302 行 | 编译通过 295 行 |
