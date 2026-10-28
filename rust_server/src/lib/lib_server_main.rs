use std::io;
use std::path::Path;

use rust_obm::mod_load_balancer::OffloadSite;
use rust_obm::mod_load_balancer::config::{Config, Decode, Encode, Render};

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

pub fn compute_t_net(return_header: &str, elapsed: i64) -> Option<i64> {
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
    Some((elapsed - (queue + asset + work)).max(0))
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
        assert!(parse_cluster_csv("id,port,decoding,composition,encoding\n1,7010,gpu,gpu,cpu").is_err());
        let bad = "ip,port,decoding,composition,encoding\nh,7010,gpu,vulkan,cpu";
        assert!(parse_cluster_csv(bad).unwrap_err().contains("composition 'vulkan'"));
        assert!(parse_cluster_csv("ip,port,decoding,composition,encoding\nh,99999,gpu,gpu,cpu").is_err());
        assert!(parse_cluster_csv("ip,port,decoding,composition,encoding\nh,7010,gpu").is_err());
    }
}
