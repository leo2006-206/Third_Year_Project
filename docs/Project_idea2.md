# TYP Proejct idea #

Experimental project

title: **Measuring CPU hardware behaviours for CSR graph with different integer compressions**

## Compressed Sparse Row Graph ##

`node_array` is a strictly equal or increasing integer array.<br>
Where `node_array[ node_x ]` return the starting position of `node_x`'s edges.<br>
Integer value could be duplicated.

`edge_array` is a array of integer array.<br>
Integer value cannot be duplicated.<br>
Usually sorted the sub-array to enable binary-search and improve compression ratio.


If $|E| \gt (2 ^ {32} - 1)$<br>
Use `uint64_t` for the `node_array`.<br>
Else use `uint32_t` for the `node_array`.



```c
// Uncompressed
typedef struct csr_graph{
	uint32_t*	node_array;
	size_t		node_size;
	uint32_t*	edge_array;
	size_t		edge_size;
}

// Compressed edge_array
typedef struct comp_edge_graph{
	uint32_t*	node_array;
	size_t		node_size;
	uint8_t*	edge_comp_array;
	size_t		edge_byte_size;
}

// Compressed node_array
typedef struct comp_node_graph{
	uint8_t*	node_comp_array;
	size_t		node_byte_size;
	uint32_t*	edge_array;
	size_t		edge_size;
}

// Compressed both node_array and edge_array
typedef struct comp_graph{
	uint8_t*	node_comp_array;
	size_t		node_byte_size;
	uint8_t*	edge_comp_array;
	size_t		edge_byte_size;
}
```

## Compression Schemes  ##

On purposely chose the new recent Integer Encoders.
<br>
Hugely influenced by these 2 compression survey paper:
* [Inverted Index Compression, 2018 Paper](https://arxiv.org/pdf/1908.10598)
* [Lossless Compression of Time Series Data, 2025 Paper](https://arxiv.org/abs/2510.07015)

**Compression Methods**:
* intermediate transformation
	* Delta Coding
		* Key for sorted data
	* QuaRs
		* "smaller-numbers-less-bits" principle
		* reshapes arbitrary distributions into unimodal ones centered around zero
		* [2025, Paper](https://arxiv.org/abs/2501.12929v1)
* encoder
	* Sprintz
	<br>(IoT Time Series data, SIMD + multi-methods), [2018 Paper](https://arxiv.org/abs/1808.02515)
	* partitioned Elias-Fano
	<br>(Integer data, partition + Elias-Fano), [2014 Paper](https://dl.acm.org/doi/10.1145/2600428.2609615),
	* opt_vbyte<br>
	(Integer data, partition + SIMD + Variable-Byte), [2020 Paper](https://ieeexplore.ieee.org/document/8691421)
	* Stream VByte<br>
	(Integer data, SIMD + Variable-Byte), [2018 Paper](https://arxiv.org/abs/1709.08990)
* optional encoder
	* bzip2		(General, Dictionary)
	* Brotli	(General, Dictionary)
	* LZ4		(General, Dictionary)
	* ...

