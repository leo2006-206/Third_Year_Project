//! OBM request paths: which show, which segment, which layer combination.
//!
//! A segment request is `/offload/show/<show>.json/<t_start>/<t_end>/<rest>`, e.g.
//! `/offload/show/f1_full.json/0/10/1280/720/driver/landscape/2/driver|race/race|Race`:
//! a 10 s segment (`t_end = t_start + 10`, `t_start` a multiple of 10) and `rest` = output size +
//! layout + layer options, i.e. the combination. Two requests with the same show and `rest` watch
//! the same stream; only the segment differs.
//!
//! Everything here works on the RAW path (no percent-decoding): the segment cache and the
//! coalescer key on the raw path, so a path built here must be spelled like the client's.
//! The whole module is honest (no I/O; the show durations are compiled in).

use std::collections::HashMap;
use std::sync::LazyLock;

/// Length of one segment (s), as the client requests them (generate_request_seq.py SEG_LEN_S)
pub const SEGMENT_LEN_S: u32 = 10;

const SHOW_PREFIX: &str = "/offload/show/";

/// servers_benchmark/video_duration.csv (`video,duration`), compiled in
const VIDEO_DURATION_CSV: &str = include_str!("../../servers_benchmark/video_duration.csv");

/// Show file (e.g. "f1_full.json") -> duration (s), from video_duration.csv. A show not in it
/// has no known length (no prefetch past its segment).
static SHOW_DURATION_S: LazyLock<HashMap<&'static str, u32>> = LazyLock::new(|| {
    parse_duration_csv(VIDEO_DURATION_CSV)
        .unwrap_or_else(|e| panic!("servers_benchmark/video_duration.csv: {e}"))
});

/// [Honest] Parses `video,duration` CSV text (header first, blank lines skipped).
fn parse_duration_csv(text: &str) -> Result<HashMap<&str, u32>, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some("video,duration") => {}
        other => return Err(format!("header {other:?}, expected \"video,duration\"")),
    }
    lines
        .map(|line| {
            let (video, duration) = line
                .split_once(',')
                .ok_or_else(|| format!("line {line:?}: expected video,duration"))?;
            let duration = duration
                .trim()
                .parse()
                .map_err(|e| format!("line {line:?}: {e}"))?;
            Ok((video.trim(), duration))
        })
        .collect()
}

/// [Honest] Duration (s) of `show` (file name, e.g. "spiders.json"), None if unknown.
pub fn show_duration_s(show: &str) -> Option<u32> {
    SHOW_DURATION_S.get(show).copied()
}

/// [Honest] Index of the show's last segment: 90 s -> 8 (segments 0..=8), None if unknown.
pub fn last_segment(show: &str) -> Option<u32> {
    show_duration_s(show).map(|d| d.div_ceil(SEGMENT_LEN_S).saturating_sub(1))
}

/// A segment request split into its parts, borrowing from the raw path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPath<'a> {
    /// show file, e.g. "f1_full.json"
    pub show: &'a str,
    /// segment index (t_start / 10)
    pub segment: u32,
    /// everything after t_end: output size, layout and layer options (the combination)
    pub rest: &'a str,
}

impl<'a> SegmentPath<'a> {
    /// [Honest] Parses a segment request path; None if it is not one (other prefix, missing
    /// parts, times that are not one aligned 10 s segment).
    pub fn parse(path: &'a str) -> Option<Self> {
        let mut parts = path.strip_prefix(SHOW_PREFIX)?.splitn(4, '/');
        let show = parts.next().filter(|s| !s.is_empty())?;
        let t_start: u32 = parts.next()?.parse().ok()?;
        let t_end: u32 = parts.next()?.parse().ok()?;
        let rest = parts.next().filter(|s| !s.is_empty())?;
        if !t_start.is_multiple_of(SEGMENT_LEN_S) || t_end != t_start + SEGMENT_LEN_S {
            return None;
        }
        Some(Self {
            show,
            segment: t_start / SEGMENT_LEN_S,
            rest,
        })
    }

    /// [Honest] The same show and combination at segment `segment` (raw spelling kept).
    pub fn with_segment(&self, segment: u32) -> String {
        let t_start = segment * SEGMENT_LEN_S;
        format!(
            "{SHOW_PREFIX}{}/{t_start}/{}/{}",
            self.show,
            t_start + SEGMENT_LEN_S,
            self.rest
        )
    }

    /// [Honest] The next segment's path, None at the show's last segment or for an unknown show.
    pub fn next_path(&self) -> Option<String> {
        let next = self.segment + 1;
        (next <= last_segment(self.show)?).then(|| self.with_segment(next))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const F1: &str =
        "/offload/show/f1_full.json/0/10/1280/720/driver/landscape/2/driver|race/race|Race";

    #[test]
    fn test_durations_from_csv() {
        assert_eq!(show_duration_s("f1_full.json"), Some(90));
        assert_eq!(show_duration_s("spiders.json"), Some(60));
        assert_eq!(show_duration_s("forest720.json"), Some(30));
        assert_eq!(show_duration_s("syn_combo.json"), None);
        assert_eq!(SHOW_DURATION_S.len(), 6);
    }

    #[test]
    fn test_last_segment() {
        // the segments the workload uses (stat_predict/features.csv max t_start / 10)
        assert_eq!(last_segment("f1.json"), Some(8));
        assert_eq!(last_segment("forecast.json"), Some(8));
        assert_eq!(last_segment("spiders.json"), Some(5));
        assert_eq!(last_segment("forest720_leaves.json"), Some(2));
        assert_eq!(last_segment("nope.json"), None);
    }

    #[test]
    fn test_parse_duration_csv_errors() {
        assert!(parse_duration_csv("video,duration\na.json,12\n\n").is_ok());
        assert!(parse_duration_csv("show,len\na.json,12").is_err());
        assert!(parse_duration_csv("video,duration\na.json").is_err());
        assert!(parse_duration_csv("video,duration\na.json,x").is_err());
    }

    #[test]
    fn test_parse_segment_path() {
        let p = SegmentPath::parse(F1).unwrap();
        assert_eq!(p.show, "f1_full.json");
        assert_eq!(p.segment, 0);
        assert_eq!(p.rest, "1280/720/driver/landscape/2/driver|race/race|Race");
        let p = SegmentPath::parse("/offload/show/spiders.json/50/60/a/b").unwrap();
        assert_eq!(p.segment, 5);
    }

    #[test]
    fn test_parse_rejects_non_segments() {
        for bad in [
            "/assets/x.png",
            "/offload/show/",
            "/offload/show/a.json/0/10",         // no combination
            "/offload/show/a.json/0/10/",        // empty combination
            "/offload/show/a.json/0/20/x",       // two segments
            "/offload/show/a.json/5/15/x",       // not aligned
            "/offload/show/a.json/x/10/x",       // not a number
            "/offload/show//0/10/x",             // no show
            "/times/offload/show/a.json/0/10/x", // /times is stripped before this
        ] {
            assert_eq!(SegmentPath::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn test_with_segment_keeps_raw_spelling() {
        let raw = "/offload/show/f1.json/10/20/1280/720/a%7Cb/c|d";
        let p = SegmentPath::parse(raw).unwrap();
        assert_eq!(p.with_segment(1), raw);
        assert_eq!(
            p.with_segment(2),
            "/offload/show/f1.json/20/30/1280/720/a%7Cb/c|d"
        );
    }

    #[test]
    fn test_next_path() {
        let p = SegmentPath::parse(F1).unwrap();
        assert_eq!(
            p.next_path().as_deref(),
            Some(
                "/offload/show/f1_full.json/10/20/1280/720/driver/landscape/2/driver|race/race|Race"
            )
        );
        let last = SegmentPath::parse("/offload/show/forest720.json/20/30/x").unwrap();
        assert_eq!(last.next_path(), None);
        let unknown = SegmentPath::parse("/offload/show/syn_res.json/0/10/x").unwrap();
        assert_eq!(unknown.next_path(), None);
    }
}
