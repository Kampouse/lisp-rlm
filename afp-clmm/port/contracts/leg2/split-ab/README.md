# splt — splitter (leg 2: split optimality)

Walks the combined pool `pd`, computes the equalized-split quote `y1 =
qA_gross(sqpC)` (capped), then routes the two legs to `pa` and `pb`. Pins
the AFP **split-optimality theorem**: no legal (pa,pb) split beats this
allocation beyond per-cell muldiv floor dust (`VIOLWIN`).

Credits `paid:<trader>` additively — see the state.bin pitfall in the port
README before ad-hoc reruns. Exercises: `opt-exact`, `opt-cross`,
`opt-offgrid` (optimum), plus every leg-2 scenario routes through it.
Regenerate with `python3 gen.py`.
