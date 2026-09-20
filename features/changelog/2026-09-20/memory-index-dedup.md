Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# 记忆索引去冗余

## 动机

记忆 index 的职责是索引，不是解析细节：代码本身就是细节的最终事实。此前各模块/能力 index 累积了大量行为叙述（交互细节、默认值、字段语义、竞态成因、变更迁移解说），与代码和 changelog 形成冗余，且随迭代漂移。

## 变更

- `agents/*/index.md` 全部 23 个模块索引统一为「一句话定位 + 路径指针索引 + 相关链接」；删除行为叙述、参数默认值、交互细节与变更历史措辞。
- 保留且仅保留三类事实：模块边界/接缝（`Arc<dyn Store>`、DTO LOCKED、fail-closed、五字段执行索引等契约短语）、测试入口路径、指向 features/changelog 的案例短链。
- `features/index.md` 压为一行一条能力索引，交互细节移交 `docs/dag-dynamic.md` 等文档链接；`features/agent-platform`、`features/brain` 压缩为定位 + 相关链接。
- 失效指针修正：`session`（subagent/compaction/cancel 路径）、`llm`（`ChatStream` 实际位于 `src/stream.rs`）、`agent`（changelog 相对层级）。

## 兼容

纯记忆文档整改，无代码改动。根 `agents.md` 逻辑地图本身已是一行式索引，未动。changelog 目录保持为唯一的变更流水账载体。

## 验证

| 项 | 结果 |
| --- | --- |
| 全量 markdown 相对链接校验（agents/ + features/ + agents.md，排除 changelog） | 107 个链接 0 失效（脚本扫描） |
| 指针存在性 | 各文件逐条 `ls` 验证 |
| 行数 | 索引层合计 862 → 437 行（-49%），单文件均 ≤ 26 行 |
| 代码影响 | 无（`git diff` 仅涉及 md 与此前工作区已有改动） |
