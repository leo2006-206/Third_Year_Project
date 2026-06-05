# TYP Idea #

Efficiency static compressed sparse row (csr) graph representation.

Assumption:
* The graph is **static**, that **read only**, no updates like insert or remove nodes/edges. <br>
  This make csr graph data structure easier to implement.
* The graph only hold **integer** values for nodes, edges and weights.
* The graph requires **zero-indexed nodes**. <br>All node IDs must be contiguous integers strictly in the range [0, node_size - 1].

## Compressed Sparse Row (csr) graph ##
given a directed graph as image showed below.

<img width="306" height="179" alt="image" src="https://github.com/user-attachments/assets/87afd30c-59e0-471a-8caa-5b5e7779cea0" />

***

In Adjacency List form:
  * $0 : \set{1, 2, 3}$
  * $1 : \set{2}$
  * $2 : \set{3}$
  * $3 : $

Adjacency List only use $O( |V| + |E|)$ space, which is good especially for sparse graph. <br>
In trandition Random Access Machine (RAM) model, to iterate a list need $O(N)$ time complexity. <br>
However in a real processor, random memory access could be costly depended on the memory location and caches. <br>
This makes the iteration of list always slower that iteration of array. <br>
Even both have the same $O(N)$ iteration time complexity in theory.

***

In Adjacency Matrix:

$$
A = \begin{bmatrix} 
0 & 1 & 1 & 1 \\ 
0 & 0 & 1 & 0 \\ 
0 & 0 & 0 & 1 \\ 
0 & 0 & 0 & 0 
\end{bmatrix}
$$

Adjacency Matrix proivdes contiguons memorys access for edges iteration. <br>
This proivdes better iteration performance that Adjacency List. <br>
However, this representation required $O({\rvert V \lvert}^2)$ memory space.

***

In Compressed Sparse Row:

$$
\text{nodes array}:\set{ \begin{array}{|c|c|c|c|}
\hline
0 & 3 & 4 & 5 \\
\hline
\end{array}}
$$
$$
\text{edges array}:\set{ \begin{array}{|c|c|c|c|c|}
\hline
1 & 2 & 3 & 2 & 3 \\
\hline
\end{array}}
$$

In Compressed Sparse Row,it provides contiguons memorys access with only $O(|V| + |E|)$ space. <br>
The follow c++ code explains how to read the edges for a given node $X$.
```c
int edge_source = /*node*/ x;
int start_index = nodes_array[edge_source];
int end_index = nodes_array[edge_source + 1];
//also check the edge case if x is the last node
for(int i = start_index; i < end_index; ++i){
  int edge_dest = edges_array[i];
  //function( edge_source, edge_dest )
}
```
Example:
1. say $X$ is node 0
2. start = node_array[0] //0
3. end = node_array[0 + 1] //3
4. iterating the edge_array from index 0 to 2, that {1, 2, 3}

[Open Compiler Explorer to run example code](https://godbolt.org/#z:OYLghAFBqd5QCxAYwPYBMCmBRdBLAF1QCcAaPECAMzwBtMA7AQwFtMQByARg9KtQYEAysib0QXACx8BBAKoBnTAAUAHpwAMvAFYTStJg1AB9U8lJL6yAngGVG6AMKpaAVxYM9DgDJ4GmADl3ACNMYhAAJg1SAAdUBUJbBmc3Dz04hJsBX38gllDwqItMKyyGIQImYgIU908uYtKkiqqCHMCQsMjohUrq2rSG3tb2vILugEoLVFdiZHYOAFIIgGY/ZDcsAGpFlcde/FQAOgRd7EWNAEELy4A3VDx0LZjiPwJjZAViY0x0YEwIDctsCtm8AFRbBgYTDGKpkIEg8FbX7/WHEeFXEGgwSQ6HGBIAL0wpARwLeyL%2BMMJxNJ2IIWwUMzmmBuE0WAHYAEK0vBUCCM2bzHYrc4rAAiuKw%2BLwRLZXOImAIswYu257LFNx5OOG1WFEqhUrhiwArJyBcyTRqVtzMYicQ5VTy%2BeahctOVsuMLRfq8dS5TbLlisQ49RTUdTHbbgRyNVHkbQlByA0GQSHdj7DeiTWama6Iu6uJbI4GQTHNXH%2BMQIOS8KGdQRVaDhY5kQx0I23W68P7aViXm8%2BcsIgbMDsIsankQxxOTY4VREIqQGbniWGYUbTXhLWzrbSy3H%2B4JBwvZ/OIjuA/vrldySwmH4IPdHj242gGL06ZKqTLR%2BmtpJiyxN8P3JFFvyJUNjUAu16RHNFiCYABPbMAONCU/yTaIthWJdpC2KD1Wgsl7UpeCkOzKC0NDJMGi2RdsKXeiVjLXcrlpStqxxODCFDDRG24%2BldhbOCI2tMduXzASXxLFND3eT5vjAiA4LhJClzAsjEKXESf3U0jqW0vFCAvPdCLYtj1Q4KZaE4Y1eE8DgtFIVBOBbF1f1WHhSAITQrKmABrEBjUkI4NFWSQAE5JAADgANg0SKIoidl9E4SR7N85zOF4BQQGiHzHKs0g4FgJBMFUTBkFcIgyAoCAqmABRlEMEohAQVAAHcHK8tAWBiOgmDKZr/FoNrOocpzev6%2BhwmQYAuFihoproMIAlYBZeGWmaAHlqrGrrMvKyrLmIRrstII7kAqfAHN4fhBBEMR2CkGRBEUFR1EK0hdAaAwjBAUwPn0PBglyyAplQGIylyjheFQW4wleLAwYgKZiFcQQ8DYAAVVAXBRqZ3L0A4/GG1r2oO7heA6hCYk4HhrNsjKvpcjhsAqqqaq2VQ4oAWliyQtmAZBkA9WKjk9CBHCXXBCBIMcVi4CZeAKrQJimBBMCYLBwlR0hAskY0jgi9lYuNDRjS4FYIvNjR2RwmyOHS0gJrh87cvy3z1dSjgImZpzWZVr2pgR4gEjsSQgA)

This memory representation offer excellent memory locality, <br>
due to contiguous compact memory storage.
However, this also make this data structure diffcult to update like sorted array.

***
## Common problem in graph processing ##

In most graph algorithms, they mostly depended on different graph traversals.<br>
Which means jump from one node to the next node.<br>
This becomes a pointer chasing problem that mostly is a memory-bound problem.

***

### Pointer Chasing example ###
[This is the original article that explain this problem very good](https://sourav-k-paul.medium.com/pointer-chasing-in-modern-systems-why-it-hurts-when-it-helps-and-how-real-applications-use-4de83de4a6ac)

In short, pointer chasing look like this.

```c
node = head
while(node.next != NULL){
  process( node.data );
  node = node.next
}
```

The CPU don't know that address `node.next` until `node` is loaded into CPU.<br>
So force the CPU to wait the currect node to complete.

Additionally, CPU cannot prefetch the `node.next`.<br>
Because there are no obvious among the `node.next` address,<br>
unlike array that have contiguons memory storage.<br>
This increased the number of cache miss that decline the performance.

***

### Graph Traversal example ###

Here is a block of c++ pseudo code for graph breadth-first search:
```c++
// initialise data structure and var
graph<int> g = incoming_graph;
queue<int> q;
array<int> visited = {false};

// setup the source node
q.enqueue( /*node*/ source );
visited[ source ] = true

//start searching
while( q.empty() == false ){
  int current_node = q.dequeue();
  processing( current_node );

  for(node neighbor : g.get_neighbors( current_node )){
    if( visited[ neighbor ] == false ){
      visited[ neighbor ] = true;
      q.enqueue( neighbor );
    }
  }
}
```

We can earily see the similar pattern between pointer chasing and graph traversal.<br>
Where pointer chasing need to wait for the `node` loaded into CPU,<br>
and graph traversal need to wait for the `g.get_neighbors( current_node )` loaded into CPU.

To improve the graph traversal performance for most graph algorithm.<br>
We can reduce the latency of graph to load the `g.get_neighbors( current_node )`.

***

## Directions to reduce graph loading latency ##

All directions here will be based on Compressed Sparse Row (csr) graph.

***

1. **Integer Compression**

Key part.

We can compression the size of data to improve spatial locality.<br>
Where the caches can load more element to reduce the number of cache misses.

Say iterating a array will take $O(N)$ operations. <br>
If we iterate a compressed array.<br>
It will need $O(N + N)$ operations to iterations. //extra $N$ opeations for decoding<br>
But this could be faster in real world.<br>
Because this require less memory loading and when data in CPU,<br>
the operations is fast.

This closely align to the idea of **External Memory** and **Cache-oblivious**  model.<br>
[Ref to External Memory wiki](https://en.wikipedia.org/wiki/External_memory_algorithm)<br>
[Ref to Cache-oblivious wiki](https://en.wikipedia.org/wiki/Cache-oblivious_algorithm)


Becasue the csr graph basically is 2 integer array.<br>
where `nodes_array` is a sorted strictly increasing array with $O(1)$ indexing requirement,<br>
and `edges_array` is a collection of sub sorted strictly increasing arrays<br>
// Sorted the array before building csr graph.

$$
\text{nodes array}:\set{ \begin{array}{|c|c|c|c|}
\hline
0 & 3 & 4 & 5 \\
\hline
\end{array}}
$$
$$
\text{edges array}:\set{ \begin{array}{|c|c|c|}
\hline
\text{sub array 1: } \set{1, 2, 3} & \text{sub array 2: } \set{2} & \text{sub array 3: } \set{3} \\
\hline
\end{array}}
$$

A simple way to compress sorted int array //without indexing operation <br>
We can use **Delta Encoding**,<br>
where we only storing the differences between the current and next values.<br>
Where we can apply to the sub array in `edges_array`

example:<br>
`[1, 3, 7, 9]`, become `[1, 2, 4, 2]`<br>
[Ref to Delta encoding wiki](https://en.wikipedia.org/wiki/Delta_encoding)

For sorted indexable int array.<br>
We can use **Elias-Fano Encoding** from **Succinct data structure** field.

[Ref to nice Elias-Fano Encoding article](https://www.antoniomallia.it/sorted-integers-compression-with-elias-fano-encoding.html)<br>
[Ref to Succinct data structure wiki](https://en.wikipedia.org/wiki/Succinct_data_structure)

To utilise the modern CPU SIMD instructions.<br>
We could use **SIMDable Variable-length quantity**

[Ref to nice article by Daniel Lemire, the author of many simd vlq libraries](https://lemire.me/blog/2017/09/27/stream-vbyte-breaking-new-speed-records-for-integer-compression/)

***
2. **Graph Reordering**

Future optional part

Basically reduce the number of cahce miss by reordering/renumbering the graph.

[Ref to paper Speedup Graph Processing by Graph Ordering](https://dl.acm.org/doi/10.1145/2882903.2915220)

[Ref to blog Locality Analysis of Graph Reordering Algorithms](https://blogs.qub.ac.uk/dipsa/locality-analysis-of-graph-reordering-algorithms/)

***
3. **Cache-awareness graph algorithms**

Future future optional part

[Ref to External_memory_graph_traversal wiki](https://en.wikipedia.org/wiki/External_memory_graph_traversal)

[Ref to paper Cache-Oblivious Priority Queue andGraph Algorithm Applications](https://erikdemaine.org/papers/BufferTrees_STOC2002/paper.pdf)
