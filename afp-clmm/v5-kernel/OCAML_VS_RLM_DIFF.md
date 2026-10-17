# CLMM v5: OCaml oracle (Isabelle export) vs RLM contract kernel diff

## Function correspondence

| Isabelle / OCaml oracle | RLM contract | Notes |
|---|---|---|
| `G i` | `p5-G i` | Both compute `isqrt(1.0001^i * 2^128)` using integer isqrt. Oracle uses nat/int; RLM uses limb arithmetic. |
| `dy_in L S S2` | `p5-dyi L S S2` (or `p5-dy L S2 S`) | `dy_in = L*(S-S2) div 2^64`. RLM `p5-dy L S2 S` is same: `L*(S-S2)>>64`. |
| `dx_out L S S2` | `p5-dxo L S S2` (or `p5-dx L S2 S`) | `dx_out = L*(S-S2) div (S*S2)`. RLM swaps args. |
| `dx_in L S S2` | `p5-dx L S S2` | `dx_in = L*(S2-S) div (S*S2)`. RLM `p5-dx` matches. |
| `dy_out L S S2` | `p5-dy L S S2` | `dy_out = L*(S2-S) div 2^64`. RLM `p5-dy` matches. |
| `S_for_dy_in L S dy` | `p5-sfdyi L S dy` | Oracle: `if L<=0 then 1 else max 1 (S - (2^64*(dy+1)-1) div L)`. RLM `p5-sfdyi` uses `p5-tstar!` to compute `(2^64*(dy+1)-1)//L`, then `max(1, S - t*)`. Same. |
| `S_for_dx_in L S dx` | `p5-sfdx L S dx` | Oracle: `if L<=0 | S<=0 then S else D = L + (dx+1)*S; N = L*S+1; q = N div D; if N mod D = 0 then max S q else max S (q+1)`. RLM computes `ceil((L*S+1)/D)`, min with 2^127, max with S. The `max S q` (and +1 for remainder) matches; the RLM 2^127 clamp is a large upper bound equivalent to no clamp for realistic inputs. |
| `fee_eff v shift` | `p5-shr8` / `(if feep? (li-sub v F) v)` | Oracle: `v - v div 2^shift`. RLM: `F = v >> 8` (fee), then `v_eff = v - F`. Identical for shift=8. |
| `quote_y_in LQ S P imin imax v shift` | `p5-swarm-dn` | Both do `v_eff = v - fee`, then walk down while `rem > 0` and `P > imin`, consuming cell `P` and moving to `G(P-1)`. Same per-cell formulas. |
| `quote_x_in LQ S P imin imax v shift` | `p5-swarm-up` | Both walk up while `rem > 0` and `P < imax+1`, consuming cell `P` and moving to `G(P+1)`. Same per-cell formulas. |

## Structural differences

1. **Fuel vs condition loop.** Isabelle uses `fuel = nat(P - imin + 1)` or `nat(imax + 1 - P)` to make the recursion executable and total. RLM uses a general `loop` with explicit stop conditions `(<= pp imin)` / `(>= pp (+ imax 1))` and `(= rm 0)`. Mathematically equivalent given enough fuel; the fuel bound is chosen to exceed the needed steps.

2. **Grid cell indexing.**
   - Oracle `walk_dn`: current cell `P` has liquidity `LQ P`; next price `S2 = G(P-1)`.
   - RLM `p5-swarm-dn`: current cell `pp`, liquidity at `LQ:pp`, next price `gl = G(pp-1)`. Same.

3. **Early-exit / no-op.** Oracle returns `(acc, S, P)` unchanged when `rem <= 0`. RLM returns `"acc|rm"` with no `P/S` writes when both `acc` and `rm` are zero relative to initial `v0`. Equivalent.

4. **Final-state `P`.** Oracle `walk_dn` returns `P` unchanged when no step is taken, and `P-1` when crossing a full cell. RLM writes `6|P` as `pp-1` on full crossing or when falling inside the lower cell (`s2` in `[G(pp-1), G(pp))`). Matches.

5. **Fee path.** Oracle applies `fee_eff` once up front; RLM does the same for `p5-swap` non-custody (`feep?=0` currently), and applies fee-skimming separately in custody path. The differential fuzz only compares non-custody swaps, so the kernels match bit-exact.

## Verification status

- Differential fuzz (`tests/test_pool_v5_fuzz.rs`) seeds 1–3 × 40 ops green after latest `p5-view` LQ change.
- No formal equivalence proof yet between `CLMM_v5_Kernel.thy` and `lib/clmm_pool_v5.lisp`.
- `afp-clmm/proof/CLMM_Curve.thy` proves a fee-slack law on an abstract 2-cell nat curve, not this exact v5 kernel.
