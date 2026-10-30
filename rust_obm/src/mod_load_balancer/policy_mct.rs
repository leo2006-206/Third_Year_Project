use super::base_balancer::*;
use std::sync::{Mutex, MutexGuard};

// exponentially weighted moving average
pub struct EWMA {
    learn_rate: f64,
    value: f64,
}

impl EWMA {
    fn new(a: f64, init: f64) -> Self {
        assert!(a > 0.0);
        assert!(a < 1.0);
        assert!(init > 0.0);
        Self {
            learn_rate: a,
            value: init,
        }
    }
    fn update(&mut self, x: f64) {
        self.value = (1.0 - self.learn_rate) * self.value + self.learn_rate * x;
    }
    fn get(&self) -> f64 {
        self.value
    }
}

// What select remembers for complete: the exact cost it added (so complete removes the same
// number, even if the factors changed meanwhile) and the predicted work for the factor update
pub struct MctTicket {
    cost: f64,         // s added to the site's makespan
    work: Option<f64>, // machine-neutral predicted work (s), None if the path was unknown
}

pub struct MctWorker {
    id: String,       // short site id for the log, e.g. om1
    machine_f: EWMA,  // actual work / predicted work
    network_f: EWMA,  // net + asset time (s)
    makespan: f64,    // sum of the costs of the requests in flight (s), self-correcting
    in_flight: usize, // for the fallback when a path has no prediction
}

impl MctWorker {
    fn new(id: &str, mach: f64, net: f64, learn: f64) -> Self {
        Self {
            id: id.to_string(),
            machine_f: EWMA::new(learn, mach),
            network_f: EWMA::new(learn, net),
            makespan: 0.0,
            in_flight: 0,
        }
    }
    // this request's own cost on this site
    fn taskspan(&self, predicted_work: f64) -> f64 {
        self.network_f.get() + (self.machine_f.get() * predicted_work)
    }
    // estimated completion time from now
    fn predict(&self, predicted_work: f64) -> f64 {
        self.makespan + self.taskspan(predicted_work)
    }
    fn add(&mut self, predicted_taskspan: f64) {
        self.makespan += predicted_taskspan;
        self.in_flight += 1;
    }
    // returns the raw (unclamped) work ratio, None if there was no sample to learn from
    fn remove_update(
        &mut self,
        ticket: &MctTicket,
        feedback: Option<&TimesFeedback>,
    ) -> Option<f64> {
        // max: float rounding must not leave a negative backlog
        self.makespan = (self.makespan - ticket.cost).max(0.0);
        self.in_flight = self.in_flight.saturating_sub(1);

        // None = failed / timed out; t_work 0 = the site sent no X-Dana-Timings: no real sample
        let fb = feedback.filter(|fb| fb.t_work > 0)?;

        // feedback is in ms, the model in s
        self.network_f
            .update((fb.t_asset + fb.t_net) as f64 / 1000.0);
        let work = ticket.work.filter(|w| *w > 0.0)?;
        // clamped so one odd render (or a tiny prediction) can't swing the factor
        let ratio = (fb.t_work as f64 / 1000.0) / work;
        self.machine_f.update(ratio.clamp(0.25, 4.0));
        Some(ratio)
    }
}

// Minimum Completion Time: send to the site with the earliest estimated finish
// makespan + network + machine factor * predict(path, site).
// predict(path, site) gives the machine-neutral work (s) of the path on that site, None if unknown
// (e.g. CostPredictor::predict, or a constant). Unknown path: ESCE if fallback, else the request
// fails (panic in its own task)
pub struct MCT<F> {
    predict: F,
    fallback: bool,
    mct_vec: Mutex<Vec<MctWorker>>,
}

impl<F> MCT<F>
where
    F: Fn(&str, &OffloadSite) -> Option<f64> + Send + Sync,
{
    pub fn with_predict(offloads: &[OffloadSite], predict: F, fallback: bool) -> Self {
        const LEARNING: f64 = 0.1;

        let mut vec = Vec::with_capacity(offloads.len());

        // priors: om = reference machine, near; rs ~25% faster, twice the RTT
        // (main server site names look like "id=om1_ip=..."; tests use plain "om1")
        for of in offloads {
            let id = short_id(&of.name);
            if id.starts_with("om") {
                vec.push(MctWorker::new(id, 1.0, 1.5, LEARNING));
            } else {
                vec.push(MctWorker::new(id, 0.75, 3.0, LEARNING));
            }
        }

        Self {
            predict,
            fallback,
            mct_vec: Mutex::new(vec),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<MctWorker>> {
        // A panic while holding the lock leaves the counts themselves consistent, so keep using them
        self.mct_vec
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl<F> LoadBalancerPolicy for MCT<F>
where
    F: Fn(&str, &OffloadSite) -> Option<f64> + Send + Sync,
{
    type Ticket = MctTicket;

    // the site list alone can't make a predict function: build with MCT::with_predict and
    // LoadBalancer::with_policy instead
    fn new(_offloads: &[OffloadSite]) -> Self {
        panic!("MCT needs a predict function, use MCT::with_predict + LoadBalancer::with_policy")
    }

    fn select(&self, key: Option<&str>, workers: &[OffloadSite]) -> (usize, MctTicket) {
        // predictions don't need the lock
        let works: Option<Vec<f64>> =
            key.and_then(|path| workers.iter().map(|w| (self.predict)(path, w)).collect());
        // before the lock: the panic only ends this request's task (smol catches it)
        if works.is_none() && !self.fallback {
            panic!(
                "mct: no prediction for path {} and fallback is off",
                key.unwrap_or("-")
            );
        }

        let mut vec = self.lock();
        let mut est = Vec::new(); // (site, estimated finish) for the log

        let (idx, work) = match works {
            Some(works) => {
                let mut min = f64::MAX;
                let mut idx = usize::default();
                // strict < : ties go to the lowest index
                for (i, &work) in works.iter().enumerate() {
                    let finish = vec[i].predict(work);
                    est.push(format!("{}:{finish:.1}", vec[i].id));
                    if finish < min {
                        min = finish;
                        idx = i;
                    }
                }
                (idx, Some(works[idx]))
            }
            None => {
                let idx = (0..vec.len()).min_by_key(|&i| vec[i].in_flight).unwrap();
                (idx, None)
            }
        };

        // unknown work: only the network part is counted in the backlog
        let cost = match work {
            Some(w) => vec[idx].taskspan(w),
            None => vec[idx].network_f.get(),
        };
        vec[idx].add(cost);
        let pick = vec[idx].id.clone();
        drop(vec); // print outside the lock

        println!(
            "mct select path={} pick={pick} est=[{}] cost={cost:.1} work={} fallback={}",
            key.unwrap_or("-"),
            est.join(" "),
            fmt_opt(work, 1),
            u8::from(work.is_none()),
        );
        (idx, MctTicket { cost, work })
    }

    fn complete(&self, worker_idx: usize, ticket: MctTicket, feedback: Option<&TimesFeedback>) {
        let line = {
            let mut vec = self.lock();
            let w = &mut vec[worker_idx];
            let ratio = w.remove_update(&ticket, feedback);
            // t_net = net + asset: the sample for the network estimate n
            format!(
                "mct done   site={} cost={:.1} work={} t_work={} t_net={} ratio={} f={:.3} n={:.3} makespan={:.1}",
                w.id,
                ticket.cost,
                fmt_opt(ticket.work, 1),
                fmt_opt(feedback.map(|fb| fb.t_work as f64 / 1000.0), 2),
                fmt_opt(
                    feedback.map(|fb| (fb.t_net + fb.t_asset) as f64 / 1000.0),
                    2
                ),
                fmt_opt(ratio, 2),
                w.machine_f.get(),
                w.network_f.get(),
                w.makespan,
            )
        };
        println!("{line}");
    }
}

pub type MctLB<F> = LoadBalancer<MCT<F>>;

// "id=om1_ip=..." (main server site name) -> "om1"; a plain name is kept
fn short_id(name: &str) -> &str {
    let name = name.trim_start_matches("id=");
    name.split('_').next().unwrap_or(name)
}

// "-" for a missing value in the log
fn fmt_opt(v: Option<f64>, digits: usize) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.digits$}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, Decode, Encode, Render};

    // only "/p" is known: 1 s on a gpu render site, 4 s on a cpu one
    fn work(path: &str, site: &OffloadSite) -> Option<f64> {
        (path == "/p").then(|| match site.config.render {
            Render::Gpu => 1.0,
            _ => 4.0,
        })
    }

    // om1 gpu, om2 cpu: same priors (factor 1.0, net 1.5 s)
    fn sites() -> Vec<OffloadSite> {
        let cfg = |r| Config::new(Decode::Gpu, r, Encode::Cpu);
        vec![
            OffloadSite::new("a:1", "om1", cfg(Render::Gpu)),
            OffloadSite::new("b:1", "om2", cfg(Render::Cpu)),
        ]
    }

    fn mct(s: &[OffloadSite]) -> MCT<fn(&str, &OffloadSite) -> Option<f64>> {
        MCT::with_predict(s, work, true)
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn test_mct_picks_earliest_finish() {
        let s = sites();
        let p = mct(&s);
        // gpu 0 + 1.5 + 1 = 2.5 vs cpu 5.5
        let (i0, t0) = p.select(Some("/p"), &s);
        assert_eq!(i0, 0);
        assert!(approx(t0.cost, 2.5));
        // gpu 2.5 + 2.5 = 5.0 < 5.5
        assert_eq!(p.select(Some("/p"), &s).0, 0);
        // gpu 7.5 > cpu 5.5
        assert_eq!(p.select(Some("/p"), &s).0, 1);
    }

    #[test]
    fn test_mct_complete_removes_cost_and_learns() {
        let s = sites();
        let p = mct(&s);
        let (i, t) = p.select(Some("/p"), &s);
        let fb = TimesFeedback {
            t_net: 1000,
            t_queue: 0,
            t_work: 2000, // 2x the prediction
            t_asset: 1000,
        };
        p.complete(i, t, Some(&fb));
        let vec = p.lock();
        assert!(approx(vec[0].makespan, 0.0));
        assert_eq!(vec[0].in_flight, 0);
        assert!(approx(vec[0].machine_f.get(), 0.9 * 1.0 + 0.1 * 2.0));
        assert!(approx(vec[0].network_f.get(), 0.9 * 1.5 + 0.1 * 2.0));
    }

    #[test]
    fn test_mct_ignores_missing_timings() {
        let s = sites();
        let p = mct(&s);
        let (i, t) = p.select(Some("/p"), &s);
        // no X-Dana-Timings -> all-zero feedback
        p.complete(i, t, Some(&TimesFeedback::default()));
        let (i, t) = p.select(Some("/p"), &s);
        p.complete(i, t, None);
        let vec = p.lock();
        assert!(approx(vec[0].machine_f.get(), 1.0));
        assert!(approx(vec[0].network_f.get(), 1.5));
        assert!(approx(vec[0].makespan, 0.0));
    }

    #[test]
    #[should_panic(expected = "fallback is off")]
    fn test_mct_no_fallback_rejects_unknown_path() {
        let s = sites();
        let p = MCT::with_predict(&s, work, false);
        assert_eq!(p.select(Some("/p"), &s).0, 0); // known path still fine
        p.select(Some("/unknown"), &s);
    }

    #[test]
    fn test_mct_unknown_path_falls_back_to_least_in_flight() {
        let s = sites();
        let p = mct(&s);
        let (i, t) = p.select(Some("/unknown"), &s);
        assert_eq!(i, 0);
        assert!(t.work.is_none());
        assert_eq!(p.select(None, &s).0, 1);
        assert_eq!(p.select(None, &s).0, 0);
    }

    #[test]
    fn test_mct_priors_from_cluster_csv_names() {
        let cfg = Config::new(Decode::Gpu, Render::Gpu, Encode::Cpu);
        let s = vec![
            OffloadSite::new("a:1", "id=om1_ip=obm-offload-om1_port=7010", cfg),
            OffloadSite::new("b:1", "id=rs1_ip=10.10.0.2_port=7010", cfg),
        ];
        let p = mct(&s);
        let vec = p.lock();
        assert!(approx(vec[0].machine_f.get(), 1.0));
        assert!(approx(vec[1].machine_f.get(), 0.75));
        assert!(approx(vec[1].network_f.get(), 3.0));
    }

    #[test]
    fn test_mct_constant_predict_closure() {
        // same work everywhere (no cost model): equal sites alternate by backlog
        let s = sites();
        let w = 2.0;
        let p = MCT::with_predict(&s, move |_: &str, _: &OffloadSite| Some(w), false);
        assert_eq!(p.select(Some("/any"), &s).0, 0);
        assert_eq!(p.select(Some("/any"), &s).0, 1);
        assert_eq!(p.select(Some("/any"), &s).0, 0);
    }

    #[test]
    fn test_mct_guard_drop_releases() {
        let s = sites();
        let lb = MctLB::with_policy(s.clone(), mct(&s));
        {
            let g = lb.select(Some("/p"));
            assert_eq!(g.name, "om1");
        } // dropped without finish
        assert_eq!(lb.select(Some("/p")).name, "om1"); // backlog back to 0, gpu still cheapest
    }
}
