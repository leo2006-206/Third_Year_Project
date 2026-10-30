# stat_predict: offload request cost model

Predicts the **work time** (decode + render + encode) of an OBM offload request on a site, from
**generic metadata only**: the show's layer graph, the options the request picks, each asset's meta
file and the segment file size. Show and layer names are never used, so it applies to unseen shows.
Network, asset and queue time are left to the load balancer.

```
python3 stat_predict/evaluate.py      # dataset, all test splits, models -> model_all.json, model_all_runs.json
python3 stat_predict/export.py        # model_all_runs.json -> coef.csv + features.csv for the Rust load balancer
```

| File | Contents |
|---|---|
| `features.py` | request → feature vector; full and **partial** offload (features of the first N root nodes only) |
| `model.py` | `CostModel`: fit (non-negative least squares), predict, explain, save/load (JSON) |
| `evaluate.py` | dataset from `stat/*_timing.csv` (BBC calibration) + `stat/syn_timings.csv` (synthetic shows), 6 splits, baseline, final model |
| `model_all.json` | model fitted on calibration (BBC + synthetic), ~2 KB |
| `model_all_runs.json` | model fitted on calibration + replay requests (recommended for the load balancer) |
| `evaluation_report.txt` | full output of the last `evaluate.py` run |
| `export.py` | writes `coef.csv` (one row per stage model) and `features.csv` (features of 1,362 request paths: every BBC / synthetic kind × every segment, plus all paths in sequences and measurements) |
| `coef.csv`, `features.csv` | read by `rust_obm/src/cost_predict.rs` |

## Rust predictor (`rust_obm/src/cost_predict.rs`)

```rust
let p = CostPredictor::load("stat_predict/coef.csv", "stat_predict/features.csv")?;  // checks the files
let work_s: Option<f64> = p.predict(path, &site);   // site.config gives the decode / render / encode backends
```

- Returns **machine-neutral seconds** (om); the machine factor, network, asset and queue time are the
  load balancer's job. `None` if the path has no features or the site's backends have no model.
- Checks on load: numbers finite, coefficients ≥ 0, every model feature present in `features.csv`, a
  model for every stage, `seg_frac` in [0, 1], same column count on every row, no duplicate path.
- Paths are matched as given or percent-decoded (`%7C` → `|`, `%20` → space).
- Features are precomputed for the experiments; in a real deployment they would be extracted when an
  unseen request arrives (`features.py` logic).
- Same predictions as Python (forecast on om: gpu 8.4153 s, cpu 31.949 s in both).

## Model

```
work(request, site) = machine_factor[machine]
                      × ( decode[decode backend](x) + render[render backend](x) + encode[render+encode backend](x) )

each stage model:  t = a + Σ β_i x_i ,   a, β ≥ 0
```

- **One small linear model per (stage, backend).** `a` = fixed cost per task on that backend, `β` =
  cost per unit of a feature. Fixed cost + rate also captures backends that are slow for small tasks
  but fast for big ones. A new backend is profiled and fitted alone; the others are unchanged.
- **Encode is keyed by the render + encode backend pair**: Dana's encode timer includes reading the
  frame back from the GPU, so encode is cheaper when rendering on the CPU.
- **The machine factor is outside the model**: one number per machine (here the median work ratio
  to om on the same requests; in the load balancer, learned from feedback).
- Fitted on **relative error** (rows weighted by 1 / time), so cheap and expensive requests count alike.
- Lookup of measured kinds is not in this code: this is the fallback model for unseen requests.

| Stage | Features |
|---|---|
| decode | `n_video` (streams), `dec_mpix` (decoded megapixels / frame), `dec_mbps` (source bitrate) |
| render | `n_layers`, `disp_frac` (displayed area, in screens), `anim_frac`, `scaled_frac` (area of videos drawn at another size), `chroma_frac`, `alpha_frac` (area through a chroma key / alpha mask), `n_reframe` |
| encode | `video_frac` (screen area of moving video), `anim_frac` |
| (scale) | `seg_frac` = frames in this segment / 250: the model predicts a full segment and scales by it (the last segment of a show is often shorter; without this, those requests distorted the fit) |

`out_mpix` is computed but not used: it is the same (0.92 MP) in all data.

## Results (2026-10-30, `evaluate.py`)

Data: 758 measurements, 172 request kinds; 6 BBC shows (596 rows, 93 % f1/f1_full) + 5 synthetic
shows (162 rows); backends gpu / llvmpipe / cpu, machines om / rs. Error of predicted total work;
baseline = mean work of the same backend in training (no features).

| Split | MAPE | median APE | within 20 % | baseline MAPE |
|---|---|---|---|---|
| A random 80/20 of request kinds (mean of 20 seeds) | **12.4 %** | 11.0 % | 86 % | 27.8 % |
| E train BBC → test all synthetic shows | 32.4 % (gpu 16, llvm 11, **cpu 70**) | 16.5 % | 56 % | 72.8 % |
| F train synthetic → test all BBC shows | 18.0 % (gpu 20, llvm 15, cpu 34) | 18.2 % | 57 % | 26.3 % |

Leave one show out (that show unseen in training):

| Unseen show | train BBC only | train BBC + synthetic |
|---|---|---|
| f1 / f1_full | 15.6 / 11.8 % | 15.7 / 13.1 % |
| **forecast** (chroma) | **50.2 %** | **16.6 %** |
| forest720 / forest720_leaves | 8.5 / 16.8 % | 6.6 / 19.0 % |
| spiders | 14.9 % | 14.4 % |
| syn_chroma_area / combo / content / res | – | 9.4 / 8.5 / 6.5 / 9.3 % |
| syn_grid (9 streams) | – | 22.2 % |

Reading:

- With features the model is 2–3× better than the no-feature baseline on every split.
- **The synthetic shows are needed for generalisation.** In the BBC data chroma only appears in
  forecast, so with forecast held out the model has never seen chroma (50 % error). With the
  synthetic shows it learns chroma from other content and forecast drops to 17 %.
- **The cpu backend is under-measured**: only 8 BBC kinds, so BBC-trained predictions for it are poor
  (70 % on synthetic). The synthetic data trains it.
- Synthetic-only training predicts the BBC shows within 18 % (gpu/llvm 15–20 %): the generic features
  carry across content.
- Weakest spot: many video streams (`syn_grid`, 22 %).

Final coefficients (all data, seconds; machine factor rs = 0.75 × om):

| Stage model | a | notable slopes |
|---|---|---|
| decode, gpu | 0.02 | 0.46 / decoded MP, 0.12 / stream |
| render, gpu | 0.06 | chroma 4.1 / screen, alpha 2.0 / screen, scaled 0.15 / screen |
| render, llvmpipe | 1.01 | chroma 3.7, alpha 1.2, scaled 0.38, anim 0.18 / screen |
| render, cpu | 5.47 | scaled 5.7, chroma 3.4, alpha 5.3, displayed 1.3, anim 1.2 / screen |
| encode, gpu+cpu | 1.72 | 0.43 × video area |
| encode, cpu+cpu | 0.90 | 0.23 × video area (no GPU readback) |

These match the measurements: chroma costs a similar fixed ~3–4 s per screen on every backend
(CPU readback), scaling is almost free on the GPU but expensive in software, and encode is cheaper on
the cpu backend.

## In-situ replay data (`stat/run_*`)

5,559 requests measured inside the busy cluster during the replays (gpu 3,081, llvm 1,682, cpu 796),
every segment, stage times from the same Dana timers as calibration.

| Test | Result |
|---|---|
| G: calibration model → every replay request (never seen) | MAPE 15.4 %, median 12.5 %, 72 % within 20 %; unbiased (prediction / actual median 1.00, same under heavy and light load). cpu 26.6 % |
| J: unseen show, test = its replay requests: calibration only → + other shows' replays | MAPE 18.4 → **15.8 %**, within 20 % 63 → **72 %**; cpu 26.4 → 21.7 % (within 20 % 42 → 63 %), gpu 16.4 → 13.1 %, llvm unchanged |
| I: train BBC + replays → test synthetic shows | MAPE 32.4 → **22.5 %**; cpu 69.9 → 41.2 % (median 55 → 11 %) |
| H: same as C but test = idle calibration rows of the unseen show | 2–6 points worse with replays: calibration measured identical requests 10–20 % slower than the cluster (cpu 0.80, gpu 0.89, llvm 0.93 replay / calibration) |

Reading: replay data fixes the under-measured cpu backend and makes the model fit the cluster it will
be used in. The level difference to idle calibration is the kind of shift the machine factor /
load-balancer feedback absorbs. Use `model_all_runs.json` in the load balancer. Note its cpu chroma
coefficient (15.7 s / screen) is much larger than calibration's (3.4): forecast on cpu sites under
load is far slower than idle.

## Partial offload

`features()` keeps only the layers inside the first N root nodes (N = the number after the
orientation in the request). Example, forecast on om gpu: full (4 roots) 7.6 s, of which chroma
4.1 s; first 2 roots (map + weather only) 1.8 s. **Not yet validated against measurements**: no
partial-offload requests have been measured; that needs a calibration run with smaller N.

## Limitations

- Calibration data is idle, single run per kind and site, segment 0 only.
- Decode backend is always the GPU in the data; a cpu-decode model needs its own profiling.
- Occlusion (hidden layers still cost decode but little encode) and animated-image complexity are
  not features.
- The machine factor is one number per machine; rs/om differs slightly by backend (0.70–0.75).
