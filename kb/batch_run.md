How to run a replay batch (overnight cluster runs): template, checklist, watchdog, past incidents. updated 2026-11-03
Read this before writing a new temp/batch_*.sh. New scripts are fine, just keep the same shape and rules.

## Pieces (all run from the repo root on om)
| file | role |
|---|---|
| temp/batch_<name>.sh | the batch: loops jobs, calls run_driver.sh, logs to temp/logs/batch_<name>.log |
| temp/run_driver.sh `<case> "<alphas>" "<policies>"` | per (alpha, policy): fresh sites on om + rs, fresh main server, warm-up, seq_bench replay, saves `_main.log` + `_site_<id>.log`, runs temp/lb_log_check.py ([check] lines), OOM report. Log temp/logs/driver_MMDD_HHMM.log. Env SEQ_DIR (default servers_benchmark/sequences), RUN_ROOT (default stat) for other sequence sets |
| temp/cluster_lib.sh | start_case / start_case_retry (10 min) / stop_all / warm_up / save_site_logs / oom_report (sourced by the driver) |
| servers_benchmark/seq_bench.py | open-loop replay -> `<RUN_ROOT>/run_a<α>_c<case>/<label>_none_<N>.csv`, N = 1 + highest existing (so listing a policy twice = 2 runs) |
| temp/watchdog.sh | one status report; `PROBLEM:` lines + exit 1 when something needs action. Follows the newest temp/logs/batch_*.log (override BATCH_LOG=) |
| temp/lb_log_check.py | decision sanity check per run (mct argmin, aco p/tau, throttled in-flight <= LIMIT, no overtaking) |
| temp/logs/replay_current.log | the running seq_bench output (progress lines `[ 14/303] # 14 200 5304ms worker=...`) |

## Batch script template (copy of the recent style: batch_repeat.sh / batch_seed2042.sh)
```bash
#!/bin/bash
# <Name> batch (<date>): <what and why>. <N> runs, ~<h> h. Usage: temp/batch_<name>.sh
# (background; log temp/logs/batch_<name>.log). Needs rs sites (ask the user first).
cd "$(dirname "$0")/.." || exit 1
B=temp/logs/batch_<name>.log
# export SEQ_DIR=servers_benchmark/sequences/seed2042 RUN_ROOT=stat/seed2042   # only for another sequence set
echo "[$(date '+%m-%d %H:%M:%S')] BATCH START $(git log --oneline -1)" >> "$B"
for job in "1|0.5|mct esce" "1|1.2|mct esce" "2|0.5|mct esce" "2|1.2|mct esce"; do   # case|alpha|policies
  IFS='|' read -r c a p <<< "$job"
  echo "[$(date '+%m-%d %H:%M:%S')] driver case $c alpha $a policies '$p'" >> "$B"
  temp/run_driver.sh "$c" "$a" "$p"
  echo "[$(date '+%m-%d %H:%M:%S')] driver exit $?" >> "$B"
done
echo "[$(date '+%m-%d %H:%M:%S')] BATCH DONE" >> "$B"
```
- "BATCH START" / "BATCH DONE" lines are what the watchdog and the cron prompt look for: keep them.
- Repeats: list a policy twice in one job, or loop rounds OUTER (batch_seed2042.sh) so a stopped batch still has full rounds.
- Policy names: iwrr is saved as iwrr331 (driver label()). All policies: rr iwrr esce mct mct_lookup mct_const mct_const_flat aco aco_const throttled throttled2. iwrr is c1 only.
- Start detached: `nohup temp/batch_<name>.sh > /dev/null 2>&1 &` (Bash run_in_background also fine). Then set up the watchdog cron BEFORE telling the user it is running.

## Timing (for ETA and batch size)
- ~11-12 min per run incl. site start + warm-up (set 2: 46 runs 02:45-11:23 = 8.6 h; 30 runs ~6 h).
- rr on c1 is the slow one (~19 min: queues of 200+ s drain after the window; wall 1135 s). User dropped rr c1 from the repeat batch.
- seed 2042 batch: 40 runs in 8.0 h (12 min/run).
- c2 runs are slightly shorter (4 sites, ~290 requests).

## Pre-flight checklist (before starting)
1. User OK to use rs (rule: ask before changing anything on rs; starting sites there counts).
2. Code the batch needs is committed AND rs has it: rs builds the offload image from its own repo copy ~/SCC/Third_Year_Project on every start_case. The user pushes/pulls; verify with `ssh rs 'cd ~/SCC/Third_Year_Project && git log --oneline -1'` = om HEAD for offload_server code.
3. Main server image current: it COPYs target/release/main_server -> `cargo build --release --bin main_server` BEFORE `docker build` (gotcha 10-30: stale binary -> "Unknown policy mct_const"). Check the policy name is in mod.rs POLICIES and main_server.rs.
4. No leftover containers: `docker ps --filter name=obm-` on om and `ssh rs docker ps --filter name=obm-`.
5. Disk (> 5 GB) and om memory (Chrome / rust-analyzer can eat it; 10-30 start had only 3.5 GB free). Ask the user not to use Chrome on om during the batch (om sites + iGPU interference).
6. Smoke test for any NEW policy / code: temp/smoke/run.sh `<policy> [N=120] [case=1] [alpha=1.2]`, then lb_log_check on it.
7. Sequences exist at SEQ_DIR (CRLF files: `tr -d '\r'` if paths are used in shell).

## During the batch: rules
- Do NOT rebuild the main image, and do NOT edit offload_server code (each run rebuilds the offload image via run_all_offload.py; rs from its own repo). Analysis / KB / temp edits are fine.
- Never `pkill -f` / `pgrep -f` with a pattern that is in my own command line (kills my shell). Kill by PID (ps + awk on exact argv fields) or `[x]yz` patterns. Watchdog matches exact argv for the same reason.
- Remote monitor loops (`ssh rs 'while :; do docker stats ...'`) survive killing the local ssh -> rs ran hot ~40 min (10-30). Kill them by name; check `ssh rs 'ps -eo args | grep "[d]ocker stats"'`. The user's own interactive `docker stats` in a terminal: leave it.

## Watchdog + 30-min cron (standing rule: long run = user asleep)
- Memory rule long-run-watchdog: start watchdog + recurring check every 30 min, unprompted. CronCreate at off-minutes (e.g. :13/:43 or :17/:47), session-only; delete the cron when BATCH DONE.
- Cron prompt used (adapt names):
  "Overnight batch watchdog: run temp/watchdog.sh. If BATCH DONE in temp/logs/batch_<name>.log: verify no obm containers on om/rs, count failures + Dana resends ('Dana gave no response' in site logs), check all [check] lines, summarise per (case, alpha, policy) mean/p50/p90/p99, update KB (experiments, findings, todo, handoff, history), delete this cron, report to the user; data stays uncommitted. Else: act on PROBLEM lines (fix only tiny issues; otherwise stop and mark the run _e), append one 'watchdog HH:MM ...' line to experiments.md (progress, failures, temps, finished runs' mean/p50/p90/p99)."
- Watchdog checks: driver + seq_bench alive, failed requests in the current replay, stall >= 20 min without a finished request, site count per case on om + rs (c1 3+3, c2 2+2), main server up, OOM / restarts, CPU pkg >= 95 C (median of 3 samples), rs acpitz >= 95 C (critical trip 100 = shutdown), NVIDIA >= 85 C, om available memory < 800 MB, disk < 5 GB, live decision check on the running main server, [check] PROBLEM in the driver log.
- KB line per check, e.g. `- watchdog 05:54 ok: c1 a1.2 rr 132/303, 0 fail; om 66 C, rs acpitz 79 C. Done: <runs with ok/fail mean/p50/p90/p99>. [check] ALL OK.`

## When something goes wrong
- Bad run: rename its files with `_e` (`mct_none_1_e.csv` + `_main.log` + `_site_*.log`); the plot scripts (stat/latency_plot.py, latency_detail_plot.py) skip `_e` files. Note it in experiments.md and tell the user.
- Single failed request: note it (request idx, site, error), don't mark the run.
- Stop a batch: kill batch script, driver, seq_bench BY PID; `stop_all` (source temp/cluster_lib.sh) or docker rm -f on om + `ssh rs` docker rm -f; delete partial CSVs of the interrupted run; delete the cron. Then restart with the finished jobs removed from the list.
- After the batch: no obm containers on both machines, cron deleted, summary + handoff written, data uncommitted until the user asks.

## Incident log (what happened, what we changed)
- 11-03 seed 2042: rr on c1 hits the main server's 600 s timeout (HTTP 504, every 6th req = cpu-render site's rr slot): overload, not a fault; run kept, rr tail stats are censored. Also fixed clipped plot layouts (tight_layout gave up when rank badges sat above axes -> subplots_adjust).
- 10-29 23:15 batch stopped by the user mid-run to free machines (a1.2 iwrr 3 min in): no partial file kept; missing runs listed in experiments.md and run later (batch_v3_rest).
- 10-30 rs overheating during a batch: acpitz 85-89 C, heavy throttling; caused partly by a leftover remote `docker stats` loop. Fixes: kill remote loops by name, rs power profile balanced (om performance), watchdog acpitz >= 95 alert. rs work times may be throttle-affected (machine-factor confound).
- 10-30 stale main_server binary in the image -> "Unknown policy". Fix: cargo build before docker build (pre-flight 3).
- 10-30 om low memory at start (Chrome ~5 GB, swap full): closed Chrome; keep 2 GB per-site caps.
- 10-31 watchdog false alarms: om CPU 97 C one-second spikes -> median of 3 samples; aco p-sum rounding -> lb_log_check tolerance 0.035.
- 11-01 set 2 batch: 1 failed request (502) = Dana recycle race (Dana quits after 10 handled requests when its queue is empty; the next job hits a reset). Fixed 0e5ded6: offload_server resends when Dana closed without any response bytes (MAX_RESENDS 20). Since then 0 failures / 0 resends in 46 runs.
- 11-01 throttled checker false alarm (site queue 2.18 s = previous reply still finishing) -> threshold 2 -> 3 s; 11-02 site-queue > 3 s is only a note when in-flight <= LIMIT.
- 11-02 01:59 repeat batch started, then the user asked to fix the Dana race first: stopped by PID at 02:2x, kept the complete iwrr #2, removed the partial esce run, committed the fix, user pushed + rs pulled (verified HEAD on rs), restarted 02:45 with that job removed. Done 11:23, 46 runs 0 failures.

- 11-05 05:33 seeds3 (rs journal): 05:31:44 rs disconnected from both eduroam APs (signal was strong, -41 dBm), 05:32:26 'ssid-not-found' = eduroam not visible at all (network side, likely early-morning AP outage, not rs), back 05:40:19 (~8.5 min). rs eduroam dropped -> rs docker build cannot resolve auth.docker.io (FROM metadata needs registry even when cached) -> 'rs sites FAILED to start' -> driver skips, batch races through all remaining jobs to BATCH DONE in 3 min. Fix: resume script with a DNS guard (wait for `ssh rs getent hosts auth.docker.io`) before each single-policy driver call (temp/batch_seeds3_resume.sh). Lesson: a fast BATCH DONE = skipped runs, count CSVs. FIX 11-05 (user): retry built into the driver - temp/cluster_lib.sh start_case_retry (start_case retried every 60 s up to START_RETRY_S=600 s, then skip as before), run_driver.sh uses it; stub-tested. Every batch gets it for free. Resume-style per-policy DNS-guard script (temp/batch_seeds3_resume.sh) = OPTIONAL, only to re-run a list of already-skipped runs. Watchdog will still show 'rs sites FAILED to start' PROBLEM lines while retrying -> look for 'start attempt N failed' / 'started on attempt N' / 'giving up' in the driver log before acting.

## Past batches (most recent first)
| script | when | what | runs | result |
|---|---|---|---|---|
| batch_seeds3.sh + _resume.sh | 11-05 00:14-05:33, resume 05:56- | seeds 2043/2044/2045 x (mct flat throttled2 esce all + rr c2), 1 round -> stat/seedS/ | 54 (30 + 24 resume, done 10:14) | rs wifi/DNS drop 05:33 skipped 24 -> resume; 0 fail, 1 recovered resend, checks 54/54 OK |
| batch_seed2042.sh | 11-03 01:47-09:45 | seed 2042 set: mct mct_const_flat throttled2 esce rr x 4 seqs x 2 rounds -> stat/seed2042/ | 40 (8 h) | 0 fail except rr c1 22x 504 (600 s timeout, overload), checks OK |
| batch_repeat.sh | 11-02 01:59 / restart 02:45-11:23 | set 2: every (case, alpha, policy) +1, never-run +2, rr c1 dropped | 46 | 0 fail, checks OK |
| batch_throttled2.sh | 11-01 15:18 | throttled2 x 4 seqs | 4 | ok |
| batch_set2.sh | 11-01 02:53-08:50 | set 2 full (8 policies x 4 seqs, iwrr c1 only) | 30 | 1 fail (Dana race) |
| batch_set2_esce.sh | 10-31 | esce on set 2 (first look) | 4 | ok |
| batch_fifo1.sh | 10-31 04:17-09:58 | FIFO offload era, 8 policies x set 1 | 30 | 0 fail, checks 30/30 |
| batch_v4.sh | 10-30 14:05-16:15 | mct_const + c2 at rho 0.88 | 10 | 0 fail |
| batch_v3_rest.sh | 10-30 05:12-07:03 | missing v3 runs | 10 | 0 fail |
Older data (pre-FIFO gate, set 1) lives in stat/old_nonfifo/ and stat/set1_unload/; current set 2 in stat/run_a*_c*/.
- [11-05] Feature runs: set OBM_* vars + FEATURES in the driver shell (see env.md); cluster_lib forwards all OBM_* into the main container and logs 'main server env:'; driver refuses cache var with FEATURES=none. Pre-flight for feature runs: rebuild main image first, check main log shows 'Segment cache ON'.
