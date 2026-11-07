//! The main server's state and everything that builds it at startup: the cluster CSV (offload
//! sites), the load balancer builders (cost model / lookup table / ACO seed), and the features
//! from their env vars (segment cache, coalescing, prefetch). All of it runs once, before the
//! server accepts a connection; the request path is in lib_segment.rs and main_server.rs.

use std::io;
use std::path::Path;
use std::sync::Arc;

use rust_obm::cost_lookup::CostLookup;
use rust_obm::cost_predict::CostPredictor;
use rust_obm::mod_feature::{
    BinCache, CachePolicy, Coalescer, MergedPrefetch, PolicyCache, SegmentCache,
};
use rust_obm::mod_load_balancer::base_balancer::{LoadBalancer, LoadBalancerPolicy};
use rust_obm::mod_load_balancer::config::{Config, Decode, Encode, Render};
use rust_obm::mod_load_balancer::policy_aco::ACO;
use rust_obm::mod_load_balancer::policy_mct::MCT;
use rust_obm::mod_load_balancer::{AcoLB, MctLB, OffloadSite};

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
    /// in-flight table: a request for a path already rendering waits for that render. Built when
    /// coalescing or prefetch is on (prefetches lead in it even with coalescing off)
    pub coalescer: Option<SegmentCoalescer>,
    /// OBM_COALESCE: real requests may lead in the table (and so merge with each other). Off with
    /// prefetch on: real requests only follow prefetch leaders, never lead
    pub coalesce_real: bool,
    /// merged prefetch ("2 continue" rule), None = off
    pub prefetch: Option<MergedPrefetch>,
}

/// What a coalescing leader hands its followers: the segment body (status 200 only), shared
/// with the leader's own client write and the cache add
pub type SegmentCoalescer = Coalescer<Arc<Vec<u8>>>;

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
        let coalesce_real = coalesce_loader()?;
        println!("Coalescing {}", if coalesce_real { "ON" } else { "OFF" });
        let prefetch = prefetch_loader()?;
        match &prefetch {
            Some(p) => println!("Prefetch ON: {}", p.describe()),
            None => println!("Prefetch OFF"),
        }
        let coalescer = (coalesce_real || prefetch.is_some()).then(Coalescer::new);
        Ok(Self {
            balancer,
            segment_cache,
            coalescer,
            coalesce_real,
            prefetch,
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

/// [Dishonest I/O] Request coalescing from OBM_COALESCE: 1 = on, unset / 0 = off, anything else
/// is an error. Independent of the segment cache and prefetch (any combination can be on).
pub fn coalesce_loader() -> io::Result<bool> {
    env_switch("OBM_COALESCE")
}

/// [Dishonest I/O] Merged prefetch from OBM_PREFETCH (1 = on, unset / 0 = off),
/// OBM_PREFETCH_WINDOW_S (default 30: k-1 and k within it -> prefetch k+1) and
/// OBM_PREFETCH_HOLD_S (default 30: a finished prefetch stays readable this long). Independent of
/// the segment cache and coalescing.
pub fn prefetch_loader() -> io::Result<Option<MergedPrefetch>> {
    if !env_switch("OBM_PREFETCH")? {
        return Ok(None);
    }
    let window_s = util::env_parse::<f64>("OBM_PREFETCH_WINDOW_S")?.unwrap_or(30.0);
    let hold_s = util::env_parse::<f64>("OBM_PREFETCH_HOLD_S")?.unwrap_or(30.0);
    let hold = std::time::Duration::try_from_secs_f64(hold_s)
        .ok()
        .filter(|_| window_s.is_finite() && window_s >= 0.0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "OBM_PREFETCH_WINDOW_S={window_s} / OBM_PREFETCH_HOLD_S={hold_s}: expected >= 0"
                ),
            )
        })?;
    Ok(Some(MergedPrefetch::new(window_s, hold)))
}

/// [Dishonest I/O] A 0/1 switch: 1 = on, unset / 0 = off, anything else is an error.
fn env_switch(name: &str) -> io::Result<bool> {
    match util::env_parse::<u8>(name)? {
        None | Some(0) => Ok(false),
        Some(1) => Ok(true),
        Some(other) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name}={other}: expected 0 or 1"),
        )),
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
