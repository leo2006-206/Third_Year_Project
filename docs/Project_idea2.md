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

1. Delta Coding:

* Decorrelates the data by storing differences between consecutive values .

2. RLE (Run-Length Encoding):
* Compresses long consecutive runs of the exact same value (optional, but highly recommended for zeroes).
* Not for csr graph, bs less or no duplicated value.

3. QuaRs:
* Reshuffles the remaining jumpy/sparse values to cluster them tightly around zero . (Reduce Average Absolute Deviation)

4. Entropy Coder:
* A fast integer encoder (like Sprintz, Pcodec, or a custom SIMD bit-packer) that packs the now-tiny integers into binary .

**Compression Methods**:
* // intermediate transformation
* Delta Coding
* QuaRs
* // encoder
* bzip2		(General, Huffman coding)
* Brotli	(General, LZ77)
* LZ4		(General, LZ77)
* Sprintz	(Integer, )
* partitioned Elias-Fano
* opt_vbyte
* Stream VByte

