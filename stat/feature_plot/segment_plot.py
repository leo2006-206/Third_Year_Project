#!/usr/bin/env python3
"""Segment cache (main server): mct with no feature vs mct+Slru / Slfu (one cache, lru / lfu)
and mct+B6lru / B8lru / B8lfu (2 cost bins, threshold 6 / 8 s, lru / lfu); labels without runs are skipped.
Single-feature runs only (pairs are analysed separately, feature_summary.py).

Usage: stat/feature_plot/segment_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/segment_plot.py stat/pre-sitefix/seed2042/run_a1.2_c2
Left: queue (site queue + central wait) / work / asset + net / total as % of no feature (100 %).
Right: mean time per request stacked. Hits count 0 s. See feature_breakdown.py.
Default output stat/feature_plot/segment_<run dir name>.png.
"""

from feature_breakdown import main

if __name__ == "__main__":
    main(
        ["Slru", "Slfu", "B6lru", "B8lru", "B8lfu"], "Segment cache", "segment", __doc__
    )
