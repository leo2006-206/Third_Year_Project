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
