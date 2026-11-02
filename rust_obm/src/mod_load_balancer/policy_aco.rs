use super::base_balancer::*;
use super::policy_mct::{MctTicket, MctWorker, fmt_opt, mct_workers};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

// Online Ant Colony: every request is one ant. Site j is drawn with probability
//   p(j) ~ tau[show][j]^ALPHA * (1 / est_finish(j))^BETA
// est_finish is MCT's estimate (makespan + network + machine factor * predict(path, site)), so
// a huge BETA gives MCT back. tau (pheromone) starts at 1 and on every completion moves towards
// reward = predicted service time / actual (asset + work + net, no queue): > 1 = the site did
// better than expected for this show. The reward clamp keeps tau in [0.25, 4], so no site's
// probability goes to 0 (keeps exploring). Unknown path: same fallback rule as MCT
const ALPHA: f64 = 1.0; // weight of the pheromone (learned memory)
const BETA: f64 = 2.0; // weight of the current estimate
const EVAPORATION: f64 = 0.2; // tau = (1 - EVAPORATION) tau + EVAPORATION reward
const REWARD_MIN: f64 = 0.25;
const REWARD_MAX: f64 = 4.0;

pub struct AcoTicket {
    mct: MctTicket, // cost added to the makespan + predicted work, as in MCT
    show: String,   // pheromone row to update
}

pub struct ACO<F> {
    predict: F,
    fallback: bool,
    state: Mutex<AcoState>,
}

struct AcoState {
    sites: Vec<MctWorker>,
    pheromone: HashMap<String, Vec<f64>>, // show -> tau per site, a new show starts at 1
    rng: fastrand::Rng,
}

impl<F> ACO<F>
where
    F: Fn(&str, &OffloadSite) -> Option<f64> + Send + Sync,
{
    pub fn with_predict(offloads: &[OffloadSite], predict: F, fallback: bool, seed: u64) -> Self {
        Self {
            predict,
            fallback,
            state: Mutex::new(AcoState {
                sites: mct_workers(offloads, None),
                pheromone: HashMap::new(),
                rng: fastrand::Rng::with_seed(seed),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, AcoState> {
        // A panic while holding the lock leaves the state itself consistent, so keep using it
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<F> LoadBalancerPolicy for ACO<F>
where
    F: Fn(&str, &OffloadSite) -> Option<f64> + Send + Sync,
{
    type Ticket = AcoTicket;

    // the site list alone can't make a predict function: build with ACO::with_predict and
    // LoadBalancer::with_policy instead
    fn new(_offloads: &[OffloadSite]) -> Self {
        panic!("ACO needs a predict function, use ACO::with_predict + LoadBalancer::with_policy")
    }

    fn select(&self, key: Option<&str>, workers: &[OffloadSite]) -> (usize, AcoTicket) {
        // predictions don't need the lock
        let works: Option<Vec<f64>> =
            key.and_then(|path| workers.iter().map(|w| (self.predict)(path, w)).collect());
        // before the lock: the panic only ends this request's task (smol catches it)
        if works.is_none() && !self.fallback {
            panic!(
                "aco: no prediction for path {} and fallback is off",
                key.unwrap_or("-")
            );
        }
        let show = show_of(key).to_string();

        let mut state = self.lock();
        let AcoState {
            sites,
            pheromone,
            rng,
        } = &mut *state;
        let tau = pheromone
            .entry(show.clone())
            .or_insert_with(|| vec![1.0; sites.len()]);
        let mut log = Vec::new(); // site:finish/tau/p for the log

        let (idx, work) = match works {
            Some(works) => {
                let weights: Vec<f64> = (0..sites.len())
                    .map(|i| tau[i].powf(ALPHA) * (1.0 / sites[i].predict(works[i])).powf(BETA))
                    .collect();
                let total: f64 = weights.iter().sum();
                // roulette wheel; the last site takes any float rounding left over
                let mut r = rng.f64() * total;
                let idx = weights
                    .iter()
                    .position(|&w| {
                        r -= w;
                        r < 0.0
                    })
                    .unwrap_or(sites.len() - 1);
                for i in 0..sites.len() {
                    log.push(format!(
                        "{}:{:.1}/{:.2}/{:.2}",
                        sites[i].id,
                        sites[i].predict(works[i]),
                        tau[i],
                        weights[i] / total
                    ));
                }
                (idx, Some(works[idx]))
            }
            None => {
                let idx = (0..sites.len())
                    .min_by_key(|&i| sites[i].in_flight)
                    .unwrap();
                (idx, None)
            }
        };

        // unknown work: only the network part is counted in the backlog
        let cost = match work {
            Some(w) => sites[idx].taskspan(w),
            None => sites[idx].network_f.get(),
        };
        sites[idx].add(cost);
        let pick = sites[idx].id.clone();
        drop(state); // print outside the lock

        println!(
            "aco select path={} pick={pick} est/tau/p=[{}] cost={cost:.1} work={} fallback={}",
            key.unwrap_or("-"),
            log.join(" "),
            fmt_opt(work, 1),
            u8::from(work.is_none()),
        );
        let ticket = AcoTicket {
            mct: MctTicket { cost, work },
            show,
        };
        (idx, ticket)
    }

    fn complete(&self, worker_idx: usize, ticket: AcoTicket, feedback: Option<&TimesFeedback>) {
        let line = {
            let mut state = self.lock();
            let AcoState {
                sites, pheromone, ..
            } = &mut *state;
            let w = &mut sites[worker_idx];
            // makespan, machine factor and network as in MCT
            w.remove_update(&ticket.mct, feedback);

            // pheromone: only with real timings for a known path (same rule as the factors)
            let actual = feedback
                .filter(|fb| fb.t_work > 0 && ticket.mct.work.is_some())
                .map(|fb| (fb.t_asset + fb.t_work + fb.t_net) as f64 / 1000.0)
                .filter(|&a| a > 0.0);
            let reward = actual.map(|a| (ticket.mct.cost / a).clamp(REWARD_MIN, REWARD_MAX));
            let tau = pheromone
                .get_mut(&ticket.show)
                .map(|row| &mut row[worker_idx]);
            if let (Some(r), Some(t)) = (reward, tau) {
                *t = (1.0 - EVAPORATION) * *t + EVAPORATION * r;
            }
            let tau_now = pheromone.get(&ticket.show).map(|row| row[worker_idx]);
            format!(
                "aco done   site={} show={} cost={:.1} actual={} reward={} tau={} f={:.3} n={:.3} makespan={:.1}",
                w.id,
                ticket.show,
                ticket.mct.cost,
                fmt_opt(actual, 2),
                fmt_opt(reward, 2),
                fmt_opt(tau_now, 3),
                w.machine_f.get(),
                w.network_f.get(),
                w.makespan,
            )
        };
        println!("{line}");
    }
}

pub type AcoLB<F> = LoadBalancer<ACO<F>>;

// "/offload/show/<show>/..." -> "<show>" (pheromone row); "-" if there is no path or show
fn show_of(key: Option<&str>) -> &str {
    key.and_then(|path| path.split('/').nth(3))
        .filter(|s| !s.is_empty())
        .unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, Decode, Encode, Render};

    // only paths of show "a" or "b" are known: 1 s on a gpu render site, 4 s on a cpu one
    fn work(path: &str, site: &OffloadSite) -> Option<f64> {
        matches!(show_of(Some(path)), "a" | "b").then_some(match site.config.render {
            Render::Gpu => 1.0,
            _ => 4.0,
        })
    }
    const PATH_A: &str = "/offload/show/a/0/10";
    const PATH_B: &str = "/offload/show/b/0/10";

    // om1 gpu, om2 cpu: same priors (factor 1.0, net 1.5 s)
    fn sites() -> Vec<OffloadSite> {
        let cfg = |r| Config::new(Decode::Gpu, r, Encode::Cpu);
        vec![
            OffloadSite::new("a:1", "om1", cfg(Render::Gpu)),
            OffloadSite::new("b:1", "om2", cfg(Render::Cpu)),
        ]
    }

    fn aco(s: &[OffloadSite], seed: u64) -> ACO<fn(&str, &OffloadSite) -> Option<f64>> {
        ACO::with_predict(s, work, true, seed)
    }

    fn fb(asset_ms: i64, work_ms: i64, net_ms: i64) -> TimesFeedback {
        TimesFeedback {
            t_net: net_ms,
            t_queue: 0,
            t_work: work_ms,
            t_asset: asset_ms,
        }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn test_show_of_path() {
        assert_eq!(
            show_of(Some("/offload/show/forest720.json/0/10/1280")),
            "forest720.json"
        );
        assert_eq!(show_of(Some("/offload")), "-");
        assert_eq!(show_of(None), "-");
    }

    #[test]
    fn test_aco_prefers_earlier_finish() {
        // idle: gpu est 2.5 s vs cpu 5.5 s -> p(gpu) = 2.5^-2 / (2.5^-2 + 5.5^-2) ~ 0.83
        let s = sites();
        let p = aco(&s, 1);
        let gpu = (0..1000)
            .filter(|_| {
                let (i, t) = p.select(Some(PATH_A), &s);
                p.complete(i, t, None); // back to idle, no learning
                i == 0
            })
            .count();
        assert!((780..880).contains(&gpu), "gpu picked {gpu}/1000");
    }

    #[test]
    fn test_aco_same_seed_same_choices() {
        let s = sites();
        let picks = |seed| {
            let p = aco(&s, seed);
            (0..20)
                .map(|_| p.select(Some(PATH_A), &s).0)
                .collect::<Vec<_>>()
        };
        assert_eq!(picks(7), picks(7));
    }

    #[test]
    fn test_aco_pheromone_learns_per_show_and_site() {
        let s = sites();
        let p = aco(&s, 3);
        // draw until show "a" lands on the cpu site, then report it 4x slower than predicted
        let (i, t) = loop {
            let (i, t) = p.select(Some(PATH_A), &s);
            if i == 1 {
                break (i, t);
            }
            p.complete(i, t, None);
        };
        let cost = t.mct.cost; // 1.5 + 1.0 * 4 = 5.5 s predicted
        assert!(approx(cost, 5.5));
        p.complete(i, t, Some(&fb(0, 20_000, 2_000))); // actual 22 s -> reward 0.25
        let state = p.lock();
        let tau_a = &state.pheromone["a"];
        assert!(approx(tau_a[1], 0.8 * 1.0 + 0.2 * 0.25));
        assert!(approx(tau_a[0], 1.0)); // other site untouched
        assert!(!state.pheromone.contains_key("b")); // other show untouched (not seen yet)
    }

    #[test]
    fn test_aco_low_pheromone_shifts_choice() {
        let s = sites();
        let p = aco(&s, 5);
        p.lock().pheromone.insert("a".into(), vec![1.0, 0.25]);
        let cpu_a = (0..1000)
            .filter(|_| {
                let (i, t) = p.select(Some(PATH_A), &s);
                p.complete(i, t, None);
                i == 1
            })
            .count();
        let cpu_b = (0..1000)
            .filter(|_| {
                let (i, t) = p.select(Some(PATH_B), &s);
                p.complete(i, t, None);
                i == 1
            })
            .count();
        // show a: 0.25 * 5.5^-2 vs 2.5^-2 -> p(cpu) ~ 0.05; show b unchanged ~ 0.17
        assert!(cpu_a < 90, "cpu for a {cpu_a}/1000");
        assert!(cpu_b > 120, "cpu for b {cpu_b}/1000");
    }

    #[test]
    fn test_aco_missing_timings_no_learning() {
        let s = sites();
        let p = aco(&s, 9);
        let (i, t) = p.select(Some(PATH_A), &s);
        p.complete(i, t, Some(&TimesFeedback::default()));
        let state = p.lock();
        assert!(state.pheromone["a"].iter().all(|&t| approx(t, 1.0)));
        assert!(state.sites.iter().all(|w| approx(w.makespan, 0.0)));
    }

    #[test]
    #[should_panic(expected = "fallback is off")]
    fn test_aco_no_fallback_rejects_unknown_path() {
        let s = sites();
        let p = ACO::with_predict(&s, work, false, 1);
        p.select(Some("/offload/show/zzz/0/10"), &s);
    }

    #[test]
    fn test_aco_complete_releases_backlog() {
        let s = sites();
        let p = aco(&s, 1);
        let tickets: Vec<_> = (0..5).map(|_| p.select(Some(PATH_A), &s)).collect();
        for (i, t) in tickets {
            p.complete(i, t, None); // failed / dropped request: backlog freed, no learning
        }
        let state = p.lock();
        assert!(
            state
                .sites
                .iter()
                .all(|w| approx(w.makespan, 0.0) && w.in_flight == 0)
        );
        assert!(state.pheromone["a"].iter().all(|&t| approx(t, 1.0)));
    }
}
