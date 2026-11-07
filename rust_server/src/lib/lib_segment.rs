//! Getting a segment for the main server, without touching any client: where it comes from
//! (segment_lookup: cache / in-flight table / render), rendering it on a site through the load
//! balancer (fetch_offload), sharing it with the cache and the followers (offload_cache_add), and
//! the prefetch that does all of it with no client (prefetch_task). main_server.rs keeps the
//! client side: the request flow (serve_offload) and the three ways to answer (cache file,
//! follower, own render).

use std::sync::Arc;

use smol::{io, net::TcpStream};

use rust_obm::mod_feature::{BinCache, Follower, Leader, Role, SegmentCache};
use rust_obm::mod_load_balancer as lb;
use rust_obm::mod_load_balancer::base_balancer as blb;

use crate::lib_http as http;
use crate::lib_main_state::{SegmentCoalescer, ServerState};
use crate::lib_server_main as sm;
use crate::lib_util as util;

/// Who asks [`segment_lookup`]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LookupMode {
    /// a client request
    Real,
    /// a prefetch: never reads the cache (a peek only), never follows
    Prefetch,
}

/// Where a segment comes from, decided before any render
pub enum SegmentLookup<'a> {
    /// Real: in the segment cache (opened, counted as a use)
    Hit(std::fs::File),
    /// Real: the path is in the in-flight table, wait for its leader
    Follow(Follower<Arc<Vec<u8>>>),
    /// render it; Some = leader in the in-flight table (followers wait on it), None = no table
    /// entry (coalescing and prefetch off, or a real request with only prefetch on)
    Lead(Option<Leader<'a, Arc<Vec<u8>>>>),
    /// Prefetch: nothing to do ("cached" / "in flight")
    Skip(&'static str),
}

/// [Honest given the state] Cache first, then the in-flight table:
/// - Real: cache hit -> Hit; table: coalescing on -> join (Follow / Lead(Some)), off -> follow a
///   prefetch if one is there (Follow), else Lead(None);
/// - Prefetch: cached -> Skip (peek, not a use); key in the table -> Skip (not counted as a
///   reader); else Lead(Some) as a speculative leader.
pub fn segment_lookup<'a, P: blb::LoadBalancerPolicy>(
    state: &'a ServerState<P>,
    path: &str,
    mode: LookupMode,
) -> SegmentLookup<'a> {
    let coalescer: Option<&'a SegmentCoalescer> = state.coalescer.as_ref();
    match mode {
        LookupMode::Real => {
            if let Some(cache) = &state.segment_cache
                && let Some((file, _size)) = cache.get(path)
            {
                return SegmentLookup::Hit(file);
            }
            match coalescer {
                None => SegmentLookup::Lead(None),
                Some(table) if state.coalesce_real => match table.join(path) {
                    Role::Follower(follower) => SegmentLookup::Follow(follower),
                    Role::Leader(leader) => SegmentLookup::Lead(Some(leader)),
                },
                Some(table) => match table.try_follow(path) {
                    Some(follower) => SegmentLookup::Follow(follower),
                    None => SegmentLookup::Lead(None),
                },
            }
        }
        LookupMode::Prefetch => {
            if state
                .segment_cache
                .as_ref()
                .is_some_and(|c| c.contains_key(path))
            {
                return SegmentLookup::Skip("cached");
            }
            match coalescer.and_then(|table| table.lead_speculative(path)) {
                Some(leader) => SegmentLookup::Lead(Some(leader)),
                None => SegmentLookup::Skip("in flight"),
            }
        }
    }
}

/// A segment a site rendered, ready to send to the client and to share with the followers and
/// the cache.
pub struct RenderedSegment<'a> {
    /// the site's response head with the main server's timing headers appended
    pub head: Vec<u8>,
    /// the video bytes (the response body), shared by the client write, the followers and the
    /// cache add without copying
    pub data: Arc<Vec<u8>>,
    /// status 200: only those are cached / handed to followers, an error body must never be replayed
    pub status_ok: bool,
    /// the site's timings and render config, for the bin cache's cost
    pub times: lb::TimesFeedback,
    pub config: lb::config::Config,
    /// site name and wait + net + queue + asset + work (ms), for the [OFFLOAD] log line
    pub site: &'a str,
    pub total_ms: i64,
}

/// A site failed before the body was complete: what `sm::reply_gateway_error` needs.
pub struct OffloadError<'a> {
    pub site: &'a str,
    pub err: io::Error,
}

/// Renders the request on a site chosen by the load balancer and returns the whole segment;
/// never talks to the client. Inner Err = the site failed before the body was complete (the
/// caller answers 502 / 504); outer Err = sending the request to the site failed.
pub async fn fetch_offload<'a, P: blb::LoadBalancerPolicy + 'static>(
    state: &'a Arc<ServerState<P>>,
    path: &str,
) -> io::Result<Result<RenderedSegment<'a>, OffloadError<'a>>> {
    use http::{request_get, send_raw};
    use std::time::{Duration, Instant};
    use util::{copy_with_idle_timeout, with_timeout};
    const MAIN_SERVER_AGENT: &str = "OBM Load balancer/1.0";
    // A live site accepts within ~30-60 ms; longer means it is down or the packets are dropped
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    // Until the response headers: includes the site's queue + asset + render time
    const HEADER_TIMEOUT: Duration = Duration::from_secs(600);
    // Body streaming: limit on a gap with no data, not on the whole transfer
    const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

    // A policy may hold the request until a site is free (throttled): that wait is reported
    // on its own (X-OBM-Wait), and the network time starts after it
    let t_wait_start = Instant::now();
    let target_offload = state.balancer.select_wait(Some(path)).await;
    let t_wait = t_wait_start.elapsed().as_millis() as i64;
    let t_start = Instant::now();
    let site = target_offload.name;

    // On an early return the selection guard is dropped, which reports the site as done
    let connect = with_timeout(CONNECT_TIMEOUT, TcpStream::connect(target_offload.endpoint)).await;
    let mut offload_steam = match connect {
        Ok(stream) => stream,
        Err(err) => return Ok(Err(OffloadError { site, err })),
    };

    let message = request_get(path, target_offload.endpoint, MAIN_SERVER_AGENT, &[]);
    send_raw(&mut offload_steam, &message).await?;

    // 1. Response head (queue + asset + render happen before it); the body keeps arriving
    //    in the socket meanwhile. A missing / oversized head is a gateway error like a timeout
    let split = with_timeout(HEADER_TIMEOUT, http::split_header_body(&mut offload_steam)).await;
    let (head, mut body) = match split {
        Ok(parts) => parts,
        Err(err) => return Ok(Err(OffloadError { site, err })),
    };
    let (new_head, times) = sm::time_append_header(&head, t_start, t_wait, target_offload.endpoint);

    // 2. Whole body into a buffer (the cache / followers take it from here) and check its
    //    length against Content-Length; nothing has been sent to the client yet, so a short or
    //    stalled body is still a clean gateway error
    let expected_len = http::parse_content_length(&head);
    // capacity capped so a bogus Content-Length can't make a huge allocation up front
    let capacity = expected_len.unwrap_or(0).min(64 << 20) as usize;
    let mut data = Vec::with_capacity(capacity);
    let received = copy_with_idle_timeout(&mut body, &mut data, BODY_IDLE_TIMEOUT).await;
    let checked = received.and_then(|n| match expected_len {
        Some(len) if len != n => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("body {n} B, Content-Length {len} B"),
        )),
        _ => Ok(()),
    });
    if let Err(err) = checked {
        return Ok(Err(OffloadError { site, err }));
    }

    // The site's part is over: report it (with timings) before anyone talks to the client
    let total_ms = t_wait + times.t_net + times.t_queue + times.t_asset + times.t_work;
    let config = target_offload.config;
    target_offload.finish(Some(&times));

    Ok(Ok(RenderedSegment {
        head: new_head,
        data: Arc::new(data),
        status_ok: http::is_status_ok(&head),
        times,
        config,
        site,
        total_ms,
    }))
}

/// Shares a rendered segment: stores it in the segment cache (plain or bins), if one is on, and
/// hands it to the coalescing leader's followers, if this request leads. Only status 200 is
/// shared; otherwise the leader is dropped and its followers answer 502.
/// The cache file write runs on a blocking thread so it never stalls the async executor.
/// Logs: '[CACHE] added | add failed | skip' (cache on), '[COALESCE] add | skip' (leader).
pub async fn offload_cache_add<P: blb::LoadBalancerPolicy + 'static>(
    state: &Arc<ServerState<P>>,
    path: &str,
    leader: Option<Leader<'_, Arc<Vec<u8>>>>,
    segment: &RenderedSegment<'_>,
) {
    if !segment.status_ok {
        if state.segment_cache.is_some() {
            util::log_message(&format!("[CACHE] skip: status not 200 | {path}"));
        }
        if let Some(leader) = leader {
            let n = leader.followers();
            util::log_message(&format!(
                "[COALESCE] skip: status not 200 | {n} followers | {path}"
            ));
        }
        return;
    }
    if state.segment_cache.is_some() {
        let add = cache_add_task(state, path, segment);
        match &leader {
            // a leader leaves the table when it finishes: the segment must be in the cache by
            // then, else a request (or a prefetch) in the 0-27 ms of the write finds neither
            // and renders it again (the old gap). Its followers wait for the write too.
            Some(_) => {
                add.await;
            }
            // no table entry to leave: the client does not wait for the disk
            None => add.detach(),
        }
    }
    if let Some(leader) = leader {
        let n = leader.followers();
        leader.finish(Arc::clone(&segment.data)).await;
        util::log_message(&format!("[COALESCE] add | {n} followers | {path}"));
    }
}

/// Starts the segment cache write on a blocking thread and logs its outcome
/// ('[CACHE] added | add failed'); the task's value = it was added. `.await` it when the caller
/// is a table leader (real or prefetch), `.detach()` it otherwise.
fn cache_add_task<P: blb::LoadBalancerPolicy + 'static>(
    state: &Arc<ServerState<P>>,
    path: &str,
    segment: &RenderedSegment<'_>,
) -> smol::Task<bool> {
    let key = path.to_string();
    // the blocking thread needs its own handles ('static): the state and the shared body
    let state = Arc::clone(state);
    let data = Arc::clone(&segment.data);
    let (times, config) = (segment.times, segment.config);
    smol::unblock(move || {
        let cache = state
            .segment_cache
            .as_ref()
            .expect("caller checked is_some");
        let size = data.len();
        // bins: which side of the threshold this segment fell
        let cost = match cache {
            SegmentCache::Bin { .. } => {
                format!(" cost {:.2} s", BinCache::cost_s(&times, &config))
            }
            SegmentCache::Single { .. } => String::new(),
        };
        let result = cache.add(&key, &data, &times, &config);
        let added = result.is_ok();
        let outcome = match result {
            Ok(bin) => {
                let to_bin = bin.map(|b| format!(" to {b}")).unwrap_or_default();
                format!("added {size} B{to_bin} (used {} B){cost}", cache.used())
            }
            Err(e) => {
                let reason = match e.kind() {
                    // a concurrent miss on the same path stored it first; the bins say where
                    // ("in upper bin" / "in other bin, upper"), the plain LRU has no message
                    io::ErrorKind::AlreadyExists => match e.get_ref() {
                        Some(where_) => format!("already cached ({where_})"),
                        None => "already cached".to_string(),
                    },
                    io::ErrorKind::InvalidInput => "bigger than the cache".to_string(),
                    _ => e.to_string(),
                };
                format!("add failed ({size} B){cost}: {reason}")
            }
        };
        util::log_message(&format!("[CACHE] {outcome} | {key}"));
        added
    })
}

/// A prefetch of `target` (started by the real request `trigger`), detached from any client:
/// skip if cached / in flight; else lead in the in-flight table, render through the load
/// balancer, publish (in-flight followers wake up), hold the entry for the hold time (later
/// requests get it at once), then: read by any request + segment cache on -> add to the cache
/// (awaited, so no request falls between table and cache); drop the leader.
/// A failed render drops the leader without a value: its followers render themselves.
/// Logs: '[PREFETCH] skip: <reason> | target', 'issue | trigger -> target',
/// 'done | site | Tms | work W asset A | N followers | target', 'failed | site | reason | target',
/// 'expire | N readers | cache added / cache add failed / not read / cache off | target' (+ the cache's own [CACHE] line).
pub async fn prefetch_task<P: blb::LoadBalancerPolicy + 'static>(
    state: Arc<ServerState<P>>,
    trigger: String,
    target: String,
) {
    let log = |what: String| util::log_message(&format!("[PREFETCH] {what}"));
    let leader = match segment_lookup(&state, &target, LookupMode::Prefetch) {
        SegmentLookup::Lead(Some(leader)) => leader,
        SegmentLookup::Skip(reason) => return log(format!("skip: {reason} | {target}")),
        // a Prefetch lookup gives nothing else; the prefetch table always exists when on
        _ => return log(format!("skip: no table | {target}")),
    };
    log(format!("issue | {trigger} -> {target}"));

    let segment = match fetch_offload(&state, &target).await {
        Ok(Ok(segment)) if segment.status_ok => segment,
        Ok(Ok(segment)) => {
            let site = segment.site;
            return log(format!("failed | {site} | status not 200 | {target}"));
        }
        Ok(Err(OffloadError { site, err })) => {
            return log(format!("failed | {site} | {err} | {target}"));
        }
        Err(err) => return log(format!("failed | - | {err} | {target}")),
    };
    leader.publish(Arc::clone(&segment.data)).await;
    let t = &segment.times;
    log(format!(
        "done | {} | {}ms | work {} asset {} | {} followers | {target}",
        segment.site,
        segment.total_ms,
        t.t_work,
        t.t_asset,
        leader.followers()
    ));

    smol::Timer::after(
        state
            .prefetch
            .as_ref()
            .map_or_else(Default::default, |p| p.hold()),
    )
    .await;

    let readers = leader.joined();
    let outcome = if readers == 0 {
        "not read"
    } else if state.segment_cache.is_some() {
        // refused = the path is cached already (a real render of it stored first) or too big;
        // the [CACHE] line just before says which
        if cache_add_task(&state, &target, &segment).await {
            "cache added"
        } else {
            "cache add failed"
        }
    } else {
        "cache off"
    };
    log(format!("expire | {readers} readers | {outcome} | {target}"));
    // leader drops here: the entry leaves the table
}
