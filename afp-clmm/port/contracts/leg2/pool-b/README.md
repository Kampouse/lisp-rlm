# pb — pool B (leg 2 partner)

Grid `[1, 3, 4]e9` — **deliberately different from pa's** so the refinement
step is real work: `pd` cuts pb's `[1,3]` cell at `2e9`. Liquidity
`[3e22, 9e22]`, fee 0.3%.

The other half of the pairwise-combination theorem. Same scenario set as
`pa`. Regenerate with `python3 gen.py`.
