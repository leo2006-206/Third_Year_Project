use std::fs::File;
use std::io;

use crate::mod_feature::{Bin, BinCache, PolicyCache};
use crate::mod_load_balancer::TimesFeedback;
use crate::mod_load_balancer::config::Config;

/// The segment cache of a run: one plain cache (LRU or LFU), or two LRU bins split by cost. They do the same
/// job (store rendered segments, answer repeats), so a run has at most one: being one enum,
/// they can never both be on.
pub enum SegmentCache {
    Single {
        cache: PolicyCache,
        mib: usize,
    },
    Bin {
        cache: BinCache,
        lower_mib: usize,
        upper_mib: usize,
        threshold_s: f64,
    },
}

impl SegmentCache {
    /// Opens the cached file for `key`. None on a miss.
    pub fn get(&self, key: &str) -> Option<(File, usize)> {
        match self {
            Self::Single { cache, .. } => cache.get(key),
            Self::Bin { cache, .. } => cache.get(key),
        }
    }

    /// True if `key` is cached. A peek: not a use (LRU / LFU state unchanged), no file opened.
    pub fn contains_key(&self, key: &str) -> bool {
        match self {
            Self::Single { cache, .. } => cache.contains_key(key),
            Self::Bin { cache, .. } => cache.contains_key(key),
        }
    }

    /// Caches a rendered segment. `times` and `config` are the site's timings and render config
    /// for it: the bins need them for the cost, the single cache ignores them.
    /// Returns the bin it went to (None for the single cache). Errors as `PolicyCache::add` /
    /// `BinCache::add` (a bin's AlreadyExists names the bin that holds the key).
    pub fn add(
        &self,
        key: &str,
        data: &[u8],
        times: &TimesFeedback,
        config: &Config,
    ) -> io::Result<Option<Bin>> {
        match self {
            Self::Single { cache, .. } => cache.add(key, data).map(|()| None),
            Self::Bin { cache, .. } => cache.add(times, config, key, data).map(Some),
        }
    }

    /// Bytes held (both bins together for Bin).
    pub fn used(&self) -> usize {
        match self {
            Self::Single { cache, .. } => cache.used(),
            Self::Bin { cache, .. } => cache.used(),
        }
    }

    /// For the startup line, e.g. "45 MiB, lru" or "bins lower 30 MiB, upper 15 MiB, threshold 6 s, lru"
    pub fn describe(&self) -> String {
        match self {
            Self::Single { cache, mib } => format!("{mib} MiB, {}", cache.policy()),
            Self::Bin {
                lower_mib,
                upper_mib,
                threshold_s,
                cache,
            } => format!(
                "bins lower {lower_mib} MiB, upper {upper_mib} MiB, threshold {threshold_s} s, {}",
                cache.policy()
            ),
        }
    }
}
