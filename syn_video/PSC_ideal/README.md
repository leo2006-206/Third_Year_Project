# forecast_psc - IDEAL-WORLD (FAKE) PSC SHOW

**Not a realistic workload.** A best-case show built only to show the regime where the partial segment cache (PSC) wins
("what if there is an ideal world for PSC", user D95 / D98). Report it next to the BBC-set results, never as one of them.

## Show
`shows/forecast_psc.json`: the forecast `core` variant (1280x720, 2075 frames, 9 segments) with
- map / weather / presenter fixed to their default option (`map_colour`, `Meteorological`, `hearing`) + `presenter_chroma` + `time`
  = 4 roots = the heavy prefix (same as the BBC forecast default path),
- `badge_a`: anim_img, 10 options `A0`..`A9`, BIG badge 480x120 at (40, 560),
- `badge_b`: anim_img, 10 options `B0`..`B9`, SMALL badge 160x40 at (40, 40).

6 roots, 100 request kinds (`request_kind_psc.txt`). PSC cuts k = 1..5; the heavy cut k = 4 gives ONE sub per segment shared by all
100 kinds. Badge size is set in the AnimateAF entity (like forecast `time`); the layer display is full frame, so the cost model
prices both badge layers the same (anim_frac = 1 each).

## Cost per layer
Segment 0, combo A3 B7, Dana direct mode on host om (gpu, serial, min of 3 reps). Cumulative = the same request rendered with count
1..6 roots; layer = difference to the row above. Two runs (laptop clock drift: the same full render took 6.6-10.6 s across the day),
so read the shares, not the exact seconds. Model = exported BBC cost model (unchanged), stat_predict/psc_ideal/features.csv rows.
Source: temp/psc_ideal/layer_cost.py -> temp/psc_ideal/out/layer_cost{_run1,}.{csv,txt}.

| root added (count k) | type | cumulative s (run 1 / run 2) | this layer s (run 1 / run 2) | model cumulative s | model layer s |
|---|---|---|---|---|---|
| 1 map (`map_colour`) | anim_img, full frame | 1.49 / 2.68 | 1.49 / 2.68 (mostly the per-segment encode, any first layer pays it) | 1.66 | 1.66 |
| 2 weather (`Meteorological`) | anim_img | 1.55 / 2.67 | ~0 | 1.66 | 0 |
| 3 presenter (`hearing`) + presenter_chroma | video + chroma key | 9.32 / 10.30 | **7.8 / 7.6** (decode ~1.0, render 6.3-7.0) | 8.42 | 6.75 |
| 4 time | anim_img 176x24 | 10.63 / 10.54 | 1.3 / 0.2 | 8.42 | 0 |
| 5 badge_a (BIG 480x120) | anim_img | 10.62 / 10.46 | ~0 | 8.42 | 0 |
| 6 badge_b (SMALL 160x40) | anim_img | 10.57 / 8.09 (noisy reps 8.1-10.4) | ~0 | 8.42 | 0 |

- Heavy prefix = roots 1-4 (cut k = 4): ~all of the cost, ~75 % of it is the presenter video + chroma key.
- The two badge layers are below the run noise (earlier test: forecast default 6.9 s vs forecast_psc 7.6 s -> ~+0.35 s per badge);
  big vs small badge cannot be told apart. The model also prices them at 0 (full-frame anim_img, size invisible).
- PSC on this show (temp/psc_ideal/ideal_test.py, seg 0 median): direct 7.6 s, pair +1.8 s, rebuild from the cached k = 4 sub 2.2 s
  -> saves ~5.1 s per reuse (details kb/partial_segment.md 21).

## Play it
```
# one combination, all 9 segments (renders ~70 s on om with Dana, then opens ffplay)
syn_video/PSC_ideal/play.sh A3 B7

# an already rendered file (raw H.264, no container -> give the format + frame rate)
ffplay -f h264 -framerate 25 temp/psc_ideal/out/play/A3B7_full.h264
ffplay -f h264 -framerate 25 temp/psc_ideal/out/A3B7_t0_direct.h264      # segment 0 from the test
```
play.sh needs the host bench temp/psc_ideal/obm (obm + project site overlay) and starts the asset server
(temp/psc_ideal/server_key.py www on 127.0.0.1:8080) if it is not running. The H2i sub files (*.sub.h2i) are not plain H.264.

## Build
```
dnc ../../obm/tools/AnimateAF.dn -o tools/AnimateAF.o; dnc ../../obm/tools/SegmentAF.dn -o tools/SegmentAF.o   # once
python3 build.py [--force]
```
Writes `assets/psc_ideal/{png,meta}` (badge PNGs, .af animations, 9 x 10 s segments each: 2250 frames because SegmentAF drops a
partial last segment), `shows/forecast_psc.json`, `request_kind_psc.txt`, `contact.png` (all badges). obm/ is not modified.

## Use
- Features / cuts (SEPARATE from the BBC files, which stay byte-identical):
  `python3 stat_predict/psc_cuts.py --kinds syn_video/PSC_ideal/request_kind_psc.txt --out stat_predict/psc_ideal/psc_cuts.csv --asset-root syn_video/PSC_ideal`
  `python3 stat_predict/export.py --kinds syn_video/PSC_ideal/request_kind_psc.txt --out-dir stat_predict/psc_ideal --asset-root syn_video/PSC_ideal`
- Local Dana test: `temp/psc_ideal/` (ideal_test.py, model_check.py, sim_calibrated.py; web root of symlinks in temp/psc_ideal/www).
- A cluster run would need the assets + show installed into the site web root and OBM_COST_FEATURES / OBM_PSC_CUTS pointing at
  the stat_predict/psc_ideal files (not done; user D98 = stop after the local test).
