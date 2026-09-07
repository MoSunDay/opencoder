//! 执行器内联 spec 的类型与校验（纯函数，无 IO）：team 的内联团队定义
//! `TeamSpec`、brain 的能力路由表 `BrainRoutes`，以及 API 写入 todo
//! 之前用的 `validate_spec`（dag spec 委托 `opencoder_dag` 校验）。
//! 全部类型 serde-default 宽容，旧数据/部分字段缺失都能解码。
//!
//! 住在 store（而非 opencoder-project）的原因：该模块被 web 与控制面
//! 共同编译的 `api_project_todos.rs` 消费，而控制面按 P0 拆分不得链接
//! project 执行引擎；此处依赖只有 dag 域类型 + 本 crate 的
//! `ProjectExecutorKind`。project crate 经 `executor::spec` 再导出。

use anyhow::{bail, Context as _, Result};
use serde::{Deserialize, Serialize};

use opencoder_dag::{validate as validate_dag, DagSpec};

use crate::ProjectExecutorKind;

/// 校验内联 executor_spec（web API 在写入 todo 之前调用；纯函数）。
pub fn validate_spec(kind: ProjectExecutorKind, spec_json: &str) -> Result<()> {
    match kind {
        ProjectExecutorKind::Agent => bail!("agent executor takes no spec"),
        ProjectExecutorKind::Team => {
            let spec: TeamSpec = serde_json::from_str(spec_json).context("parse team spec")?;
            validate_team_spec(&spec)
        }
        ProjectExecutorKind::Dag => {
            let spec: DagSpec = serde_json::from_str(spec_json).context("parse dag spec")?;
            validate_dag(&spec)
                .map_err(|errs| anyhow::anyhow!("dag spec invalid: {}", errs.join("; ")))
        }
        ProjectExecutorKind::Brain => {
            let routes: BrainRoutes =
                serde_json::from_str(spec_json).context("parse brain routes")?;
            validate_brain_routes(&routes)
        }
    }
}

// ── TeamSpec：team 执行器的内联定义 ─────────────────────────────────────

/// team 执行器的内联 spec（todo.executor_spec）。`name` 仅作展示，真正
/// 落盘的团队名由 team_drive 按 todo id 物化，保证 NFS 目录名安全。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamSpec {
    pub name: String,
    pub captain: TeamMemberRef,
    #[serde(default)]
    pub members: Vec<TeamMemberSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberRef {
    pub node_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemberSpec {
    pub node_id: String,
    pub name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn non_empty(s: &str) -> bool {
    !s.trim().is_empty()
}

fn validate_team_spec(spec: &TeamSpec) -> Result<()> {
    if !non_empty(&spec.name) {
        bail!("team spec name is empty");
    }
    if !non_empty(&spec.captain.node_id) || !non_empty(&spec.captain.name) {
        bail!("team captain requires non-empty node_id and name");
    }
    let mut seen_nodes: Vec<&str> = Vec::new();
    for (idx, m) in spec.members.iter().enumerate() {
        if !non_empty(&m.node_id) || !non_empty(&m.name) {
            bail!("team member #{idx} requires non-empty node_id and name");
        }
        // 队长可同时以成员身份出现（角色不同），但同一节点不能在成员里
        // 坐两把椅子：team_drive 按成员物化时会撞出重复派单。
        if seen_nodes.contains(&m.node_id.as_str()) {
            bail!("duplicate team member node_id: {}", m.node_id);
        }
        seen_nodes.push(m.node_id.as_str());
        let mut seen_caps: Vec<&str> = Vec::new();
        for cap in &m.capabilities {
            // 能力路由按 capability 精确匹配，重复项语义含糊且掩盖笔误。
            if seen_caps.contains(&cap.as_str()) {
                bail!("member #{idx} lists duplicate capability: {cap}");
            }
            seen_caps.push(cap.as_str());
        }
    }
    Ok(())
}

// ── BrainRoutes：能力 → 执行器路由表 ───────────────────────────────────

/// brain 执行器的路由表（todo.executor_spec）：capability id 精确命中
/// `routes` 之一则用该路由，否则落到 `default`。两个字段均可缺省。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainRoutes {
    #[serde(default)]
    pub routes: Vec<BrainRoute>,
    #[serde(default)]
    pub default: BrainRoute,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrainRoute {
    pub capability: String,
    pub kind: BrainRouteKind,
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub ref_: Option<String>,
}

/// 路由目标执行器（没有 brain：brain→brain 嵌套在类型层面即被拒绝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrainRouteKind {
    Agent,
    Team,
    Dag,
}

impl From<BrainRouteKind> for ProjectExecutorKind {
    fn from(kind: BrainRouteKind) -> Self {
        match kind {
            BrainRouteKind::Agent => ProjectExecutorKind::Agent,
            BrainRouteKind::Team => ProjectExecutorKind::Team,
            BrainRouteKind::Dag => ProjectExecutorKind::Dag,
        }
    }
}

impl Default for BrainRoute {
    /// 缺省路由：交给 act 代理直驱。
    fn default() -> Self {
        BrainRoute {
            capability: String::new(),
            kind: BrainRouteKind::Agent,
            ref_: Some("act".into()),
        }
    }
}

impl BrainRoutes {
    /// capability 精确匹配优先，未命中走 default。
    pub fn pick(&self, capability: &str) -> &BrainRoute {
        self.routes
            .iter()
            .find(|r| r.capability == capability)
            .unwrap_or(&self.default)
    }
}

/// 校验 brain 路由表：`routes` 里 capability 不得重复——`pick` 按首个
/// 命中取值，重复键会让路由结果依赖数组顺序，语义含糊，直接拒绝。
fn validate_brain_routes(routes: &BrainRoutes) -> Result<()> {
    let mut seen: Vec<&str> = Vec::new();
    for r in &routes.routes {
        if seen.contains(&r.capability.as_str()) {
            bail!("duplicate brain route capability: {}", r.capability);
        }
        seen.push(r.capability.as_str());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM_OK: &str = r#"{"name":"服务端小队","captain":{"node_id":"act","name":"队长"},
        "members":[{"node_id":"act","name":"队员","capabilities":["rust"]}]}"#;

    #[test]
    fn validate_spec_accepts_good_team_and_dag_specs() {
        validate_spec(ProjectExecutorKind::Team, TEAM_OK).unwrap();
        let dag =
            r#"{"name":"etl","steps":[{"name":"fetch","kind":{"type":"agent","prompt":"p"}}]}"#;
        validate_spec(ProjectExecutorKind::Dag, dag).unwrap();
    }

    #[test]
    fn validate_spec_rejects_bad_team_specs() {
        // 空队长 / 空成员字段 / 成员名缺失。
        for bad in [
            r#"{"name":"t","captain":{"node_id":"","name":"队长"}}"#,
            r#"{"name":"t","captain":{"node_id":"act","name":" "},"members":[]}"#,
            r#"{"name":"t","captain":{"node_id":"act","name":"c"},"members":[{"node_id":"act"}]}"#,
        ] {
            assert!(
                validate_spec(ProjectExecutorKind::Team, bad).is_err(),
                "should reject: {bad}"
            );
        }
        // 仅队长（无成员）合法。
        validate_spec(
            ProjectExecutorKind::Team,
            r#"{"name":"t","captain":{"node_id":"act","name":"队长"}}"#,
        )
        .unwrap();
    }

    #[test]
    fn validate_spec_rejects_duplicate_team_members_and_capabilities() {
        // 多成员、节点与能力互不相同：仍然合法。
        validate_spec(
            ProjectExecutorKind::Team,
            r#"{"name":"t","captain":{"node_id":"captain","name":"队长"},
                "members":[{"node_id":"n1","name":"a","capabilities":["rust","sql"]},
                    {"node_id":"n2","name":"b","capabilities":["go"]}]}"#,
        )
        .unwrap();
        // 同一节点在成员里出现两次：拒绝并点名该 node_id。
        let dup_node = validate_spec(
            ProjectExecutorKind::Team,
            r#"{"name":"t","captain":{"node_id":"c","name":"队长"},
                "members":[{"node_id":"n1","name":"a"},{"node_id":"n1","name":"b"}]}"#,
        )
        .unwrap_err();
        assert!(
            dup_node
                .to_string()
                .contains("duplicate team member node_id: n1"),
            "{dup_node}"
        );
        // 成员能力列表重复：拒绝并带成员序号。
        let dup_cap = validate_spec(
            ProjectExecutorKind::Team,
            r#"{"name":"t","captain":{"node_id":"c","name":"队长"},
                "members":[{"node_id":"n1","name":"a","capabilities":["rust","rust"]}]}"#,
        )
        .unwrap_err();
        assert!(
            dup_cap
                .to_string()
                .contains("member #0 lists duplicate capability: rust"),
            "{dup_cap}"
        );
    }

    #[test]
    fn validate_spec_rejects_duplicate_brain_route_capabilities() {
        // 路由 capability 各不相同：合法。
        validate_spec(
            ProjectExecutorKind::Brain,
            r#"{"routes":[{"capability":"cap-1","kind":"dag","ref":null},
                {"capability":"cap-2","kind":"team","ref":"t1"}]}"#,
        )
        .unwrap();
        // 重复 capability：pick 只取首个命中，结果依赖顺序，直接拒绝。
        let err = validate_spec(
            ProjectExecutorKind::Brain,
            r#"{"routes":[{"capability":"cap-1","kind":"dag","ref":null},
                {"capability":"cap-1","kind":"team","ref":"t1"}]}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("duplicate brain route capability: cap-1"),
            "{err}"
        );
    }

    #[test]
    fn validate_spec_rejects_bad_dag_and_agent_specs() {
        // 空步骤列表 / 重复步骤名 / 未知依赖。
        let errs = [
            r#"{"name":"e","steps":[]}"#,
            r#"{"name":"e","steps":[{"name":"a","kind":{"type":"agent","prompt":"p"}},
                {"name":"a","kind":{"type":"agent","prompt":"q"}}]}"#,
            r#"{"name":"e","steps":[{"name":"a","depends_on":["ghost"],"kind":{"type":"agent","prompt":"p"}}]}"#,
        ];
        for bad in errs {
            assert!(
                validate_spec(ProjectExecutorKind::Dag, bad).is_err(),
                "should reject: {bad}"
            );
        }
        // agent 执行器不接受任何 spec。
        assert!(validate_spec(ProjectExecutorKind::Agent, "{}").is_err());
    }

    #[test]
    fn validate_spec_parses_brain_routes() {
        validate_spec(
            ProjectExecutorKind::Brain,
            r#"{"routes":[{"capability":"cap-1","kind":"dag","ref":null}]}"#,
        )
        .unwrap();
        // 非法 kind / 结构错误（serde 结构体也接受空序列形态，用纯垃圾
        // 文本验证解码失败路径）。
        assert!(validate_spec(
            ProjectExecutorKind::Brain,
            r#"{"routes":[{"capability":"c","kind":"brain"}]}"#
        )
        .is_err());
        assert!(validate_spec(ProjectExecutorKind::Brain, "not-json").is_err());
    }

    #[test]
    fn brain_routes_pick_exact_then_default() {
        let routes: BrainRoutes = serde_json::from_str(
            r#"{"routes":[{"capability":"cap-1","kind":"dag","ref":"my-dag"}],
                "default":{"capability":"","kind":"team","ref":"fleet-team"}}"#,
        )
        .unwrap();
        let hit = routes.pick("cap-1");
        assert_eq!(hit.kind, BrainRouteKind::Dag);
        assert_eq!(hit.ref_.as_deref(), Some("my-dag"));
        let miss = routes.pick("cap-other");
        assert_eq!(miss.kind, BrainRouteKind::Team);
        assert_eq!(miss.ref_.as_deref(), Some("fleet-team"));
    }

    #[test]
    fn brain_routes_default_when_json_omits_everything() {
        let routes: BrainRoutes = serde_json::from_str("{}").unwrap();
        let picked = routes.pick("anything");
        assert_eq!(picked.kind, BrainRouteKind::Agent);
        assert_eq!(picked.ref_.as_deref(), Some("act"));
        // 显式 default 覆盖内置缺省。
        let explicit: BrainRoutes =
            serde_json::from_str(r#"{"default":{"capability":"x","kind":"dag","ref":"d1"}}"#)
                .unwrap();
        assert_eq!(explicit.pick("x").kind, BrainRouteKind::Dag);
    }

    #[test]
    fn team_spec_decodes_without_optional_members() {
        let spec: TeamSpec =
            serde_json::from_str(r#"{"name":"t","captain":{"node_id":"act","name":"队长"}}"#)
                .unwrap();
        assert!(spec.members.is_empty());
        validate_team_spec(&spec).unwrap();
    }
}
