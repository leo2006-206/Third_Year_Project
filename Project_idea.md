# TYP proposal #

Efficiency static compressed sparse row (csr) graph representation.

Assumption:
* The graph is **static**, that **read only**, no updates like insert or remove nodes/edges. <br>
  This make csr graph data structure easier to implement.
* The graph only hold **integer** values for nodes, edges and weights.
* The graph requires **zero-indexed nodes**. <br>All node IDs must be contiguous integers strictly in the range [0, node_size - 1].

## Compressed Sparse Row (csr) graph ##
given a directed graph as image showed below.

<img width="306" height="179" alt="image" src="https://github.com/user-attachments/assets/87afd30c-59e0-471a-8caa-5b5e7779cea0" />

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
The follow c code explains how to read the edges for a given node $X$.
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
4. iterating the edge_array from index 0 to 2, then {1, 2, 3}

[Open Compiler Explorer to run example code](https://godbolt.org/#z:OYLghAFBqd5QCxAYwPYBMCmBRdBLAF1QCcAaPECAMzwBtMA7AQwFtMQByARg9KtQYEAysib0QXACx8BBAKoBnTAAUAHpwAMvAFYTStJg1AB9U8lJL6yAngGVG6AMKpaAVxYM9DgDJ4GmADl3ACNMYhAAJg1SAAdUBUJbBmc3Dz04hJsBX38gllDwqItMKyyGIQImYgIU908uYtKkiqqCHMCQsMjohUrq2rSG3tb2vILugEoLVFdiZHYOAFIIgGY/ZDcsAGpFlcde/FQAOgRd7EWNAEELy4A3VDx0LZjiPwJjZAViY0x0YEwIDctsCtm8AFRbBgYTDGKpkIEg8FbX7/WHEeFXEGgwSQ6HGBIAL0wpARwLeyL%2BMMJxNJ2IIWwUMzmmBuE0WAHYAEK0vBUCCM2bzHYrc4rAAiuKw%2BLwRLZXOImAIswYu257LFNx5OOG1WFEqhUrhiwArJyBcyTRqVtzMYicQ5VTy%2BeahctOVsuMLRfq8dS5TbLlisQ49RTUdTHbbgRyNVHkbQlByA0GQSHdj7DeiTWama6Iu6uJbI4GQTHNXH%2BMQIOS8KGdQRVaDhY5kQx0I23W68P7aViXm8%2BcsIgbMDsIsankQxxOTY4VREIqQGbniWGYUbTXhLWzrbSy3H%2B4JBwvZ/OIjuA/vrldySwmH4IPdHj242gGL06ZKqTLR%2BmtpJiyxN8P3JFFvyJUNjUAu16RHNFiCYABPbMAONCU/yTaIthWJdpC2KD1Wgsl7UpeCkOzKC0NDJMGi2RdsKXeiVjLXcrlpStqxxODCFDDRG24%2BldhbOCI2tMduXzASXxLFND3eT5vjAiA4LhJClzAsjEKXESf3U0jqW0vFCAvPdCLYtj1Q4KZaE4Y1eE8DgtFIVBOBbF1f1WHhSAITQrKmABrEBjUkI4NFWSQAE5JAADgANg0SKIoidl9E4SR7N85zOF4BQQGiHzHKs0g4FgJBMFUTBkFcIgyAoCAqmABRlEMEohAQVAAHcHK8tAWBiOgmDKZr/FoNrOocpzev6%2BhwmQYAuFihoproMIAlYBZeGWmaAHlqrGrrMvKyrLmIRrstII7kAqfAHN4fhBBEMR2CkGRBEUFR1EK0hdAaAwjBAUwPn0PBglyyAplQGIylyjheFQW4wleLAwYgKZiFcQQ8DYAAVVAXBRqZ3L0A4/GG1r2oO7heA6hCYk4HhrNsjKvpcjhsAqqqaq2VQ4oAWliyQtmAZBkA9WKjk9CBHCXXBCBIMcVi4CZeAKrQJimBBMCYLBwlR0hAskY0jgi9lYuNDRjS4FYIvNjR2RwmyOHS0gJrh87cvy3z1dSjgImZpzWZVr2pgR4gEjsSQgA)







