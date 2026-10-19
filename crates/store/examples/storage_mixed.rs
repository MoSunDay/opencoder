//! Repeatable mixed read/write benchmark. Only uses a disposable database.
//! Run the release binary with an output directory and optional FULL durability.
use anyhow::{ensure, Result};
use opencoder_core::Message;
use opencoder_store::{
    Delivery, EventKind, SessionEventRecord, SessionFilter, SessionInput, SessionMeta, Store,
    TursoStore,
};
use serde_json::json;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

const SESSIONS: usize = 256;
const HISTORY: usize = 64;
const OPERATIONS: usize = 32768;

fn meta(id: String) -> SessionMeta {
    SessionMeta {
        id,
        title: Some("Benchmark session".into()),
        agent: None,
        model: None,
        autopilot_mode: None,
        workdir_hash: None,
        created_at: 0,
        updated_at: 0,
        summary: None,
        summary_seq: None,
        summary_images: vec![],
        handoff_seq: None,
        handoff_plan: None,
        skill: None,
        task_type: None,
        requirement: None,
        kind: None,
    }
}

async fn fixture(path: &Path, full: bool) -> Result<()> {
    let store = TursoStore::open(path).await?;
    if full {
        store
            .conn()
            .await?
            .execute("PRAGMA synchronous=FULL", ())
            .await?;
    }
    for index in 0..SESSIONS {
        let id = format!("session-{index:04}");
        store.create_session(&meta(id.clone())).await?;
        let messages = (0..HISTORY)
            .map(|i| Message::user(format!("seed-{index}-{i}"), "x".repeat(2048)))
            .collect::<Vec<_>>();
        store.append_messages(&id, &messages).await?;
        let events = (0..HISTORY)
            .map(|i| SessionEventRecord {
                session_id: id.clone(),
                kind: EventKind::TextDelta,
                payload: json!({"text": "e".repeat(256)}),
                ts: i as i64,
                seq: None,
                sse_kind: None,
            })
            .collect::<Vec<_>>();
        store.append_events(&events).await?;
    }
    // Every claim in the timed workload has an input; empty polls cannot mask cost.
    for operation in 0..OPERATIONS {
        if operation % 10 != 2 {
            continue;
        }
        store
            .admit_input(&SessionInput {
                seq: None,
                id: format!("input-{operation}"),
                session_id: format!("session-{:04}", operation % SESSIONS),
                delivery: Delivery::Queue,
                prompt: "benchmark input".into(),
                images: vec![],
                display_text: None,
                admitted_seq: operation as i64,
                promoted_seq: None,
            })
            .await?;
    }
    let mut rows = store
        .conn()
        .await?
        .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
        .await?;
    while rows.next().await?.is_some() {}
    Ok(())
}

async fn operation(store: &TursoStore, index: usize) -> Result<()> {
    let id = format!("session-{:04}", index % SESSIONS);
    match index % 10 {
        0 => {
            store
                .append_message(
                    &id,
                    &Message::user(format!("write-{index}"), "w".repeat(2048)),
                )
                .await?;
        }
        1 => {
            store
                .append_event(&SessionEventRecord {
                    session_id: id,
                    kind: EventKind::TextDelta,
                    payload: json!({"text": "v".repeat(256)}),
                    ts: index as i64,
                    seq: None,
                    sse_kind: None,
                })
                .await?;
        }
        2 => {
            ensure!(
                store.claim_next_queue(&id).await?.is_some(),
                "claim lost input"
            );
        }
        3 | 4 => {
            ensure!(
                store
                    .list_sessions(&SessionFilter {
                        limit: 50,
                        ..Default::default()
                    })
                    .await?
                    .len()
                    == 50
            );
        }
        5..=7 => {
            ensure!(store.load_messages(&id).await?.len() >= HISTORY);
        }
        _ => {
            ensure!(store.events_after(&id, 0).await?.len() >= HISTORY);
        }
    }
    Ok(())
}

fn percentile(values: &mut [f64], percentile: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() - 1) as f64 * percentile).ceil() as usize]
}

fn cpu_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}

fn rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

async fn trial(seed: &Path, concurrency: usize, repetition: usize, full: bool) -> Result<()> {
    let dir = tempfile::tempdir_in(seed.parent().unwrap())?;
    let path = dir.path().join("trial.db");
    std::fs::copy(seed, &path)?;
    let store = Arc::new(TursoStore::open(&path).await?);
    if full {
        store
            .conn()
            .await?
            .execute("PRAGMA synchronous=FULL", ())
            .await?;
    }
    // Warm the same reads before each trial.
    for index in [3, 5, 8] {
        operation(&store, index).await?;
    }
    let mut pragma = store.conn().await?.query("PRAGMA synchronous", ()).await?;
    let synchronous: i64 = pragma.next().await?.unwrap().get(0)?;
    drop(pragma);
    let barrier = Arc::new(tokio::sync::Barrier::new(concurrency + 1));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = stop.clone();
    let heartbeat = tokio::spawn(async move {
        let mut lags = Vec::new();
        while !signal.load(std::sync::atomic::Ordering::Relaxed) {
            let expected = tokio::time::Instant::now() + Duration::from_millis(10);
            tokio::time::sleep_until(expected).await;
            lags.push(
                tokio::time::Instant::now()
                    .saturating_duration_since(expected)
                    .as_secs_f64()
                    * 1000.0,
            );
        }
        lags
    });
    let mut workers = Vec::new();
    for worker in 0..concurrency {
        let store = store.clone();
        let barrier = barrier.clone();
        workers.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            barrier.wait().await;
            for index in (worker..OPERATIONS).step_by(concurrency) {
                let start = Instant::now();
                operation(&store, index).await?;
                latencies.push((index % 10, start.elapsed().as_secs_f64() * 1000.0));
            }
            Ok::<_, anyhow::Error>(latencies)
        }));
    }
    let cpu_before = cpu_ticks();
    let start = Instant::now();
    barrier.wait().await;
    let mut latencies = Vec::new();
    for task in workers {
        latencies.extend(task.await??);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let cpu_used = cpu_ticks()
        .zip(cpu_before)
        .map(|(after, before)| after - before);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut lag = heartbeat.await?;
    ensure!(latencies.len() == OPERATIONS);
    let mut by_operation = serde_json::Map::new();
    for (name, kinds) in [
        ("message", vec![0]),
        ("event", vec![1]),
        ("claim", vec![2]),
        ("list", vec![3, 4]),
        ("history", vec![5, 6, 7]),
        ("events", vec![8, 9]),
    ] {
        let mut values = latencies
            .iter()
            .filter(|(kind, _)| kinds.contains(kind))
            .map(|(_, ms)| *ms)
            .collect::<Vec<_>>();
        by_operation.insert(name.into(), json!(percentile(&mut values, 0.95)));
    }
    let mut latencies = latencies.into_iter().map(|(_, ms)| ms).collect::<Vec<_>>();
    println!(
        "{}",
        json!({
            "engine": store.backend_name(), "concurrency": concurrency, "repetition": repetition,
            "synchronous": synchronous, "sessions": SESSIONS, "history": HISTORY,
            "operations": OPERATIONS, "seconds": elapsed, "ops_per_second": OPERATIONS as f64 / elapsed,
            "p50_ms": percentile(&mut latencies, 0.50), "p95_ms": percentile(&mut latencies, 0.95),
            "p99_ms": percentile(&mut latencies, 0.99), "cpu_ticks": cpu_used, "peak_rss_kib": rss_kib(),
            "operation_p95_ms": by_operation,
            "runtime_lag_p99_ms": if lag.is_empty() { 0.0 } else { percentile(&mut lag, 0.99) },
        })
    );
    Ok(())
}

#[tokio::main(worker_threads = 4)]
async fn main() -> Result<()> {
    let output = std::env::args().nth(1).ok_or_else(|| {
        anyhow::anyhow!(
            "usage: storage_mixed OUTPUT_DIRECTORY [FULL|DEFAULT] [CHECKPOINTED_FIXTURE] [CONCURRENCY] [REPETITION]"
        )
    })?;
    let full = std::env::args().nth(2).as_deref() == Some("FULL");
    let output = Path::new(&output);
    std::fs::create_dir_all(output)?;
    let seed = output.join("fixture.db");
    ensure!(!seed.exists(), "use a new output directory");
    if let Some(source) = std::env::args().nth(3) {
        std::fs::copy(source, &seed)?;
    } else {
        fixture(&seed, full).await?;
    }
    let selected_concurrency = std::env::args()
        .nth(4)
        .map(|v| v.parse::<usize>())
        .transpose()?;
    let selected_repetition = std::env::args()
        .nth(5)
        .map(|v| v.parse::<usize>())
        .transpose()?;
    ensure!(
        selected_concurrency.is_none_or(|v| [1, 8, 32].contains(&v)),
        "invalid concurrency"
    );
    ensure!(
        selected_repetition.is_none_or(|v| (1..=3).contains(&v)),
        "invalid repetition"
    );
    for concurrency in [1, 8, 32] {
        if selected_concurrency.is_some_and(|v| v != concurrency) {
            continue;
        }
        for repetition in 1..=3 {
            if selected_repetition.is_some_and(|v| v != repetition) {
                continue;
            }
            eprintln!("starting concurrency={concurrency} repetition={repetition}");
            trial(&seed, concurrency, repetition, full).await?;
        }
    }
    Ok(())
}
