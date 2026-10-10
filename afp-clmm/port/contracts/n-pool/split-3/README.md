# splt3 — 3-way splitter (leg 4: N-pool optimality)

Splits a deposit across `pa` + `n1` + `n2` at the **equal-ending-price
optimum p\***: net out is concave in y, so the equal-fee optimum equalizes
ending price across pools — p\* solves `sum_k qgross_k(p*) = y`, found by a
FLAT UNROLLED binary search (48 storage round-trips, no nesting — the
loop-carried string mutation pitfall forbids the loop form; see port
README).

`y_k = min(cap_k, qgross_k(p*))` — per-pool per-cell muldiv floor dust is
bounded by `VIOLWIN = 2*N` out-units; the sweep suite (33³ fraction grid +
20k random splits) found zero violations. Exercises: `npool-over/two/
star/under`, `opt-n`. Regenerate with `python3 gen3.py`.
