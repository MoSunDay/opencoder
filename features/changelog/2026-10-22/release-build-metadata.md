Commit: 6b8410c1bb5a175cfd8d0d6c2259fa357e136024

# 共享构建缓存中的发布版本信息

## 原因与修复

关联工作树共用 Cargo target 时，依赖 crate 的 build script 可能复用前一个工作树的结果，继续监视旧 HEAD。源码路径已切换，但编译提交仍指向前一个版本；成套发布包的提交核验会拒绝该产物。

发布脚本向应用和原生运行器的全部构建传入当前提交 `OPENCODER_BUILD_COMMIT`。Core 把它声明为缓存刷新条件，并验证预期提交等于实际检出提交。没有放宽成套程序一致性、干净源码或包摘要检查；运行协议和数据库格式不变。

## 验证

| 行为 | 测试 |
|---|---|
| 两个工作树共用缓存，依次构建 A、B、A 都得到自己的提交；错误预期提交拒绝构建 | [test_release_commit_refreshes_shared_worktree_cache_and_rejects_mismatch](../../../scripts/platform/release/test_build_metadata.py) |

测试使用真实 Git 工作树与 Cargo，一个二进制依赖包含实际 build script 的 core crate。原实现返回旧提交而失败；修复后同一用例通过。`cargo clippy -p opencoder-core --all-targets -- -D warnings` 通过；成套回滚包的七个二进制均通过提交、构建信息及摘要核验。只验证此次构建变更，未运行全量测试。

## 相关说明

- [Core 逻辑](../../../agents/core/index.md)
- [发布与回滚](../../../docs/smooth-release.md)
