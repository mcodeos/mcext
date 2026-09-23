//! Parse debounce scheduler
//!
//! Every `did_change` triggers, but mcc parsing is O(file_size) with significant overhead.
//! Here we implement simple debounce using per-URI sequence number:
//!
//! ```text
//! t=0   did_change  → seq=1
//! t=50  did_change  → seq=2   (overwrites seq=1)
//! t=100 did_change  → seq=3   (overwrites seq=2)
//! t=250            → check seq=3 == current seq=3 → trigger parse
//! ```
//!
//! Using sequence instead of oneshot avoids cancel race (see `ReparseScheduler`).

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower_lsp::lsp_types::Url;

/// Reparse scheduler
///
/// Each URI maintains a sequence number:
/// - `schedule(uri, on_fire)` increments seq, sleeps `debounce`
/// - After waking, checks seq: if still its own, calls on_fire; otherwise overwritten, discarded
///
/// Not a lazy timer, but optimistic locking based on seq — avoids race condition during cancel.
///
/// The debounce is a shared cell, not a fixed field: `did_change_configuration`
/// can retune it while the server is live and every later `schedule` picks up
/// the new value without rebuilding the scheduler (pending sequence numbers
/// stay valid across the change).
#[derive(Debug, Clone)]
pub struct ReparseScheduler {
    sequences: Arc<DashMap<Url, Arc<AtomicU64>>>,
    debounce_ms: Arc<AtomicU64>,
}

impl ReparseScheduler {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            sequences: Arc::new(DashMap::new()),
            debounce_ms: Arc::new(AtomicU64::new(debounce_ms)),
        }
    }

    pub fn debounce(&self) -> Duration {
        Duration::from_millis(self.debounce_ms.load(Ordering::SeqCst))
    }

    /// Retune the debounce live (did_change_configuration).
    pub fn set_debounce_ms(&self, ms: u64) {
        self.debounce_ms.store(ms, Ordering::SeqCst);
    }

    /// Schedule a callback
    ///
    /// - If same URI already has pending, this seq replaces the old one
    /// - `on_fire` executes in spawned tokio task (after `debounce`)
    pub fn schedule<F>(&self, uri: Url, on_fire: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let seq = self
            .sequences
            .entry(uri.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        let my_seq = seq.fetch_add(1, Ordering::SeqCst) + 1;

        let sequences = Arc::clone(&self.sequences);
        let debounce_ms = Arc::clone(&self.debounce_ms);
        let uri_for_task = uri.clone();

        tokio::spawn(async move {
            // Read at fire-scheduling time: a config change after this point
            // applies to the next edit, not retroactively to this one.
            let debounce = Duration::from_millis(debounce_ms.load(Ordering::SeqCst));
            tokio::time::sleep(debounce).await;
            // After waking, check: if current seq == my seq, I am the latest
            let current = sequences
                .get(&uri_for_task)
                .map(|s| s.load(Ordering::SeqCst))
                .unwrap_or(0);
            if current == my_seq {
                // I won, execute
                on_fire();
            }
            // Otherwise overwritten by newer, discard
        });
    }

    /// Fire immediately (don't wait for debounce). Used for scenarios like did_save that "must be now".
    pub fn fire_immediately<F>(&self, uri: Url, on_fire: F)
    where
        F: FnOnce() + Send + 'static,
    {
        // Increment seq to make all pending tasks discard
        let seq = self
            .sequences
            .entry(uri.clone())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        seq.fetch_add(1, Ordering::SeqCst);
        on_fire();
    }

    /// Remove URI's seq record (on did_close)
    pub fn remove(&self, uri: &Url) {
        self.sequences.remove(uri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AOrd};
    use std::time::Duration;

    #[tokio::test]
    async fn debounce_coalesces_rapid_calls() {
        let scheduler = ReparseScheduler::new(50);
        let counter = Arc::new(AtomicUsize::new(0));
        let url = Url::parse("file:///test.mc").unwrap();

        // Rapidly schedule 5 times in succession
        for _ in 0..5 {
            let counter = Arc::clone(&counter);
            scheduler.schedule(url.clone(), move || {
                counter.fetch_add(1, AOrd::SeqCst);
            });
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        // 等 debounce 过
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should only trigger once
        assert_eq!(counter.load(AOrd::SeqCst), 1);
    }

    #[tokio::test]
    async fn fire_immediately_runs_now() {
        let scheduler = ReparseScheduler::new(60_000); // Long debounce
        let counter = Arc::new(AtomicUsize::new(0));
        let url = Url::parse("file:///test.mc").unwrap();

        let counter2 = Arc::clone(&counter);
        scheduler.fire_immediately(url.clone(), move || {
            counter2.fetch_add(1, AOrd::SeqCst);
        });

        // Execute immediately, don't wait for debounce
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(counter.load(AOrd::SeqCst), 1);
    }

    #[tokio::test]
    async fn remove_clears_state() {
        let scheduler = ReparseScheduler::new(50);
        let url = Url::parse("file:///test.mc").unwrap();
        scheduler.schedule(url.clone(), || {});
        scheduler.remove(&url);
        // Should not panic; subsequent schedule still works
        scheduler.schedule(url.clone(), || {});
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn set_debounce_ms_applies_to_next_schedule() {
        let scheduler = ReparseScheduler::new(60_000);
        let counter = Arc::new(AtomicUsize::new(0));
        let url = Url::parse("file:///test.mc").unwrap();

        // Retune down live: the next schedule fires on the new value even
        // though the scheduler was built with a long one.
        scheduler.set_debounce_ms(20);
        let counter2 = Arc::clone(&counter);
        scheduler.schedule(url.clone(), move || {
            counter2.fetch_add(1, AOrd::SeqCst);
        });
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(counter.load(AOrd::SeqCst), 1);
    }
}
