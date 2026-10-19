# 目录维护

从部署主机以只读 SQLite 及实际资源根生成清单。只清理 DAG 定义和二进制池；保留认证数据、执行历史、日志、产物和工作区。未知时间、引用或无法读取的资源不能当作可删除项。

```sh
python3 -m scripts.maintenance.catalog_cleanup preview \
  --data-dir /absolute/server-state \
  --binary-root /absolute/binaries --agents-root /absolute/agents \
  --config-path /absolute/server-config.json \
  --retention-days 7 --output /absolute/unique-evidence
python3 -m scripts.maintenance.catalog_cleanup apply \
  --plan /absolute/unique-evidence/plan.json \
  --base-url https://server.example --token-file /absolute/existing-token
python3 -m scripts.maintenance.catalog_cleanup restore \
  --plan /absolute/unique-evidence/plan.json --kind binary --name exact-name
```

`preview` 不写部署数据。独立闲置项的创建、更新时间和最近执行受理时间须都超过保留期；当前执行和待派发请求单独保护。完全相同的额外副本可立即列入候选；优先保留被引用、最近成功受理或最早创建的条目。DAG 只忽略名称和管理元数据，二进制比较 current 与完整版本序列的摘要、大小及编号；不使用名称关键词判断价值。

`apply` 要求 Server `/api/health` 声明 `catalog_guard: 1`，重新扫描引用和候选，核对预览摘要及详情 ETag，持久化备份后发送 `If-Match` DELETE。备份包含完整定义或资源池全部版本，逐文件校验摘要；回执逐项写盘，失败项跳过。重复执行同一清单跳过已删除项，不生成新清理范围。

`restore` 校验备份并取得同一目录锁，只恢复指定对象；目标名已被不同内容占用则拒绝。DAG 恢复为原记录，二进制通过暂存目录原子放回，不覆盖整个数据库。命令只应由部署主机管理员运行；不配置周期任务。

验证：`python3 -m unittest scripts.maintenance.catalog_cleanup.test_cleanup scripts.maintenance.catalog_cleanup.test_apply -v`。
