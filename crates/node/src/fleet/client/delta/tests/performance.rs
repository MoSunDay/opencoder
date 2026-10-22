use super::*;
use opencoder_core::fleet::inventory::InventoryLog;

#[test]
#[ignore = "explicit local performance comparison"]
fn incremental_source_cost_does_not_grow_with_idle_history() {
    let mut previous_p95 = None;
    for count in [2_000, 20_000, 100_000] {
        let records: Vec<_> = (0..count).map(record).collect();
        let mut previous = DeltaState::default();
        previous.prepare(&mut report(records.clone(), 1)).unwrap();
        previous.commit();
        let mut log = InventoryLog::default();
        for row in &records {
            log.set(row.clone());
        }
        let mut current = DeltaState::default();
        current
            .prepare_source(&report(records.clone(), 1), log.revision(), true, vec![])
            .unwrap();
        current.commit();
        let mut before = Vec::new();
        let mut after = Vec::new();
        for sequence in 2..32 {
            let started = Instant::now();
            assert!(previous
                .prepare(&mut report(records.clone(), sequence))
                .unwrap()
                .is_none());
            before.push(started.elapsed());
            let started = Instant::now();
            let changes = log.read(current.source_cursor());
            assert!(current
                .prepare_source(
                    &report(changes.records, sequence),
                    changes.revision,
                    changes.full,
                    changes.removed_ids
                )
                .unwrap()
                .is_none());
            after.push(started.elapsed());
        }
        before.sort();
        after.sort();
        let b = before[28];
        let c = after[28];
        eprintln!(
            "inventory_idle count={count} baseline_p95_us={} source_p95_us={}",
            b.as_micros(),
            c.as_micros()
        );
        assert!(
            c.as_nanos() * 2 < b.as_nanos(),
            "idle source must improve at least 50%"
        );
        if let Some(p95) = previous_p95 {
            assert!(c <= std::cmp::max(p95, Duration::from_millis(1)).mul_f64(1.2));
        }
        previous_p95 = Some(c);

        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut changed = records;
        for sequence in 32..62 {
            changed[0].created_at = sequence as i64;
            let started = Instant::now();
            assert!(previous
                .prepare(&mut report(changed.clone(), sequence))
                .unwrap()
                .is_some());
            previous.commit();
            before.push(started.elapsed());
            let started = Instant::now();
            log.set(changed[0].clone());
            let changes = log.read(current.source_cursor());
            assert_eq!(changes.records.len(), 1);
            assert!(current
                .prepare_source(
                    &report(changes.records, sequence),
                    changes.revision,
                    changes.full,
                    changes.removed_ids
                )
                .unwrap()
                .is_some());
            current.commit();
            after.push(started.elapsed());
        }
        before.sort();
        after.sort();
        eprintln!(
            "inventory_one_change count={count} baseline_p95_us={} source_p95_us={}",
            before[28].as_micros(),
            after[28].as_micros()
        );
        assert!(
            after[28].as_nanos() * 2 < before[28].as_nanos(),
            "low-change source must improve at least 50%"
        );
    }
}
