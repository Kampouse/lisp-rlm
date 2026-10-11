# afp-clmm — AFP CLMM entry ported to lisp-rlm

Machine-checked Isabelle/HOL model of Uniswap-v3-style concentrated-liquidity
ops (Echenim, AFP 2025) ported to lisp-rlm contracts, differentially verified.

- **theory/** — verbatim AFP entry, read-only reference (`*.thy`, ~12.5K lines)
- **escrow/** — the escrow-solvency theory (`CLMM_Escrow.thy`, STRICT
  session: 0 sorries) ported as contract twins under `port/contracts/escrow/`
- **port/** — the port itself. See `port/README.md` for the tour.

```
cd port
make prove      — pool-join+fee-join 9/9 · n-pool 5/5 · TS 14/14 — ALL GREEN or exit 1
make escrow     — escrow twins: 4 scenarios + 3/3 mutation gate — ALL GREEN or exit 1
```
