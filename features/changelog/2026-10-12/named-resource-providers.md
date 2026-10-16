Commit: 253aecf4ef9d88e32e99ebe81ba8de598eb925ad

# 按名称和版本隔离资源准入

不同执行可以选择各自的资源提供方，共用 Server 的派发队列。管理员通过 `/api/resource-admission-providers/:name/versions/:version` 登记不可变配置；请求用 `_resource_provider: {name, version}` 选择。既有全局资源准入接口继续工作。

首次派发在原执行锁内、assignment 持久化前冻结提供方。冻结记录仅保留在控制面，节点不接收凭据路径；重启、排队、查询、取消与释放沿用同一提供方，不随新版本配置改变。

| 功能 | 验证 |
|---|---|
| 命名隔离、版本不可变、重启与旧任务冻结、终态释放限制 | `named_authorities_are_isolated_immutable_and_frozen_across_restart` |
| 既有资源等待、取消、缺失提供方 | `cargo test -p opencoder-control --test resource_admission`：3 项通过 |
| 不阻塞普通任务、按序派发、取消去重 | `cargo test -p opencoder-control --test e2e resource_admission`：3 项通过 |
| Rust 静态检查 | `cargo clippy -p opencoder-control --all-targets -- -D warnings`：通过 |

仅运行改动及直接影响链路的测试。真实容器与发布验收回执由发布流程单独保存，不以以上检查代替。
