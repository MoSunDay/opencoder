# 平滑发布

固定 Nginx 入口连接当前 Server。稳定 Node ID 属于 Agent Host；每个版本使用独立 Runtime 进程、systemd unit、目录和 `runtime.db`。新版本激活只改变全新执行的归属。已有执行、续聊、TODO 重跑、Brain 子执行、历史和产物继续访问原 Runtime。

## 首次迁移

首次迁移需要一次维护窗口。先让当前节点的执行、队列和工具自然结束；迁移工具复核空闲后冻结接入，停止旧 Agent/Server，建立一致性备份，再启用独立资源服务、Host、Runtime 和入口。保留 Node ID、原执行目录、Server 库和凭证。不通过恢复旧库回滚版本。

首次迁移前，在现有 Server `opencoder.json` 中加入 `deployment`，按当前机器填写路径、服务账号、监听地址和原并发上限：

```json
{
  "deployment": {
    "state_dir": "/var/lib/opencoder-platform",
    "server_workdir": "/etc/opencoder/server",
    "server_data": "/var/lib/opencoder-server",
    "server_user": "opencoder-server",
    "agent_workdir": "/srv/opencoder",
    "legacy_agent_data": "/var/lib/opencoder-node",
    "token_file": "/etc/opencoder/server.token",
    "node_name": "worker-a",
    "max_runs": 20,
    "public_url": "http://127.0.0.1:18081",
    "listen": "0.0.0.0:18081"
  }
}
```

默认 Host 本地入口为 `127.0.0.1:18082`，资源管理服务为 `127.0.0.1:18084`，版本实例端口从 `19000` 分配。SQLite 和锁文件必须放在本机本地磁盘。保留现有服务环境配置及原凭证文件，配置和发布清单不包含凭证明文。

```bash
# 没有 Nginx 时执行一次；不会启动业务监听。
scripts/platform/ingress/install.sh

scripts/platform/release/build.sh --output /srv/releases/opencoder-first
scripts/platform/deploy.sh --bundle /srv/releases/opencoder-first --migration-receipt
scripts/platform/deploy.sh --bundle /srv/releases/opencoder-first --migrate --wait-seconds 600
```

迁移收据列出 Node ID、旧服务、数据目录和备份位置。等待预算用尽会报错，不能据此中断任务。已停止旧服务后的中断可用同一命令继续；`migration_stage` 和已完成备份决定恢复位置。资源服务升级、整机重启和破坏性存储迁移属于另行安排的维护操作。

## 兼容版本发布与回滚

```bash
scripts/platform/release/build.sh --output /srv/releases/opencoder-next
scripts/platform/deploy.sh --bundle /srv/releases/opencoder-next
scripts/platform/deploy.sh --status
scripts/platform/rollback.sh
```

发布包包含四个二进制、摘要、`release_id`、协议及数据兼容范围。发布工具核验所有仍保留的版本，取得互斥锁，检查磁盘和可用内存，再依次执行：

1. **校验、预热：**启动候选 Runtime，执行确定 ID 的 WASM 探针；启动候选 Host 和 Server，连接并同步完整索引，核验资源可读且导出只读。
2. **就绪、切换：**先落盘切换意图，再激活 Runtime/Host，graceful reload Nginx。Host 通道按递增交接编号切换；旧 Host 等待各 Server 的完整索引确认及入口确认。
3. **验证、完成：**公共入口核对版本并实际执行探针，更新命令行二进制入口，记录完成。旧 Server 停止监听并等现有响应结束；旧 Runtime 单独回收。

候选探针使用统一任务容量，不能抢占旧任务。候选所需资源不足或探针在等待预算内未完成时，预热报错，当前版本继续服务。同一发布 ID 可以重试；请求与探针不会重新生成 ID。

切换前失败保留当前服务；切换后验证失败回到上一兼容版本。回滚启动上一版本的额外 Server/Host 实例，不等待仍在处理请求的旧实例退出；已被新版本接收的任务留在新 Runtime。回滚不恢复数据库，不撤销已发生的工具副作用。不兼容版本直接拒绝平滑发布。

## 归属、容量与恢复

- `control.db` 分开保存请求指纹、冻结 assignment、派发阶段和五字段索引。相同 ID/相同请求重放同一回执；变更内容返回 409。已明确拒绝的派发也保留回执。Server 恢复时自动重试仍待确认的持久派发。
- Host 的 `host.db` 保存 Runtime 注册、不可变执行归属和全机容量队列。所有版本合计使用同一个并发上限和 FIFO；降低上限不会停止当前执行。多版本 Host 不支持 LIFO。
- 跨进程长操作使用本机文件锁；SQLite 的写事务仅覆盖短提交，不覆盖模型或网络调用。锁文件不能被清理；进程退出由内核释放锁。
- Runtime 写入 `host-binding.json` 后使用共享容量账本，在启动执行前取得槽位，完成持久化后释放。心跳失联、Server 退出、发布或回滚均不释放运行槽位。
- Runtime 意外退出遗留运行槽位时，启动明确拒绝未解决的状态；需在独立维护流程中核实原进程和容器已退出，不能直接按超时回收或自动重跑。
- 休眠要求无运行、排队、工具进程、执行 future、持久化错误和待确认 Brain outbox。Host 保留最终索引、版本包和数据；原执行查询或续跑会通过独立 unit 唤醒 Runtime。回收与使用同一 Runtime 的 RPC 使用互斥/共享文件锁协调。

## 页面、事件与接口

节点页的「发布状态」展示当前版本、候选、阶段、失败原因和每个 Runtime 的排队/运行任务。旧版本可以打开执行详情；休眠与回收失败明确显示。

| 接口 | 用途 |
| --- | --- |
| `GET /api/admin/release` | 发布记录及 Host 的 Runtime/容量状态 |
| `POST /api/admin/release/retire` | 仅退役当前 Server，不改变集群 admission |
| `GET /api/executions/:id/receipt` | 查询持久派发阶段和确定回执 |
| `GET/POST/DELETE /api/admin/drain` | 管理员显式冻结/复开，与发布退役分离 |

Server 退役时 SSE 发出 `reconnect` 和最后已发送的游标。页面自动重连、补齐后续事件；已有游标不会因版本切换被头部水位替换。普通响应没有发布强杀期限。

独立资源服务使用原 Server 账号、工作目录、导出目录和 NFS 端口。Server 的两路 NFS 管理接口转发到资源服务。挂载与 Runtime 不再使用 `PartOf=opencoder-server.service`；常规发布只 reload Nginx，保持 NFS 进程和挂载。

## 备份与验收

```bash
scripts/platform/deploy.sh --backup /srv/backups/opencoder-online-001
```

在线备份逐库使用 SQLite backup API 并检查完整性，明确标记为独立数据库备份，不能当成跨库同一时刻快照。首次维护窗口另行保存停服后的完整一致性备份，包括嵌套数据库。中断的备份保留在独立 staging 目录；重试不覆盖已有备份。

发布验收必须包含跨切换 TODO 依赖链、持续 DAG 工具任务和持续新任务流，并核对原进程、执行归属、FIFO、容量、日志游标、历史与产物。模拟故障覆盖发布工具/Server 中断、重复请求、回滚和三版并存。最终切换后观察 15 分钟。仓库测试与真实运行证据分别记录，不能用启动成功或模拟测试代替真实验收。
