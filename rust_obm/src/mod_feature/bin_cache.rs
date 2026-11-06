use std::fs::File;
use std::io;
use std::path::Path;

use crate::mod_feature::LruCache;
use crate::mod_load_balancer::TimesFeedback;
use crate::mod_load_balancer::config::{Config, Render};

/// Which of the two bins a segment went to (or was found in).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bin {
    Lower,
    Upper,
}

impl std::fmt::Display for Bin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Bin::Lower => "lower",
            Bin::Upper => "upper",
        })
    }
}

/// Two LRU caches split by cost: segments that cost less than `threshold_s` to make go to the
/// lower cache, the rest to the upper one. Cost = asset time + work time / render factor, in
/// seconds; the factor turns work on a slow renderer into gpu-equivalent work. Asset loading is
/// not divided: it does not depend on the renderer.
pub struct BinCache {
    lower_cache: LruCache,
    upper_cache: LruCache,
    threshold_s: f64,
}

impl BinCache {
    /// Errors: InvalidInput if `threshold_s` is not a positive number or both caches would use
    /// the same directory (each LruCache deletes files it evicts, so they must not share);
    /// or the I/O error from creating a cache directory.
    pub fn new(
        lower_path: impl AsRef<Path>,
        upper_path: impl AsRef<Path>,
        lower_cap: usize,
        upper_cap: usize,
        threshold_s: f64,
    ) -> io::Result<Self> {
        if !(threshold_s.is_finite() && threshold_s > 0.0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("bin threshold must be a positive number of seconds, got {threshold_s}"),
            ));
        }
        if lower_path.as_ref() == upper_path.as_ref() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "lower and upper bin caches need different directories",
            ));
        }
        Ok(Self {
            lower_cache: LruCache::new(lower_path, lower_cap)?,
            upper_cache: LruCache::new(upper_path, upper_cap)?,
            threshold_s,
        })
    }

    /// Cost in seconds of making this segment again, in gpu-equivalent time.
    pub fn cost_s(feedback: &TimesFeedback, site_config: &Config) -> f64 {
        const CPU_FACTOR: f64 = 4.6;
        const LLVM_FACTOR: f64 = 1.33;
        const GPU_FACTOR: f64 = 1.0;

        let work_time = feedback.t_work as f64;

        let work_cost = match site_config.render {
            Render::Cpu => work_time / CPU_FACTOR,
            Render::Llvmpipe => work_time / LLVM_FACTOR,
            Render::Gpu => work_time / GPU_FACTOR,
        };

        (work_cost + feedback.t_asset as f64) / 1000.0
    }

    /// Caches `buffer` in the bin its cost picks and returns that bin. Errors as
    /// `LruCache::add`; an AlreadyExists names the bin holding the key: "in upper bin" (same
    /// bin), or "in other bin, upper" when this copy's cost fell on the other side of the
    /// threshold. Two concurrent misses can still both add, one per bin: harmless, as `get`
    /// returns either copy and LRU evicts the unused one.
    pub fn add(
        &self,
        feedback: &TimesFeedback,
        site_config: &Config,
        key: &str,
        buffer: &[u8],
    ) -> io::Result<Bin> {
        let bin = if Self::cost_s(feedback, site_config) >= self.threshold_s {
            Bin::Upper
        } else {
            Bin::Lower
        };
        let other = match bin {
            Bin::Lower => Bin::Upper,
            Bin::Upper => Bin::Lower,
        };
        if self.cache(other).contains_key(key) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("in other bin, {other}"),
            ));
        }
        match self.cache(bin).add(key, buffer) {
            Ok(()) => Ok(bin),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("in {bin} bin"),
            )),
            Err(e) => Err(e),
        }
    }

    fn cache(&self, bin: Bin) -> &LruCache {
        match bin {
            Bin::Lower => &self.lower_cache,
            Bin::Upper => &self.upper_cache,
        }
    }

    /// Opens the cached file for `key` from whichever bin holds it. None on a miss.
    pub fn get(&self, key: &str) -> Option<(File, usize)> {
        self.lower_cache
            .get(key)
            .or_else(|| self.upper_cache.get(key))
    }

    /// Bytes held by both bins.
    pub fn used(&self) -> usize {
        self.lower_cache.used() + self.upper_cache.used()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::mod_load_balancer::config::{Decode, Encode};
    use std::fs;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("obm_bin_cache_{name}_{}", std::process::id()))
    }

    fn feedback(asset_ms: i64, work_ms: i64) -> TimesFeedback {
        TimesFeedback {
            t_asset: asset_ms,
            t_work: work_ms,
            ..Default::default()
        }
    }

    const GPU: Config = Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu);
    const CPU: Config = Config::new(Decode::Cpu, Render::Cpu, Encode::Cpu);

    #[test]
    fn test_new_rejects_bad_input() {
        let dir = test_dir("bad");
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let err = BinCache::new(dir.join("lo"), dir.join("hi"), 10, 10, bad).err();
            assert_eq!(err.map(|e| e.kind()), Some(io::ErrorKind::InvalidInput));
        }
        let err = BinCache::new(dir.join("same"), dir.join("same"), 10, 10, 6.0).err();
        assert_eq!(err.map(|e| e.kind()), Some(io::ErrorKind::InvalidInput));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_cost_divides_work_not_asset() {
        // 2 s asset + 9.2 s cpu work = 2 + 9.2 / 4.6 = 4 s
        assert!((BinCache::cost_s(&feedback(2000, 9200), &CPU) - 4.0).abs() < 1e-9);
        assert!((BinCache::cost_s(&feedback(2000, 9200), &GPU) - 11.2).abs() < 1e-9);
    }

    #[test]
    fn test_routes_by_cost_and_gets_from_both() {
        let dir = test_dir("route");
        let cache = BinCache::new(dir.join("lo"), dir.join("hi"), 100, 100, 6.0).unwrap();

        let cheap = cache.add(&feedback(1000, 2000), &GPU, "cheap", &[1; 4]); // 3 s
        let dear = cache.add(&feedback(2000, 5000), &GPU, "dear", &[2; 8]); // 7 s

        assert_eq!(cheap.unwrap(), Bin::Lower);
        assert_eq!(dear.unwrap(), Bin::Upper);
        assert!(cache.lower_cache.contains_key("cheap"));
        assert!(cache.upper_cache.contains_key("dear"));
        assert_eq!(cache.get("cheap").unwrap().1, 4);
        assert_eq!(cache.get("dear").unwrap().1, 8);
        assert!(cache.get("none").is_none());
        assert_eq!(cache.used(), 12);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_key_not_cached_in_both_bins() {
        let dir = test_dir("both");
        let cache = BinCache::new(dir.join("lo"), dir.join("hi"), 100, 100, 6.0).unwrap();

        let added = cache.add(&feedback(1000, 2000), &GPU, "a", &[0; 4]);
        assert_eq!(added.unwrap(), Bin::Lower);

        // would be upper: refused, naming the bin that holds it
        let err = cache
            .add(&feedback(2000, 5000), &GPU, "a", &[0; 4])
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(err.to_string(), "in other bin, lower");
        assert!(!cache.upper_cache.contains_key("a"));

        // same bin again: the plain duplicate, also named
        let err = cache
            .add(&feedback(1000, 2000), &GPU, "a", &[0; 4])
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(err.to_string(), "in lower bin");
        assert_eq!(cache.used(), 4);
        fs::remove_dir_all(&dir).unwrap();
    }
}
