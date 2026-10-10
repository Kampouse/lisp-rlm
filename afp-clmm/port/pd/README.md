# pd — pool D = pool_join(refine(pa), refine(pb)) (leg 2)

Combined pool on the common refinement grid `[1, 2, 3, 4]e9`: pb's `[1,3]`
cell is CUT at `2e9` (proportional split, exact — constants divide), per-cell
liquidity = sum of both pools' density on the cell, and each cell gets its
own gross book `Q = ceil(L*DEN/(DEN-NUM))` (cst_fee, phi=3e15/1e18).

**Pins the leg-2 theorem:** swapping through `pd` ≡ swapping through pa+pb
byte-exactly, from any starting sqp (interior entry via optional `start`).
Exercises: `exact`, `inside`, `cross`, `offgrid` compare pd vs the originals.
