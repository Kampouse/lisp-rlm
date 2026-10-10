# pool-join/ — the core theorem: two pools on DIFFERENT grids, same fee

**Why this exists:** the heart of the AFP port — `pool_combination` on pools
whose price grids disagree. Pool B's [1,3]e9 cell doesn't exist on A's grid, so
`refine` cuts it proportionally at 2e9 and the join happens on the common
refinement [1,2,3,4]e9. The splitter then proves the equalized-ending-price
routing is optimal.

| contract | role |
|---|---|
| `pool-a` | baseline, grid [1,2,4]e9, fee 0.3% |
| `pool-b` | the different grid [1,3,4]e9 — forces real refine/slice (not identity) |
| `join-ab` | `pool_join(a, b)` on the common refinement — swapping it ≡ swapping a+b |
| `split-ab` | walks join-ab, routes legs to a/b at the optimal split point |

Scenarios: `exact` `offgrid` `cross` `inside` + optimality `opt-*` (see `../../scenarios/README.md`).
