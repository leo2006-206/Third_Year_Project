// src/lib/lib.rs

pub mod server_util;

// Optional: re-export directly so callers don't have to write server_util::...
pub use server_util::*;
