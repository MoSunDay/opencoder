use super::Capability;

/// Every public CLI domain must have a platform catalog entry.
pub const CLI_DOMAINS: &[&str] = &[
    "storage", "health", "ready", "time", "drain", "exec", "session", "nodes", "dag", "todo",
    "project", "brain", "teams", "agents", "schedule", "ontology", "raw",
];

// Longest prefix wins. New public HTTP families belong here before use.
const ENTRIES: &[(&str, &str, &str, &[&str])] = &[
    (
        "models",
        "模型目录",
        "查询可用模型的操作入口",
        &["/api/models"],
    ),
    (
        "skills",
        "技能目录",
        "查询可用技能的操作入口",
        &["/api/skills"],
    ),
    ("storage", "本地数据库工具", "只读查询与数据库备份", &[]),
    (
        "health",
        "服务健康",
        "查询控制面健康与版本",
        &["/api/health"],
    ),
    ("ready", "服务就绪", "查询服务受理条件", &["/api/ready"]),
    ("time", "服务时间", "查询服务端时间", &["/api/time"]),
    (
        "drain",
        "受理控制",
        "查询、暂停与恢复受理",
        &["/api/admin/drain"],
    ),
    (
        "exec",
        "执行管理",
        "创建执行，读取结果、消息、事件和产物",
        &["/api/executions"],
    ),
    (
        "session",
        "会话管理",
        "创建会话、交互与读取过程",
        &["/api/sessions"],
    ),
    (
        "nodes",
        "节点管理",
        "查询节点、模型、技能与调度配置",
        &["/api/nodes"],
    ),
    (
        "dag",
        "DAG 工作流",
        "维护定义与派发工作流",
        &["/api/dag", "/api/workflows"],
    ),
    (
        "todo",
        "TODO 工作流",
        "管理模板、环境和工作流执行",
        &["/api/todo", "/api/todos"],
    ),
    (
        "project",
        "项目管理",
        "管理项目、子项、任务、执行关联及评论的追加、编辑和删除",
        &["/api/project"],
    ),
    (
        "brain",
        "Brain 编排",
        "维护能力库、版本化计划和分层运行",
        &["/api/brain"],
    ),
    (
        "teams",
        "Team 组队",
        "维护团队成员与团队定义",
        &["/api/teams"],
    ),
    (
        "agents",
        "Agent 定义与资源",
        "维护 Agent、版本资源与只读导出",
        &["/api/agents"],
    ),
    (
        "schedule",
        "定时任务",
        "管理调度定义与触发历史",
        &["/api/schedules"],
    ),
    (
        "ontology",
        "Ontology",
        "组织能力、实体、关系与图谱；托管发布仅当前 Server 实例可同步；旧库缺少结构须维护升级",
        &["/api/ontology"],
    ),
    (
        "raw",
        "原生 API 调用",
        "通过方法和路径调用已授权的接口",
        &[],
    ),
    (
        "identity",
        "身份与权限",
        "查看身份、管理用户和凭据的操作入口",
        &["/api/me", "/api/users", "/api/tokens", "/api/auth"],
    ),
    (
        "harness",
        "执行器配置",
        "管理执行器及注册配置的操作入口",
        &["/api/harnesses"],
    ),
    (
        "resource-admission",
        "资源准入",
        "管理资源提供方与受理约束",
        &[
            "/api/resource-admission-provider",
            "/api/resource-admission-providers",
        ],
    ),
    (
        "release",
        "服务版本管理",
        "查询版本与维护服务切换；校验读取兼容及共享库结构，缺表时先维护迁移",
        &["/api/admin/release"],
    ),
    (
        "metrics",
        "调度监控",
        "读取服务调度指标",
        &["/api/metrics", "/metrics"],
    ),
    ("tui", "终端能力目录", "查询终端可选能力", &["/api/tui"]),
];

pub fn platform_capabilities() -> Vec<Capability> {
    ENTRIES
        .iter()
        .map(|(id, name, description, paths)| Capability {
            key: format!("platform:{id}"),
            source: "platform".into(),
            source_id: (*id).into(),
            kind: "platform".into(),
            name: (*name).into(),
            description: (*description).into(),
            version: crate::version::VERSION_LONG.into(),
            cli: if CLI_DOMAINS.contains(id) {
                vec![format!("opencoder-cli {id} --help")]
            } else {
                vec!["opencoder-cli raw call METHOD PATH".into()]
            },
            api: paths.iter().map(|path| (*path).to_owned()).collect(),
            ..Default::default()
        })
        .collect()
}

pub fn platform_for_path(path: &str) -> Option<&'static str> {
    ENTRIES
        .iter()
        .flat_map(|(id, _, _, prefixes)| prefixes.iter().map(move |prefix| (*id, *prefix)))
        .filter(|(_, prefix)| {
            path == *prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|(_, prefix)| prefix.len())
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_identifiers_are_unique_and_routes_use_path_boundaries() {
        let entries = platform_capabilities();
        let keys: std::collections::BTreeSet<_> = entries.iter().map(|entry| &entry.key).collect();
        assert_eq!(entries.len(), keys.len());
        for domain in CLI_DOMAINS {
            assert!(entries.iter().any(|entry| entry.source_id == *domain));
        }
        assert_eq!(
            platform_for_path("/api/ontology/envs/debug/entities"),
            Some("ontology")
        );
        assert_eq!(platform_for_path("/api/project/todos"), Some("project"));
        assert_eq!(
            platform_for_path("/api/project/todos/task/comments/1"),
            Some("project")
        );
        assert!(entries
            .iter()
            .find(|entry| entry.source_id == "project")
            .unwrap()
            .description
            .contains("编辑和删除"));
        assert_eq!(platform_for_path("/api/project-unregistered"), None);
        assert_eq!(platform_for_path("/api/unknown"), None);
        assert!(entries
            .iter()
            .all(|entry| !entry.name.is_empty() && !entry.description.is_empty()));
    }
}
