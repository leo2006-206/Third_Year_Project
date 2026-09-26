## Object-Based Media Load Balancing

Key Question:

- Why is this not just naive load balancing?

Naive load balancing assumes that **latency is relative to network distance**.

But OBM could break this assumption,<br>
because **latency is also affected by local compute and/or each offload site to generate the requested video.**

Like:
$$
T\_\\text{total} = T\_\\text{network} + T\_\\text{queue} + T\_\\text{assert acquisition} + T\_\\text{render} + T\_\\text{encode} + T\_\\text{return}
$$

## Approaches

- Reactive vs Pro-active
- Offloading and rendering scheduler (full/partial offloading, cache re-use, server utilization)
- Caching of final rendered products, intermediate rendered layers and raw object layers
- Prefetching objects, Predicting requests, Pre-rendering popular combination
- Potential parallel processing of independent layers
- Reducing the object data movement (different codec or better scheduler)

## Evaluation

First, perform **small-scale experiments** which provide representative compute costs grounded in reality.

Collect data points from the experiments.

Then, scale this up using a simulator and plugging in those data points.

Key:

- User request patterns
- Network characteristics
- //Or other things

## Metrics

- key -> Server utilisation
- key -> Client-perceived latency
- Throughput
- 95th-percentile latency
- Queue time
- Cache hit rate
- Amount of asset transfer
- ...

## Planning

### Stage 1

|||
|:-:|:-:|
|Start| 01/08/2026|
|End| 26/09/2026|

Setting on the existing system from the paper

Deploying the system with real domains:

- docker container, for adjusting container resources
- cloudflare tunnel, for real networking with https
- network emulator, for more realistic networking conditions

Planning URLs include:

- `obm_main_server`, for the main obm server
- `obm_offload_server_1`, for offloading server 1
- `obm_offload_server_2`, for offloading server 2
- `obm_client`, for user interface

Usage:

- `main server`, store the assets, also be the load-balancing server to select offloading server
- `offloading server`, computing server to offload rendering for user
- `testing page`, user interface to configure show options and run evaluation benchmarks, user should only use this URL

Detail:

- `offloading server`: actually is a rust server process with a dana offload process, where rust handle request and dana rendering the segments. They communicating over localhost http.
- `testing page`: A lightweight, static HTML/JS that able to select video combinations and record the send and receive time with size.

***

### Stage 2

|||
|:-:|:-:|
|Start| 26/09/2026|
|End| |

#### Goals

- Implement the basic load-balancing server:
  - Forwarding request
  - Logging for basic info
  - Implement baseline load-balancing policy:
    - Round robin
    - Lowest-network latency
    - Least-loaded (shortest queuing)
- Collect data point from the experiments

> [!NOTE]
> **Status (as of 26/09/2026):** Rust main server and offload nodes are fully containerized in Docker (`obm-net`) with GPU VA-API and CPU support. Server-side rendering offload and reverse proxy streaming are working end-to-end, currently dispatching requests without dynamic policy. Webpage benchmark harness records request latency and segment sizes.
