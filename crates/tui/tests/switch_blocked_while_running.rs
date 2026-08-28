//! A control command submitted mid-flight NEVER interrupts the running turn:
//! the FIFO worker gate refuses it while busy, and the switch applies only
//! after the in-flight turn settles. Uses a hanging MockChatClient to hold
//! the LLM call open deterministically.

use std::sync::Arc;
use std::time::Duration;

use opencoder_core::{resolve_agent, Config};
use opencoder_llm::MockChatClient;
use opencoder_session::{SessionEvent, SessionState};
use opencoder_store::{LibsqlStore, SessionMeta, Store};
use opencoder_tui::worker::{gate_switch, process_cmd, SwitchGate, UiCmd, UiEvent};
use tokio::sync::{mpsc, Notify};

async fn mem_store() -> Arc<dyn Store> {
    Arc::new(LibsqlStore::open_memory().await.unwrap())
}

async fn act_session(id: &str, mock: Arc<MockChatClient>) -> SessionState {
    let store = mem_store().await;
    store
        .create_session(&SessionMeta {
            id: id.into(),
            agent: Some("act".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    SessionState::new(
        id,
        resolve_agent("act").unwrap(),
        Config {
            model: "m/g".into(),
            ..Config::default()
        },
        mock as Arc<dyn opencoder_llm::ChatStream>,
        std::env::temp_dir(),
    )
    .with_store(store)
    .mark_session_created()
}

async fn wait_for_call(mock: &MockChatClient, want: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while mock.call_count() < want {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the LLM call never started"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn drain(rx: &mut mpsc::Receiver<UiEvent>) -> Vec<UiEvent> {
    let mut events = Vec::new();
    let collect = async {
        while let Some(ev) = rx.recv().await {
            events.push(ev);
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(10), collect).await;
    events
}

/// While a turn is in flight the TUI gate refuses to start another one
/// (SkipRunning), and only after the in-flight turn settles does a
/// `/sandbox` submission apply — with no extra LLM call for the switch.
#[tokio::test]
async fn control_command_waits_for_inflight_turn_to_settle() {
    let release = Arc::new(Notify::new());
    let mock = Arc::new(MockChatClient::new().push_hang(release.clone()));
    let mut sess = act_session("blocked-switch", mock.clone()).await;

    let idle_gate = gate_switch(false);
    assert_eq!(idle_gate, SwitchGate::Run, "idle submissions start a turn");

    // Start the in-flight turn on a task so the test can observe it mid-air.
    let (tx, mut rx) = mpsc::channel::<UiEvent>(256);
    // The worker OWNS the session while the turn runs — the UI literally has
    // no `&mut SessionState` mid-flight (mirrored here by moving it into the
    // task), so nothing can switch the agent during the turn.
    let handle = tokio::spawn(async move {
        let quit = process_cmd(
            UiCmd::Prompt("long running task".into(), vec![]),
            &mut sess,
            &tx,
        )
        .await;
        (quit, sess)
    });
    wait_for_call(&mock, 1).await;

    // Mid-flight: the gate refuses a new turn. The UI queues the raw text for
    // the runner's idle boundary instead of starting a second process_cmd.
    assert_eq!(
        gate_switch(true),
        SwitchGate::SkipRunning,
        "a busy worker must refuse to start another turn"
    );

    // Release the hang: the turn settles (the empty stream surfaces as an
    // Error event) and the worker reports TurnDone.
    release.notify_waiters();
    let (quit, mut sess) = handle.await.expect("worker task joins");
    assert!(!quit, "the settled turn must not signal quit");
    assert_eq!(
        sess.agent.name, "act",
        "the turn must settle on the original agent"
    );
    let turn_events = drain(&mut rx).await;
    assert!(
        turn_events
            .iter()
            .any(|e| matches!(e, UiEvent::TurnDone(n) if n == "act")),
        "the in-flight turn must settle with TurnDone(act) before any switch"
    );
    assert!(
        !turn_events
            .iter()
            .any(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(_)))),
        "the mid-flight window must not emit a switch"
    );

    // Only now does the switch apply — and it applies without an LLM turn.
    let (tx2, mut rx2) = mpsc::channel::<UiEvent>(64);
    let quit = process_cmd(UiCmd::Prompt("/sandbox".into(), vec![]), &mut sess, &tx2).await;
    assert!(!quit);
    let switch_events = drain(&mut rx2).await;
    assert!(switch_events.iter().any(|e| matches!(
        e,
        UiEvent::Session(SessionEvent::AgentSwitch(ref n)) if n == "sandbox"
    )));
    assert_eq!(sess.agent.name, "sandbox");
    assert_eq!(
        mock.call_count(),
        1,
        "the switch itself must consume no LLM turn (still just the hung call)"
    );
}

/// FIFO ordering: the events of the in-flight turn are fully delivered before
/// the post-settle switch produces anything — a consumer never interleaves a
/// switch announcement into a streaming turn.
#[tokio::test]
async fn switch_events_never_interleave_a_running_turn() {
    let release = Arc::new(Notify::new());
    let mock = Arc::new(MockChatClient::new().push_hang(release.clone()));
    let mut sess = act_session("ordered-switch", mock.clone()).await;

    let (tx, mut rx) = mpsc::channel::<UiEvent>(256);
    let handle = tokio::spawn(async move {
        let quit = process_cmd(
            UiCmd::Prompt("streaming turn".into(), vec![]),
            &mut sess,
            &tx,
        )
        .await;
        (quit, sess)
    });
    wait_for_call(&mock, 1).await;
    assert_eq!(gate_switch(true), SwitchGate::SkipRunning);

    release.notify_waiters();
    let (_, mut sess) = handle.await.expect("worker task joins");
    let mut all = drain(&mut rx).await;

    // The switch arrives strictly after the turn's terminal TurnDone.
    let turn_done_pos = all
        .iter()
        .position(|e| matches!(e, UiEvent::TurnDone(_)))
        .expect("the turn ends with TurnDone");
    assert!(
        all[..turn_done_pos]
            .iter()
            .all(|e| !matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(_)))),
        "no AgentSwitch may precede the turn's TurnDone"
    );

    // Now run the queued switch through the same worker and append its stream.
    let (tx2, mut rx2) = mpsc::channel::<UiEvent>(64);
    let _ = process_cmd(UiCmd::Prompt("/sandbox".into(), vec![]), &mut sess, &tx2).await;
    all.extend(drain(&mut rx2).await);
    let switch_pos = all
        .iter()
        .position(|e| matches!(e, UiEvent::Session(SessionEvent::AgentSwitch(n)) if n == "sandbox"))
        .expect("the switch is announced after settle");
    assert!(
        switch_pos > turn_done_pos,
        "AgentSwitch(sandbox) must follow the settled turn's TurnDone"
    );
}
