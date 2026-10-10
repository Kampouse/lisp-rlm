# n-pool/ — our generalization beyond the AFP entry: N=3 composition

**Why this exists:** the AFP entry defines PAIRWISE joins only. This group
generalizes to N=3 pools on a shared grid (same fee) and asks for the optimal
route. By concavity of net output, the optimum EQUALIZES THE ENDING PRICE
across all pools — `split-3` implements that search (binary search on the
equalized price), and the `npool-*` scenarios pin each budget regime.

| contract | role |
|---|---|
| `pool-n1`, `pool-n2` | the two extra pools (grid = pool-a's, fee 0.3%) |
| `split-3` | 3-way splitter: equal-ending-price optimum across a+n1+n2 |

Scenarios: `npool-star/two/over/under` (active-set regimes) + `opt-n` — the
headline counterexample: all-in on the least-liquidity pool nets LESS than the
composed optimum. That's why composition exists.
