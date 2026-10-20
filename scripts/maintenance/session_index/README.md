# 历史会话查询索引

当会话清单使用 `MAX(updated_at, created_at), id` 排序、历史记录较多且查询计划出现临时排序时，可补齐 `idx_sessions_activity_visible`。该工具只创建会话排序索引并刷新 `sessions` 的查询统计；不变更 schema 版本，不修改应用记录，不停止或重启节点。它不替代执行详情、完整消息和结果验收。

该工具仅用于尚未切换的 SQLite/libsql 数据库。存在 `.engine.json` 标记时，预览和执行都会在打开数据库前拒绝；Turso 数据库必须使用原生引擎维护。

先执行只读预览，再使用同一数据库、15 分钟内的预览回执执行：

```sh
PYTHONPATH=scripts/maintenance python3 -m session_index \
  --database /absolute/runtime.db --output /private/preview.json
PYTHONPATH=scripts/maintenance python3 -m session_index \
  --database /absolute/runtime.db --preview /private/preview.json \
  --output /private/applied.json
```

预览绑定文件设备号、inode 和相关表结构摘要。执行要求 WAL 模式，使用有界写事务；锁竞争、身份变化、索引定义冲突、未采用预期查询计划或截止时间到达都会失败并回滚。默认时限 20 秒，上限 60 秒。重复执行保留同一索引。SQL 授权器拒绝应用行写入及鉴权表读取；输出不能覆盖数据库或其 WAL/SHM 文件。

执行后继续核对真实会话列表和执行详情的延迟、完整内容与节点身份。索引生效不代表所有查询已经稳定，也不代表执行结果完成。源码与数据库地址、回执、真实会话内容分开保存。

定向测试：`PYTHONPATH=scripts/maintenance python3 -m unittest session_index.test_index -v`。
