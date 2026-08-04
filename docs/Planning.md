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
* Server utilisation
* Client-perceived latency
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

Implement a basic user app if needed, dispaly basic timing

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