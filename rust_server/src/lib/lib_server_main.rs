//! Main-server response helpers used on the request path: the gateway-error reply, the timing
//! headers appended to a site's response (and parsing the site's X-Dana-Timings), and the head
//! sent to a coalescing / prefetch follower. The server state and its startup loaders are in
//! lib_main_state.rs.

use std::io;
use std::time::Instant;

use rust_obm::mod_load_balancer::TimesFeedback;

use crate::lib_core_type as ct;
use crate::lib_http as http;
use crate::lib_util as util;

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

/// [Honest / Pure Domain Logic]
/// Response head for a follower: the zero timing headers of a cache hit (so the benchmark
/// parsers keep working), worker "coalesce" (leader was a real request) or "prefetch" (leader
/// was a prefetch; `speculative`), and X-OBM-Coalesce = ms it waited for the leader (read by
/// nothing old: a new header, the CSV is unchanged).
pub fn coalesce_response_head(body_len: u64, wait_ms: u128, speculative: bool) -> Vec<u8> {
    // Dana's Content-type for segments (OffloadSite_new.dn sendDataChunks)
    const SEGMENT_CONTENT_TYPE: &str = "h264";
    let wait = wait_ms.to_string();
    let headers = [
        ("X-OBM-Coalesce", wait.as_str()),
        (
            "X-OBM-Worker",
            if speculative { "prefetch" } else { "coalesce" },
        ),
        (
            "X-Dana-Timings",
            "queue=0;asset=0;work=0;decode=0;render=0;encode=0",
        ),
        ("X-OBM-Net", "0"),
        ("X-OBM-Wait", "0"),
        ("Server-Timing", "net;dur=0, wait;dur=0"),
        ("Access-Control-Allow-Origin", "*"),
        (
            "Access-Control-Expose-Headers",
            "X-Dana-Timings, Server-Timing, X-OBM-Net, X-OBM-Wait, X-OBM-Worker, X-OBM-Coalesce",
        ),
    ];
    http::format_response_header("200 OK", SEGMENT_CONTENT_TYPE, body_len, &headers)
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

    #[test]
    fn test_coalesce_response_head() {
        let head = String::from_utf8(coalesce_response_head(1234, 56, false)).unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains("Content-Length: 1234\r\n"));
        assert!(head.contains("X-OBM-Coalesce: 56\r\n"));
        assert!(head.contains("X-OBM-Worker: coalesce\r\n"));
        let prefetch = String::from_utf8(coalesce_response_head(1, 0, true)).unwrap();
        assert!(prefetch.contains("X-OBM-Worker: prefetch\r\n"));
        assert!(head.ends_with("\r\n\r\n"));
        // the benchmark reads the same zero timings as a cache hit
        let times = parse_times_feedback(&head, 0).unwrap();
        assert_eq!(
            (times.t_queue, times.t_asset, times.t_work, times.t_net),
            (0, 0, 0, 0)
        );
    }
}
