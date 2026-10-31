pub mod base_balancer;
pub mod policy_aco;
pub mod policy_esce;
pub mod policy_iwrr;
pub mod policy_mct;
pub mod policy_robin;
pub mod policy_throttled;

pub use base_balancer::{OffloadSite, SelectionGuard, TimesFeedback, config};
pub use policy_aco::AcoLB;
pub use policy_esce::EsceLB;
pub use policy_iwrr::InterleavedWeightedRRLB;
pub use policy_mct::MctLB;
pub use policy_robin::RoundRobinLB;
pub use policy_throttled::ThrottledLB;

/// Every policy name main_server accepts (for usage / error messages); keep in sync with its match.
pub const POLICIES: [&str; 8] = [
    "rr",
    "iwrr",
    "esce",
    "mct",
    "mct_const",
    "aco",
    "aco_const",
    "throttled",
];
