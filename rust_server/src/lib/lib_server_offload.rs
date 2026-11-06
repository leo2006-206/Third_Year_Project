use smol::net::TcpStream;
use std::time::Instant;

// One render request waiting for Dana. The Dana worker takes them one at a time in arrival
// order (FIFO channel, single receiver) and answers on `client` itself
pub struct DanaJob {
    pub client: TcpStream,
    pub path: String,
    pub enqueued: Instant, // for X-Queue-Time: time spent waiting in the queue
}

/// [Dishonest I/O] The site's asset cache from env vars, None = off:
/// OBM_ASSET_CACHE_MB (unset / 0 = off), OBM_ASSET_CACHE_POLICY lru | lfu (default lru),
/// files in OBM_ASSET_CACHE_DIR (default /tmp/obm_cache/asset, emptied at start).
/// A bad value is an error, so a typo never silently runs without the cache.
pub fn asset_cache_loader() -> std::io::Result<Option<rust_obm::mod_feature::AssetCache>> {
    use crate::lib_util::env_parse;
    use rust_obm::mod_feature::{AssetCache, CachePolicy};

    let mb = env_parse::<usize>("OBM_ASSET_CACHE_MB")?.unwrap_or(0);
    if mb == 0 {
        return Ok(None);
    }
    let policy = env_parse::<CachePolicy>("OBM_ASSET_CACHE_POLICY")?.unwrap_or(CachePolicy::Lru);
    let dir =
        std::env::var("OBM_ASSET_CACHE_DIR").unwrap_or_else(|_| "/tmp/obm_cache/asset".to_string());
    let cache = AssetCache::new(&dir, mb << 20, policy)
        .map_err(|e| std::io::Error::new(e.kind(), format!("asset cache dir {dir}: {e}")))?;
    Ok(Some(cache))
}
