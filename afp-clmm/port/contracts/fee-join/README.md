# fee-join/ — what changes when fees differ: `pool_fee_join`

**Why this exists:** pool-join assumed one fee. Here A (0.3%) meets C (0.5%)
on the same grid. The AFP result: gross books add EXACTLY (QBK = QA + QC,
integer add, no rounding) — the only rounded quantity is the blended per-cell
fee `Fhat` (fee-scale nested floors), exported by the contract for inspection.

| contract | role |
|---|---|
| `pool-c` | same grid as A, different fee (0.5%) — the P2 of this group |
| `fee-join-ac` | `pool_fee_join(a, c)` — the heterogeneous-fee join |

Scenarios: `join` `inside-join` — fee-join vs its parts, with/without interior entry.
