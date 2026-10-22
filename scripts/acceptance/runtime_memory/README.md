# Runtime 内存验收

在 Linux 下比较两个本地 `opencoder-agent` 程序。所有历史、配置和数据库写入指定的临时输出目录；Runtime 只监听本机，模型请求只进入脚本启动的本地服务。

```sh
python3 scripts/acceptance/runtime_memory/probe.py \
  --before /path/to/baseline/opencoder-agent \
  --after /path/to/candidate/opencoder-agent \
  --output /tmp/runtime-memory-comparison
```

默认分别生成 0、1 GiB、4 GiB 历史。每条记录包含冻结 runtime 和定义快照，读取真实 Runtime 的库存、上报与 Inspect 接口；连续 5 轮，每轮并发读取 256 条历史。输出 `results.json` 包含程序 SHA-256、历史字节数、启动峰值、各轮 RSS、查询耗时和上报耗时。

通过条件：修复版有历史时的最终 RSS 比空历史增加不超过 256 MiB，比对应基线至少下降 80%；最后两轮 RSS 相差不超过 32 MiB，查询没有超过 30 秒。

加入 `--tasks-per-round 16 --sizes 0` 可连续执行 5 批任务，每批 16 个。任务使用内置 Agent、本地固定回答和 1 MiB 冻结 runtime 数据；完成一轮对话后持久取消，再测量 RSS。该场景检查任务结束后的内存积累，最后两轮仍使用 32 MiB 稳定性阈值。

同一个 Runtime 在各轮之间持续运行；脚本不会调用分配器回收函数或通过重启清空内存。每个场景结束时只关闭自己启动的子进程。比较可用 `--resume` 继续，已完成场景会校验二进制摘要与测量参数后复用结果。

耗时受磁盘负载影响。脚本等待启动最多 15 分钟，同时记录实际启动时间；这不是性能通过标准。
