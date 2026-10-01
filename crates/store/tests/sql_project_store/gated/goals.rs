use super::*;

pub(super) async fn seed_goals(p: &dyn ProjectStore, goal: &str, goal2: &str, ts: i64) {
    p.create_goal(&ProjectGoalRecord {
        id: goal.to_owned(),
        title: "goal".into(),
        detail_md: None,
        status: ProjectGoalStatus::Active,
        sort: 2,
        created_at: ts,
        updated_at: ts,
    })
    .await
    .unwrap();
    p.create_goal(&ProjectGoalRecord {
        id: goal2.to_owned(),
        title: "goal two".into(),
        detail_md: None,
        status: ProjectGoalStatus::Active,
        sort: 1,
        created_at: ts + 1,
        updated_at: ts + 1,
    })
    .await
    .unwrap();
    let ok = p
        .patch_goal(
            goal,
            &ProjectGoalPatch {
                title: Some("renamed".into()),
                detail_md: Some("details".into()),
                status: Some(ProjectGoalStatus::Active),
                sort: Some(2),
            },
            ts + 2,
        )
        .await
        .unwrap();
    assert!(ok, "patch of a live goal reports true");
    assert!(
        !p.patch_goal(&format!("missing-{goal}"), &ProjectGoalPatch::default(), ts)
            .await
            .unwrap(),
        "patch of a missing goal reports false"
    );
    let goals_ok = || async {
        let goals = p.list_goals().await.unwrap();
        let Some(mine) = goals.iter().find(|g| g.id == goal) else {
            return false;
        };
        mine.title == "renamed"
            && mine.detail_md.as_deref() == Some("details")
            && mine.updated_at == ts + 2
            && match (
                goals.iter().position(|g| g.id == goal2),
                goals.iter().position(|g| g.id == goal),
            ) {
                (Some(p2), Some(p1)) => p2 < p1,
                _ => false,
            }
    };
    eventually("goal patch visible + list order", goals_ok).await;
}
