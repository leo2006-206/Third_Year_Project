# OBM project knowledge base (Claude's working notes; repo root kb/ (moved from temp/kb 11-07, temp/kb = symlink))
Query: `grep -rn -i "<term>" kb/` ; each file starts with a 1-line summary.
Update rule: add a line to history.md for every user request; after any finding / decision / run / user correction, edit the topic file
(replace stale facts, don't append contradictions) and bump "updated" date in it.
ALSO: any change to an LB policy or feature (code, env var, run done, finding, limit, status) -> update its card in lb_features.md in the same step; a NEW policy / feature -> new card. A user decision -> new row in decisions.md.
HANDOFF = KB AUDIT first (any session length): for each thing done this session check history.md, topic file, decisions.md, lb_features.md card, run_table.md, env.md, todo.md, plot.md; fix gaps, tell the user what was missing.

| file | what |
|---|---|
| lb_features.md | ONE CARD PER LB POLICY + FEATURE (how it works, env vars, code, data, tests, findings, analysis tools, limits) + analysis tools index — start here for any policy / feature question |
| decisions.md | every USER DECISION, numbered D01.. (date, decision, why, source); never deleted, replaced rows marked — check before re-proposing something |
| map.md | WHERE things are in the repo (files, dates, staleness) — start here for lookups |
| cluster.md | two-laptop cluster: machines, IPs, netem, ssh, how to start/stop a case |
| experiments.md | workload params, run procedure, overnight driver, run log + results table |
| batch_run.md | HOW to run a replay batch: script template, pre-flight, watchdog + cron prompt, incidents, past batches — read before any new batch |
| env.md | every env var that changes an experiment (main server OBM_*, driver FEATURES/SEQ_DIR/..., site) + how it reaches the container — add a row for every new var |
| findings.md | analysis conclusions (rs slowness, iwrr vs rr, calibration) |
| code.md | main server / cache / http lib design + decisions the user made |
| user.md | how the user wants me to work (commits, rs, teaching mode) |
| plot.md | how the user wants plots (colours, layout, counts) + every plot program, input, output |
| todo.md | open / offered tasks |
| history.md | dated one-line index of requests and outcomes (when / why things changed) |
| run_table.md | WHICH runs are done / planned (part 1 LB policies x 5 seeds, part 2 mct + one feature, part 3 later search) + pointer to each data folder; update a cell when a run finishes |
| prediction.md | prefetch / prediction feature design notes (workload facts, real-world designs, design axes) |
| handoff.md | end-of-session handoffs, newest first: current state, next steps, gotchas — READ FIRST when resuming |
