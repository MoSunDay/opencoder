//! Reproducible synthetic workload; no production records or credentials.
use super::*;
use opencoder_core::fleet::{ExecutionKind, ExecutionStatus};

fn cpu_ticks() -> u64 {
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| {
            let fields = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .collect::<Vec<_>>();
            Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
        })
        .unwrap_or(0)
}

#[tokio::test]
async fn warmed_ownership_projection_avoids_large_assignment_json_work() {
    let benchmark = std::env::var_os("OPENCODER_CPU_BENCH").is_some();
    let (count, assignments, payload) = if benchmark {
        (20000, 4000, 131072)
    } else {
        (32, 16, 4096)
    };
    let store = FleetStore::open_memory().await.unwrap();
    store.db.write(move |conn| async move {
        let records = (0..count).map(|n| ExecutionIndex {
            id: format!("agent-perf-{n}"), created_at: n as i64, kind: ExecutionKind::Agent,
            node_id: "node-perf".into(), status: ExecutionStatus::Done,
        }).collect::<Vec<_>>();
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        for index in records.iter().take(assignments) {
            let body = serde_json::json!({"index":index,"request":{"input":{"payload":"x".repeat(payload)}}}).to_string();
            conn.execute("INSERT INTO execution_assignments VALUES(?1,?2)", params![index.id.clone(),body]).await?;
        }
        let encoded = serde_json::to_string(&records)?;
        rows::apply(&conn, &encoded).await?;
        conn.execute("COMMIT", ()).await?;
        let old_cpu = cpu_ticks();
        let old_start = std::time::Instant::now();
        // The previous ownership query alone, excluding its other report work.
        for _ in 0..3 {
            let mut rows = conn.query(
                "SELECT a.id FROM json_each(?1) incoming JOIN execution_assignments a
                 ON a.id=CAST(json_extract(incoming.value,'$.id') AS TEXT)
                 WHERE json_extract(a.assignment,'$.index.created_at')<>json_extract(incoming.value,'$.created_at')
                    OR json_extract(a.assignment,'$.index.kind')<>json_extract(incoming.value,'$.kind')
                    OR json_extract(a.assignment,'$.index.node_id')<>json_extract(incoming.value,'$.node_id') LIMIT 1", [encoded.as_str()]).await?;
            assert!(rows.next().await?.is_none());
        }
        let old_elapsed = old_start.elapsed();
        let old_cpu = cpu_ticks() - old_cpu;
        let new_cpu = cpu_ticks();
        let new_start = std::time::Instant::now();
        for _ in 0..3 {
            conn.execute("BEGIN IMMEDIATE", ()).await?;
            rows::apply(&conn, &encoded).await?;
            conn.execute("COMMIT", ()).await?;
        }
        let new_elapsed = new_start.elapsed();
        let new_cpu = cpu_ticks() - new_cpu;
        eprintln!("indexes={count} assignment_payload_bytes={} old_ms={} new_ms={} old_cpu_ticks={old_cpu} new_cpu_ticks={new_cpu}",
            assignments * payload, old_elapsed.as_millis(), new_elapsed.as_millis());
        if benchmark {
            assert!(new_elapsed.as_secs_f64() < old_elapsed.as_secs_f64() * 0.2,
                "ownership projection should remove at least 80% of repeated JSON work");
        }
        Ok(())
    }).await.unwrap();
}
