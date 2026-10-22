# 索引同步的完整链路 CPU 验收

脚本启动独立的 Server、Host、Runtime，使用相同合成历史和固定速率的就绪、历史查询请求，测量三个进程的 CPU 总和。默认 20,000 条历史，其中 4,000 条含 128 KiB 冻结数据；预热 30 秒，采样 120 秒，执行三轮，交替新旧程序的先后顺序。

```sh
python3 scripts/acceptance/cpu_inventory/main.py \
  --before /path/to/baseline/bin \
  --after /path/to/candidate/bin \
  --output /tmp/inventory-cpu-comparison
python3 -m unittest discover -s scripts/acceptance/cpu_inventory -p 'test_*.py' -v
```

输出保存每个程序的 SHA-256、各进程 CPU 时间和 RSS、请求数、查询 P95、完整历史核验摘要以及磁盘压力。三个进程的 CPU 百分比之和以一个核心为 100%；不能把它当成整台机器的 CPU 百分比。通过条件为三轮中位 CPU 合计下降至少 50%，查询 P95 不劣化超过 10%。请求失败、未保持固定请求速率、历史数量或归属变化、进程重启均拒绝验收，不能用减少工作量制造降幅。

`requests.json` 保存逐次延迟、请求类型和实际发送时刻，结果另列就绪查询与历史查询各自的 P95，便于定位波动。判断结果仍使用全部请求的 P95，不能删除慢请求或用 CPU 降幅抵消延迟失败。

所有程序使用输出目录下的配置和数据，只监听回环地址，且不调用生产接口。合成冻结请求写入已停止的临时控制数据库；Host 的临时注册和 Runtime 的会话记录同样在停止状态准备，以模拟已经运行的历史 Runtime。每次历史查询同时核对执行结果和会话身份，拒绝外层成功但会话详情含错误的响应。脚本不生成假的执行探针，不通过发布接口激活候选，不验证 DAG 执行或发布切换；这些仍由独立的真实容器和发布演练验收。

结果中的 `fixture_version: 2` 表示包含会话记录。早期只有 Journal 的夹具实际触发了“执行结束但会话未初始化”的回退读取；其性能记录须保留，不能与正常会话样本混合计算或改写成通过。

首次完整上报在预热前完成，结束时再次分页检查全部历史。本场景衡量正常周期上报和读取的持续成本；首次同步、单条变化、断线恢复和 Brain ACK 由对应定向测试覆盖，2 千、2 万、10 万规模的源轮询比较由单独场景补充。内存上限使用 `runtime_memory` 场景验证。

```sh
python3 scripts/acceptance/cpu_inventory/sources.py \
  --before /path/to/baseline/bin/opencoder-agent \
  --after /path/to/candidate/bin/opencoder-agent \
  --output /tmp/inventory-source-comparison
```

源轮询场景依次比较 2 千、2 万、10 万条历史。旧程序读取完整 `/inventory`，新程序先核验完整分页，再连续读取 30 次空增量；每种规模分别要求 CPU 时间下降至少 50%、P95 不劣化超过 10%。首次加载和完整同步耗时单独记录，不能将源轮询结果当成整条链路的 CPU 结果。

```sh
python3 scripts/acceptance/cpu_inventory/throughput.py \
  --before /path/to/baseline/bin/opencoder-agent \
  --after /path/to/candidate/bin/opencoder-agent \
  --output /tmp/inventory-active-throughput
```

吞吐场景复用 `runtime_memory` 的本地模型夹具，默认执行三轮新旧版交替顺序的对照；每轮每版连续执行 5 批、每批 32 项任务，并发为 4，共各 480 项。每项携带 1 MiB 冻结数据，模型响应完成后关闭会话，核验执行终态、冻结请求和排队配置均已持久保存。以三轮全部任务数除以全部批次总耗时计算吞吐，新版不得低于旧版的 95%；不把批间内存采样时间计入执行耗时。该场景验证执行路径，不代替真实模型与容器的发布连续性验证。

运行正式比较时不要并行构建、备份或执行另一组性能测试。保留失败记录和复测记录；合成数据结果不代替生产版本生效及观察证据。

吞吐比较保留每批时长，并拒绝缺失轮次、程序在轮次之间变化、任务或落盘快照不足。不得去掉较慢的首轮或只选最好的一轮；单轮失败后追加的固定对照必须把原失败样本一并计入总耗时。
