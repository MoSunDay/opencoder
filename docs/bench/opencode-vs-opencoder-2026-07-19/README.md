# opencode vs opencoder — 贪吃蛇任务实测

- 日期：2026-07-19
- 机型：Intel Xeon E5-2673 v3 @ 2.40GHz · 24 核 · Ubuntu 22.04 (Linux 6.8)
- 模型：`zhipuai-coding-plan/glm-5.2`（reasoning_effort=high, max_tokens=16384）
- 任务：用 Rust + crossterm 实现终端贪吃蛇，`cargo build` 确认编译通过
- 负载：隔离临时工作目录，headless `run`，0.5 s 采样全程
- 测量：`/usr/bin/time -v` + 自写进程树采样器（`/proc/<pid>/stat` 的 utime+stime → CPU%；`/proc/<pid>/status` VmRSS）

## 口径（重要）

CSV 每行记录 6 列，**agent 主进程与整树分开**：

```
ts, root_cpu, root_rss_kb, tree_cpu, tree_rss_kb, npids
```

- `root_*` = ROOT_PID 本身（opencode/opencoder agent 进程，含其线程），**不含** cargo/rustc 编译子进程。README 表格用的是这组。
- `tree_*` = ROOT_PID + 全部后代（含 cargo/rustc）。
- `npids` = 进程树节点数。

## 数据文件

- `opencoder_snake_cpu.csv` — opencoder 0.1.0，112.2 s，209 样本
- `opencode_snake_cpu.csv` — opencode 1.17.8，172.3 s，321 样本

## 汇总（agent 主进程本身，对应 README 表格）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 |
| --- | --- | --- |
| wall | 172.3 s | 112.2 s |
| CPU 全程均值 | 54.43 % | 0.13 % |
| CPU 中位 (p50) | 40.9 % | 0.0 % |
| CPU p95 | 109.6 % | 1.9 % |
| CPU 峰值 | 2631.6 % | 3.7 % |
| Agent RSS 均值 | 451.6 MB | 11.8 MB |
| Agent RSS 峰值 | 557.5 MB | 12.1 MB |
| 结果 | 编译通过 242 行 | 编译通过 351 行 |

## 旁证：编译对 CPU 的贡献极小

opencode 的 root CPU 均值（54.43 %）≈ tree CPU 均值（55.11 %）—— 两者几乎相等，
说明 cargo 编译对 opencode 的 CPU 几乎没有额外贡献，持续高占用全部来自 V8 运行时。
opencoder 相反：root CPU 峰值仅 3.7 %，tree CPU 峰值 142.6 % 的尖峰才是 cargo 编译
贡献的；排除编译后 agent 在等待 provider 回包时 CPU 长期为 0。
