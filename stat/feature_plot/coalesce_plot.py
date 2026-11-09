#!/usr/bin/env python3
"""In-flight sharing (main server): mct with no feature vs mct+C (request coalescing = reactive:
an identical request that arrives while a render is in flight waits for it) and mct+P (merged
prefetch = proactive: after segments k-1, k of a show + combo it renders k+1 ahead, real requests
for it follow the prefetch). Labels without runs are skipped.

Usage: stat/feature_plot/coalesce_plot.py <run dir> [--policy mct] [--out PNG]
  e.g. stat/feature_plot/coalesce_plot.py stat/pre-sitefix/seed2042/run_a1.2_c2
Left: queue (site queue + central wait) / work / asset + net / follower wait / total as % of no
feature (100 %; follower wait has no % - it is 0 without C / P).
Right: mean time per request stacked (follower = its whole latency as 'follower wait').
Bottom: followers, site renders (real + prefetch), prefetches issued / read. See feature_breakdown.py.
Default output stat/feature_plot/coalesce_<run dir name>.png.
"""
from feature_breakdown import main

if __name__ == "__main__":
    main(["C", "P", "SlruP", "CP"], "Coalescing", "coalesce", __doc__,
         names={"C": "C\n(reactive)", "P": "P\n(proactive,\nmerged prefetch)", "SlruP": "SlruP\n(Slru + P)", "CP": "CP\n(C + P)"})
