How the user wants plots + every plot program (where, purpose, output). updated 2026-11-07

## CLI OUTPUT RULE (user 11-07)
- Every plot tool must ALSO print the numbers it plots as text on stdout (table with a header line, same values as the bars / lines, units stated), not only write the PNG, so an AI can analyse a run without reading the image. Last line may stay 'plot -> <path>'. Status per tool: lb_features.md section 3 (CLI column); missing ones = todo 'PLOT CLI OUTPUT'.

## Style rules (user preferences, collected 11-01..11-05)
- Consistent policy colours everywhere (from stat/latency_plot.py STYLE): colour by FAMILY, shade within it; hatched '//' = no cost model.
  rr #8a8984, iwrr331 #5c5b57 | esce #0e7d55, throttled #1baf7a, throttled2 #6fd3ab | aco #eb6834, aco_const #f2a07c (hatched) | mct #104281, mct_lookup #2a78d6, mct_const #6da7ec (hatched), mct_const_flat #9ec5f4 (hatched). Unknown policy #c3c2b7.
- Bar order inside a group = by family: rr, esce, throttled2, mct, mct_const_flat (static -> count-based -> model).
- Statistics shown: mean, p50, p90, p99 (each its own panel / own y axis when scales differ). Latency of status 200 requests only, per run, then mean over runs (error bar = std over runs).
- Values written on bars; rank badges (top-5 lowest = numbered black circles) per panel; outlier bars (rr) CUT at ~1.1x 2nd highest with the value printed inside + arrow, never let rr flatten the axis (or leave rr out and say so).
- Wide figures (e.g. 18x10 for timelines, ~20x6.7 for 4-panel latency); legend in ONE row on top; no clipped labels (use fixed subplots_adjust, NOT tight_layout - 11-03 clipping bug).
- Light surface #fcfcfb, ink #3b3a37, muted #52514e, grid #e4e3df, top/right spines off, recessive grid.
- Count / win tables: compare on exact (unrounded) values -> whole counts (user confused by 0.5 split ties, 11-05); show best and worst SEPARATELY.
- Output location: per-sequence-set plots live with their data (stat/seed<S>/latency_seed<S>_run_*.png, user moved them 11-05); set 2 plots stay in stat/. Only latency_detail_run_* (set 2) kept; no latency_detail for seeds (user deleted 11-05). Analysis / slide figures -> temp/figs/. PNGs are gitignored (except highlight/*.png).
- No dual y axes; one figure = one message (slide-ready title saying what is compared).

## Programs
| program | purpose | input | output |
|---|---|---|---|
| stat/latency_plot.py `<run dir> [--out PNG] [--exclude POLICY]` | per sequence: mean/p50/p90/p99 per policy, 4 panels, badges, cut bars; prints table ± std | stat/[seedS/]run_a*_c*/ | stat/latency_<dir>.png (nested sets -> latency_seed<S>_run_...; move to stat/seed<S>/) |
| stat/latency_plot.py `<run dir> --features [LABEL..] [--policy mct]` (11-06) | feature mode: reads <policy>_<feat>_<N>.csv too (label no '_'); bars '<policy>' + '<policy>+<feat>'; NO bar cut (baseline must show); colours S #6b3fa0 / B #b28ad9 (segment caches purple), Alru #e8b74a / Alfu #b9770e (asset caches amber) | stat/[seedS/]run_a*_c*/ mct_{none,S,B,Alru,Alfu}_* | stat/feature_plot/latency_features_<dir>.png (8 made: 2037 + 2042 x 4 seqs; feature n=1 so std 0) |
| stat/feature_plot/feature_detail_plot.py `<run dir> [--policy mct] [--out]` (moved 11-06 from stat/; cut to ONE panel 11-06 per user) | COMMON comparison: one stacked bar per run type mct / +S / +B / +Alru / +Alfu = mean time per request, all requests (hit = 0): site queue / central wait / work / asset+net. Still computes + prints hit ratios, per-site share/queue/asset (module = metric source for feature_breakdown). Segment hits = csv worker 'cache'; bins from main log; asset from site logs [ASSET] hit/added/add failed/skip (miss = fetch) | stat/[seedS/]run_a*_c*/mct_*_N + _main.log + _site_*.log | stat/feature_plot/feature_detail_<dir>.png (8) |
| stat/feature_plot/segment_plot.py, asset_plot.py `<run dir> [--policy mct]` (11-06; shared code feature_breakdown.py, imports feature_detail_plot) | one program per feature family: segment = mct none vs +S/+B, asset = none vs +Alru/+Alfu. LEFT: queue (site queue + central wait) / work / asset+net / total as % of no feature (100 %, dashed line), label 'N% (-M%)'; RIGHT: stacked s per request (all requests, hit = 0); BOTTOM bold row per feature: colour square (bar colour) + '<bar>: (hit H / miss M req|files) = X% hit (Y% of bytes)' (+ B '[lower N, upper M]'), hit ratio = hit/(hit+miss), asset summed over sites | same as feature_detail | stat/feature_plot/{segment,asset}_<dir>.png (8 each) |
| stat/latency_detail_plot.py `<run dir>` | stacked mean breakdown: site queue / central wait / work / asset+net, badges, top-5 lists | same | stat/latency_detail_<dir>.png (set 2 only kept) |
| servers_benchmark/sequences/plot_seq.py `<seq csv...>` | request sequence timeline (arrivals per bin, show share legend) | seq_a*_case*.csv | <seq>_timeline.png next to the csv |
| temp/seed_compare_plot.py | slide: seed 2037 vs 2042 mean/p99 x 4 seqs (rr left out) | stat/, stat/seed2042 | temp/figs/seed_compare.png (offered: extend to 5 seeds) |
| temp/seed_rank.py `[--csv out]` | 5-seed rank table + Kendall W + mct-best counts (text, no plot) | stat/, stat/seed204{2..5} | stdout, temp/analysis_v4/seed_rank_5seeds.csv |
| temp/case_wins_plot.py `[--case c1/c2] [--out]` | how often each policy is best / worst per statistic over 5 seeds x 2 alphas; 2 panels (best, worst); prints both tables | all 5 seed roots | temp/figs/<case>_wins.png |
| highlight/make_figures.py | README highlight figures (headline, latency_c1, breakdown_c1, cost_model, offline_bracket, workload_c1) | set 2 data + highlight/data | highlight/*.png (tracked) |
| temp/plot_tier_comparison.py | old (10-2x) tier comparison | old data | - |
