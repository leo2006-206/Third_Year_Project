#!/bin/bash
# IDEAL-WORLD (FAKE) PSC show forecast_psc: render one badge combination (all 9 segments, Dana direct mode on the host bench
# temp/psc_ideal/obm) and play it with ffplay. Segments start with SPS/PPS + IDR, so they can be joined with cat.
# Usage: syn_video/PSC_ideal/play.sh [A0..A9] [B0..B9]      (default A3 B7; ~70 s to render on om gpu)
set -e
A=${1:-A3}; B=${2:-B7}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BENCH=$ROOT/temp/psc_ideal; OUT=$BENCH/out/play
mkdir -p "$OUT"
# asset server (web root of symlinks) on 127.0.0.1:8080, started here if not running
if ! curl -s -o /dev/null http://127.0.0.1:8080/shows/forecast_psc.json; then
  (cd "$BENCH" && python3 server_key.py www > out/server_key.log 2>&1 &) ; sleep 1; STARTED=1
fi
cd "$BENCH/obm"
for t in 0 10 20 30 40 50 60 70 80; do
  dana OffloadSite_new -req "/offload/show/forecast_psc.json/$t/$((t+10))/1280/720/core/landscape/6/map|map_colour/weather|Meteorological/presenter|hearing/presenter_chroma|chroma/time|time/badge_a|$A/badge_b|$B" \
       -out "$OUT/seg_$(printf %02d $t).h264" -raw > /dev/null 2>&1
done
cat "$OUT"/seg_*.h264 > "$OUT/${A}${B}_full.h264"
[ -n "$STARTED" ] && pkill -f "python3 server_key.py www" || true
echo "rendered $OUT/${A}${B}_full.h264"
ffplay -f h264 -framerate 25 -window_title "forecast_psc $A $B (ideal-world, fake)" "$OUT/${A}${B}_full.h264"
