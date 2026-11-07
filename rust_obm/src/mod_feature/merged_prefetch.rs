//! Merged prefetch: guesses the next segment from real requests and renders it early, merged into
//! the coalescer (the prefetch is the key's leader, real requests for it follow).
//!
//! Prediction rule "2 continue" (no per-user tracking, the system has no user id): when segment
//! k-1 and segment k of the same show + combination arrive within `window_s` of each other (from
//! any users), segment k+1 is prefetched. Only real requests feed the rule; a prefetch never
//! triggers another one. In the viewer model a viewer past segment 0 continues with the same
//! combination with p 0.72 (after segment 0 only 0.20), which is what the k-1 + k condition picks.
//!
//! Life of a prefetch (driven by the main server, which does the I/O):
//! 1. a real request -> [`MergedPrefetch::observe`] -> target path (or None);
//! 2. skip if the target is cached or already in the coalescer table;
//! 3. `Coalescer::lead_speculative` -> render through the load balancer;
//! 4. `Leader::publish` (in-flight followers wake up), then hold `hold` with the entry still in
//!    the table (later requests get it at once);
//! 5. at the end of the hold: read by any request (`Leader::joined` > 0) and a segment cache on ->
//!    add to the cache (its policy takes over); then drop the leader (entry gone; the bytes are
//!    freed when the last reader is done).
//!
//! This module is honest: the clock is passed in (`now_s`), so the rule is tested with made-up
//! times. [`MergedPrefetch::now_s`] is the one dishonest helper (reads the clock).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::obm_parse::SegmentPath;

/// Prune the arrival table when it grows past this many keys (entries older than the window can
/// never trigger again)
const PRUNE_AT: usize = 1024;

/// Arrival table key: (show, combination rest, segment)
type StreamSegment = (String, String, u32);

pub struct MergedPrefetch {
    /// k-1 and k within this many seconds -> prefetch k+1
    window_s: f64,
    /// how long a finished prefetch stays in the coalescer table
    hold: Duration,
    /// last real arrival (s, `now_s` clock) per (show, combination, segment)
    last_seen: Mutex<HashMap<StreamSegment, f64>>,
    /// clock origin for `now_s`
    epoch: Instant,
}

impl MergedPrefetch {
    /// [Honest] Empty arrival table.
    pub fn new(window_s: f64, hold: Duration) -> Self {
        Self {
            window_s,
            hold,
            last_seen: Mutex::new(HashMap::new()),
            epoch: Instant::now(),
        }
    }

    pub fn window_s(&self) -> f64 {
        self.window_s
    }

    pub fn hold(&self) -> Duration {
        self.hold
    }

    /// For the startup line, e.g. "window 30 s, hold 30 s"
    pub fn describe(&self) -> String {
        format!(
            "window {} s, hold {} s",
            self.window_s,
            self.hold.as_secs_f64()
        )
    }

    /// [Dishonest clock] Seconds since this prefetcher was made (the `now_s` for `observe`).
    pub fn now_s(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// [Honest] Records a real request arriving at `now_s` and returns the path to prefetch, if
    /// the "2 continue" rule fires: the same show + combination's previous segment arrived
    /// within the window, and a next segment exists. Not a segment path -> None, nothing stored.
    pub fn observe(&self, path: &str, now_s: f64) -> Option<String> {
        let seg = SegmentPath::parse(path)?;
        let key = |k: u32| (seg.show.to_owned(), seg.rest.to_owned(), k);
        let mut seen = self.lock();
        if seen.len() >= PRUNE_AT {
            seen.retain(|_, t| now_s - *t <= self.window_s);
        }
        seen.insert(key(seg.segment), now_s);
        let prev = seg.segment.checked_sub(1)?;
        let t_prev = *seen.get(&key(prev))?;
        drop(seen);
        // now_s - t_prev can be < 0 only if the caller's clock went backwards: treat as recent
        if now_s - t_prev <= self.window_s {
            seg.next_path()
        } else {
            None
        }
    }

    /// [Honest] Keys in the arrival table (tests / logging).
    pub fn tracked(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<StreamSegment, f64>> {
        // never held across an await or user code: a poisoned map is still consistent
        self.last_seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const A: &str = "/offload/show/f1.json";
    const COMBO: &str = "1280/720/driver/landscape/2/driver|race";

    fn seg(k: u32) -> String {
        format!("{A}/{}/{}/{COMBO}", k * 10, k * 10 + 10)
    }

    fn pf() -> MergedPrefetch {
        MergedPrefetch::new(30.0, Duration::from_secs(30))
    }

    #[test]
    fn test_two_continue_fires() {
        let p = pf();
        assert_eq!(p.observe(&seg(0), 0.0), None); // seg 0 alone: no
        assert_eq!(p.observe(&seg(1), 5.0), Some(seg(2)));
        assert_eq!(p.observe(&seg(2), 15.0), Some(seg(3)));
    }

    #[test]
    fn test_order_does_not_matter_only_latest_arrival() {
        // k arrives, then k-1 (another viewer) within the window: the k-1 arrival is the
        // trigger for k (its previous is k-2), not for k+1
        let p = pf();
        assert_eq!(p.observe(&seg(3), 0.0), None);
        assert_eq!(p.observe(&seg(2), 3.0), None);
        // a later k uses the recent k-1
        assert_eq!(p.observe(&seg(3), 10.0), Some(seg(4)));
    }

    #[test]
    fn test_window() {
        let p = pf();
        p.observe(&seg(0), 0.0);
        assert_eq!(p.observe(&seg(1), 30.0), Some(seg(2))); // edge: inside
        let p = pf();
        p.observe(&seg(0), 0.0);
        assert_eq!(p.observe(&seg(1), 30.5), None);
    }

    #[test]
    fn test_other_combination_or_show_does_not_count() {
        let p = pf();
        p.observe(&seg(0), 0.0);
        assert_eq!(p.observe(&format!("{A}/10/20/1280/720/other"), 2.0), None);
        assert_eq!(
            p.observe(
                "/offload/show/f1_full.json/10/20/1280/720/driver/landscape/2/driver|race",
                2.0
            ),
            None
        );
    }

    #[test]
    fn test_last_segment_and_unknown_show() {
        let p = pf();
        p.observe(&seg(7), 0.0);
        assert_eq!(p.observe(&seg(8), 1.0), None); // f1 is 90 s: 8 is the last
        let p = pf();
        p.observe("/offload/show/syn_res.json/0/10/x", 0.0);
        assert_eq!(p.observe("/offload/show/syn_res.json/10/20/x", 1.0), None);
    }

    #[test]
    fn test_non_segment_paths_ignored() {
        let p = pf();
        assert_eq!(p.observe("/offload/show/f1.json/0/20/x", 0.0), None);
        assert_eq!(p.observe("/assets/a.png", 0.0), None);
        assert_eq!(p.tracked(), 0);
    }

    #[test]
    fn test_prune_drops_old_keys_only() {
        let p = pf();
        for i in 0..PRUNE_AT {
            p.observe(&format!("{A}/0/10/combo{i}"), 0.0);
        }
        p.observe(&seg(0), 100.0);
        assert_eq!(p.observe(&seg(1), 101.0), Some(seg(2)));
        assert_eq!(p.tracked(), 2);
    }
}
