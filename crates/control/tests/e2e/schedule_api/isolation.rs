use super::*;
use std::sync::{atomic::Ordering, Arc};

#[tokio::test]
async fn slow_admission_does_not_block_other_schedules_or_duplicate_its_tick() {
    let h = Harness::new().await;
    write_fast_scan(&h);
    let gate = Arc::new(tokio::sync::Notify::new());
    *h.node.create_gate.lock().unwrap() = Some(("agent-a_slow-".into(), gate.clone()));
    for id in ["a_slow", "z_fast"] {
        let (status, reply) = create_schedule(
            &h,
            json!({"id":id,"cron":USER_AGENT_CRON,"kind":"agent","target":"act",
                   "overlap":"allow"}),
        )
        .await;
        assert_eq!(status, 200, "{reply}");
    }
    tokio::time::timeout(Duration::from_secs(25), async {
        while h.node.gated_creates.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("slow schedule reached node admission");
    let before = h.state.lifecycle.scheduler.snapshot().scans_total;
    tokio::time::timeout(Duration::from_secs(30), async {
        while h.state.lifecycle.scheduler.snapshot().scans_total < before + 2
            || fired_count(&h, "z_fast").await < 2
        {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("healthy schedule must keep firing while another admission is blocked");
    assert_eq!(h.node.gated_creates.load(Ordering::SeqCst), 1);
    assert_eq!(fired_count(&h, "a_slow").await, 0);
    let before_release = runs_of(&h, "a_slow").await;
    let previous = before_release["runs"].as_array().unwrap();
    assert!(previous.iter().all(|row| row["status"] == "missed"));
    for id in ["a_slow", "z_fast"] {
        assert_eq!(
            h.req(
                reqwest::Method::PATCH,
                &format!("/api/schedules/{id}"),
                Some(json!({"enabled":false}))
            )
            .await
            .0,
            200
        );
    }
    gate.notify_one();
    let rows = poll_runs(&h, "a_slow", |body| {
        body["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["status"] == "fired")
    })
    .await;
    assert_eq!(
        rows["runs"].as_array().unwrap().len(),
        previous.len() + 1,
        "release must add exactly one fire without changing the catch-up history: {rows}"
    );
    assert_eq!(fired_count(&h, "a_slow").await, 1);
}
