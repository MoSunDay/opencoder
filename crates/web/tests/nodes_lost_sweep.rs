//! Lost-node sweep wired into the read path (`GET /api/nodes`), plan T2.
//!
//! A running task whose node stopped heartbeating must be converged by the
//! registry read itself: task → `error("node lost")` (+ terminal closure event
//! persisted + fanned out on the NodeHub), node busy bit released, so a live
//! worker (or this test playing one) can claim again with zero code changes.
//!
//! The stale heartbeat is backfilled through a SECOND connection to a
//! file-backed db (`LibsqlStore::conn()` clones share the same underlying
//! database) because in-memory dbs cannot be reached cross-connection.
//! Server harness mirrors `node_e2e_support::spawn_server`, reusing its HTTP /
//! SSE helpers against the SAME signature token.

mod node_e2e_support;
mod support;

use std::sync::Arc;
use std::time::Duration;

use libsql::params;
use opencoder_llm::MockChatClient;
use opencoder_store::{LibsqlStore, Store};

use node_e2e_support::{collect_until, get_json, open_sse, post_json, wait_for, TOKEN};

/// Heartbeat age written during backfill: far beyond [`STALE_AFTER_MS`] so
/// wall-clock jitter between UPDATE and GET cannot un-stale the row.
const AGE_MS: i64 = 4 * opencoder_web::nodes_state::STALE_AFTER_MS;

struct Srv {
    base: String,
    store: Arc<dyn Store>,
    /// Second connection into the SAME file-backed database (backfill path).
    raw: libsql::Connection,
    shutdown: tokio::sync::watch::Sender<bool>,
    _dir: tempfile::TempDir,
}

impl Drop for Srv {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// `node_e2e_support::spawn_server`, but backed by an on-disk libsql file so
/// the test can reach the rows from a second connection.
async fn spawn_file_server() -> Srv {
    let dir = tempfile::tempdir().unwrap();
    let ls = LibsqlStore::open(dir.path().join("sweep.db"))
        .await
        .unwrap();
    let raw = ls.conn().await.unwrap();
    let _ = raw.busy_timeout(Duration::from_secs(5));
    let store: Arc<dyn Store> = Arc::new(ls);
    let state = Arc::new(opencoder_web::AppState {
        store: Arc::clone(&store),
        workdir: std::env::temp_dir(),
        handles: opencoder_web::handle::new_handle_map(),
        nodes: Arc::new(opencoder_web::nodes_state::NodeHub::new()),
        controls: Arc::new(opencoder_web::control_state::ControlHub::new()),
        client_override: Some(Arc::new(MockChatClient::new())),
    });
    let app = opencoder_web::build_app(state, Some(TOKEN.to_string()), true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let graceful = axum::serve(listener, app).with_graceful_shutdown(async move {
            let mut rx = rx;
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        });
        let _ = graceful.await;
    });
    Srv {
        base: format!("http://{addr}"),
        store,
        raw,
        shutdown: tx,
        _dir: dir,
    }
}

async fn register(base: &str, name: &str) -> String {
    let (_, b) = post_json(base, "/api/nodes/register", Some(serial(name))).await;
    b["node_id"].as_str().unwrap().to_string()
}

async fn dispatch(base: &str, node_id: &str, prompt: &str) -> (String, String) {
    let (_, d) = post_json(
        base,
        &format!("/api/nodes/{node_id}/tasks"),
        Some(serde_json::json!({ "prompt": prompt })),
    )
    .await;
    (
        d["task_id"].as_str().unwrap().to_string(),
        d["session_id"].as_str().unwrap().to_string(),
    )
}

/// The worker's own claiming surface — puts the task into `running`.
async fn worker_claim(base: &str, node_id: &str) -> Option<String> {
    let path = format!("/api/nodes/tasks/claim?node_id={node_id}");
    let (tsh, ts, sigh, sig) = support::sig_headers(TOKEN, "GET", &path, b"");
    let r = node_e2e_support::http()
        .get(format!("{base}{path}"))
        .header(tsh, ts)
        .header(sigh, sig)
        .send()
        .await
        .unwrap();
    if r.status().as_u16() == 204 {
        return None;
    }
    assert_eq!(r.status().as_u16(), 200, "claim must succeed or 204");
    let v: serde_json::Value = r.json().await.unwrap();
    Some(v["task"]["task_id"].as_str().unwrap().to_string())
}

fn serial(name: &str) -> serde_json::Value {
    serde_json::json!({ "name": name })
}

async fn backfill_last_seen(srv: &Srv, name: &str, ts: i64) {
    srv.raw
        .execute(
            "UPDATE nodes SET last_seen_at = ?1 WHERE name = ?2",
            params![ts, name],
        )
        .await
        .unwrap();
}

async fn error_frames(srv: &Srv, sid: &str) -> Vec<opencoder_store::SessionEventRecord> {
    srv.store
        .events_after(sid, -1)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.sse_kind.as_deref() == Some("error"))
        .collect()
}

// ── mandated chain ──────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sweep_converges_running_task_and_unblocks_queue() {
    let srv = spawn_file_server().await;

    // 1. register → dispatch t1 → claim (worker口径) ⇒ running.
    let node_id = register(&srv.base, "swept-node").await;
    let (t1, sid1) = dispatch(&srv.base, &node_id, "job one").await;
    assert_eq!(
        worker_claim(&srv.base, &node_id).await.as_deref(),
        Some(t1.as_str())
    );
    let pre = srv.store.get_node_task(&t1).await.unwrap().unwrap();
    assert_eq!(pre.status.as_str(), "running", "precondition");

    // 2. Backfill a stale heartbeat, then GET /api/nodes triggers the sweep.
    backfill_last_seen(&srv, "swept-node", now_ms() - AGE_MS).await;
    let (_, fleet) = get_json(&srv.base, "/api/nodes").await;
    let row = fleet["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == node_id.as_str())
        .expect("registered node listed")
        .clone();
    // Busy bit freed + still-stale timestamp ⇒ computed status is `lost`.
    assert_eq!(row["status"], "lost", "{row}");

    let t = srv.store.get_node_task(&t1).await.unwrap().unwrap();
    assert_eq!(t.status.as_str(), "error", "{t:?}");
    assert_eq!(t.error.as_deref(), Some("node lost"));
    assert!(t.finished_at.is_some(), "terminal stamp set");

    // 3. The synthetic session ends in a persisted error tail frame.
    let errs = error_frames(&srv, &sid1).await;
    assert_eq!(errs.len(), 1, "exactly one closure frame");
    assert_eq!(errs[0].payload["task_id"], t1.as_str());
    assert_eq!(errs[0].payload["error"], "node lost");
    assert_eq!(errs[0].payload["ok"], serde_json::json!(false));
    assert!(errs[0].payload.get("cancel").is_none());

    // 4. Zombie row no longer blocks the queue: next dispatch claims cleanly.
    let (t2, _) = dispatch(&srv.base, &node_id, "job two").await;
    let reclaimed = wait_for(10, 50, || worker_claim(&srv.base, &node_id)).await;
    assert_eq!(reclaimed, t2, "re-claim after sweep must return t2");

    // 5. Idempotent: a second GET re-sweeps nothing → no second error frame.
    let before = error_frames(&srv, &sid1).await.len();
    let before_total = srv.store.events_after(&sid1, -1).await.unwrap().len();
    get_json(&srv.base, "/api/nodes").await;
    let after_total = srv.store.events_after(&sid1, -1).await.unwrap().len();
    assert_eq!(after_total, before_total, "no new events of any kind");
    assert_eq!(error_frames(&srv, &sid1).await.len(), before);
    let again = srv.store.get_node_task(&t1).await.unwrap().unwrap();
    assert_eq!(again.status.as_str(), "error");
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ── optional: live SSE view closes on the swept frame ───────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_sse_view_receives_error_closure_during_sweep() {
    let srv = spawn_file_server().await;
    let node_id = register(&srv.base, "swept-live").await;
    let (tid, _sid) = dispatch(&srv.base, &node_id, "watched job").await;
    assert_eq!(
        worker_claim(&srv.base, &node_id).await.as_deref(),
        Some(tid.as_str())
    );

    // Attach FIRST (a browser would already be watching), then trigger the
    // sweeping registry read while the stream is parked on the hub.
    let mut sse = open_sse(&srv.base, &format!("/api/nodes/tasks/{tid}/events?after=0")).await;
    tokio::time::sleep(Duration::from_millis(300)).await; // let the hub attach

    let reader = tokio::spawn(async move { collect_until(&mut sse, |f| f.kind == "error").await });
    backfill_last_seen(&srv, "swept-live", now_ms() - AGE_MS).await;
    let (st, _) = get_json(&srv.base, "/api/nodes").await;
    assert_eq!(st.as_u16(), 200);
    let frames = reader.await.unwrap();

    let closure = frames
        .iter()
        .find(|f| f.kind == "error" && f.data["task_id"] == tid.as_str())
        .expect("live stream must end in the sweep's error closure");
    assert_eq!(closure.data["error"], "node lost");
    let t = srv.store.get_node_task(&tid).await.unwrap().unwrap();
    assert_eq!(t.status.as_str(), "error");
}
