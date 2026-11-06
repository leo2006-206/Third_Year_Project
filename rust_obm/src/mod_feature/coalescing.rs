//! Request coalescing: identical requests that are in flight at the same time share one result.
//!
//! The first caller for a key becomes the [`Leader`] and does the work; callers that arrive
//! while it runs become [`Follower`]s and wait for the leader's value. The table only holds work
//! that is still running: the entry leaves the map when the leader finishes (or is dropped), so
//! a request that arrives after that starts fresh (a cache, if any, is checked before this table).
//!
//! Removal is tied to the leader's lifetime (`Drop` of [`Leader`]), so the entry also goes away
//! on an early return, a panic or a cancelled task. The value itself lives in an `Arc` shared by
//! the leader and every follower and is freed when the last of them drops it (no timer).
//!
//! Honest vs dishonest: an honest function depends only on its inputs and the struct's own
//! state; a dishonest one also depends on the outside world (OS, file system, network, clock).
//! Honest code is deterministic, so it is tested by calling it, with no sockets or temp dirs.
//! This whole table is honest: it never renders, reads a socket or looks at the time. The main
//! server does the dishonest part (render via the LB, send to the client) and only passes the
//! result in through `finish`, the same split as the http lib returning `Vec<u8>` instead of
//! writing to the client stream.
//!
//! ```ignore
//! match table.join(key) {
//!     Role::Leader(leader) => {
//!         let resp = Arc::new(render(key).await);
//!         // (if a segment cache is on, add to it HERE, before finish, so no request falls
//!         //  in the gap between "left the table" and "in the cache")
//!         leader.finish(resp.clone()).await;
//!         send(resp)
//!     }
//!     Role::Follower(follower) => match follower.wait().await {
//!         Some(resp) => send(resp),
//!         None => /* leader gave up without a value: handle as a normal miss */,
//!     },
//! }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_lock::OnceCell;

/// Shared slot of one in-flight key: `Some(value)` when the leader finished, `None` when the
/// leader was dropped without a value.
type Slot<T> = Arc<OnceCell<Option<T>>>;

/// Table of in-flight keys. `T` is what followers receive, typically `Arc<Response>` so that
/// handing it out is a pointer copy.
pub struct Coalescer<T> {
    inflight: Mutex<HashMap<String, Slot<T>>>,
}

/// What [`Coalescer::join`] made of the caller.
pub enum Role<'a, T> {
    /// Nobody was working on the key: do the work, then call [`Leader::finish`].
    Leader(Leader<'a, T>),
    /// Someone else is working on the key: call [`Follower::wait`].
    Follower(Follower<T>),
}

impl<T> Default for Coalescer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Coalescer<T> {
    /// [Honest] Empty table.
    pub fn new() -> Self {
        Self {
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// [Honest] Becomes the leader of `key` if nobody works on it, otherwise a follower of the current
    /// leader. Check-and-insert happen under one lock, so exactly one caller leads.
    pub fn join(&self, key: &str) -> Role<'_, T> {
        let mut map = self.lock();
        if let Some(slot) = map.get(key) {
            return Role::Follower(Follower { slot: slot.clone() });
        }
        let slot: Slot<T> = Arc::new(OnceCell::new());
        map.insert(key.to_owned(), slot.clone());
        Role::Leader(Leader {
            table: self,
            key: key.to_owned(),
            slot,
        })
    }

    /// [Honest] Number of keys in flight.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// [Honest] No key in flight.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// [Honest] Removes `key` only if it still maps to `slot` (never another leader's entry).
    fn remove(&self, key: &str, slot: &Slot<T>) {
        let mut map = self.lock();
        if map.get(key).is_some_and(|s| Arc::ptr_eq(s, slot)) {
            map.remove(key);
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Slot<T>>> {
        // the lock is never held across an await or user code, so poison carries no broken state
        self.inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The one caller doing the work for a key. Dropping it removes the key from the table; if it
/// is dropped without [`finish`](Leader::finish), waiting followers get `None`.
pub struct Leader<'a, T> {
    table: &'a Coalescer<T>,
    key: String,
    slot: Slot<T>,
}

impl<T> Leader<'_, T> {
    /// [Honest] Publishes `value` to every follower (they wake up), then leaves the table.
    pub async fn finish(self, value: T) {
        // only the leader ever sets the slot, so this never waits on anyone
        let _ = self.slot.set(Some(value)).await;
        // `self` drops here -> entry removed
    }

    /// [Honest] Followers waiting on this leader right now (for logging: one may still join
    /// until the entry leaves the table). The slot is held by the table, the leader and each
    /// follower.
    pub fn followers(&self) -> usize {
        Arc::strong_count(&self.slot).saturating_sub(2)
    }

    /// [Honest] The key this leader works on.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl<T> Drop for Leader<'_, T> {
    /// [Honest] Leaves the table; releases waiting followers with `None` if never finished.
    fn drop(&mut self) {
        if !self.slot.is_initialized() {
            // dropped without finish (error path, panic, cancelled task): release the followers.
            // Not contended (only the leader sets the slot), so this does not block.
            let _ = self.slot.set_blocking(None);
        }
        self.table.remove(&self.key, &self.slot);
    }
}

/// A caller waiting for the leader of the same key.
pub struct Follower<T> {
    slot: Slot<T>,
}

impl<T: Clone> Follower<T> {
    /// [Honest] Waits for the leader: `Some(value)` when it finished, `None` when it was dropped
    /// without a value.
    pub async fn wait(self) -> Option<T> {
        self.slot.wait().await.clone()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    fn leader<'a>(table: &'a Coalescer<u32>, key: &str) -> Leader<'a, u32> {
        match table.join(key) {
            Role::Leader(l) => l,
            Role::Follower(_) => panic!("expected leader for {key}"),
        }
    }

    fn follower(table: &Coalescer<u32>, key: &str) -> Follower<u32> {
        match table.join(key) {
            Role::Follower(f) => f,
            Role::Leader(_) => panic!("expected follower for {key}"),
        }
    }

    #[test]
    fn test_followers_get_leaders_value() {
        let table = Coalescer::new();
        let l = leader(&table, "a");
        let f1 = follower(&table, "a");
        let f2 = follower(&table, "a");
        smol::block_on(async {
            l.finish(7).await;
            assert_eq!(f1.wait().await, Some(7));
            assert_eq!(f2.wait().await, Some(7));
        });
    }

    #[test]
    fn test_followers_count() {
        let table = Coalescer::new();
        let l = leader(&table, "a");
        assert_eq!(l.followers(), 0);
        let f1 = follower(&table, "a");
        let _f2 = follower(&table, "a");
        assert_eq!(l.followers(), 2);
        drop(f1);
        assert_eq!(l.followers(), 1);
    }

    #[test]
    fn test_finish_removes_entry() {
        let table = Coalescer::new();
        let l = leader(&table, "a");
        assert_eq!(table.len(), 1);
        smol::block_on(l.finish(1));
        assert!(table.is_empty());
        // next request for the same key starts fresh
        let _l2 = leader(&table, "a");
    }

    #[test]
    fn test_follower_after_finish_still_gets_value() {
        // follower joined before finish but waits after the entry is gone
        let table = Coalescer::new();
        let l = leader(&table, "a");
        let f = follower(&table, "a");
        smol::block_on(l.finish(3));
        assert!(table.is_empty());
        assert_eq!(smol::block_on(f.wait()), Some(3));
    }

    #[test]
    fn test_dropped_leader_releases_followers() {
        let table = Coalescer::new();
        let l = leader(&table, "a");
        let f = follower(&table, "a");
        drop(l);
        assert!(table.is_empty());
        assert_eq!(smol::block_on(f.wait()), None);
    }

    #[test]
    fn test_keys_are_independent() {
        let table = Coalescer::new();
        let la = leader(&table, "a");
        let lb = leader(&table, "b");
        let fb = follower(&table, "b");
        assert_eq!(table.len(), 2);
        smol::block_on(async {
            lb.finish(2).await;
            assert_eq!(fb.wait().await, Some(2));
        });
        assert_eq!(table.len(), 1);
        drop(la);
        assert!(table.is_empty());
    }

    #[test]
    fn test_old_leader_does_not_remove_new_entry() {
        // a finished leader's drop must not remove a later leader's entry for the same key
        let table = Coalescer::new();
        let l1 = leader(&table, "a");
        let slot1 = l1.slot.clone();
        smol::block_on(l1.finish(1));
        let _l2 = leader(&table, "a");
        table.remove("a", &slot1); // stale remove
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn test_one_leader_under_concurrency() {
        // 8 threads join the same key at once: exactly one leads, all get its value
        const N: usize = 8;
        let table = Arc::new(Coalescer::<u32>::new());
        let start = Arc::new(Barrier::new(N));
        let joined = Arc::new(Barrier::new(N));
        let handles: Vec<_> = (0..N)
            .map(|i| {
                let (table, start, joined) = (table.clone(), start.clone(), joined.clone());
                thread::spawn(move || {
                    start.wait();
                    let role = table.join("k");
                    joined.wait(); // everyone joined before the leader finishes
                    smol::block_on(async {
                        match role {
                            Role::Leader(l) => {
                                l.finish(i as u32 + 100).await;
                                (true, None)
                            }
                            Role::Follower(f) => (false, f.wait().await),
                        }
                    })
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(lead, _)| *lead).count(), 1);
        let values: Vec<_> = results.iter().filter_map(|(_, v)| *v).collect();
        assert_eq!(values.len(), N - 1);
        assert!(values.windows(2).all(|w| w[0] == w[1]));
        assert!(table.is_empty());
    }
}
