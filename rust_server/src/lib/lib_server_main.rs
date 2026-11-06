use std::io;
use std::path::Path;
use std::time::Instant;

use rust_obm::cost_lookup::CostLookup;
use rust_obm::cost_predict::CostPredictor;
use rust_obm::mod_feature::{BinCache, CachePolicy, PolicyCache, SegmentCache};
use rust_obm::mod_load_balancer::base_balancer::{LoadBalancer, LoadBalancerPolicy};
use rust_obm::mod_load_balancer::config::{Config, Decode, Encode, Render};
use rust_obm::mod_load_balancer::policy_aco::ACO;
use rust_obm::mod_load_balancer::policy_mct::MCT;
use rust_obm::mod_load_balancer::{AcoLB, MctLB, OffloadSite, TimesFeedback};

use crate::lib_core_type as ct;
use crate::lib_http as http;
use crate::lib_util as util;

/// [Dishonest / I/O Boundary Driver]
/// Reads a cluster endpoint CSV (e.g. servers_container/cluster_endpoint.csv) into offload sites.
pub fn load_cluster_csv(path: impl AsRef<Path>) -> io::Result<Vec<OffloadSite>> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    parse_cluster_csv(&text).map_err(|e| io::Error::other(format!("{}: {e}", path.display())))
}

/// [Honest / Pure Domain Logic]
/// Parses cluster endpoint CSV text into offload sites. Lines starting with '#' and blank lines
/// are skipped; the first remaining line is the header. Required columns: ip, port, decoding,
/// composition, encoding (any extra columns are kept in the name only).
/// endpoint = "{ip}:{port}", name = "{col}={value}" for every column joined by '_'
/// (e.g. "id=1_ip=obm-offload-1_port=7010_decoding=gpu_composition=gpu_encoding=cpu").
pub fn parse_cluster_csv(text: &str) -> Result<Vec<OffloadSite>, String> {
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));

    let (_, header_line) = lines.next().ok_or("no header line")?;
    let header: Vec<&str> = header_line.split(',').map(str::trim).collect();
    let col = |name: &str| {
        header
            .iter()
            .position(|h| *h == name)
            .ok_or(format!("missing column '{name}'"))
    };
    let (ip, port, decoding, composition, encoding) = (
        col("ip")?,
        col("port")?,
        col("decoding")?,
        col("composition")?,
        col("encoding")?,
    );

    let mut sites = Vec::new();
    for (line_no, line) in lines {
        let row: Vec<&str> = line.split(',').map(str::trim).collect();
        if row.len() != header.len() {
            return Err(format!(
                "line {line_no}: expected {} fields, got {}",
                header.len(),
                row.len()
            ));
        }
        let err = |what: &str, value: &str| format!("line {line_no}: invalid {what} '{value}'");

        let decode = match row[decoding] {
            "cpu" => Decode::Cpu,
            "gpu" => Decode::Gpu,
            v => return Err(err("decoding", v)),
        };
        let render = match row[composition] {
            "cpu" => Render::Cpu,
            "llvmpipe" => Render::Llvmpipe,
            "gpu" => Render::Gpu,
            v => return Err(err("composition", v)),
        };
        let encode = match row[encoding] {
            "cpu" => Encode::Cpu,
            "gpu" => Encode::Gpu,
            v => return Err(err("encoding", v)),
        };
        row[port]
            .parse::<u16>()
            .map_err(|_| err("port", row[port]))?;

        let name = header
            .iter()
            .zip(&row)
            .map(|(h, v)| format!("{h}={v}"))
            .collect::<Vec<_>>()
            .join("_");

        sites.push(OffloadSite::new(
            format!("{}:{}", row[ip], row[port]),
            name,
            Config::new(decode, render, encode),
        ));
    }

    if sites.is_empty() {
        return Err("no offload sites".into());
    }
    Ok(sites)
}

/// [Dishonest / I/O Boundary Driver]
/// Logs a failed offload and answers 504 (timeout) or 502 (other error) before any headers are sent.
pub async fn reply_gateway_error(
    client_stream: &mut impl ct::Writer,
    site: &str,
    path: &str,
    err: io::Error,
) -> io::Result<()> {
    util::log_message(&format!("[OFFLOAD FAIL] -> {site} | {err} | {path}"));
    let resp = http::response_gateway_error(err.kind() == io::ErrorKind::TimedOut);
    http::send_raw(client_stream, &resp).await
}

/// [Dishonest / Clock Read]
/// Takes the offload site's response head (as from `split_header_body`, without the final
/// "\r\n\r\n") and returns it with the main server's timing headers appended (X-OBM-Net,
/// X-OBM-Wait, Server-Timing, X-OBM-Worker), ready to send to the client. `t_start` is when the
/// request was sent to the site, `t_wait` the ms it waited at the main server for a site before
/// that (0 unless the policy holds requests). Call it right after the head arrives.
/// Returns the new head and the timing feedback for the load balancer (all zero if the site
/// sent no X-Dana-Timings).
pub fn time_append_header(
    head: &[u8],
    t_start: Instant,
    t_wait: i64,
    worker: &str,
) -> (Vec<u8>, TimesFeedback) {
    let elapsed = t_start.elapsed().as_millis() as i64;

    let head_str = String::from_utf8_lossy(head);
    let times = parse_times_feedback(&head_str, elapsed).unwrap_or_default();
    let t_net_str = times.t_net.to_string();
    let t_wait_str = t_wait.to_string();
    let server_timing_str = format!("net;dur={}, wait;dur={t_wait}", times.t_net);
    let extra_headers = [
        ("X-OBM-Net", t_net_str.as_str()),
        ("X-OBM-Wait", t_wait_str.as_str()),
        ("Server-Timing", server_timing_str.as_str()),
        ("X-OBM-Worker", worker),
        (
            "Access-Control-Expose-Headers",
            "X-Dana-Timings, Server-Timing, X-OBM-Net, X-OBM-Wait, X-OBM-Worker",
        ),
    ];
    (http::response_append(&head_str, &extra_headers), times)
}

/// [Honest / Pure Domain Logic]
/// Reads queue / asset / work from the site's X-Dana-Timings header; the network time is what
/// is left of `elapsed` (ms from sending the request to the head), at least 0.
/// None if the header is missing.
pub fn parse_times_feedback(return_header: &str, elapsed: i64) -> Option<TimesFeedback> {
    let timing_line = return_header
        .lines()
        .find(|l| l.starts_with("X-Dana-Timings:"))?;

    let (mut queue, mut asset, mut work) = (0i64, 0i64, 0i64);
    for part in timing_line
        .trim_start_matches("X-Dana-Timings:")
        .trim()
        .split(';')
    {
        if let Some((k, v)) = part.split_once('=') {
            let val = v.trim().parse::<i64>().unwrap_or(0);
            match k.trim() {
                "queue" => queue = val,
                "asset" => asset = val,
                "work" => work = val,
                _ => {}
            }
        }
    }
    Some(TimesFeedback {
        t_net: (elapsed - (queue + asset + work)).max(0),
        t_queue: queue,
        t_work: work,
        t_asset: asset,
    })
}

/// [Dishonest I/O] ACO's random seed: OBM_ACO_SEED, default 2026 (same as the sequences)
pub fn aco_seed() -> u64 {
    std::env::var("OBM_ACO_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2026)
}

/// [Dishonest I/O] Loads the work lookup table from OBM_COST_LOOKUP
/// (default /app/stat_predict/lookup.csv, baked into the main server image)
pub fn load_cost_lookup() -> io::Result<CostLookup> {
    let csv = std::env::var("OBM_COST_LOOKUP")
        .unwrap_or_else(|_| "/app/stat_predict/lookup.csv".to_string());
    CostLookup::load(&csv)
        .map_err(|e| io::Error::new(e.kind(), format!("MCT: can't load lookup table {csv}: {e}")))
}

/// [Dishonest I/O] Loads the cost model from OBM_COST_COEF / OBM_COST_FEATURES
/// (default /app/stat_predict/{coef,features}.csv, baked into the main server image)
pub fn load_cost_model() -> io::Result<CostPredictor> {
    let coef =
        std::env::var("OBM_COST_COEF").unwrap_or_else(|_| "/app/stat_predict/coef.csv".to_string());
    let features = std::env::var("OBM_COST_FEATURES")
        .unwrap_or_else(|_| "/app/stat_predict/features.csv".to_string());
    CostPredictor::load(&coef, &features).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("MCT: can't load cost model {coef}, {features}: {e}"),
        )
    })
}

/// Everything the main server shares across connections for its whole life: the load balancer
/// and the optional features (None = off). A new feature is one more field here.
pub struct ServerState<P: LoadBalancerPolicy> {
    pub balancer: LoadBalancer<P>,
    pub segment_cache: Option<SegmentCache>,
}

impl<P: LoadBalancerPolicy> ServerState<P> {
    /// [Dishonest I/O] Wraps `balancer` and loads every feature from its env vars, printing
    /// what is on.
    pub fn new(balancer: LoadBalancer<P>) -> io::Result<Self> {
        println!(
            "Load balancer type = {}",
            std::any::type_name_of_val(&balancer)
        );
        let segment_cache = segment_cache_loader()?;
        match &segment_cache {
            Some(cache) => println!("Segment cache ON: {}", cache.describe()),
            None => println!("Segment cache OFF"),
        }
        Ok(Self {
            balancer,
            segment_cache,
        })
    }
}

/// [Dishonest I/O] The segment cache from env vars, None = off. Either:
/// - single cache: OBM_SEGMENT_CACHE_MB (unset / 0 = off), OBM_SEGMENT_CACHE_POLICY lru | lfu
///   (default lru), files in OBM_SEGMENT_CACHE_DIR (default /tmp/obm_cache/segment);
/// - cost bins: OBM_BIN_CACHE_LOWER_MB + OBM_BIN_CACHE_UPPER_MB (both unset / sum 0 = off; an
///   unset one is 0 MB), OBM_BIN_CACHE_THRESHOLD_S (default 6.0), files in
///   OBM_BIN_CACHE_DIR/{lower,upper} (default /tmp/obm_cache/bin).
///
/// Both on is an error: they are the same feature. Folders are emptied at start.
pub fn segment_cache_loader() -> io::Result<Option<SegmentCache>> {
    let single_mb = util::env_parse::<usize>("OBM_SEGMENT_CACHE_MB")?.unwrap_or(0);
    let lower_mb = util::env_parse::<usize>("OBM_BIN_CACHE_LOWER_MB")?.unwrap_or(0);
    let upper_mb = util::env_parse::<usize>("OBM_BIN_CACHE_UPPER_MB")?.unwrap_or(0);
    let bin_on = lower_mb + upper_mb > 0;

    match (single_mb > 0, bin_on) {
        (false, false) => Ok(None),
        (true, true) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OBM_SEGMENT_CACHE_MB and OBM_BIN_CACHE_*_MB are both set: they are the same \
             feature (segment cache), set only one",
        )),
        (true, false) => {
            let dir = std::env::var("OBM_SEGMENT_CACHE_DIR")
                .unwrap_or_else(|_| "/tmp/obm_cache/segment".to_string());
            let policy = util::env_parse::<CachePolicy>("OBM_SEGMENT_CACHE_POLICY")?
                .unwrap_or(CachePolicy::Lru);
            let cache = PolicyCache::new(&dir, single_mb << 20, policy)
                .map_err(|e| io::Error::new(e.kind(), format!("segment cache dir {dir}: {e}")))?;
            Ok(Some(SegmentCache::Single {
                cache,
                mb: single_mb,
            }))
        }
        (false, true) => {
            let threshold_s = util::env_parse::<f64>("OBM_BIN_CACHE_THRESHOLD_S")?.unwrap_or(6.0);
            let dir = std::env::var("OBM_BIN_CACHE_DIR")
                .unwrap_or_else(|_| "/tmp/obm_cache/bin".to_string());
            let dir = Path::new(&dir);
            let cache = BinCache::new(
                dir.join("lower"),
                dir.join("upper"),
                lower_mb << 20,
                upper_mb << 20,
                threshold_s,
            )
            .map_err(|e| io::Error::new(e.kind(), format!("bin cache {}: {e}", dir.display())))?;
            Ok(Some(SegmentCache::Bin {
                cache,
                lower_mb,
                upper_mb,
                threshold_s,
            }))
        }
    }
}

// ---- Load balancer builders: one per policy that needs more than `new`. Each returns one
// concrete type (MCT / ACO hold a closure, so the type can't be written: `impl` hides it), so
// the policy `match` stays in main_server and every arm calls `serve` with its own type.

/// Work time (s) for the _const policies: mean over the v3 replays (stat/old_nonfifo/run_a*_c1,
/// run_a*_c2_p75, 4027 ok requests), weighted by how often each request really arrives
pub const CONST_WORK_S: f64 = 5.59;

/// [Dishonest I/O] OBM_MCT_FALLBACK=1 turns on the ESCE fallback (mct / aco); default off = the
/// pure policy for the experiments
fn mct_fallback() -> bool {
    std::env::var("OBM_MCT_FALLBACK").is_ok_and(|v| v == "1")
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

/// [Dishonest I/O] mct: MCT with the cost model (load_cost_model)
pub fn mct_balancer(sites: Vec<OffloadSite>) -> io::Result<LoadBalancer<impl LoadBalancerPolicy>> {
    let predictor = load_cost_model()?;
    let fallback = mct_fallback();
    println!(
        "MCT: {} paths, fallback {}",
        predictor.len(),
        on_off(fallback)
    );
    let predict = move |path: &str, site: &OffloadSite| predictor.predict(path, site);
    let mct = MCT::with_predict(&sites, predict, fallback);
    Ok(MctLB::with_policy(sites, mct))
}

/// [Dishonest I/O] mct_lookup: MCT with measured work per (site config, path) from the replay
/// runs (load_cost_lookup); a pair never measured falls back to the cost model (logged as
/// "mct_lookup miss")
pub fn mct_lookup_balancer(
    sites: Vec<OffloadSite>,
) -> io::Result<LoadBalancer<impl LoadBalancerPolicy>> {
    let lookup = load_cost_lookup()?;
    let predictor = load_cost_model()?;
    println!(
        "MCT: lookup {} (config, path) rows, cost model {} paths for the rest",
        lookup.len(),
        predictor.len()
    );
    let predict = move |path: &str, site: &OffloadSite| {
        lookup.predict(path, site).or_else(|| {
            println!("mct_lookup miss site={} path={path}", site.name);
            predictor.predict(path, site)
        })
    };
    let mct = MCT::with_predict(&sites, predict, false);
    Ok(MctLB::with_policy(sites, mct))
}

/// mct_const: same work for every path and site, MCT without the cost model (ablation)
pub fn mct_const_balancer(sites: Vec<OffloadSite>) -> LoadBalancer<impl LoadBalancerPolicy> {
    let const_predict = |_: &str, _: &OffloadSite| Some(CONST_WORK_S);
    println!("MCT: constant work {CONST_WORK_S} s");
    let mct = MCT::with_predict(&sites, const_predict, false);
    MctLB::with_policy(sites, mct)
}

/// mct_const_flat: mct_const without the om / rs priors (every site starts at factor 1.0,
/// network 1.0 s)
pub fn mct_const_flat_balancer(sites: Vec<OffloadSite>) -> LoadBalancer<impl LoadBalancerPolicy> {
    let const_predict = |_: &str, _: &OffloadSite| Some(CONST_WORK_S);
    println!("MCT: constant work {CONST_WORK_S} s, flat prior (factor 1.0, network 1.0 s)");
    let mct = MCT::with_prior(&sites, const_predict, false, Some((1.0, 1.0)));
    MctLB::with_policy(sites, mct)
}

/// [Dishonest I/O] aco: ACO with the cost model, same fallback switch as mct, seed from
/// OBM_ACO_SEED
pub fn aco_balancer(sites: Vec<OffloadSite>) -> io::Result<LoadBalancer<impl LoadBalancerPolicy>> {
    let predictor = load_cost_model()?;
    let fallback = mct_fallback();
    let seed = aco_seed();
    println!(
        "ACO: {} paths, fallback {}, seed {seed}",
        predictor.len(),
        on_off(fallback)
    );
    let predict = move |path: &str, site: &OffloadSite| predictor.predict(path, site);
    let aco = ACO::with_predict(&sites, predict, fallback, seed);
    Ok(AcoLB::with_policy(sites, aco))
}

/// [Dishonest I/O] aco_const: same work for every path and site, the pheromone has to learn
/// the sizes
pub fn aco_const_balancer(sites: Vec<OffloadSite>) -> LoadBalancer<impl LoadBalancerPolicy> {
    let const_predict = |_: &str, _: &OffloadSite| Some(CONST_WORK_S);
    let seed = aco_seed();
    println!("ACO: constant work {CONST_WORK_S} s, seed {seed}");
    let aco = ACO::with_predict(&sites, const_predict, false, seed);
    AcoLB::with_policy(sites, aco)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_times_feedback() {
        let head = "HTTP/1.0 200 OK\r\nContent-length: 10\r\nX-Dana-Timings: queue=3;asset=611;work=2546;decode=631\r\nConnection: close";
        let times = parse_times_feedback(head, 3600).unwrap();
        assert_eq!(
            times,
            TimesFeedback {
                t_net: 440,
                t_queue: 3,
                t_work: 2546,
                t_asset: 611
            }
        );
        // clock coarser than the site's own timings: network time never negative
        assert_eq!(parse_times_feedback(head, 3000).unwrap().t_net, 0);
        assert!(parse_times_feedback("HTTP/1.0 200 OK\r\nServer: x", 100).is_none());
    }

    const CSV: &str = "# Columns:\n#   id: worker id\n\nid,ip,port,decoding,composition,encoding\n1,obm-offload-1,7010,gpu,gpu,cpu\n2, 100.116.177.11 ,7020,gpu,llvmpipe,cpu\n";

    #[test]
    fn test_parse_cluster_csv() {
        let sites = parse_cluster_csv(CSV).unwrap();
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].endpoint, "obm-offload-1:7010");
        assert_eq!(
            sites[0].name,
            "id=1_ip=obm-offload-1_port=7010_decoding=gpu_composition=gpu_encoding=cpu"
        );
        assert_eq!(sites[1].endpoint, "100.116.177.11:7020");
        assert_eq!(sites[1].config.render, Render::Llvmpipe);
        assert_eq!(sites[1].config.decode, Decode::Gpu);
        assert_eq!(sites[1].config.encode, Encode::Cpu);
    }

    #[test]
    fn test_parse_cluster_csv_errors() {
        assert!(parse_cluster_csv("").is_err());
        assert!(parse_cluster_csv("id,ip,port,decoding,composition,encoding\n").is_err());
        assert!(
            parse_cluster_csv("id,port,decoding,composition,encoding\n1,7010,gpu,gpu,cpu").is_err()
        );
        let bad = "ip,port,decoding,composition,encoding\nh,7010,gpu,vulkan,cpu";
        assert!(
            parse_cluster_csv(bad)
                .unwrap_err()
                .contains("composition 'vulkan'")
        );
        assert!(
            parse_cluster_csv("ip,port,decoding,composition,encoding\nh,99999,gpu,gpu,cpu")
                .is_err()
        );
        assert!(parse_cluster_csv("ip,port,decoding,composition,encoding\nh,7010,gpu").is_err());
    }
}
