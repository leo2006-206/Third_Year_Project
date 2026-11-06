use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::file_cache::CacheTable;

/// Which entry a full cache evicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// least recently used
    Lru,
    /// least frequently used (uses while cached, forgotten on eviction); ties: least recent
    Lfu,
}

impl CachePolicy {
    /// Eviction order key: the entry with the smallest key goes first.
    fn rank(self, entry: &Entry) -> (u64, u64) {
        match self {
            CachePolicy::Lru => (entry.last_use, 0),
            CachePolicy::Lfu => (entry.uses, entry.last_use),
        }
    }
}

impl std::fmt::Display for CachePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CachePolicy::Lru => "lru",
            CachePolicy::Lfu => "lfu",
        })
    }
}

impl std::str::FromStr for CachePolicy {
    type Err = String;
    /// "lru" or "lfu" (any case)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "lru" => Ok(CachePolicy::Lru),
            "lfu" => Ok(CachePolicy::Lfu),
            other => Err(format!(
                "unknown cache policy '{other}' (expected lru | lfu)"
            )),
        }
    }
}

/// Per-entry state: when it was last used, how often, and its size for the byte budget.
struct Entry {
    last_use: u64,
    uses: u64,
    size: usize,
}

impl Entry {
    fn touch(&mut self, tick: u64) {
        self.last_use = tick;
        self.uses += 1;
    }
}

/// Everything guarded by the one lock: the entries, the bytes they hold, and the use clock.
#[derive(Default)]
struct Inner {
    map: HashMap<String, Entry>,
    used: usize,
    tick: u64,
}

impl Inner {
    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

/// Cache of files with a byte cap and an eviction policy (LRU or LFU). `CacheTable` holds the
/// bytes on disk; this holds the policy state. File writes and deletes run outside the lock, so a
/// slow write never blocks `get`. Only entries in the map count toward `used`, so disk use can
/// briefly exceed `byte_cap` by files being written or victims not yet deleted.
pub struct PolicyCache {
    table: CacheTable,
    inner: Mutex<Inner>,
    byte_cap: usize,
    policy: CachePolicy,
}

impl PolicyCache {
    pub fn new(
        base_path: impl AsRef<Path>,
        byte_cap: usize,
        policy: CachePolicy,
    ) -> io::Result<Self> {
        Ok(Self {
            table: CacheTable::new(base_path)?,
            inner: Mutex::new(Inner::default()),
            byte_cap,
            policy,
        })
    }

    pub fn policy(&self) -> CachePolicy {
        self.policy
    }

    /// Caches `buffer` under `key`, evicting entries in the policy's order until it fits.
    /// Errors: AlreadyExists if the key is cached (it counts as a use) or being added by
    /// someone else; InvalidInput if the item is bigger than the whole cache; or the I/O error
    /// from writing the file (nothing is evicted then).
    pub fn add(&self, key: &str, buffer: &[u8]) -> io::Result<()> {
        let size = buffer.len();
        if size > self.byte_cap {
            return Err(io::ErrorKind::InvalidInput.into());
        }

        // Lock 1: already cached -> just a use
        {
            let mut inner = self.lock();
            let tick = inner.next_tick();
            if let Some(entry) = inner.map.get_mut(key) {
                entry.touch(tick);
                return Err(io::ErrorKind::AlreadyExists.into());
            }
        }

        // No lock: write the file. CacheTable lets only one concurrent add of a key succeed,
        // and `get` cannot see the key until it is in the map below.
        self.table.insert(key, buffer)?;

        // Lock 2: evict victims from the map, then record the new entry
        let victims = {
            let mut inner = self.lock();
            let mut victims = Vec::new();
            while inner.used + size > self.byte_cap {
                let Some(victim) = inner
                    .map
                    .iter()
                    .min_by_key(|(_, e)| self.policy.rank(e))
                    .map(|(k, _)| k.clone())
                else {
                    break; // empty map: cannot happen since size <= byte_cap
                };
                let entry = inner.map.remove(&victim).expect("victim came from the map");
                inner.used -= entry.size;
                victims.push(victim);
            }
            let last_use = inner.next_tick();
            let entry = Entry {
                last_use,
                uses: 1,
                size,
            };
            inner.map.insert(key.to_string(), entry);
            inner.used += size;
            victims
        };

        // No lock: delete the victims' files. They already left the map, so `get` misses them,
        // and readers that opened one earlier keep their handle.
        for victim in victims {
            self.table.remove(&victim);
        }
        Ok(())
    }

    /// Opens the cached file for `key` and marks it used. None on a miss.
    pub fn get(&self, key: &str) -> Option<(File, usize)> {
        {
            let mut inner = self.lock();
            let tick = inner.next_tick();
            inner.map.get_mut(key)?.touch(tick);
        }
        // Outside the lock: if the entry was evicted just now, this is simply a miss
        self.table.get(key)
    }

    /// True if `key` is cached. A peek: does not count as a use and opens no file.
    pub fn contains_key(&self, key: &str) -> bool {
        self.lock().map.contains_key(key)
    }

    /// Bytes held by cached entries.
    pub fn used(&self) -> usize {
        self.lock().used
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the state consistent, so keep using it
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("obm_policy_cache_{name}_{}", std::process::id()))
    }

    fn read_all(mut file: File) -> Vec<u8> {
        let mut out = Vec::new();
        file.read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn test_add_get() {
        let dir = test_dir("basic");
        let cache = PolicyCache::new(&dir, 100, CachePolicy::Lru).unwrap();

        assert!(cache.get("a").is_none());
        cache.add("a", b"hello").unwrap();
        let (file, size) = cache.get("a").unwrap();
        assert_eq!(size, 5);
        assert_eq!(read_all(file), b"hello");
        assert_eq!(cache.used(), 5);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_evicts_least_recently_used() {
        let dir = test_dir("evict");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lru).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        cache.add("b", &[0; 4]).unwrap();
        cache.get("a").unwrap(); // now b is the oldest
        cache.add("c", &[0; 4]).unwrap();

        assert!(cache.get("a").is_some());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
        assert_eq!(cache.used(), 8);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_big_item_evicts_several() {
        let dir = test_dir("several");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lru).unwrap();

        cache.add("a", &[0; 3]).unwrap();
        cache.add("b", &[0; 3]).unwrap();
        cache.add("c", &[0; 3]).unwrap();
        cache.add("big", &[0; 8]).unwrap(); // needs a, b and c gone

        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_none());
        assert!(cache.get("big").is_some());
        assert_eq!(cache.used(), 8);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_too_big_rejected() {
        let dir = test_dir("too_big");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lru).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        let err = cache.add("huge", &[0; 11]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(cache.get("a").is_some()); // nothing evicted
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_contains_key_is_not_a_use() {
        let dir = test_dir("contains");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lru).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        cache.add("b", &[0; 4]).unwrap();
        assert!(cache.contains_key("a")); // peek: a stays the oldest
        assert!(!cache.contains_key("z"));
        cache.add("c", &[0; 4]).unwrap(); // evicts a

        assert!(!cache.contains_key("a"));
        assert!(cache.contains_key("b"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_duplicate_add_counts_as_use() {
        let dir = test_dir("dup");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lru).unwrap();

        cache.add("a", &[1; 4]).unwrap();
        cache.add("b", &[2; 4]).unwrap();
        let err = cache.add("a", &[9; 4]).unwrap_err(); // refreshes a, keeps old bytes
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        cache.add("c", &[3; 4]).unwrap(); // evicts b, not a

        assert_eq!(read_all(cache.get("a").unwrap().0), [1; 4]);
        assert!(cache.get("b").is_none());
        assert_eq!(cache.used(), 8);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_lfu_keeps_frequently_used() {
        let dir = test_dir("lfu");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lfu).unwrap();

        cache.add("hot", &[0; 4]).unwrap();
        cache.get("hot").unwrap();
        cache.get("hot").unwrap(); // 3 uses
        cache.add("cold", &[0; 4]).unwrap(); // 1 use, but more recent than hot
        cache.add("new", &[0; 4]).unwrap(); // LRU would evict hot, LFU evicts cold

        assert!(cache.contains_key("hot"));
        assert!(!cache.contains_key("cold"));
        assert!(cache.contains_key("new"));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_lfu_tie_goes_to_least_recent() {
        let dir = test_dir("lfu_tie");
        let cache = PolicyCache::new(&dir, 10, CachePolicy::Lfu).unwrap();

        cache.add("a", &[0; 4]).unwrap();
        cache.add("b", &[0; 4]).unwrap(); // a and b: 1 use each, a older
        cache.add("c", &[0; 4]).unwrap(); // evicts a

        assert!(!cache.contains_key("a"));
        assert!(cache.contains_key("b"));
        assert!(cache.contains_key("c"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_policy_from_str() {
        assert_eq!("lru".parse::<CachePolicy>(), Ok(CachePolicy::Lru));
        assert_eq!("LFU".parse::<CachePolicy>(), Ok(CachePolicy::Lfu));
        assert!("fifo".parse::<CachePolicy>().is_err());
        assert_eq!(CachePolicy::Lfu.to_string(), "lfu");
    }
}
