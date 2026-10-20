use std::sync::Mutex;

use super::base_balancer as bb;
use super::base_balancer::config::Render;

const fn get_weight(render: Render) -> usize {
    match render {
        Render::Cpu => 1,
        Render::Llvmpipe => 3,
        Render::Gpu => 3,
    }
}

fn gcd_slice(weights: impl Iterator<Item = usize>) -> usize {
    let gcd = |mut a, mut b| -> usize {
        while b != 0 {
            let rem = a % b;
            a = b;
            b = rem;
        }
        a
    };

    weights.reduce(gcd).unwrap_or(1)
}

struct State {
    i: usize,
    cw: usize,
}

pub struct InterleavedWeightedRR {
    weights: Vec<usize>,
    max_weight: usize,
    gcd: usize,
    state: Mutex<State>,
}

impl bb::LoadBalancerPolicy for InterleavedWeightedRR {
    fn new(offloads: &[bb::OffloadSite]) -> Self {
        assert!(!offloads.is_empty());

        let weights: Vec<usize> = offloads
            .iter()
            .map(|x| get_weight(x.config.render))
            .collect();

        let max_weight = weights.iter().copied().max().unwrap_or(0);
        let gcd = gcd_slice(weights.iter().copied()).max(1);
        let n = offloads.len();

        Self {
            weights,
            max_weight,
            gcd,
            state: Mutex::new(State {
                // Initializing i to n - 1 ensures the first request wraps to 0
                // and sets cw = max_weight immediately
                i: if n > 0 { n - 1 } else { 0 },
                cw: 0,
            }),
        }
    }

    fn select(&self, _key: Option<&str>, num_workers: usize) -> usize {
        if num_workers == 0 || self.max_weight == 0 {
            return 0;
        }

        let mut state = self.state.lock().expect("mutex poisoned");

        loop {
            state.i = (state.i + 1) % num_workers;

            // When completing a full loop through all servers, decrease weight threshold
            if state.i == 0 {
                state.cw = state.cw.saturating_sub(self.gcd);
                if state.cw == 0 {
                    state.cw = self.max_weight;
                }
            }

            // Check if current worker's weight is eligible for this threshold
            if self.weights[state.i] >= state.cw {
                return state.i;
            }
        }
    }
}

pub type InterleavedWeightedRRLB = bb::LoadBalancer<InterleavedWeightedRR>;
