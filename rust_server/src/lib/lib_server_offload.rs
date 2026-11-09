use smol::net::TcpStream;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

// One render request waiting for Dana. The Dana worker takes them one at a time in arrival
// order (FIFO channel, single receiver) and answers on `client` itself
pub struct DanaJob {
    pub client: TcpStream,
    pub path: String,
    pub enqueued: Instant, // for X-Queue-Time: time spent waiting in the queue
    /// PSC rebuild ('<k>+<n>'): the sub segment the main server pushed with the job (POST body)
    pub sub: Option<Arc<Vec<u8>>>,
}

/// The PSC rebuild Dana is rendering now (the front sends Dana one job at a time, so there is at
/// most one): Dana fetches its sub as GET /psc/show/<show>/<t0>/... and gets `sub` if show and t0
/// match; `served` = it got it (else the rebuilt picture would miss its bottom layers).
pub struct CurrentSub {
    pub show: String,
    pub t0: u32,
    pub sub: Arc<Vec<u8>>,
    pub served: bool,
}

/// Shared by the Dana worker (sets / takes it around a rebuild job) and the /psc route
pub type CurrentJob = Mutex<Option<CurrentSub>>;

/// Lock that survives a poisoned mutex (never held across an await: the value stays consistent)
pub fn lock(current: &CurrentJob) -> MutexGuard<'_, Option<CurrentSub>> {
    current
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Largest PSC sub accepted with a rebuild job (a 10 s sub is ~1-5 MB)
pub const MAX_SUB_BYTES: usize = 64 << 20;

/// [Honest] (show, t0) of a PSC rebuild job: /offload/show/<show>/<t0>/<t1>/<w>/<h>/<variant>/
/// <style>/<k>+<n>/... (optionally /times first); None for any other request.
pub fn rebuild_target(path: &str) -> Option<(String, u32)> {
    let path = path.strip_prefix("/times").unwrap_or(path);
    let p: Vec<&str> = path.split('/').collect();
    if p.get(1) != Some(&"offload") || p.get(2) != Some(&"show") {
        return None;
    }
    let (k, n) = p.get(10)?.split_once('+')?;
    k.parse::<u32>().ok()?;
    n.parse::<u32>().ok()?;
    Some((p.get(3)?.to_string(), p.get(4)?.parse().ok()?))
}

/// [Honest] (show, t0) of Dana's own sub fetch: /psc/show/<show>/<t0>/... (the site's Experience
/// builds it with requestPre 'psc/show/'); None if malformed.
pub fn sub_target(path: &str) -> Option<(String, u32)> {
    let mut p = path.strip_prefix("/psc/show/")?.split('/');
    let show = p.next().filter(|s| !s.is_empty())?;
    Some((show.to_string(), p.next()?.parse().ok()?))
}

/// [Honest] For the first read of a request: (index where the body starts, Content-Length);
/// None if the head is not complete in `first`.
pub fn request_body_start(first: &[u8]) -> Option<(usize, Option<u64>)> {
    let end = first.windows(4).position(|w| w == b"\r\n\r\n")?;
    Some((
        end + 4,
        crate::lib_http::parse_content_length(&first[..end]),
    ))
}

/// [Honest] 400 with a short text reason (the main server logs the status)
pub fn bad_request(reason: &str) -> Vec<u8> {
    format!(
        "HTTP/1.0 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reason}",
        reason.len()
    )
    .into_bytes()
}

/// [Honest] What goes back for a PSC rebuild whose sub Dana never fetched: a 200 from Dana would be
/// a picture without its bottom layers, so it becomes 502 + X-PSC-Rebuild: failed (the main server
/// passes it to the client). Any other reply (sub served, or Dana's own error) is kept.
pub fn rebuild_reply(response: Vec<u8>, sub_served: bool) -> Vec<u8> {
    if sub_served || !crate::lib_http::is_status_ok(&response) {
        return response;
    }
    const BODY: &str = "PSC rebuild: Dana did not fetch its sub segment";
    format!(
        "HTTP/1.0 502 Bad Gateway\r\nX-PSC-Rebuild: failed\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
        BODY.len()
    )
    .into_bytes()
}

/// [Dishonest I/O] The site's asset cache from env vars, None = off:
/// OBM_ASSET_CACHE_MIB (cap in MiB; unset / 0 = off; old name OBM_ASSET_CACHE_MB is refused),
/// OBM_ASSET_CACHE_POLICY lru | lfu (default lru),
/// files in OBM_ASSET_CACHE_DIR (default /tmp/obm_cache/asset, emptied at start).
/// A bad value is an error, so a typo never silently runs without the cache.
pub fn asset_cache_loader() -> std::io::Result<Option<rust_obm::mod_feature::AssetCache>> {
    use crate::lib_util::env_parse;
    use rust_obm::mod_feature::{AssetCache, CachePolicy};

    crate::lib_util::refuse_renamed("OBM_ASSET_CACHE_MB", "OBM_ASSET_CACHE_MIB")?;
    let mib = env_parse::<usize>("OBM_ASSET_CACHE_MIB")?.unwrap_or(0);
    if mib == 0 {
        return Ok(None);
    }
    let policy = env_parse::<CachePolicy>("OBM_ASSET_CACHE_POLICY")?.unwrap_or(CachePolicy::Lru);
    let dir =
        std::env::var("OBM_ASSET_CACHE_DIR").unwrap_or_else(|_| "/tmp/obm_cache/asset".to_string());
    let cache = AssetCache::new(&dir, mib << 20, policy)
        .map_err(|e| std::io::Error::new(e.kind(), format!("asset cache dir {dir}: {e}")))?;
    Ok(Some(cache))
}

#[cfg(test)]
mod test {
    use super::*;

    const REBUILD: &str =
        "/offload/show/f1.json/20/30/1280/720/race/landscape/1+2/race|race/drivers|Sam";

    #[test]
    fn test_rebuild_and_sub_targets() {
        assert_eq!(rebuild_target(REBUILD), Some(("f1.json".to_string(), 20)));
        assert_eq!(
            rebuild_target(&format!("/times{REBUILD}")),
            Some(("f1.json".to_string(), 20))
        );
        // plain, pair, bad counts -> not a rebuild
        assert_eq!(rebuild_target(&REBUILD.replace("1+2", "2")), None);
        assert_eq!(rebuild_target(&REBUILD.replace("1+2", "1*2")), None);
        assert_eq!(rebuild_target(&REBUILD.replace("1+2", "x+2")), None);
        assert_eq!(rebuild_target("/assets/a+b"), None);
        assert_eq!(
            sub_target("/psc/show/f1.json/20/30/1280/720/race/landscape/1/race|race"),
            Some(("f1.json".to_string(), 20))
        );
        // a raw space cut the request line: show + t0 still there
        assert_eq!(
            sub_target("/psc/show/forecast.json/0/10/1280/720/core/landscape/1/w|Weather"),
            Some(("forecast.json".to_string(), 0))
        );
        assert_eq!(sub_target("/psc/show//0/10"), None);
        assert_eq!(sub_target("/offload/show/f1.json/0/10"), None);
    }

    #[test]
    fn test_request_body_start_and_replies() {
        let req = b"POST /x HTTP/1.1\r\nContent-Length: 5\r\n\r\nab";
        assert_eq!(request_body_start(req), Some((req.len() - 2, Some(5))));
        assert_eq!(request_body_start(b"GET /x HTTP/1.1\r\nHost: a\r\n"), None);
        assert!(bad_request("no").starts_with(b"HTTP/1.0 400 "));
        let ok = b"HTTP/1.0 200 OK\r\nContent-length: 1\r\n\r\nx".to_vec();
        assert_eq!(rebuild_reply(ok.clone(), true), ok);
        let failed = rebuild_reply(ok, false);
        assert!(failed.starts_with(b"HTTP/1.0 502 "));
        assert_eq!(
            crate::lib_http::header_value(&failed, "X-PSC-Rebuild").as_deref(),
            Some("failed")
        );
        // Dana's own error passes unchanged
        let err = b"HTTP/1.0 500 Internal\r\n\r\n".to_vec();
        assert_eq!(rebuild_reply(err.clone(), false), err);
    }
}
