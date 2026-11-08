//! Partial segment cache (PSC) policy: which bottom sub segment to create, and when to rebuild a
//! request from a cached one. Decisions only; the main server owns the store (a `PolicyCache`),
//! the sites and the clock.
//!
//! A request `/offload/show/<show>/<t0>/<t1>/<w>/<h>/<variant>/<style>/<n>/<layer|opt>...` can be
//! cut after root k (1 <= k < n). The sub segment = roots 0..k-1 encoded on their own; every
//! request with the same bottom layers at the same segment shares it (f1 drivers Sam / Alex: one
//! race sub). Which tokens belong to the bottom needs the show's layer graph, so the cuts come
//! precomputed: stat_predict/psc_cuts.csv ([`CutTable`]).
//!
//! The site understands two count tokens besides `<n>`:
//! - `<k>*<n>` PAIR: one pass renders the full AND the sub (the viewer gets the full, the sub is
//!   stored) = how a sub is created;
//! - `<k>+<n>` REBUILD: the bottom comes from the cached sub (pushed with the job), roots k..n-1
//!   are rendered on top.
//!
//! [`PscPolicy::decide`] on a segment cache miss (same rule as temp/psc_study online()):
//! 1. a cut whose sub is cached and saves time -> Rebuild from the one with the biggest save;
//! 2. else count the request in every cut's popularity window, and create (Pair) one that saves
//!    at least heavy, was seen at least thr times in the window, is not cached and not being
//!    created: the smallest k
//!    ([`CutPick::Smallest`], max sharing) or the biggest save ([`CutPick::BiggestSave`]);
//! 3. else Plain (normal full request). A sub being created is not waited for (PSC coalescing
//!    off): other requests for it render normally until it is stored.
//!
//! save = cost(full) - cost(rebuild), from the caller's cost function (the cost model on a
//! reference site, machine-neutral). The module is honest: the clock (`now_s`) and the cost are
//! passed in; [`PscPolicy::now_s`] and [`CutTable::load`] are the dishonest helpers.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use crate::cost_predict::percent_decode;
use crate::obm_parse::SegmentPath;

/// Index of the count token in a request path split by '/'
const COUNT: usize = 10;

/// Prune the popularity table when it grows past this many keys
const PRUNE_AT: usize = 4096;

/// psc_cuts.csv header (stat_predict/psc_cuts.py)
const CUTS_HEADER: &str = "rest,k,n,roots,prefix_pos,one_to_one,sub_id,show";

/// One valid cut of one request kind (a row of psc_cuts.csv)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    /// roots 0..k-1 form the sub
    pub k: u32,
    /// the request's count token
    pub n: u32,
    /// indices of the request's layer tokens that belong to the sub
    pub prefix_pos: Vec<usize>,
    /// request size == the variant's size (else the pair's full is not exact: never pair)
    pub one_to_one: bool,
    /// show/w/h/variant/style/k/sorted sub tokens: the sub key without the segment
    pub sub_id: String,
}

/// Cuts per request kind: (show file, `rest` = the path after t_end, '|' and spaces decoded) ->
/// cuts by k. The show is part of the key: f1.json and f1_full.json have the same rests.
#[derive(Debug, Default)]
pub struct CutTable {
    by_kind: HashMap<(String, String), Vec<Cut>>,
}

impl CutTable {
    /// [Dishonest I/O] Loads and checks psc_cuts.csv.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Self::from_csv(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// [Honest] Parses psc_cuts.csv text. Checks: exact header, 8 fields per row (no quoted
    /// fields: a comma inside an option would need quoting, refused rather than mis-split),
    /// 1 <= k < n, k < roots, prefix positions numbers, no duplicate (show, rest, k).
    pub fn from_csv(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        match lines.next() {
            Some(h) if h.trim() == CUTS_HEADER => {}
            other => {
                return Err(format!(
                    "psc_cuts.csv: header {other:?}, expected {CUTS_HEADER}"
                ));
            }
        }
        let mut by_kind: HashMap<(String, String), Vec<Cut>> = HashMap::new();
        for (i, line) in lines.enumerate() {
            let row = i + 2;
            if line.contains('"') {
                return Err(format!(
                    "psc_cuts.csv row {row}: quoted field not supported"
                ));
            }
            let f: Vec<&str> = line.split(',').collect();
            if f.len() != 8 {
                return Err(format!(
                    "psc_cuts.csv row {row}: expected 8 fields, got {}",
                    f.len()
                ));
            }
            let num = |s: &str, what: &str| {
                s.parse::<u32>()
                    .map_err(|_| format!("psc_cuts.csv row {row}: bad {what} '{s}'"))
            };
            let (k, n, roots) = (num(f[1], "k")?, num(f[2], "n")?, num(f[3], "roots")?);
            if k == 0 || k >= n || k >= roots {
                return Err(format!(
                    "psc_cuts.csv row {row}: invalid cut k {k} n {n} roots {roots}"
                ));
            }
            let prefix_pos = f[4]
                .split(';')
                .map(|p| {
                    p.parse::<usize>()
                        .map_err(|_| format!("psc_cuts.csv row {row}: bad prefix_pos '{}'", f[4]))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let one_to_one = match f[5] {
                "1" => true,
                "0" => false,
                v => return Err(format!("psc_cuts.csv row {row}: bad one_to_one '{v}'")),
            };
            let cuts = by_kind
                .entry((f[7].to_string(), f[0].to_string()))
                .or_default();
            if cuts.iter().any(|c| c.k == k) {
                return Err(format!(
                    "psc_cuts.csv row {row}: duplicate cut k {k} for {} {}",
                    f[7], f[0]
                ));
            }
            cuts.push(Cut {
                k,
                n,
                prefix_pos,
                one_to_one,
                sub_id: f[6].to_string(),
            });
        }
        for cuts in by_kind.values_mut() {
            cuts.sort_by_key(|c| c.k);
        }
        Ok(Self { by_kind })
    }

    /// [Honest] Cuts of a request kind (`rest` decoded), smallest k first; empty if none.
    pub fn cuts(&self, show: &str, rest: &str) -> &[Cut] {
        self.by_kind
            .get(&(show.to_string(), rest.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// Number of request kinds with at least one cut
    pub fn kinds(&self) -> usize {
        self.by_kind.len()
    }
}

/// Which cut to create when several qualify
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutPick {
    /// smallest k: the shortest bottom = shared by the most requests (D65, default)
    Smallest,
    /// the cut that saves the most per reuse
    BiggestSave,
}

impl std::fmt::Display for CutPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CutPick::Smallest => "smallest",
            CutPick::BiggestSave => "save",
        })
    }
}

impl std::str::FromStr for CutPick {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "smallest" => Ok(CutPick::Smallest),
            "save" => Ok(CutPick::BiggestSave),
            other => Err(format!(
                "unknown PSC cut pick '{other}' (expected smallest | save)"
            )),
        }
    }
}

/// One cut of one request, ready to use
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub k: u32,
    pub n: u32,
    /// store key: sub_id|segment
    pub key: String,
    /// `<k>*<n>` request in the client's spelling (sent to a site to create the sub)
    pub pair_path: String,
    /// `<k>+<n>` request in the client's spelling (sent to a site with the sub)
    pub rebuild_path: String,
    /// decoded full and rebuild paths, for the cost function (features.csv spelling)
    pub full_cost_path: String,
    pub rebuild_cost_path: String,
}

/// The three numbers behind a decision (s, cost model on the reference site)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Costs {
    pub full_s: f64,
    pub rebuild_s: f64,
    /// full - rebuild: saved per reuse
    pub save_s: f64,
}

impl std::fmt::Display for Costs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "full {:.2} s rebuild {:.2} s save {:.2} s",
            self.full_s, self.rebuild_s, self.save_s
        )
    }
}

/// What to do with a request (after a segment cache miss)
#[derive(Debug)]
pub enum Plan<'a> {
    /// send `cand.rebuild_path` with the cached sub `cand.key`
    Rebuild { cand: Candidate, costs: Costs },
    /// send `cand.pair_path`; store the sub under `cand.key`, then drop `guard`
    Create {
        cand: Candidate,
        costs: Costs,
        guard: Creating<'a>,
    },
    /// normal full request; why (for the log): "no cut", "no cost", "not heavy", "not popular",
    /// "creating"
    Plain(&'static str),
}

/// The 'pair active' mark of one sub key: while it lives, no other request creates that sub.
/// Drop it after the sub is stored (or the pair failed).
#[derive(Debug)]
pub struct Creating<'a> {
    set: &'a Mutex<HashSet<String>>,
    key: String,
}

impl Creating<'_> {
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl Drop for Creating<'_> {
    fn drop(&mut self) {
        lock(self.set).remove(&self.key);
    }
}

pub struct PscPolicy {
    table: CutTable,
    /// create only cuts that save at least this (s per reuse)
    heavy_s: f64,
    /// create only after this many requests for the cut in the window (this one included)
    thr: usize,
    window_s: f64,
    pick: CutPick,
    /// arrival times (s, `now_s` clock) per sub key, only requests that were not rebuilt
    seen: Mutex<HashMap<String, VecDeque<f64>>>,
    /// sub keys whose pair is in flight
    creating: Mutex<HashSet<String>>,
    epoch: Instant,
}

impl PscPolicy {
    /// [Honest] Empty popularity table.
    pub fn new(table: CutTable, heavy_s: f64, thr: usize, window_s: f64, pick: CutPick) -> Self {
        Self {
            table,
            heavy_s,
            thr: thr.max(1),
            window_s,
            pick,
            seen: Mutex::new(HashMap::new()),
            creating: Mutex::new(HashSet::new()),
            epoch: Instant::now(),
        }
    }

    /// For the startup line, e.g. "heavy 1 s, thr 2, window 600 s, pick smallest, 115 kinds"
    pub fn describe(&self) -> String {
        format!(
            "heavy {} s, thr {}, window {} s, pick {}, {} kinds with cuts",
            self.heavy_s,
            self.thr,
            self.window_s,
            self.pick,
            self.table.kinds()
        )
    }

    /// [Dishonest clock] Seconds since this policy was made (the `now_s` for `decide`).
    pub fn now_s(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    /// [Honest] The usable cuts of a request (1:1 only), smallest k first. Empty if the path is
    /// not a plain segment request (already a pair / rebuild, unknown kind, not aligned).
    pub fn candidates(&self, path: &str) -> Vec<Candidate> {
        let decoded = percent_decode(path);
        let Some(seg) = SegmentPath::parse(&decoded) else {
            return Vec::new();
        };
        let raw: Vec<&str> = path.split('/').collect();
        let dec: Vec<&str> = decoded.split('/').collect();
        // the count token must be the plain <n> (a '*' / '+' request is never cut again)
        if raw.len() <= COUNT || dec.len() <= COUNT || dec[COUNT].parse::<u32>().is_err() {
            return Vec::new();
        }
        let with = |parts: &[&str], token: &str| {
            let mut p = parts.to_vec();
            p[COUNT] = token;
            p.join("/")
        };
        self.table
            .cuts(seg.show, seg.rest)
            .iter()
            .filter(|c| c.one_to_one)
            .map(|c| {
                let (pair, rebuild) = (format!("{}*{}", c.k, c.n), format!("{}+{}", c.k, c.n));
                Candidate {
                    k: c.k,
                    n: c.n,
                    key: format!("{}|{}", c.sub_id, seg.segment),
                    pair_path: with(&raw, &pair),
                    rebuild_path: with(&raw, &rebuild),
                    full_cost_path: decoded.clone(),
                    rebuild_cost_path: with(&dec, &rebuild),
                }
            })
            .collect()
    }

    /// [Honest given the state] Decides what to do with `path` at `now_s` (see the module doc).
    /// `cached(key)`: is that sub in the store (a peek); `cost(path)`: work (s) of a request on
    /// the reference site, None if unknown (that cut is then unusable).
    pub fn decide(
        &self,
        path: &str,
        now_s: f64,
        cached: impl Fn(&str) -> bool,
        cost: impl Fn(&str) -> Option<f64>,
    ) -> Plan<'_> {
        let cands = self.candidates(path);
        if cands.is_empty() {
            return Plan::Plain("no cut");
        }
        let costs_of = |c: &Candidate| {
            let full_s = cost(&c.full_cost_path)?;
            let rebuild_s = cost(&c.rebuild_cost_path)?;
            Some(Costs {
                full_s,
                rebuild_s,
                save_s: full_s - rebuild_s,
            })
        };
        let priced: Vec<(Candidate, Costs)> = cands
            .into_iter()
            .filter_map(|c| costs_of(&c).map(|x| (c, x)))
            .collect();
        if priced.is_empty() {
            return Plan::Plain("no cost");
        }

        // 1. reuse: a cached sub that saves time, the biggest save
        if let Some((cand, costs)) = priced
            .iter()
            .filter(|(c, x)| x.save_s > 0.0 && cached(&c.key))
            .max_by(|a, b| a.1.save_s.total_cmp(&b.1.save_s))
        {
            return Plan::Rebuild {
                cand: cand.clone(),
                costs: *costs,
            };
        }

        // 2. popularity of every cut (this request included), then pick one to create
        let counts: Vec<usize> = {
            let mut seen = lock(&self.seen);
            if seen.len() >= PRUNE_AT {
                seen.retain(|_, q| q.back().is_some_and(|t| now_s - t <= self.window_s));
            }
            priced
                .iter()
                .map(|(c, _)| {
                    let q = seen.entry(c.key.clone()).or_default();
                    q.push_back(now_s);
                    while q.front().is_some_and(|t| now_s - t > self.window_s) {
                        q.pop_front();
                    }
                    q.len()
                })
                .collect()
        };
        let heavy: Vec<usize> = (0..priced.len())
            .filter(|&i| priced[i].1.save_s >= self.heavy_s)
            .collect();
        if heavy.is_empty() {
            return Plan::Plain("not heavy");
        }
        let popular: Vec<usize> = heavy
            .into_iter()
            .filter(|&i| counts[i] >= self.thr)
            .collect();
        if popular.is_empty() {
            return Plan::Plain("not popular");
        }
        let mut creating = lock(&self.creating);
        let free = popular
            .into_iter()
            .filter(|&i| !cached(&priced[i].0.key) && !creating.contains(&priced[i].0.key));
        // priced is in k order: the first free one is the smallest
        let chosen = match self.pick {
            CutPick::Smallest => free.min_by_key(|&i| priced[i].0.k),
            CutPick::BiggestSave => {
                free.max_by(|&a, &b| priced[a].1.save_s.total_cmp(&priced[b].1.save_s))
            }
        };
        let Some(i) = chosen else {
            return Plan::Plain("creating");
        };
        let (cand, costs) = priced[i].clone();
        creating.insert(cand.key.clone());
        drop(creating);
        Plan::Create {
            guard: Creating {
                set: &self.creating,
                key: cand.key.clone(),
            },
            cand,
            costs,
        }
    }

    /// [Honest] Sub keys whose pair is in flight (tests / logging)
    pub fn creating(&self) -> usize {
        lock(&self.creating).len()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // never held across an await or user code: a poisoned value is still consistent
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod test {
    use super::*;

    const HEAD: &str = "/offload/show/f1.json";
    // rest of a 4-root kind with cuts k = 1, 2, 3 (and a non-1:1 kind)
    const REST: &str = "1280/720/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam";
    const CSV: &str = "rest,k,n,roots,prefix_pos,one_to_one,sub_id,show\n\
        1280/720/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam,1,4,4,0,1,f1.json/1280/720/race/landscape/1/race|race,f1.json\n\
        1280/720/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam,2,4,4,0;1,1,f1.json/1280/720/race/landscape/2/driver|Off/race|race,f1.json\n\
        1280/720/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam,3,4,4,0;1;2,1,f1.json/1280/720/race/landscape/3/driver|Off/race|race/track|off,f1.json\n\
        1280/720/core/landscape/2/w|Weather Icons/time|time,1,2,2,0,1,forecast.json/1280/720/core/landscape/1/w|Weather Icons,forecast.json\n\
        1920/1080/race/landscape/4/race|race/driver|Off/track|off/drivers|Sam,1,4,4,0,0,f1.json/1920/1080/race/landscape/1/race|race,f1.json\n";

    fn path(t0: u32) -> String {
        format!("{HEAD}/{t0}/{}/{REST}", t0 + 10)
    }

    /// cost: full 6 s; rebuild k=1 5.5 (save 0.5), k=2 4 (save 2), k=3 3 (save 3)
    fn cost(p: &str) -> Option<f64> {
        let count = p.split('/').nth(COUNT)?;
        Some(match count {
            "4" => 6.0,
            "1+4" => 5.5,
            "2+4" => 4.0,
            "3+4" => 3.0,
            "1" | "2" | "1+2" => 1.0,
            _ => return None,
        })
    }

    fn policy(heavy: f64, thr: usize, pick: CutPick) -> PscPolicy {
        PscPolicy::new(CutTable::from_csv(CSV).unwrap(), heavy, thr, 600.0, pick)
    }

    fn none(_: &str) -> bool {
        false
    }

    #[test]
    fn test_table_parse_and_checks() {
        let t = CutTable::from_csv(CSV).unwrap();
        assert_eq!(t.kinds(), 3);
        let cuts = t.cuts("f1.json", REST);
        assert_eq!(cuts.iter().map(|c| c.k).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(cuts[2].prefix_pos, [0, 1, 2]);
        assert!(t.cuts("f1.json", "nope").is_empty());
        // same rest, other show: its own kind
        assert!(t.cuts("f1_full.json", REST).is_empty());
        let bad =
            |from: &str, to: &str| CutTable::from_csv(&CSV.replacen(from, to, 1)).unwrap_err();
        assert!(bad("rest,k", "rest,kk").contains("header"));
        assert!(bad(",1,4,4,0,1,", ",4,4,4,0,1,").contains("invalid cut"));
        assert!(bad(",1,4,4,0,1,", ",0,4,4,0,1,").contains("invalid cut"));
        assert!(bad(",2,4,4,0;1,1,", ",1,4,4,0;1,1,").contains("duplicate"));
        assert!(bad(",2,4,4,0;1,1,", ",2,4,4,0;x,1,").contains("prefix_pos"));
        assert!(bad(",2,4,4,0;1,1,", ",2,4,4,0;1,2,").contains("one_to_one"));
        assert!(bad("w|Weather Icons/time", "\"w|a,b\"/time").contains("quoted"));
        // the real export parses
        let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stat_predict/psc_cuts.csv");
        if let Ok(t) = CutTable::load(real) {
            assert!(t.kinds() > 100, "{}", t.kinds());
        }
    }

    #[test]
    fn test_candidates_paths_and_keys() {
        let p = policy(1.0, 2, CutPick::Smallest);
        let c = p.candidates(&path(20));
        assert_eq!(c.len(), 3);
        assert_eq!(
            c[1].key,
            "f1.json/1280/720/race/landscape/2/driver|Off/race|race|2"
        );
        assert_eq!(c[1].pair_path, path(20).replace("/4/race", "/2*4/race"));
        assert_eq!(c[1].rebuild_path, path(20).replace("/4/race", "/2+4/race"));
        // %7C spelling: same cuts and keys, site paths keep the client's spelling, cost paths decoded
        let enc = path(20).replace('|', "%7C");
        let e = p.candidates(&enc);
        assert_eq!(e[1].key, c[1].key);
        assert!(e[1].pair_path.contains("/2*4/race%7Crace"));
        assert_eq!(e[1].rebuild_cost_path, c[1].rebuild_path);
        // a space in an option (raw or %20) matches the table
        let fc = "/offload/show/forecast.json/0/10/1280/720/core/landscape/2/w%7CWeather%20Icons/time%7Ctime";
        assert_eq!(p.candidates(fc).len(), 1);
        // not cut: non-1:1, already a pair / rebuild, unknown kind, not a segment path
        assert!(
            p.candidates(&path(0).replace("1280/720", "1920/1080"))
                .is_empty()
        );
        assert!(
            p.candidates(&path(0).replace("/4/race", "/2*4/race"))
                .is_empty()
        );
        assert!(
            p.candidates(&path(0).replace("/4/race", "/2+4/race"))
                .is_empty()
        );
        assert!(p.candidates(&path(0).replace("Sam", "Alex")).is_empty());
        assert!(p.candidates("/offload/show/f1.json/5/15/x").is_empty());
    }

    #[test]
    fn test_create_after_thr_smallest_heavy() {
        let p = policy(1.0, 2, CutPick::Smallest);
        // 1st request: k=2, k=3 heavy but seen once
        assert!(matches!(
            p.decide(&path(0), 0.0, none, cost),
            Plan::Plain("not popular")
        ));
        // 2nd (another viewer, same segment): create the smallest heavy cut k=2, all 3 numbers
        match p.decide(&path(0), 5.0, none, cost) {
            Plan::Create { cand, costs, guard } => {
                assert_eq!(cand.k, 2);
                assert_eq!(
                    costs,
                    Costs {
                        full_s: 6.0,
                        rebuild_s: 4.0,
                        save_s: 2.0
                    }
                );
                assert_eq!(guard.key(), cand.key);
                assert_eq!(p.creating(), 1);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(p.creating(), 0); // guard dropped
        // other segment: own popularity
        assert!(matches!(
            p.decide(&path(10), 6.0, none, cost),
            Plan::Plain("not popular")
        ));
    }

    #[test]
    fn test_pick_biggest_save_and_heavy_filter() {
        let p = policy(1.0, 1, CutPick::BiggestSave);
        match p.decide(&path(0), 0.0, none, cost) {
            Plan::Create { cand, costs, .. } => assert_eq!((cand.k, costs.save_s), (3, 3.0)),
            other => panic!("{other:?}"),
        }
        // heavy 2.5: only k=3; heavy 4: none
        let p = policy(2.5, 1, CutPick::Smallest);
        assert!(
            matches!(p.decide(&path(0), 0.0, none, cost), Plan::Create { cand, .. } if cand.k == 3)
        );
        let p = policy(4.0, 1, CutPick::Smallest);
        assert!(matches!(
            p.decide(&path(0), 0.0, none, cost),
            Plan::Plain("not heavy")
        ));
    }

    #[test]
    fn test_creating_blocks_same_sub_then_next_cut() {
        let p = policy(1.0, 1, CutPick::Smallest);
        let first = p.decide(&path(0), 0.0, none, cost);
        assert!(matches!(&first, Plan::Create { cand, .. } if cand.k == 2));
        // pair of k=2 in flight: the next request creates k=3 (the other heavy cut)
        let second = p.decide(&path(0), 1.0, none, cost);
        assert!(matches!(&second, Plan::Create { cand, .. } if cand.k == 3));
        // both in flight: normal request
        assert!(matches!(
            p.decide(&path(0), 2.0, none, cost),
            Plan::Plain("creating")
        ));
        drop(first);
        assert!(
            matches!(p.decide(&path(0), 3.0, none, cost), Plan::Create { cand, .. } if cand.k == 2)
        );
        drop(second);
    }

    #[test]
    fn test_rebuild_from_cached_biggest_save() {
        let p = policy(1.0, 2, CutPick::Smallest);
        let key = |k: u32| p.candidates(&path(0))[k as usize - 1].key.clone();
        let (k1, k2, k3) = (key(1), key(2), key(3));
        // k=1 cached saves 0.5 (> 0): rebuild even below heavy
        match p.decide(&path(0), 0.0, |k| k == k1, cost) {
            Plan::Rebuild { cand, costs } => assert_eq!((cand.k, costs.save_s), (1, 0.5)),
            other => panic!("{other:?}"),
        }
        // k=2 and k=3 cached: the biggest save
        assert!(
            matches!(p.decide(&path(0), 0.0, |k| k == k2 || k == k3, cost),
            Plan::Rebuild { cand, .. } if cand.k == 3)
        );
        // a rebuild does not count as popularity: next request still sees count 1 -> not popular
        assert!(matches!(
            p.decide(&path(0), 1.0, none, cost),
            Plan::Plain("not popular")
        ));
    }

    #[test]
    fn test_window_expires_counts() {
        let p = policy(1.0, 2, CutPick::Smallest);
        assert!(matches!(
            p.decide(&path(0), 0.0, none, cost),
            Plan::Plain("not popular")
        ));
        // 601 s later the first arrival left the window
        assert!(matches!(
            p.decide(&path(0), 601.0, none, cost),
            Plan::Plain("not popular")
        ));
        assert!(matches!(
            p.decide(&path(0), 602.0, none, cost),
            Plan::Create { .. }
        ));
    }

    #[test]
    fn test_no_cut_no_cost() {
        let p = policy(1.0, 1, CutPick::Smallest);
        assert!(matches!(
            p.decide("/offload/show/x.json/0/10/a", 0.0, none, cost),
            Plan::Plain("no cut")
        ));
        assert!(matches!(
            p.decide(&path(0), 0.0, none, |_| None),
            Plan::Plain("no cost")
        ));
        // only k=3 priced: it is the one created
        let only3 = |q: &str| {
            if q.contains("/3+4/") || q.split('/').nth(COUNT) == Some("4") {
                cost(q)
            } else {
                None
            }
        };
        assert!(
            matches!(p.decide(&path(0), 0.0, none, only3), Plan::Create { cand, .. } if cand.k == 3)
        );
        // CutPick parse / display
        assert_eq!("save".parse::<CutPick>(), Ok(CutPick::BiggestSave));
        assert!("big".parse::<CutPick>().is_err());
        assert_eq!(CutPick::Smallest.to_string(), "smallest");
    }

    /// Real data: psc_cuts.csv + the exported cost model on the paths of one replay sequence
    /// (as seq_bench sends them). Skipped if the files are missing.
    #[test]
    fn test_real_sequence_paths() {
        use crate::cost_predict::CostPredictor;
        use crate::mod_load_balancer::base_balancer::OffloadSite;
        use crate::mod_load_balancer::base_balancer::config::{Config, Decode, Encode, Render};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let sp = root.join("stat_predict");
        let table = CutTable::load(sp.join("psc_cuts.csv"));
        let model = CostPredictor::load(sp.join("coef.csv"), sp.join("features.csv"));
        let seq =
            std::fs::read_to_string(root.join("servers_benchmark/sequences/seq_a0.5_case1.csv"));
        let (Ok(table), Ok(model), Ok(seq)) = (table, model, seq) else {
            eprintln!("test_real_sequence_paths skipped: a data file is missing");
            return;
        };
        let gpu = OffloadSite::new(
            "ref:0",
            "ref",
            Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu),
        );
        let p = PscPolicy::new(table, 0.5, 1, 600.0, CutPick::Smallest);
        let paths: Vec<&str> = seq
            .lines()
            .skip(1)
            .filter_map(|l| l.splitn(3, ',').nth(2))
            .collect();
        let (mut with_cut, mut priced) = (0, 0);
        for path in &paths {
            let cands = p.candidates(path);
            with_cut += !cands.is_empty() as usize;
            priced += cands.iter().any(|c| {
                model.predict(&c.full_cost_path, &gpu).is_some()
                    && model.predict(&c.rebuild_cost_path, &gpu).is_some()
            }) as usize;
        }
        // every path with a cut can be priced (features.csv has its full + rebuild rows)
        assert!(with_cut > paths.len() / 2, "{with_cut} / {}", paths.len());
        assert_eq!(priced, with_cut);
        // forest720_leaves: a create with the three numbers
        let forest = paths
            .iter()
            .find(|p| p.contains("forest720_leaves"))
            .unwrap();
        match p.decide(forest, 0.0, |_| false, |q| model.predict(q, &gpu)) {
            Plan::Create { cand, costs, .. } => {
                println!("forest720_leaves create k={} {costs}", cand.k);
                assert!(costs.save_s >= 0.5 && costs.full_s > costs.rebuild_s);
            }
            other => panic!("{other:?}"),
        }
        println!(
            "{} paths, {with_cut} with a cut, {priced} priced",
            paths.len()
        );
    }
}
