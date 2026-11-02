
the same two traces, time-scaled to ρ = 0.7 of each cluster's measured capacity, replayed open-loop

"@inproceedings{open_vs_close,
author = {Schroeder, Bianca and Wierman, Adam and Harchol-Balter, Mor},
year = {2006},
month = {05},
pages = {},
title = {Open Versus Closed: A Cautionary Tale.},
volume = {3},
journal = {Proceedings of the 3rd Conference on Networked Systems Design & Implementation}
}"

most cloud load-balancing research is evaluated in simulation (CloudSim / CloudAnalyst); few algorithms are tested in a real environment; review focus is static, dynamic, hybrid and nature-inspired (ACO, PSO) algorithms

"@article{lohumi2023lbreview,
author = {Lohumi, Yogesh and Gangodkar, Durgaprasad and Srivastava, Prakash and Khan, Mohammad Zubair and Alahmadi, Abdulrahman and Alahmadi, Ahmed H.},
year = {2023},
title = {Load Balancing in Cloud Environment: A State-of-the-Art Review},
journal = {IEEE Access},
volume = {11},
pages = {134517--134530},
doi = {10.1109/ACCESS.2023.3337146}
}"

review of static, dynamic and nature-inspired LB (Round Robin, Throttled, ESCE, Min-Min, ACO); of the reviewed articles 22 use CloudSim and 15 CloudAnalyst (Fig. 23); metrics are mainly response time and makespan

"@article{shafiq2022lbreview,
author = {Shafiq, Dalia Abdulkareem and Jhanjhi, N. Z. and Abdullah, Azween},
year = {2022},
title = {Load balancing techniques in cloud computing environment: A review},
journal = {Journal of King Saud University - Computer and Information Sciences},
volume = {34},
number = {7},
pages = {3910--3933},
doi = {10.1016/j.jksuci.2021.02.007}
}"

Throttled and Equally Spread Current Execution (ESCE) are VM load-balancing policies of the CloudAnalyst simulator (CloudSim based) [details from memory: check against the paper]

"@inproceedings{wickremasinghe2010cloudanalyst,
author = {Wickremasinghe, Bhathiya and Calheiros, Rodrigo N. and Buyya, Rajkumar},
year = {2010},
title = {CloudAnalyst: A CloudSim-Based Visual Modeller for Analysing Cloud Computing Environments and Applications},
booktitle = {24th IEEE International Conference on Advanced Information Networking and Applications (AINA)},
pages = {446--452},
doi = {10.1109/AINA.2010.32}
}"

CloudSim: the simulator most cloud LB papers are evaluated in (tasks with a known length in MI, VMs with a known MIPS rating) [details from memory: check]

"@article{calheiros2011cloudsim,
author = {Calheiros, Rodrigo N. and Ranjan, Rajiv and Beloglazov, Anton and De Rose, C{\'e}sar A. F. and Buyya, Rajkumar},
year = {2011},
title = {CloudSim: a toolkit for modeling and simulation of cloud computing environments and evaluation of resource provisioning algorithms},
journal = {Software: Practice and Experience},
volume = {41},
number = {1},
pages = {23--50},
doi = {10.1002/spe.995}
}"

MCT (Minimum Completion Time) and Min-Min as mapping heuristics for independent tasks on heterogeneous machines: the origin of our MCT policy [details from memory: check]

"@article{braun2001eleven,
author = {Braun, Tracy D. and Siegel, Howard Jay and Beck, Noah and B{\"o}l{\"o}ni, Ladislau L. and Maheswaran, Muthucumaru and Reuther, Albert I. and Robertson, James P. and Theys, Mitchell D. and Yao, Bin and Hensgen, Debra and Freund, Richard F.},
year = {2001},
title = {A Comparison of Eleven Static Heuristics for Mapping a Class of Independent Tasks onto Heterogeneous Distributed Computing Systems},
journal = {Journal of Parallel and Distributed Computing},
volume = {61},
number = {6},
pages = {810--837},
doi = {10.1006/jpdc.2000.1714}
}"

power of two choices: sample two servers, send to the less loaded one; the basis of Envoy's least-request balancer and NGINX random two [details from memory: check]

"@article{mitzenmacher2001power,
author = {Mitzenmacher, Michael},
year = {2001},
title = {The Power of Two Choices in Randomized Load Balancing},
journal = {IEEE Transactions on Parallel and Distributed Systems},
volume = {12},
number = {10},
pages = {1094--1104},
doi = {10.1109/71.963420}
}"

in production, balance latency and requests-in-flight, not CPU load: Prequal (probing + power of d choices) runs at YouTube, lowers tail latency and errors at higher utilisation; supports our estimate-of-finish (MCT) over load counts

"@inproceedings{wydrowski2024prequal,
author = {Wydrowski, Bartek and Kleinberg, Robert and Rumble, Stephen M. and Archer, Aaron},
year = {2024},
title = {Load is not what you should balance: Introducing Prequal},
booktitle = {21st USENIX Symposium on Networked Systems Design and Implementation (NSDI 24)},
pages = {1285--1299},
url = {https://www.usenix.org/conference/nsdi24/presentation/wydrowski}
}"

what production load balancers offer (practice): AWS ALB round robin (default), least outstanding requests, weighted random; NGINX round robin, least_conn, least_time, random two; Envoy least request = power of two choices; Kubernetes Gateway API Inference Extension routes on queue depth and KV-cache use

"@misc{aws_alb_algorithms,
author = {{Amazon Web Services}},
title = {Edit target group attributes for your Application Load Balancer: routing algorithm},
howpublished = {\url{https://docs.aws.amazon.com/elasticloadbalancing/latest/application/edit-target-group-attributes.html}},
note = {Accessed 2026-11-01}
}"

"@misc{nginx_lb,
author = {{NGINX}},
title = {HTTP Load Balancing},
howpublished = {\url{https://docs.nginx.com/nginx/admin-guide/load-balancer/http-load-balancer/}},
note = {Accessed 2026-11-01}
}"

"@misc{envoy_lb,
author = {{Envoy Project}},
title = {Supported load balancers},
howpublished = {\url{https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/load_balancing/load_balancers}},
note = {Accessed 2026-11-01}
}"

"@misc{k8s_inference_ext,
author = {{Kubernetes SIG Network}},
title = {Gateway API Inference Extension},
howpublished = {\url{https://gateway-api-inference-extension.sigs.k8s.io/}},
note = {Accessed 2026-11-01}
}"
