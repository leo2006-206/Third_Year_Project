#!/usr/bin/env python3
"""Partial segment cache (PSC, main server + sites, fixed site only): mct with no feature vs
mct+PSC<heavy><small|save> (heavy = min predicted save s; small = cache the smallest cut that
saves enough, save = the cut with the biggest save). Labels without runs are skipped.

Usage: stat/feature_plot/psc_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/psc_plot.py stat/sitefix/seed2042/run_a1.2_c2 --out stat/sitefix/feature_plot/psc_seed2042_run_a1.2_c2.png
Left: queue (site queue + central wait) / work / asset + net / total as % of no feature (100 %).
Right: mean time per request stacked. Bottom: per bar rebuilds (= hits) / eligible, creates (pair
renders), plain reasons, measured site service of rebuild / create rows vs the same kind in none,
net site s. Main log tags [PSC] create / stored / rebuild / rebuild failed / plain (feature_detail_plot.psc_metrics).
Default output stat/feature_plot/psc_<run dir name>.png.
"""
from feature_breakdown import main

if __name__ == "__main__":
    main(["PSC1small", "PSC0.5small", "PSC0.5save"], "Partial segment cache", "psc", __doc__)
