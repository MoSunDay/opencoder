Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# dag-wasm 模块

DAG wasm 模块版本池：发布、NFS 导出、节点冻结分发。

## 索引
- `crates/dag-wasm/src/` — 版本池管理与发布
- NFS 导出与节点侧冻结分发接口
- `examples/dag-modules/` — 池内可发布步骤模块示例（kb-index：知识库索引，
  产物契约 `output.json` + `manifest.json`，fail-closed 缺目录 exit 1）

## 相关
- [agents/control](../control/index.md)、[agents/worker](../worker/index.md)
