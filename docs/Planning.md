## Object-Based Media Load Balancing ##

Key Question:
* Why is this not just naive load balancing?

Naive load balancing assumes that **latency is relative to network distance**.

But OBM could break this assumption,<br>
because **latency is also affected by local compute and/or each offload site to generate the requested video.**

Like:
$$
T_\text{total} = T_\text{network} + T_\text{queue} + T_\text{assert acquisition} +  T_\text{render} + T_\text{encode} + T_\text{return}
$$

## Approaches ##
* Reactive vs Pro-active
* Offloading and rendering scheduler (full/partial offloading, cache re-use, server utilization)
* Caching of final rendered products, intermediate rendered layers and raw object layers
* Prefetching objects, Predicting requests, Pre-rendering popular combination
* Potential parallel processing of independent layers
* Reducing the object data movement (different codec or better scheduler)

## Evaluation ##
First, perform **small-scale experiments** which provide representative compute costs grounded in reality.

Collect data points from the experiments.

Then, scale this up using a simulator and plugging in those data points.

Key:
* User request patterns
* Network characteristics
* //Or other things

## Metrics ##
* key -> Server utilisation
* key -> Client-perceived latency
* Throughput
* 95th-percentile latency
* Queue time
* Cache hit rate
* Amount of asset transfer
* ...

## Planning ##

### Stage 1 ###

|||
|:-:|:-:|
|Start| 01/08/2026|
|End| |

Setting on the existing system from the paper

Deploying the system with real domains:
* docker container, for adjusting container resources
* cloudflare tunnel, for real networking with https
* network emulator, for more realistic networking conditions

Planning URLs include:
* `obm_main_server`, for the main obm server
* `obm_offload_server_1`, for offloading server 1
* `obm_offload_server_2`, for offloading server 2
* `obm_client`, for user interface

Usage:
* `main server`, store the assets, also be the load-balancing server to select offloading server
* `offloading server`, computing server to offload rendering for user
* `client`, new user interface to select video with options and display video, user should only use this URL

Detail:
* `offloading server` actually is a rust server process with a dana offload process, where rust handle request and dana rendering the segments. They communicating over localhost http.
* `evaluation client (client_testing)`: A lightweight, static HTML/JS test harness served directly by the Rust server for both human and automated bot evaluation:
  * Reads `show_options.json` listing all available shows, variants, and layer options.
  * Standard dropdown selectors (`<select>`) allowing users and bots to switch video shows and layer options dynamically.
  * Headless toggle switch to disable video decoding/canvas display for lightweight, high-concurrency bot benchmarks.
  * Metric recording & CSV export: tracks request dispatch time, network latency, and for non-offload mode, records the exact timestamp of each layer element received versus the timestamp when the client finishes rendering the segment/frame.
  * *(TODO: In-browser automation mode for scheduled/randomized option switching)*.

***

### Stage 2 ###

|||
|:-:|:-:|
|Start| |
|End| |

Implement the basic load-balancing server
* forwarding request
* logging for basic info
* implement baseline load-balancing policy:
	* No policy (send all the components to user)
	* round robin
	* lowest-network latency
	* least-loaded (shortest queuing)

Collect data point from the experiments

Setting basic simulator