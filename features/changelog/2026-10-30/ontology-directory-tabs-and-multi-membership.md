# Ontology 实体目录多对多归属与实体页目录 Tabs

一句话：普通实体的目录归属放宽为多对多——同一实体可同时挂多个目录，仅“同一实体 + 同一目录”重复归属返回 409；目录节点之间保持单父树、防环、根不可移动、非空目录不可删（用户明确决策，放宽 rules/09 “每个成员最多一个父目录”不变量仅限普通实体）。实体页内容区同步改为 Tabs【目录 | 实体】（默认“实体”），“目录”tab 提供目录树编辑与成员管理，实体表格新增「所属目录」列（多 Tag，点击跳目录 tab）。

## 变更清单

- `crates/ontology/src/database/model/relationships.rs`：`create_relationship` 的目录归属（membership）分支按 source 实体类型拆分——source 为目录节点时维持单父校验，经 relationships 挂第二个父目录返回 409 `directory already has a parent directory`；source 为普通实体时允许多条归属，仅拒绝同一实体对同一目录的重复归属（409 `entity already belongs to this directory`）。
- `crates/ontology/src/api/directories.rs`：`validate_membership` 同步拆分为目录节点/普通实体两条校验路径，语义与 relationships 层一致；目录树不变量（防环、根不可移动、非空目录不可删）不变。
- 新增 `crates/ontology/tests/directory_membership.rs`：多对多归属与目录树不变量集成测试。
- `crates/ctl/tests/ontology_server/directories.rs`：更新既有用例断言——同一实体挂两个目录后成员均可见、重复归属被拒。
- `crates/web/spa/src/ontology/api.ts` + `types.ts`：目录树、成员与目录维护客户端及多归属类型；无新增 HTTP 端点。
- 新增 `crates/web/spa/src/ontology/pages/directories/`：`directoryModel.ts`（目录树纯函数模型）、`DirectoryTree.tsx`（树编辑器）、`MembersPanel.tsx`（成员面板）、`DirectoryPanel.tsx`（tab 编排）、`directories.css` 及各自测试。
- `crates/web/spa/src/ontology/pages/EntitiesPage.tsx`：内容区改为 Tabs【目录 | 实体】（默认“实体”），实体表格新增「所属目录」列（多 Tag，点击切换目录 tab 并选中该目录）。
- `crates/web/spa/dist` 重新构建入库。
- 文档：`rules/09-ontology-capabilities.md`、`rules/10-ontology-cli.md`、`features/ontology/index.md`；`repo-memory.md` 经 grep 核对无过期目录表述，未改动。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 实体挂第 2 个目录成功 + 同实体同目录重复 409 | `entity_mounts_multiple_directories_but_duplicate_membership_conflicts` | `crates/ontology/tests/directory_membership.rs` |
| 目录节点第二父 409 / move 防环 / 根不可移动 / 非空目录删除 409 | `directory_nodes_keep_single_parent_and_tree_invariants` | `crates/ontology/tests/directory_membership.rs` |
| CLI 双目录挂载可见成员 + 重复归属拒绝（扩展既有用例） | `cli_directory_tree_mounts_new_and_existing_entities_and_rejects_invalid_moves` | `crates/ctl/tests/ontology_server/directories.rs` |
| 目录树模型纯函数 | `directoryModel.test.ts` 各用例 | `crates/web/spa/src/ontology/pages/directories/directoryModel.test.ts` |
| 树编辑操作→API 调用/重名校验/根保护 | `DirectoryTree.test.tsx` 各用例 | `crates/web/spa/src/ontology/pages/directories/DirectoryTree.test.tsx` |
| 成员添加/移出 | `MembersPanel.test.tsx` 各用例 | `crates/web/spa/src/ontology/pages/directories/MembersPanel.test.tsx` |
| 实体页 tabs/所属目录列 | `EntitiesPage.test.tsx` 各用例 | `crates/web/spa/src/ontology/pages/entities/EntitiesPage.test.tsx` |
| 既有树不变量回归 | `cargo test -p opencoder-ontology --test graph` | `crates/ontology/tests/graph.rs` |

## 决策记录

- 多对多归属为用户明确决策：放宽 rules/09 “每个成员最多一个父目录”不变量，仅限普通实体；目录节点仍保持单一父目录，防环、根保护、非空目录保护不变。
- 后端拒绝语义：同一实体对同一目录的重复归属 409（`entity already belongs to this directory`）；目录节点经 relationships 加第二个父目录 409（`directory already has a parent directory`）。

## 登记核对

- 本次无新增 HTTP 端点、无新增 CLI 命令；能力目录（platform 能力登记）不含目录语义描述，故 Ontology 能力目录登记不变，同步测试无需调整。
