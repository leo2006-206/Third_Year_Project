# Graph Compression #

Traversing Large Compressed Graphs on GPUs (Paper):
* https://ieeexplore.ieee.org/document/10177491/references#references

Compression for large graph (Phd thesis):
* https://www.lucaversari.it/phd/main.pdf

Accelerating Loading WebGraphs in ParaGrapher (Paper):
* https://www.researchgate.net/publication/393260573_Accelerating_Loading_WebGraphs_in_ParaGrapher

Graph compression lecture (Pdf):
* https://www.cs.cmu.edu/afs/cs/project/pscico-guyb/realworld/www/slidesS18/compression6.pdf

CompressGraph: Efficient Parallel Graph Analytics with Rule-Based Compression (Paper):
* https://dl.acm.org/doi/10.1145/3588684

Smaller and Faster: Parallel Processing of Compressed Graphs with Ligra+ (Paper):
* https://www.cs.umd.edu/~laxman/papers/Ligra+.pdf

City-Scale Spatial Graphs (Paper):
* delta-compressed CSR storage using LEB128 varint encoding
* https://arxiv.org/html/2604.08374v1

# Integer Compression #

A Comparative Study of Variable-Length Integer Coding Techniques for Lossless Data Compression (Paper, 2026): 
* https://ieeexplore.ieee.org/document/11485875/authors#authors

Lossless Compression of Time Series Data: A Comparative Study (Paper, 2025):
* https://arxiv.org/abs/2510.07015

**Key** Techniques for Inverted Index Compression (Paper, 2020):
* https://arxiv.org/pdf/1908.10598

# Hardware Graph #

In systems research, 
graph traversal is classified as an **"irregular application."**
Because graph edges point to random memory locations,
they defeat the CPU's hardware prefetcher (which expects you to read memory sequentially).

GraphBIG: understanding graph computing in the context of industrial solutions (Paper, 2015):
* https://dl.acm.org/doi/abs/10.1145/2807591.2807626

SpZip: Architectural Support for Effective Data Compression In Irregular Applications (Paper, 2021):
* https://www.researchgate.net/publication/353696338_SpZip_Architectural_Support_for_Effective_Data_Compression_In_Irregular_Applications

Speedup Graph Processing by Graph Ordering (Paper, 2015):
* https://dl.acm.org/doi/10.1145/2882903.2915220

# Compression Methods #

**Compression Methods**:
* // intermediate transformation
* Delta Coding
* QuaRs
* // encoder
* Sprintz
* bzip2
* Brotli
* partitioned Elias-Fano
* opt_vbyte
* Stream VByte

Stream VByte: breaking new speed records for integer compression (Blog, 2018):
* https://lemire.me/blog/2017/09/27/stream-vbyte-breaking-new-speed-records-for-integer-compression/

QuaRs: A Transform for Better Lossless Compression of Integers (Paper, 2025):
* https://arxiv.org/abs/2501.12929v1

Pcodec: Better Compression for Numerical Sequences (Paper, 2025):
* https://arxiv.org/abs/2502.06112

Sprintz: Time Series Compression for the Internet of Things (Paper, 2018):
* https://arxiv.org/abs/1808.02515

