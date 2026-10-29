use std::io;
use std::path::Path;
use std::time::Instant;

use rust_obm::mod_load_balancer::{OffloadSite, TimesFeedback};
use rust_obm::mod_load_balancer::config::{Config, Decode, Encode, Render};

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
/// Server-Timing, X-OBM-Worker), ready to send to the client, plus the ms from `t_start` to now
/// (logged as the offload time). Call it right after the head arrives.
/// Returns the new head and the timing feedback for the load balancer (all zero if the site
/// sent no X-Dana-Timings).
pub fn time_append_header(head: &[u8], t_start: Instant, worker: &str) -> (Vec<u8>, TimesFeedback) {
    let elapsed = t_start.elapsed().as_millis() as i64;

    let head_str = String::from_utf8_lossy(head);
    let times = parse_times_feedback(&head_str, elapsed).unwrap_or_default();
    let t_net_str = times.t_net.to_string();
    let server_timing_str = format!("net;dur={}", times.t_net);
    let extra_headers = [
        ("X-OBM-Net", t_net_str.as_str()),
        ("Server-Timing", server_timing_str.as_str()),
        ("X-OBM-Worker", worker),
        (
            "Access-Control-Expose-Headers",
            "X-Dana-Timings, Server-Timing, X-OBM-Net, X-OBM-Worker",
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_times_feedback() {
        let head = "HTTP/1.0 200 OK\r\nContent-length: 10\r\nX-Dana-Timings: queue=3;asset=611;work=2546;decode=631\r\nConnection: close";
        let times = parse_times_feedback(head, 3600).unwrap();
        assert_eq!(
            times,
            TimesFeedback { t_net: 440, t_queue: 3, t_work: 2546, t_asset: 611 }
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
        assert!(parse_cluster_csv("id,port,decoding,composition,encoding\n1,7010,gpu,gpu,cpu").is_err());
        let bad = "ip,port,decoding,composition,encoding\nh,7010,gpu,vulkan,cpu";
        assert!(parse_cluster_csv(bad).unwrap_err().contains("composition 'vulkan'"));
        assert!(parse_cluster_csv("ip,port,decoding,composition,encoding\nh,99999,gpu,gpu,cpu").is_err());
        assert!(parse_cluster_csv("ip,port,decoding,composition,encoding\nh,7010,gpu").is_err());
    }
}
