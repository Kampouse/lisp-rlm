# afp-clmm — AFP CLMM entry ported to lisp-rlm

Machine-checked Isabelle/HOL model of Uniswap-v3-style concentrated-liquidity
ops (Echenim, AFP 2025) ported to lisp-rlm contracts, differentially verified.

- **theory/** — verbatim AFP entry, read-only reference (`*.thy`, ~12.5K lines)
- **proof/** — all Isabelle sessions in one folder: CLMM_Dynamic (fee
  ledger), CLMM_Escrow (escrow-solvency shell, STRICT: 0 sorries, ported
  as contract twins under `port/contracts/escrow/`), CLMM_Bridge
  (composition; parent session = CLMM_Escrow, imports the live theories —
  no snapshots). Build: `isabelle build -D proof`
- **port/** — the port itself. See `port/README.md` for the tour.

```
cd port
make prove      — pool-join+fee-join 9/9 · n-pool 5/5 · TS 14/14 — ALL GREEN or exit 1
make escrow     — escrow twins: 4 scenarios + 3/3 mutation gate — ALL GREEN or exit 1
```
