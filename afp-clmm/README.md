# afp-clmm — AFP CLMM entry ported to lisp-rlm

Machine-checked Isabelle/HOL model of Uniswap-v3-style concentrated-liquidity
ops (Echenim, AFP 2025) ported to lisp-rlm contracts, differentially verified.

- **theory/** — verbatim AFP entry, read-only reference (`*.thy`, ~12.5K lines)
- **port/** — the port itself. See `port/README.md` for the tour.

```
cd port
make prove      # leg2+3 9/9 · leg4 5/5 · TS twins 14/14 — ALL GREEN or exit 1
```
