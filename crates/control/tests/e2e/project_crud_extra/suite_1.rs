use super::*;

#[tokio::test]
async fn goals_list_order_shape_and_trimmed_create() {
    let h = Harness::new().await;
    let first = post_goal(&h, "First", None).await;
    assert_eq!(first["sort"], json!(0));
    assert_eq!(first["status"], json!("active"));
    assert!(first["id"].as_str().unwrap().starts_with("pg-"));

    // Title is trimmed; sort echoes back; the goal starts active.
    let second = post_goal(&h, "  Padded  ", Some(5)).await;
    assert_eq!(second["title"], json!("Padded"));
    assert_eq!(second["sort"], json!(5));
    assert_eq!(second["status"], json!("active"));
    assert!(second["id"].as_str().unwrap().starts_with("pg-"));
    assert_eq!(second["detail_md"], json!(null));

    // List: two goals in `sort` order, each a full record wire form.
    let rows = goals(&h).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], first["id"]);
    assert_eq!(rows[0]["title"], json!("First"));
    assert_eq!(rows[1]["id"], second["id"]);
    for key in [
        "id",
        "title",
        "detail_md",
        "status",
        "sort",
        "created_at",
        "updated_at",
    ] {
        assert!(rows[0].get(key).is_some(), "{key} missing: {}", rows[0]);
    }
}

#[tokio::test]
async fn goal_patch_validation_and_persistence() {
    let h = Harness::new().await;
    let goal = post_goal(&h, "G", None).await;
    let id = goal["id"].as_str().unwrap();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/goals/{id}"),
            Some(json!({"title": "   "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body["error"].as_str().unwrap().contains("empty"), "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/goals/pg-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Real fields persist across a re-read.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/goals/{id}"),
            Some(json!({"title": " G2 ", "detail_md": "d2", "sort": 7})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let rows = goals(&h).await;
    let patched = row(&rows, id);
    assert_eq!(patched["title"], json!("G2"));
    assert_eq!(patched["detail_md"], json!("d2"));
    assert_eq!(patched["sort"], json!(7));

    let (status, body) = h
        .req(Method::DELETE, "/api/project/goals/pg-none", None)
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn goal_delete_detaches_initiatives_and_preserves_todos() {
    let h = Harness::new().await;
    let goal = post_goal(&h, "G", None).await;
    let goal_id = goal["id"].as_str().unwrap().to_string();
    let initiative = post_initiative(&h, &goal_id, "M").await;
    let initiative_id = initiative["id"].as_str().unwrap().to_string();
    post_todo(&h, Some(&initiative_id), "T").await;

    let (status, body) = h
        .req(
            Method::DELETE,
            &format!("/api/project/goals/{goal_id}"),
            None,
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["deleted"], json!(true));

    assert!(goals(&h).await.is_empty());
    assert_eq!(initiatives(&h, None).await.len(), 1);
    assert!(initiatives(&h, None).await[0]["goal_id"].is_null());
    assert_eq!(todos(&h).await.len(), 1, "goal delete preserves TODOs");
}

#[tokio::test]
async fn initiative_create_validation_and_list_filters() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id": "pg-none", "title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"goal_id": "pg-none", "title": "M"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("pg-none"),
        "{body}"
    );

    let goal = post_goal(&h, "G", None).await;
    let goal_id = goal["id"].as_str().unwrap();
    let first = post_initiative(&h, goal_id, "M1").await;
    post_initiative(&h, goal_id, "M2").await;
    assert!(first["id"].as_str().unwrap().starts_with("pi-"));
    assert_eq!(first["status"], json!("planned"));

    assert_eq!(initiatives(&h, None).await.len(), 2);
    assert_eq!(initiatives(&h, Some("pg-none")).await.len(), 0);
    assert_eq!(initiatives(&h, Some(goal_id)).await.len(), 2);
}

#[tokio::test]
async fn initiative_patch_status_sort_and_reparent() {
    let h = Harness::new().await;
    let g1 = post_goal(&h, "G1", None).await;
    let g2 = post_goal(&h, "G2", None).await;
    let g1_id = g1["id"].as_str().unwrap().to_string();
    let g2_id = g2["id"].as_str().unwrap().to_string();
    let m = post_initiative(&h, &g1_id, "M").await;
    let m_id = m["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/initiatives/{m_id}"),
            Some(json!({"status": "in_progress", "sort": 2})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let rows = initiatives(&h, Some(&g1_id)).await;
    let patched = row(&rows, &m_id);
    assert_eq!(patched["status"], json!("in_progress"));
    assert_eq!(patched["sort"], json!(2));

    // Re-parenting to another real goal moves the row.
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/initiatives/{m_id}"),
            Some(json!({"goal_id": g2_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(initiatives(&h, Some(&g1_id)).await.len(), 0);
    let moved = initiatives(&h, Some(&g2_id)).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(row(&moved, &m_id)["goal_id"], json!(g2_id));

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/initiatives/{m_id}"),
            Some(json!({"goal_id": "pg-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn todo_create_validation_defaults_and_backlog_listing() {
    let h = Harness::new().await;
    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"title": " ", "draft": "d"})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::POST,
            "/api/project/todos",
            Some(json!({"initiative_id": "pi-none", "title": "T", "draft": "d"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("pi-none"),
        "{body}"
    );

    // Absent agent defaults to `act`; absent initiative ⇒ backlog.
    let backlog = post_todo(&h, None, "B1").await;
    assert!(backlog["id"].as_str().unwrap().starts_with("pt-"));
    assert_eq!(backlog["agent"], json!("act"));
    assert_eq!(backlog["status"], json!("draft"));
    assert_eq!(backlog["initiative_id"], json!(null));

    let goal = post_goal(&h, "G", None).await;
    let initiative = post_initiative(&h, goal["id"].as_str().unwrap(), "M").await;
    post_todo(&h, Some(initiative["id"].as_str().unwrap()), "T1").await;

    let rows = todos(&h).await;
    assert_eq!(rows.len(), 2, "unfiltered list must include the backlog");
    assert!(rows.iter().any(|r| r["id"] == backlog["id"]));
}

#[tokio::test]
async fn todo_patch_real_fields_and_reparent() {
    let h = Harness::new().await;
    let todo = post_todo(&h, None, "T").await;
    let id = todo["id"].as_str().unwrap().to_string();

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"title": "renamed", "draft": "d2", "agent": "build"})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], json!(true));
    let rows = todos(&h).await;
    let patched = row(&rows, &id);
    assert_eq!(patched["title"], json!("renamed"));
    assert_eq!(patched["draft"], json!("d2"));
    assert_eq!(patched["agent"], json!("build"));

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"title": "  "})),
        )
        .await;
    assert_eq!(status, 400, "{body}");

    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"initiative_id": "pi-none"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");

    // Re-parent into a real initiative persists (backlog → initiative).
    let goal = post_goal(&h, "G", None).await;
    let initiative = post_initiative(&h, goal["id"].as_str().unwrap(), "M").await;
    let m_id = initiative["id"].as_str().unwrap().to_string();
    let (status, body) = h
        .req(
            Method::PATCH,
            &format!("/api/project/todos/{id}"),
            Some(json!({"initiative_id": m_id})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    let rows = todos(&h).await;
    assert_eq!(row(&rows, &id)["initiative_id"], json!(m_id));

    let (status, body) = h
        .req(
            Method::PATCH,
            "/api/project/todos/pt-none",
            Some(json!({"title": "x"})),
        )
        .await;
    assert_eq!(status, 404, "{body}");
}

#[tokio::test]
async fn standalone_initiatives_appear_in_overview_and_guard_todos() {
    let h = Harness::new().await;
    let (status, initiative) = h
        .req(
            Method::POST,
            "/api/project/initiatives",
            Some(json!({"title":"专项"})),
        )
        .await;
    assert_eq!(status, 200, "{initiative}");
    assert!(initiative["goal_id"].is_null());
    let id = initiative["id"].as_str().unwrap();
    let todo = post_todo(&h, Some(id), "任务").await;
    let (status, overview) = h.req(Method::GET, "/api/project/overview", None).await;
    assert_eq!(status, 200, "{overview}");
    assert_eq!(
        overview["standalone_initiatives"][0]["todos"][0]["id"],
        todo["id"]
    );
    let (status, _) = h
        .req(
            Method::DELETE,
            &format!("/api/project/initiatives/{id}"),
            None,
        )
        .await;
    assert_eq!(status, 409);
    let goal = post_goal(&h, "项目", None).await;
    for body in [
        json!({"goal_id":goal["id"]}),
        json!({"goal_id":null}),
        json!({"title":"改名"}),
    ] {
        let (status, body) = h
            .req(
                Method::PATCH,
                &format!("/api/project/initiatives/{id}"),
                Some(body),
            )
            .await;
        assert_eq!(status, 200, "{body}");
    }
    assert!(initiatives(&h, None).await[0]["goal_id"].is_null());
}
