//! Process-local counters for the control-plane cron loop.
use serde::Serialize;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

#[derive(Default)]
pub struct SchedulerTelemetry {
    scans: AtomicU64,
    scan_errors: AtomicU64,
    last_scan_started_ms: AtomicI64,
    last_scan_completed_ms: AtomicI64,
    fire_attempts: AtomicU64,
    fire_errors: AtomicU64,
    missed_ticks: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchedulerCounters {
    pub scans_total: u64,
    pub scan_errors_total: u64,
    pub last_scan_started_ms: i64,
    pub last_scan_completed_ms: i64,
    pub fire_attempts_total: u64,
    pub fire_errors_total: u64,
    pub missed_ticks_total: u64,
}

impl SchedulerTelemetry {
    pub fn scan_started(&self, now_ms: i64) {
        self.scans.fetch_add(1, Ordering::Relaxed);
        self.last_scan_started_ms.store(now_ms, Ordering::Relaxed);
    }

    pub fn scan_completed(&self, now_ms: i64) {
        self.last_scan_completed_ms.store(now_ms, Ordering::Relaxed);
    }

    pub fn scan_error(&self) {
        self.scan_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn fire_attempt(&self) {
        self.fire_attempts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn fire_error(&self) {
        self.fire_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn missed_tick(&self) {
        self.missed_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> SchedulerCounters {
        SchedulerCounters {
            scans_total: self.scans.load(Ordering::Relaxed),
            scan_errors_total: self.scan_errors.load(Ordering::Relaxed),
            last_scan_started_ms: self.last_scan_started_ms.load(Ordering::Relaxed),
            last_scan_completed_ms: self.last_scan_completed_ms.load(Ordering::Relaxed),
            fire_attempts_total: self.fire_attempts.load(Ordering::Relaxed),
            fire_errors_total: self.fire_errors.load(Ordering::Relaxed),
            missed_ticks_total: self.missed_ticks.load(Ordering::Relaxed),
        }
    }
}
