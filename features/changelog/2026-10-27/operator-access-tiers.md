Commit: 4ec0637e71be1b14d82399c9d5d9fc9aabce860f
# Operator 访问档位与只读资源授读

Operator 身份层之上新增访问档位 `operator.access`：`readwrite`（默认）获得私有 TMPDIR；`readonly` 将 home/workspace 冻结为 root 属主 0555，operator 只读遍历，共享只读资源经附加组 `opencoder-ro` 授予组读。声明层（shellguard、sandbox、ToolFilter）语义不变，本变更新增 OS 级硬边界。

## 变更及边界

- `OperatorConfig` 新增 `access` 字段（serde 默认 `readwrite`），配置合并链路识别并校验非法值。
- readwrite：`TMPDIR=<home>/tmp`（0700、operator 属主）作为持久化 env pair 注入，节点重启恢复；readonly 刻意不提供 TMPDIR，工具写操作直接失败。
- readonly：目录树冻结为 root:root 0555；供给时加入共享组 `opencoder-ro`（`groupadd -f` 幂等），并对共享只读资源根（operator 配置面、技能目录等）执行 chgrp + g+rX，仅限白名单根，绝不触碰执行目录之外路径。
- 供给前补扫：清理执行目录缺失或记录终态的遗留 `ocop-` 账号，弥补队列路径终态不释放身份的缺口；在执行且目录存在的账号保留。
- 部署文档更新：agent unit 注明身份供给需 root 或 CAP_SETUID+CAP_SETGID+CAP_CHOWN 与 shadow-utils；`docs/agent-platform.md` 新增「Operator 执行身份」一节。
- Ontology 核对：Operator 定义经 `agents` 实体类型登记且仅同步元数据，本次配置档位不改变任何已登记能力的身份、入口或输入输出要求，无需目录变更。

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| readwrite 档 TMPDIR pair 与私有 tmp | `read_write_tier_sets_the_tmpdir_pair_and_a_private_tmp` |
| readonly 冻结目录树且 operator 写失败 | `read_only_tier_freezes_trees_and_rejects_operator_writes` |
| readonly 加入共享组并授予资源根组读 | `read_only_tier_joins_the_shared_group_and_grants_read_roots` |
| readwrite 不建立共享组成员关系 | `read_write_tier_grants_no_shared_group_membership` |
| 补扫清理终态/无目录账号、保留活跃账号 | `sweep_releases_directory_less_and_terminal_but_keeps_fresh_accounts` |
| access 字段解析与默认值 | `access_defaults_to_read_write_and_parses_both_tiers` |
| 配置合并识别 access 档位 | `merge_into_applies_operator_linux_identity_and_skips_invalid` 系列 |

## 验证范围

- `cargo test -p opencoder-worker --test operator_access_tiers`：5 用例通过（root 实跑，真实账号/组/权限断言）。
- `cargo test -p opencoder-core --lib config`：通过（含 access 解析与合并用例）。
- `cargo clippy -p opencoder-worker -p opencoder-core --all-targets -- -D warnings`：零警告。
