Commit: 0391933f

# 能力目录核对运行配置引用

显式引用已缺失 Codex profile 的能力曾仍可被新计划选中。目录现在核对中央登记与在线节点各执行类型的配置名称，确认缺失时标记不可用并退出可选列表；配置恢复后沿用原能力 ID 自动恢复。历史记录和已受理执行保留。

节点只返回名称，不返回配置值，也不创建缺失的配置文件。旧节点不支持查询或查询失败时保留原兼容行为，避免误拒绝节点私有配置。

## 功能与定向测试

| 功能 | 测试 |
| --- | --- |
| 缺失配置退出选择、计划拒绝、恢复后保留身份 | `brain_api::availability::missing_profiles_are_visible_unavailable_and_recover_without_recreating_agents` |
| 私有配置、旧节点及执行类型隔离 | `brain_api::availability::private_profiles_and_unknown_legacy_nodes_keep_existing_admission_behavior` |
| 只读名称查询、不泄露配置值、不生成文件 | `execution_profile_discovery_is_read_only_and_does_not_expose_settings` |
| 选择目录与历史 ID 兼容 | `brain_api::selection::library_and_tui_share_selection_while_hidden_ids_remain_valid` |

[能力目录](../../brain/index.md) · [模块边界](../../../agents/brain/index.md)
