//! Getting a segment for the main server, without touching any client: where it comes from
//! (segment_lookup: cache / in-flight table / render), rendering it on a site through the load
//! balancer (fetch_offload), sharing it with the cache and the followers (offload_cache_add), and
//! the prefetch that does all of it with no client (prefetch_task). main_server.rs keeps the
//! client side: the request flow (serve_offload) and the three ways to answer (cache file,
//! follower, own render).

use std::sync::Arc;

use smol::{io, net::TcpStream};

use rust_obm::mod_feature::{BinCache, Creating, Follower, Leader, Plan, Role, SegmentCache};
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
/// never talks to the client. `path` is what the site renders (and what the load balancer
/// prices): the client's path, or a PSC pair / rebuild path. `body` = None: a GET; Some: a POST
/// with that body (a PSC rebuild pushes its sub segment). Inner Err = the site failed before the
/// body was complete (the caller answers 502 / 504); outer Err = sending the request failed.
pub async fn fetch_offload<'a, P: blb::LoadBalancerPolicy + 'static>(
    state: &'a Arc<ServerState<P>>,
    path: &str,
    body: Option<&[u8]>,
) -> io::Result<Result<RenderedSegment<'a>, OffloadError<'a>>> {
    use http::{request_get, request_with_body, send_raw};
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

    match body {
        None => {
            let message = request_get(path, target_offload.endpoint, MAIN_SERVER_AGENT, &[]);
            send_raw(&mut offload_steam, &message).await?;
        }
        Some(body) => {
            let head = request_with_body(
                "POST",
                path,
                target_offload.endpoint,
                MAIN_SERVER_AGENT,
                body.len(),
            );
            send_raw(&mut offload_steam, &head).await?;
            send_raw(&mut offload_steam, body).await?;
        }
    }

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

/// A segment for a client request (see [`fetch_segment`]) and the PSC work left for after the
/// client write.
pub struct Fetched<'a> {
    pub segment: RenderedSegment<'a>,
    pub psc: PscDone<'a>,
}

/// PSC work to finish after the client has its segment ([`finish_psc`])
pub enum PscDone<'a> {
    Nothing,
    /// a pair made this sub: store it, then drop the 'pair active' mark
    Created {
        key: String,
        sub: Vec<u8>,
        guard: Creating<'a>,
    },
}

/// The segment for a client request, through the partial segment cache when it is on:
/// - Plain: the request as it came (also with PSC off). Logged '[PSC] plain | reason' when the
///   request had a cut (not for 'no cut');
/// - Create: the pair `<k>*<n>` is rendered; the reply is split into the full (= the segment,
///   head fixed) and the sub, kept in [`PscDone::Created`] for [`finish_psc`]. A reply without
///   X-PSC-Sub-Length keeps the whole body as the full and stores nothing;
/// - Rebuild: the cached sub is read and pushed with a POST of `<k>+<n>`; a site that could not
///   use it answers 502 (X-PSC-Rebuild: failed), passed on to the client as it is (D83: no
///   fallback render). Sub evicted / unreadable since the decision -> plain.
///
/// PSC requests get `X-OBM-PSC: create <k> | rebuild <k>` in the head. Logs: '[PSC] create k=K |
/// full F rebuild R save S | site | sub N B | path', '[PSC] rebuild k=K | ... | site | path',
/// '[PSC] create failed | ...', '[PSC] rebuild failed | site | why | path'.
pub async fn fetch_segment<'a, P: blb::LoadBalancerPolicy + 'static>(
    state: &'a Arc<ServerState<P>>,
    path: &str,
) -> io::Result<Result<Fetched<'a>, OffloadError<'a>>> {
    let plain = |segment: RenderedSegment<'a>| Fetched {
        segment,
        psc: PscDone::Nothing,
    };
    let Some(psc) = &state.psc else {
        return Ok(fetch_offload(state, path, None).await?.map(plain));
    };
    let log = |what: String| util::log_message(&format!("[PSC] {what}"));
    let plan = psc.policy.decide(
        path,
        psc.policy.now_s(),
        |key| psc.store.contains_key(key),
        |p| psc.cost_s(p),
    );
    match plan {
        Plan::Plain(why) => {
            if why != "no cut" {
                log(format!("plain | {why} | {path}"));
            }
            Ok(fetch_offload(state, path, None).await?.map(plain))
        }
        Plan::Create { cand, costs, guard } => {
            let mut segment = match fetch_offload(state, &cand.pair_path, None).await? {
                Ok(segment) => segment,
                Err(e) => {
                    log(format!("create failed | {} | {} | {path}", e.site, e.err));
                    return Ok(Err(e));
                }
            };
            let tag = format!("create {}", cand.k);
            if !segment.status_ok {
                log(format!(
                    "create failed | {} | status not 200 | {path}",
                    segment.site
                ));
                segment.head = http::head_with(&segment.head, &[("X-OBM-PSC", &tag)], &[]);
                return Ok(Ok(plain(segment)));
            }
            let Some((head, full_len)) = http::split_pair_head(&segment.head, segment.data.len())
            else {
                log(format!(
                    "create: no sub in the reply | {} | {path}",
                    segment.site
                ));
                return Ok(Ok(plain(segment)));
            };
            let mut body = Arc::try_unwrap(segment.data).unwrap_or_else(|shared| (*shared).clone());
            let sub = body.split_off(full_len);
            segment.data = Arc::new(body);
            segment.head = http::head_with(&head, &[("X-OBM-PSC", &tag)], &[]);
            log(format!(
                "create k={} | {costs} | {} | sub {} B | {path}",
                cand.k,
                segment.site,
                sub.len()
            ));
            Ok(Ok(Fetched {
                segment,
                psc: PscDone::Created {
                    key: cand.key,
                    sub,
                    guard,
                },
            }))
        }
        Plan::Rebuild { cand, costs } => {
            let Some((file, size)) = psc.store.get(&cand.key) else {
                log(format!("plain | sub evicted | {path}"));
                return Ok(fetch_offload(state, path, None).await?.map(plain));
            };
            let read = smol::unblock(move || {
                use std::io::Read;
                let mut sub = Vec::with_capacity(size);
                let mut file = file;
                file.read_to_end(&mut sub).map(|_| sub)
            })
            .await;
            let sub = match read {
                Ok(sub) => sub,
                Err(e) => {
                    log(format!("plain | sub read failed: {e} | {path}"));
                    return Ok(fetch_offload(state, path, None).await?.map(plain));
                }
            };
            let mut segment = match fetch_offload(state, &cand.rebuild_path, Some(&sub)).await? {
                Ok(segment) => segment,
                Err(e) => {
                    log(format!("rebuild failed | {} | {} | {path}", e.site, e.err));
                    return Ok(Err(e));
                }
            };
            if segment.status_ok {
                log(format!(
                    "rebuild k={} | {costs} | {} | {path}",
                    cand.k, segment.site
                ));
            } else {
                let why = http::header_value(&segment.head, "X-PSC-Rebuild")
                    .unwrap_or_else(|| "status not 200".to_string());
                log(format!(
                    "rebuild failed | {} | {why} | {path}",
                    segment.site
                ));
            }
            let tag = format!("rebuild {}", cand.k);
            segment.head = http::head_with(&segment.head, &[("X-OBM-PSC", &tag)], &[]);
            Ok(Ok(plain(segment)))
        }
    }
}

/// After the client write: a pair's sub goes into the PSC store (blocking thread), then the
/// 'pair active' mark is dropped (until then other requests for that sub render normally).
/// Logs '[PSC] stored N B (used M B) | key' or '[PSC] store failed (N B): why | key'.
pub async fn finish_psc<P: blb::LoadBalancerPolicy + 'static>(
    state: &Arc<ServerState<P>>,
    done: PscDone<'_>,
) {
    let PscDone::Created { key, sub, guard } = done else {
        return;
    };
    let shared = Arc::clone(state);
    let k = key.clone();
    let outcome = smol::unblock(move || {
        let Some(psc) = shared.psc.as_ref() else {
            return "store failed: PSC off".to_string();
        };
        let size = sub.len();
        match psc.store.add(&k, &sub) {
            Ok(()) => format!("stored {size} B (used {} B)", psc.store.used()),
            Err(e) => {
                let why = match e.kind() {
                    io::ErrorKind::AlreadyExists => "already stored".to_string(),
                    io::ErrorKind::InvalidInput => "bigger than the store".to_string(),
                    _ => e.to_string(),
                };
                format!("store failed ({size} B): {why}")
            }
        }
    })
    .await;
    util::log_message(&format!("[PSC] {outcome} | {key}"));
    drop(guard);
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

    let segment = match fetch_offload(&state, &target, None).await {
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

#[cfg(test)]
mod test {
    //! fetch_segment end to end against a fake site on 127.0.0.1 (no Dana): pair split + store,
    //! rebuild POST with the sub as body, rebuild 502 passed on, PSC off = plain GET.
    use super::*;
    use crate::lib_main_state::Psc;
    use rust_obm::cost_predict::CostPredictor;
    use rust_obm::mod_feature::{CachePolicy, CutPick, CutTable, PolicyCache, PscPolicy};
    use rust_obm::mod_load_balancer::config::{Config, Decode, Encode, Render};
    use rust_obm::mod_load_balancer::{OffloadSite, RoundRobinLB};
    use smol::io::{AsyncReadExt, AsyncWriteExt};
    use smol::net::TcpListener;

    const SAM: &str = "/offload/show/f1.json/0/10/1280/720/race/landscape/2/race|race/drivers|Sam";
    const ALEX: &str =
        "/offload/show/f1.json/0/10/1280/720/race/landscape/2/race|race/drivers|Alex";
    const CUTS: &str = "rest,k,n,roots,prefix_pos,one_to_one,sub_id,show\n\
        1280/720/race/landscape/2/race|race/drivers|Sam,1,2,2,0,1,f1.json/1280/720/race/landscape/1/race|race,f1.json\n\
        1280/720/race/landscape/2/race|race/drivers|Alex,1,2,2,0,1,f1.json/1280/720/race/landscape/1/race|race,f1.json\n";
    const COEF: &str = "stage,backend,a,n_video\n\
        decode,gpu,0,1\nrender,gpu,0,0\nencode,gpu+cpu,1,0\n";
    // full = 2 videos (3 s), rebuild = 1 video (2 s): save 1 s
    const FEATURES: &str = "n_video,seg_frac,path\n\
        2,1,/offload/show/f1.json/0/10/1280/720/race/landscape/2/race|race/drivers|Sam\n\
        1,1,/offload/show/f1.json/0/10/1280/720/race/landscape/1+2/race|race/drivers|Sam\n\
        2,1,/offload/show/f1.json/0/10/1280/720/race/landscape/2/race|race/drivers|Alex\n\
        1,1,/offload/show/f1.json/0/10/1280/720/race/landscape/1+2/race|race/drivers|Alex\n";

    /// A fake site answering each connection with the next reply; returns (address, task giving
    /// every request as (head text, body)).
    async fn fake_site(replies: Vec<Vec<u8>>) -> (String, smol::Task<Vec<(String, Vec<u8>)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let task = smol::spawn(async move {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut s, _) = listener.accept().await.unwrap();
                let (head, mut rest) = http::split_header_body(&mut s).await.unwrap();
                let len = http::parse_content_length(&head).unwrap_or(0) as usize;
                let mut body = vec![0u8; len];
                rest.read_exact(&mut body).await.unwrap();
                drop(rest);
                s.write_all(&reply).await.unwrap();
                s.flush().await.unwrap();
                seen.push((String::from_utf8_lossy(&head).into_owned(), body));
            }
            seen
        });
        (addr, task)
    }

    fn reply(status: &str, extra: &str, body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.0 {status}\r\nContent-length: {}\r\n{extra}X-Dana-Timings: queue=0;asset=1;work=2;decode=1;render=1;encode=0\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn state(
        addr: &str,
        psc_on: bool,
        dir: &str,
    ) -> Arc<ServerState<rust_obm::mod_load_balancer::policy_robin::RoundRobin>> {
        let site = OffloadSite::new(
            addr,
            "fake",
            Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu),
        );
        let dir = std::env::temp_dir().join(format!("obm_psc_test_{dir}_{}", std::process::id()));
        let psc = psc_on.then(|| Psc {
            policy: PscPolicy::new(
                CutTable::from_csv(CUTS).unwrap(),
                0.5,
                1,
                600.0,
                CutPick::Smallest,
            ),
            store: PolicyCache::new(&dir, 1 << 20, CachePolicy::Lfu).unwrap(),
            mb: 1,
            cost: CostPredictor::from_csv(COEF, FEATURES).unwrap(),
            ref_site: OffloadSite::new(
                "ref:0",
                "ref",
                Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu),
            ),
        });
        Arc::new(ServerState {
            balancer: RoundRobinLB::new(vec![site]),
            segment_cache: None,
            coalescer: None,
            coalesce_real: false,
            prefetch: None,
            psc,
        })
    }

    #[test]
    fn test_create_then_rebuild_then_failed_rebuild() {
        smol::block_on(async {
            let (full, sub) = (b"FULL-SEGMENT".to_vec(), b"SUB".to_vec());
            let pair_body = [full.clone(), sub.clone()].concat();
            let (addr, site) = fake_site(vec![
                reply("200 OK", "X-PSC-Sub-Length: 3\r\n", &pair_body),
                reply("200 OK", "", b"REBUILT"),
                reply("502 Bad Gateway", "X-PSC-Rebuild: failed\r\n", b"no sub"),
            ])
            .await;
            let st = state(&addr, true, "flow");

            // 1. Sam, nothing cached, thr 1 -> pair: client gets the full only, sub kept
            let Ok(f) = fetch_segment(&st, SAM).await.unwrap() else {
                panic!("site error")
            };
            assert_eq!(*f.segment.data, full);
            assert_eq!(
                http::header_value(&f.segment.head, "Content-Length").as_deref(),
                Some("12")
            );
            assert_eq!(
                http::header_value(&f.segment.head, "X-PSC-Sub-Length"),
                None
            );
            assert_eq!(
                http::header_value(&f.segment.head, "X-OBM-PSC").as_deref(),
                Some("create 1")
            );
            let key = match &f.psc {
                PscDone::Created { key, sub: s, .. } => {
                    assert_eq!(s, &sub);
                    key.clone()
                }
                PscDone::Nothing => panic!("no sub"),
            };
            let psc = st.psc.as_ref().unwrap();
            assert_eq!(psc.policy.creating(), 1);
            finish_psc(&st, f.psc).await;
            assert!(psc.store.contains_key(&key));
            assert_eq!(psc.policy.creating(), 0);

            // 2. Alex shares the sub -> rebuild: POST 1+2 with the sub as body
            let Ok(f) = fetch_segment(&st, ALEX).await.unwrap() else {
                panic!("site error")
            };
            assert!(f.segment.status_ok);
            assert_eq!(*f.segment.data, b"REBUILT");
            assert_eq!(
                http::header_value(&f.segment.head, "X-OBM-PSC").as_deref(),
                Some("rebuild 1")
            );

            // 3. the site could not use the sub -> its 502 goes to the client
            let Ok(f) = fetch_segment(&st, ALEX).await.unwrap() else {
                panic!("site error")
            };
            assert!(!f.segment.status_ok);
            assert!(f.segment.head.starts_with(b"HTTP/1.0 502"));
            assert_eq!(
                http::header_value(&f.segment.head, "X-OBM-PSC").as_deref(),
                Some("rebuild 1")
            );

            let seen = site.await;
            assert!(
                seen[0]
                    .0
                    .starts_with(&format!("GET {}", SAM.replace("/2/race", "/1*2/race")))
            );
            assert!(seen[0].1.is_empty());
            assert!(
                seen[1]
                    .0
                    .starts_with(&format!("POST {}", ALEX.replace("/2/race", "/1+2/race")))
            );
            assert_eq!(seen[1].1, sub);
            assert_eq!(seen[2].1, sub);
        });
    }

    #[test]
    fn test_psc_off_and_pair_reply_without_sub() {
        smol::block_on(async {
            let (addr, site) = fake_site(vec![
                reply("200 OK", "", b"PLAIN"),
                reply("200 OK", "", b"ONLY-FULL"),
            ])
            .await;
            // PSC off: the path as it came, GET, no X-OBM-PSC
            let off = state(&addr, false, "off");
            let Ok(f) = fetch_segment(&off, SAM).await.unwrap() else {
                panic!("site error")
            };
            assert_eq!(*f.segment.data, b"PLAIN");
            assert_eq!(http::header_value(&f.segment.head, "X-OBM-PSC"), None);
            // PSC on but the site's pair reply has no sub: keep the body as the full, store nothing
            let on = state(&addr, true, "nosub");
            let Ok(f) = fetch_segment(&on, SAM).await.unwrap() else {
                panic!("site error")
            };
            assert_eq!(*f.segment.data, b"ONLY-FULL");
            assert!(matches!(f.psc, PscDone::Nothing));
            assert_eq!(on.psc.as_ref().unwrap().policy.creating(), 0);
            let seen = site.await;
            assert!(seen[0].0.starts_with(&format!("GET {SAM} ")));
            assert!(seen[1].0.contains("/1*2/"));
        });
    }
}
