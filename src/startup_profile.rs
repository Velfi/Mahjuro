//! Opt-in startup timing — set `MAHJURO_STARTUP_PROFILE=1` (or any non-empty value).
//!
//! Named [`scope`] guards record elapsed time on drop. [`report_sync_boot`] and
//! [`note_async_boot_complete`] emit sorted `log::info!` tables (slowest first).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct Sample {
    name: &'static str,
    ms: f64,
}

static ENABLED: OnceLock<bool> = OnceLock::new();
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static SAMPLES: OnceLock<Mutex<Vec<Sample>>> = OnceLock::new();
static SYNC_REPORTED: AtomicBool = AtomicBool::new(false);
static ASYNC_REPORTED: AtomicBool = AtomicBool::new(false);

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("MAHJURO_STARTUP_PROFILE").is_some())
}

fn origin() -> Instant {
    *ORIGIN.get_or_init(Instant::now)
}

fn samples() -> &'static Mutex<Vec<Sample>> {
    SAMPLES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Record a duration for a named phase (e.g. async work finished on a later frame).
pub fn record(name: &'static str, elapsed: Duration) {
    if !enabled() {
        return;
    }
    let ms = elapsed.as_secs_f64() * 1000.0;
    if let Ok(mut v) = samples().lock() {
        v.push(Sample { name, ms });
    }
}

/// Open a named timing scope. No-op unless [`enabled`].
#[inline]
pub fn scope(name: &'static str) -> ScopeGuard {
    let _ = origin();
    ScopeGuard {
        name,
        start: Instant::now(),
    }
}

pub struct ScopeGuard {
    name: &'static str,
    start: Instant,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        record(self.name, self.start.elapsed());
    }
}

/// Wall-clock ms since the first [`scope`] (or this call if profiling started late).
pub fn wall_ms() -> f64 {
    if !enabled() {
        return 0.0;
    }
    origin().elapsed().as_secs_f64() * 1000.0
}

/// Sorted summary after synchronous init (App + WgpuRenderer + input).
pub fn report_sync_boot() {
    if !enabled() || SYNC_REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    emit_report("sync boot (through SDL main-loop init)");
}

/// Call once when relic/background async GPU uploads have finished.
pub fn note_async_boot_complete() {
    if !enabled() || ASYNC_REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    emit_report("async boot (relic + menu backdrop uploads)");
}

fn emit_report(phase: &str) {
    let wall = wall_ms();
    let mut rows: Vec<(String, f64)> = Vec::new();
    if let Ok(v) = samples().lock() {
        rows.reserve(v.len());
        for s in v.iter() {
            rows.push((s.name.to_string(), s.ms));
        }
    }
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    log::info!("── startup profile: {phase} (wall {wall:.0} ms) ──");
    for (name, ms) in &rows {
        log::info!("  {name:40} {ms:8.1} ms");
    }
    if rows.is_empty() {
        log::info!("  (no scoped samples recorded)");
    }
    log::info!("── end startup profile ──");
}
