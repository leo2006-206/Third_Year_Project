//! Offload work prediction (decode + render + encode, in seconds) from the `stat_predict` cost model.
//!
//! - `coef.csv`: `stage,backend,a,<feature...>`, one linear model per (stage, backend):
//!   `t = a + sum(beta_i * x_i)`. Encode is keyed by `<render>+<encode>` (its timer includes the GPU
//!   readback of the frame).
//! - `features.csv`: `<feature...>,path`, the generic features of each request path. Precomputed for
//!   the experiments; in a real deployment they would be extracted when an unseen request arrives.
//!   `path` is the last column, so a line is split a fixed number of times and the path may contain
//!   commas or spaces.
//!
//! `predict` returns machine-neutral seconds (reference machine om): the machine factor and the
//! network / asset / queue time are the load balancer's job.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::mod_load_balancer::base_balancer::OffloadSite;
use crate::mod_load_balancer::base_balancer::config::{Decode, Encode, Render};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Stage {
    Decode,
    Render,
    Encode,
}

impl Stage {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "decode" => Some(Self::Decode),
            "render" => Some(Self::Render),
            "encode" => Some(Self::Encode),
            _ => None,
        }
    }
}

/// `t = a + sum(beta * x[index])`, only the non-zero coefficients are kept.
#[derive(Debug)]
struct StageModel {
    a: f64,
    beta: Vec<(usize, f64)>, // (column in features.csv, coefficient)
}

#[derive(Debug)]
pub struct CostPredictor {
    models: HashMap<(Stage, String), StageModel>,
    features: HashMap<String, Vec<f64>>, // request path -> feature values (features.csv columns)
    seg_frac: usize,                     // column of seg_frac: frames in the segment / 250
}

impl CostPredictor {
    /// [Dishonest I/O] Loads and checks both CSV files.
    pub fn load(coef_csv: impl AsRef<Path>, features_csv: impl AsRef<Path>) -> io::Result<Self> {
        let coef = std::fs::read_to_string(coef_csv)?;
        let features = std::fs::read_to_string(features_csv)?;
        Self::from_csv(&coef, &features).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// [Honest] Parses and checks the two CSV texts:
    /// numbers finite, coefficients >= 0, every model feature present in features.csv,
    /// a model for every stage, seg_frac in [0, 1], same column count on every row, no duplicate path.
    pub fn from_csv(coef: &str, features: &str) -> Result<Self, String> {
        // features.csv
        let mut lines = features.lines().filter(|l| !l.trim().is_empty());
        let header: Vec<&str> = lines
            .next()
            .ok_or("features.csv is empty")?
            .split(',')
            .collect();
        if header.last() != Some(&"path") {
            return Err("features.csv: last column must be 'path'".into());
        }
        let n_feat = header.len() - 1;
        let column = |name: &str| header[..n_feat].iter().position(|h| *h == name);
        let seg_frac = column("seg_frac").ok_or("features.csv: missing column 'seg_frac'")?;
        let mut feature_rows = HashMap::new();
        for (i, line) in lines.enumerate() {
            let parts: Vec<&str> = line.splitn(n_feat + 1, ',').collect();
            if parts.len() != n_feat + 1 {
                return Err(format!(
                    "features.csv row {}: expected {} columns",
                    i + 2,
                    n_feat + 1
                ));
            }
            let values = parse_numbers(&parts[..n_feat])
                .map_err(|e| format!("features.csv row {}: {e}", i + 2))?;
            if !(0.0..=1.0).contains(&values[seg_frac]) {
                return Err(format!(
                    "features.csv row {}: seg_frac out of [0, 1]",
                    i + 2
                ));
            }
            if feature_rows
                .insert(parts[n_feat].to_string(), values)
                .is_some()
            {
                return Err(format!("features.csv row {}: duplicate path", i + 2));
            }
        }

        // coef.csv
        let mut lines = coef.lines().filter(|l| !l.trim().is_empty());
        let coef_header: Vec<&str> = lines
            .next()
            .ok_or("coef.csv is empty")?
            .split(',')
            .collect();
        if coef_header.get(..3) != Some(&["stage", "backend", "a"][..]) {
            return Err("coef.csv: header must start with stage,backend,a".into());
        }
        let feature_cols = coef_header[3..]
            .iter()
            .map(|name| {
                column(name).ok_or(format!("coef.csv feature '{name}' not in features.csv"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut models = HashMap::new();
        for (i, line) in lines.enumerate() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() != coef_header.len() {
                return Err(format!(
                    "coef.csv row {}: expected {} columns",
                    i + 2,
                    coef_header.len()
                ));
            }
            let stage = Stage::parse(parts[0]).ok_or(format!(
                "coef.csv row {}: unknown stage '{}'",
                i + 2,
                parts[0]
            ))?;
            let values =
                parse_numbers(&parts[2..]).map_err(|e| format!("coef.csv row {}: {e}", i + 2))?;
            if values.iter().any(|v| *v < 0.0) {
                return Err(format!("coef.csv row {}: negative coefficient", i + 2));
            }
            let beta = feature_cols
                .iter()
                .copied()
                .zip(values[1..].iter().copied())
                .filter(|(_, b)| *b != 0.0)
                .collect();
            models.insert(
                (stage, parts[1].to_string()),
                StageModel { a: values[0], beta },
            );
        }
        for stage in [Stage::Decode, Stage::Render, Stage::Encode] {
            if !models.keys().any(|(s, _)| *s == stage) {
                return Err(format!("coef.csv: no model for stage {stage:?}"));
            }
        }
        Ok(Self {
            models,
            features: feature_rows,
            seg_frac,
        })
    }

    /// [Honest] Predicted work (s) of the request `path` on `site` (machine-neutral), or None if the
    /// path has no features or the site's backends have no model.
    pub fn predict(&self, path: &str, site: &OffloadSite) -> Option<f64> {
        let x = self
            .features
            .get(path)
            .or_else(|| self.features.get(&percent_decode(path)))?;
        let cfg = &site.config;
        let decode = match cfg.decode {
            Decode::Gpu => "gpu",
            Decode::Cpu => "cpu",
        };
        let render = match cfg.render {
            Render::Gpu => "gpu",
            Render::Llvmpipe => "llvmpipe",
            Render::Cpu => "cpu",
        };
        let encode = match cfg.encode {
            Encode::Cpu => "cpu",
            Encode::Gpu => "gpu",
        };
        let stage_time = |stage: Stage, backend: String| {
            let m = self.models.get(&(stage, backend))?;
            Some(m.a + m.beta.iter().map(|(i, b)| b * x[*i]).sum::<f64>())
        };
        let full_segment = stage_time(Stage::Decode, decode.into())?
            + stage_time(Stage::Render, render.into())?
            + stage_time(Stage::Encode, format!("{render}+{encode}"))?;
        Some(full_segment * x[self.seg_frac])
    }

    /// Number of request paths with features.
    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }
}

/// [Honest] Parses CSV fields as finite f64.
fn parse_numbers(fields: &[&str]) -> Result<Vec<f64>, String> {
    fields
        .iter()
        .map(|f| match f.trim().parse::<f64>() {
            Ok(v) if v.is_finite() => Ok(v),
            _ => Err(format!("not a finite number: '{f}'")),
        })
        .collect()
}

/// [Honest] Decodes %XX escapes (the main server sees e.g. `%7C` for `|` and `%20` for a space).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_load_balancer::base_balancer::config::Config;

    const COEF: &str = "stage,backend,a,n_video,chroma_frac,video_frac\n\
                        decode,gpu,0.1,0.5,0,0\n\
                        render,gpu,0.2,0,3,0\n\
                        render,cpu,5,0,4,0\n\
                        encode,gpu+cpu,1.5,0,0,0.5\n\
                        encode,cpu+cpu,1,0,0,0\n";
    const FEATURES: &str = "n_video,chroma_frac,video_frac,seg_frac,path\n\
                            2,1,1,1,/offload/show/x.json/0/10/1280/720/main/landscape/2/bg|calm/fg|key\n\
                            1,0,1,0.4,/offload/show/x.json/80/90/1280/720/main/landscape/1/bg|calm,dots\n\
                            1,0,1,1,/offload/show/x.json/0/10/1280/720/main/landscape/1/w|Weather Icons\n";

    fn site(render: Render) -> OffloadSite {
        OffloadSite::new("h:1", "s", Config::new(Decode::Gpu, render, Encode::Cpu))
    }

    fn approx(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn test_predict_sums_stages() {
        let p = CostPredictor::from_csv(COEF, FEATURES).unwrap();
        assert_eq!(p.len(), 3);
        let path = "/offload/show/x.json/0/10/1280/720/main/landscape/2/bg|calm/fg|key";
        // decode 0.1 + 0.5*2, render 0.2 + 3*1, encode 1.5 + 0.5*1
        assert!(approx(p.predict(path, &site(Render::Gpu)), 1.1 + 3.2 + 2.0));
        // cpu render: 5 + 4*1, encode cpu+cpu 1
        assert!(approx(p.predict(path, &site(Render::Cpu)), 1.1 + 9.0 + 1.0));
    }

    #[test]
    fn test_predict_scales_short_segment_and_keeps_commas() {
        let p = CostPredictor::from_csv(COEF, FEATURES).unwrap();
        let path = "/offload/show/x.json/80/90/1280/720/main/landscape/1/bg|calm,dots";
        assert!(approx(
            p.predict(path, &site(Render::Gpu)),
            (0.6 + 0.2 + 2.0) * 0.4
        ));
    }

    #[test]
    fn test_predict_decodes_percent_escapes() {
        let p = CostPredictor::from_csv(COEF, FEATURES).unwrap();
        let encoded = "/offload/show/x.json/0/10/1280/720/main/landscape/1/w%7CWeather%20Icons";
        assert!(p.predict(encoded, &site(Render::Gpu)).is_some());
    }

    #[test]
    fn test_predict_none_when_unknown() {
        let p = CostPredictor::from_csv(COEF, FEATURES).unwrap();
        assert_eq!(
            p.predict(
                "/offload/show/unseen.json/0/10/1280/720/main/landscape/1/a|b",
                &site(Render::Gpu)
            ),
            None
        );
        // no llvmpipe model in COEF
        let path = "/offload/show/x.json/0/10/1280/720/main/landscape/2/bg|calm/fg|key";
        assert_eq!(p.predict(path, &site(Render::Llvmpipe)), None);
    }

    #[test]
    fn test_from_csv_checks() {
        let neg = COEF.replace("render,gpu,0.2", "render,gpu,-0.2");
        assert!(
            CostPredictor::from_csv(&neg, FEATURES)
                .unwrap_err()
                .contains("negative")
        );
        let missing = COEF.replace("video_frac", "bogus");
        assert!(
            CostPredictor::from_csv(&missing, FEATURES)
                .unwrap_err()
                .contains("bogus")
        );
        let no_encode: String = COEF
            .lines()
            .filter(|l| !l.starts_with("encode"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert!(
            CostPredictor::from_csv(&no_encode, FEATURES)
                .unwrap_err()
                .contains("Encode")
        );
        let bad_frac = FEATURES.replace(",0.4,", ",1.4,");
        assert!(
            CostPredictor::from_csv(COEF, &bad_frac)
                .unwrap_err()
                .contains("seg_frac")
        );
        let nan = FEATURES.replace("2,1,1,1,", "2,NaN,1,1,");
        assert!(
            CostPredictor::from_csv(COEF, &nan)
                .unwrap_err()
                .contains("finite")
        );
    }

    /// The exported model in stat_predict/ (skipped if not exported yet).
    #[test]
    fn test_load_exported_model() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../stat_predict");
        let Ok(p) = CostPredictor::load(dir.join("coef.csv"), dir.join("features.csv")) else {
            return;
        };
        assert!(p.len() > 1000);
        let forecast = "/offload/show/forecast.json/0/10/1280/720/core/landscape/4/map|map_colour/weather|Meteorological/presenter|hearing/presenter_chroma|chroma/time|time";
        let gpu = p.predict(forecast, &site(Render::Gpu)).unwrap();
        let cpu = p.predict(forecast, &site(Render::Cpu)).unwrap();
        assert!(gpu > 1.0 && cpu > gpu, "gpu {gpu}, cpu {cpu}");
    }
}
