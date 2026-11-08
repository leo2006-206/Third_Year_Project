#!/usr/bin/env python3
"""Asset cache (every site, 50 MB): mct with no feature vs mct+Alru and mct+Alfu.

Usage: stat/feature_plot/asset_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/asset_plot.py stat/seed2042/run_a1.2_c2
Left: queue (site queue + central wait) / work / asset + net / total as % of no feature (100 %).
Right: mean time per request stacked. See feature_breakdown.py.
Default output stat/feature_plot/asset_<run dir name>.png.
"""
from feature_breakdown import main

if __name__ == "__main__":
    main(["Alru", "Alfu", "SlfuAlfu"], "Asset cache", "asset", __doc__)
