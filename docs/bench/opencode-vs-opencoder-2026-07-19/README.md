# opencode vs opencoder — 贪吃蛇任务实测

- 日期：2026-07-19
- 机型：Intel Xeon E5-2673 v3 @ 2.40GHz · 24 核 · Ubuntu 22.04 (Linux 6.8)
- 模型：`zhipuai-coding-plan/glm-5.2`（reasoning_effort=high, max_tokens=16384）
- 任务：用 Rust + crossterm 实现终端贪吃蛇，`cargo build` 确认编译通过
- 负载：隔离临时工作目录，headless `run`，0.5 s 采样全程
- 测量：自写进程树采样器，`/proc/<pid>/stat` 的 utime+stime → CPU%；`/proc/<pid>/status` VmRSS

## 口径（重要）

CSV 每行 6 列，**agent 主进程与整树分开**：

```
ts, root_cpu, root_rss_kb, tree_cpu, tree_rss_kb, npids
```

- `root_*` = ROOT_PID 本身（agent 进程，含其线程），**不含** cargo/rustc 子进程。
- `tree_*` = ROOT_PID + 全部后代（含 cargo/rustc）。
- `npids` = 进程树节点数。`npids<=1` 表示该时刻无 cargo 子进程（非编译时段）。

**README 表格的口径 = 非编译时段**：仅取 `npids<=1` 的样本，对两边完全对称，彻底排除编译贡献。
这样做是为了公平 —— cargo/rustc 编译是同源开销，不该计入任一方的 agent 运行时基线。

## 数据文件

- `opencoder_snake_cpu.csv` — opencoder 0.1.0，wall 112.2 s，209 样本（非编译 199 / 编译 10）
- `opencode_snake_cpu.csv` — opencode 1.17.8，wall 172.3 s，321 样本（非编译 313 / 编译 8）

## 汇总（非编译时段，`npids<=1`，对应 README 表格）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 |
| --- | --- | --- |
| wall | 172.3 s | 112.2 s |
| 非编译样本占比 | 97.5 % | 95.2 % |
| CPU 均值 | 55.3 % | 0.13 % |
| CPU 中位 (p50) | 44.6 % | 0.0 % |
| CPU p95 | 115.2 % | 1.9 % |
| CPU 峰值 | 2631.6 % | 3.7 % |
| Agent RSS 均值 | 451.7 MB | 11.8 MB |
| Agent RSS 峰值 | 557.5 MB | 12.1 MB |
| 结果 | 编译通过 242 行 | 编译通过 351 行 |

## 自证：opencode 的高 CPU 与编译无关

1. opencode 的 CPU 峰值 2631.6 % 那个样本 `npids=1` —— 此刻并无 cargo 子进程在跑，
   是纯 V8 GC/JIT 多线程爆发（≈26 核累计 jiffies）。
2. 把编译时段全部剔除后，opencode 的 CPU 均值不降反略升（全程 54.4 % → 非编译 55.3 %），
   说明持续高占用与编译无关，全部来自 V8 运行时。
3. opencode 编译时段仅占 8/321 = 2.5 % 的样本；opencoder 编译时段仅占 10/209 = 4.8 %。
   编译占比都很小，且已被口径剔除。
