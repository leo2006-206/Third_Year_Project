//! Offload work prediction by lookup: the measured work (decode + render + encode, in seconds) of
//! a request path on a site config, from `stat_predict/lookup.csv` (`stat_predict/lookup_table.py`).
//!
//! - `lookup.csv`: `config,time_s,n,path`, one row per (site config, request path). `config` is
//!   `<decoding>_<composition>_<encoding>` as in the cluster CSVs (e.g. `gpu_llvmpipe_cpu`), `time_s`
//!   the median measured work in om-equivalent seconds, `n` the number of samples. `path` is the last
//!   column, so it may contain commas.
//!
//! Same interface as `CostPredictor::predict`: machine-neutral seconds (om), the machine factor,
//! network / asset / queue time are the load balancer's job. Only paths measured on that config are
//! known.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::cost_predict::{backend_names, percent_decode};
use crate::mod_load_balancer::base_balancer::OffloadSite;

#[derive(Debug)]
pub struct CostLookup {
    times: HashMap<(String, String), f64>, // (config, request path) -> work s (om)
}

impl CostLookup {
    /// [Dishonest I/O] Loads and checks the CSV file.
    pub fn load(csv: impl AsRef<Path>) -> io::Result<Self> {
        let text = std::fs::read_to_string(csv)?;
        Self::from_csv(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// [Honest] Parses and checks the CSV text: header, time finite and > 0, n a positive integer,
    /// config of three backend names, no duplicate (config, path).
    pub fn from_csv(text: &str) -> Result<Self, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        if lines.next() != Some("config,time_s,n,path") {
            return Err("lookup.csv: header must be config,time_s,n,path".into());
        }
        let mut times = HashMap::new();
        for (i, line) in lines.enumerate() {
            let row = i + 2;
            let parts: Vec<&str> = line.splitn(4, ',').collect();
            let [config, time, n, path] = parts[..] else {
                return Err(format!("lookup.csv row {row}: expected 4 columns"));
            };
            if config.split('_').count() != 3 {
                return Err(format!("lookup.csv row {row}: bad config '{config}'"));
            }
            let time: f64 = match time.trim().parse() {
                Ok(t) if f64::is_finite(t) && t > 0.0 => t,
                _ => return Err(format!("lookup.csv row {row}: bad time '{time}'")),
            };
            if !n.trim().parse::<u32>().is_ok_and(|n| n > 0) {
                return Err(format!("lookup.csv row {row}: bad n '{n}'"));
            }
            if times
                .insert((config.to_string(), path.to_string()), time)
                .is_some()
            {
                return Err(format!("lookup.csv row {row}: duplicate config and path"));
            }
        }
        Ok(Self { times })
    }

    /// [Honest] Measured work (s, om-equivalent) of the request `path` on `site`'s config, or None
    /// if that path was never measured on that config.
    pub fn predict(&self, path: &str, site: &OffloadSite) -> Option<f64> {
        let (decode, render, encode) = backend_names(&site.config);
        let config = format!("{decode}_{render}_{encode}");
        self.times
            .get(&(config.clone(), path.to_string()))
            .or_else(|| self.times.get(&(config, percent_decode(path))))
            .copied()
    }

    /// Number of (config, path) rows.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_load_balancer::base_balancer::config::{Config, Decode, Encode, Render};

    const CSV: &str = "config,time_s,n,path\n\
                       gpu_gpu_cpu,2.5,4,/offload/show/x.json/0/10/1280/720/main/landscape/1/w|Weather Icons\n\
                       gpu_cpu_cpu,9.0,2,/offload/show/x.json/0/10/1280/720/main/landscape/1/w|Weather Icons\n\
                       gpu_gpu_cpu,1.25,1,/offload/show/x.json/80/90/1280/720/main/landscape/1/bg|calm,dots\n";

    fn site(render: Render) -> OffloadSite {
        OffloadSite::new("h:1", "s", Config::new(Decode::Gpu, render, Encode::Cpu))
    }

    #[test]
    fn test_lookup_by_config_and_path() {
        let l = CostLookup::from_csv(CSV).unwrap();
        assert_eq!(l.len(), 3);
        let path = "/offload/show/x.json/0/10/1280/720/main/landscape/1/w|Weather Icons";
        assert_eq!(l.predict(path, &site(Render::Gpu)), Some(2.5));
        assert_eq!(l.predict(path, &site(Render::Cpu)), Some(9.0));
        // never measured on llvmpipe
        assert_eq!(l.predict(path, &site(Render::Llvmpipe)), None);
    }

    #[test]
    fn test_lookup_decodes_percent_escapes_and_keeps_commas() {
        let l = CostLookup::from_csv(CSV).unwrap();
        let encoded = "/offload/show/x.json/0/10/1280/720/main/landscape/1/w%7CWeather%20Icons";
        assert_eq!(l.predict(encoded, &site(Render::Gpu)), Some(2.5));
        let comma = "/offload/show/x.json/80/90/1280/720/main/landscape/1/bg|calm,dots";
        assert_eq!(l.predict(comma, &site(Render::Gpu)), Some(1.25));
    }

    #[test]
    fn test_from_csv_checks() {
        let header = CSV.replace("config,time_s", "cfg,time_s");
        assert!(
            CostLookup::from_csv(&header)
                .unwrap_err()
                .contains("header")
        );
        let zero = CSV.replace(",2.5,4,", ",0,4,");
        assert!(
            CostLookup::from_csv(&zero)
                .unwrap_err()
                .contains("bad time")
        );
        let nan = CSV.replace(",2.5,4,", ",NaN,4,");
        assert!(CostLookup::from_csv(&nan).unwrap_err().contains("bad time"));
        let n = CSV.replace(",2.5,4,", ",2.5,0,");
        assert!(CostLookup::from_csv(&n).unwrap_err().contains("bad n"));
        let cfg = CSV.replace("gpu_cpu_cpu,", "gpu_cpu,");
        assert!(
            CostLookup::from_csv(&cfg)
                .unwrap_err()
                .contains("bad config")
        );
        let dup = format!(
            "{CSV}gpu_cpu_cpu,1,1,/offload/show/x.json/0/10/1280/720/main/landscape/1/w|Weather Icons\n"
        );
        assert!(
            CostLookup::from_csv(&dup)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    /// The exported table in stat_predict/ (skipped if not exported yet).
    #[test]
    fn test_load_exported_table() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stat_predict");
        let Ok(l) = CostLookup::load(dir.join("lookup.csv")) else {
            return;
        };
        assert!(l.len() > 1000);
        let path = "/offload/show/f1.json/0/10/1280/720/driver/landscape/2/driver|race/race|Race";
        let gpu = l.predict(path, &site(Render::Gpu)).unwrap();
        let cpu = l.predict(path, &site(Render::Cpu)).unwrap();
        assert!(gpu > 0.5 && cpu > gpu, "gpu {gpu}, cpu {cpu}");
    }
}
