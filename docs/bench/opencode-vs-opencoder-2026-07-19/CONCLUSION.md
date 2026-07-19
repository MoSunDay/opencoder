# 对比结论：opencode vs opencoder

> 日期 2026-07-19 · 同机实测 · 口径完全对称 · 原始数据见同目录 `*.csv`

## 测试条件

| 项 | 值 |
| --- | --- |
| 机型 | Intel Xeon E5-2673 v3 @ 2.40GHz · 24 核 |
| 系统 | Ubuntu 22.04（Linux 6.8） |
| 对比对象 | opencode `1.17.8`（Node SEA）↔ opencoder `0.1.0`（Rust 原生） |
| 模型 | `zhipuai-coding-plan/glm-5.2`（reasoning_effort=high, max_tokens=16384） |
| 对照任务 | 用 Rust + crossterm 实现终端贪吃蛇 + `cargo build` 验证（**两端均编译通过**） |
| 采样 | 0.5 s 全程，`/proc/<pid>/stat`（utime+stime）+ `/proc/<pid>/status`（VmRSS） |

## 核心结论

四个维度，opencoder 在运行时基线上全面占优；任务能力等价（同一模型下产出等价结果）。

### 1. 冷启动基线（`--help`，反映运行时本身开销，排除 LLM）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 | 倍数 |
| --- | --- | --- | --- |
| 二进制大小 | 159 MiB | 11.1 MiB | **14.3×** |
| 冷启动峰值 RSS | ~197 MiB | ~5.4 MiB | **35×** |
| 冷启动耗时 | ~0.78 s | ~6 ms | **~125×** |

### 2. 任务期 CPU（贪吃蛇全程，**仅非编译时段 `npids<=1`，对两边对称**）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 | 倍数 |
| --- | --- | --- | --- |
| 平均利用率 | **55.3 %**（持续高占用） | **0.13 %**（事件驱动，等 LLM 时空闲） | **~425×** |
| 中位 p50 | 44.6 %（一半时间 ≥44 %） | 0.0 %（一半时间完全 0） | — |
| p95 | 115.2 % | 1.9 % | **~61×** |
| 峰值 | 2631.6 % | 3.7 % | — |

**关键自证**：opencode 的 CPU 峰值 2631.6 % 落在 `npids=1` 样本上（此刻无 cargo 子进程），是纯 V8 GC/JIT 多线程爆发；剔除编译时段后 opencode 均值 54.4 % → 55.3 % 不降反升。**其持续高 CPU 与编译无关，全部来自 V8 运行时。**

### 3. 任务期 Agent 进程内存（同口径，非编译时段）

| 指标 | opencode 1.17.8 | opencoder 0.1.0 | 倍数 |
| --- | --- | --- | --- |
| RSS 均值 | 451.7 MB | 11.8 MB | **~38×** |
| RSS 峰值 | 557.5 MB | 12.1 MB | **~47×** |

### 4. 存储引擎

**两者底层都是 SQLite（WAL），落盘均为 `.db`/`.db-wal`/`.db-shm`。差异在驱动层，不在引擎。**

| 维度 | opencode 1.17.8 | opencoder 0.1.0 |
| --- | --- | --- |
| 数据库 | SQLite（bun 内嵌） | libsql `0.9.30`（SQLite 兼容 C 绑定） |
| 访问层 | Drizzle ORM（JS，运行时迁移） | 手写 SQL + `Store` trait（无 ORM 转换层） |
| 核心表 | `session`/`message`/`step`/`agent`/`tool_use`/... | `sessions`/`messages`/`session_inputs`/`session_events`/`subagent_tasks` |
| WAL PRAGMA | `WAL`+`synchronous=NORMAL`+`busy_timeout=5000`+`cache_size=-64000` | `WAL`（并发模型等价） |

WAL 并发模型等价（多读一写、读不阻塞写）；存储吞吐差异来自驱动层（opencode 经 ORM+V8 跨语言，opencoder 直连 libsql）。

## 总结

- **任务能力等价**：同一模型下，两端都能独立完成贪吃蛇实现并通过 `cargo build`，产出代码量相当（242 行 / 351 行）。
- **运行时基线差异巨大**：opencode 自带 V8 运行时，常驻高 CPU（~55 %）/高内存（~450 MB）；opencoder 编译为原生异步运行时，等待 LLM 时 CPU≈0、内存 ~12 MB。
- **差异与编译无关**：已用对称口径（非编译时段）和数据自证排除 cargo 贡献。
- **适用场景**：对受约束环境（CI runner、容器、弱机）与多实例部署（按进程计 RSS 决定并发密度），opencoder 的低开销可被直接摊销。

## 口径与可信度

- 所有 CPU/RSS 数字均来自 **非编译时段** 样本（`npids<=1`，agent 主进程且无 cargo/rustc 子进程），对两端完全对称。
- **wall-time 受 LLM 采样随机性影响**，两轮方向相反，仅作参考、不宜当稳态结论；CPU 利用率与 RSS 是运行时基线的稳定差异。
- 原始 CSV（含 `root_*` / `tree_*` / `npids` 六列）与方法学见同目录 `README.md` 与 `opencode_snake_cpu.csv` / `opencoder_snake_cpu.csv`。
