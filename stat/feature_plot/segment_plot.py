#!/usr/bin/env python3
"""Segment cache (main server): mct with no feature vs mct+Slru / Slfu (one cache, lru / lfu)
and mct+B6 / B8 (2 cost bins, threshold 6 / 8 s); labels without runs are skipped.

Usage: stat/feature_plot/segment_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/segment_plot.py stat/seed2042/run_a1.2_c2
Left: queue (site queue + central wait) / work / asset + net / total as % of no feature (100 %).
Right: mean time per request stacked. Hits count 0 s. See feature_breakdown.py.
Default output stat/feature_plot/segment_<run dir name>.png.
"""
from feature_breakdown import main

if __name__ == "__main__":
    main(["Slru", "Slfu", "B6", "B8"], "Segment cache", "segment", __doc__)
