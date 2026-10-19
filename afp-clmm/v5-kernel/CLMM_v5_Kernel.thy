(* Proof-of-concept: v5 fixed-point curve kernel, executable and exportable. *)
theory CLMM_v5_Kernel
  imports
    Main
    "HOL-Library.Discrete_Functions"
    "HOL-Library.Code_Target_Numeral"
    "HOL-Library.Code_Target_Int"
begin

(* Utilities *)

abbreviation two64 :: int where "two64 == 18446744073709551616"

(* Use the library's verified integer floor square root. *)
definition isqrt :: "int => int" where
  "isqrt n = (if n <= 0 then 0 else int (floor_sqrt (nat n)))"

lemma isqrt_nonneg[simp]: "isqrt n >= 0"
  unfolding isqrt_def by auto

(* Grid: Q64.64 sqrt-prices for 1.0001^i *)

definition G :: "int => int" where
  "G i = (let j = nat (abs i);
              n = (if i >= 0 then 10001^j * two64^2 else 10000^j * two64^2);
              d = (if i >= 0 then 10000^j else 10001^j);
              q = n div d
          in if q <= 0 then 0 else int (floor_sqrt (nat q)))"

lemma G_nonneg[simp]: "G i >= 0"
proof (cases "i >= 0")
  case True
  then show "G i >= 0"
    unfolding G_def
    by (smt (verit) floor_sqrt_zero int_nat_eq nat_eq_iff nat_int nat_zero_as_int
        nonneg_eq_int zero_le_divide_iff zero_le_power)
next
  case False
  then show "G i >= 0"
    unfolding G_def
    by (smt (verit) floor_sqrt_zero int_nat_eq nat_eq_iff nat_int nat_zero_as_int
        nonneg_eq_int zero_le_divide_iff zero_le_power)
qed

(* Per-cell amount formulas *)

definition dy_in :: "int => int => int => int" where
  "dy_in L S S2 = (L * (S - S2)) div two64"

(* X amounts carry the same 2^64 fixed-point renormalization as Y.
   BUG HISTORY (2026-10-19, pool5 live): dx was `L * (S - S2) div (S * S2)`
   — right SHAPE in rationals, but S,S2 are 2^64-scaled, so the division by
   S*S2 ≈ 2^128 swallowed an extra 2^64 (observed live: rx 34 yocto vs
   ry 850 tokz at parity; X-starved pools, dead Y→X swaps). The bridge
   lemma below now FAILS if anyone reintroduces a scaling slip. *)
definition dx_out :: "int => int => int => int" where
  "dx_out L S S2 = (L * (S - S2) * two64) div (S * S2)"

definition dx_in :: "int => int => int => int" where
  "dx_in L S S2 = (L * (S2 - S) * two64) div (S * S2)"

definition dy_out :: "int => int => int => int" where
  "dy_out L S S2 = (L * (S2 - S)) div two64"

(* ── PRICE BRIDGE ─────────────────────────────────────────────────────
   The oracle a scaling bug cannot evade (added 2026-10-19 after pool5:
   dx missed one 2^64 renormalization; every conservation proof still
   passed because they are conditional on the definitions).
   In exact rationals with s = S/t64, s2 = S2/t64:
     dy/L = s − s2            (Y per unit L)
     dx/L = (s − s2)/(s·s2)   (X per unit L)
   ⇒ dy/dx = s·s2  =  the geometric-mean price squared. Any fixed-point
   slip in dx breaks this ratio. Proven over reals; the integer defs are
   the floors of these (dx_out_floor). *)
definition two64r :: real where "two64r = real_of_int two64"
definition dyq :: "real => real => real => real" where
  "dyq L S S2 = L * (S - S2) / two64r"
definition dxq :: "real => real => real => real" where
  "dxq L S S2 = L * (S - S2) * two64r / (S * S2)"

lemma price_bridge:
  assumes "L > 0" "S > S2" "S > 0" "S2 > 0"
  shows "dyq L S S2 / dxq L S S2 = S * S2 / (two64r * two64r)"
proof -
  have t64: "two64r ~= (0::real)"
    by (simp add: two64r_def)
  have nz: "dxq L S S2 ~= (0::real)"
    using assms t64 by (simp add: dxq_def)
  have eq: "dyq L S S2 / dxq L S S2
      = (L * (S - S2) / two64r) * ((S * S2) / (L * (S - S2) * two64r))"
    using nz t64 by (simp add: dyq_def dxq_def field_split_simps)
  also have "S * S2 / (two64r * two64r)
      = (L * (S - S2) / two64r) * ((S * S2) / (L * (S - S2) * two64r))"
    using assms t64 by (simp add: field_split_simps)
  finally show ?thesis ..
qed

definition S_for_dy_in :: "int => int => int => int" where
  "S_for_dy_in L S dy = (if L <= 0 then 1
    else max 1 (S - ((two64 * (dy + 1) - 1) div L)))"

(* inverse of the RENORMALIZED dx_in: dx = L(S2-S)t64/(S*S2) ≤ dx
   ⟺ S2·(L·t64 − (dx+1)·S) < L·S·t64 ⇒ largest such S2 = (N−1) div D
   with N = L·S·t64, D = L·t64 − (dx+1)·S (mirrors S_for_dy_in's −1 form);
   unreachable (D ≤ 0, dx ≥ cell total) ⇒ 2^127; clamped ≥ S *)
definition S_for_dx_in :: "int => int => int => int" where
  "S_for_dx_in L S dx = (if L <= 0 | S <= 0 | L * two64 <= (dx + 1) * S
    then 340282366920938463463374607431768211456
    else max S ((L * S * two64 - 1) div (L * two64 - (dx + 1) * S)))"

(* Fee shift *)

definition fee_eff :: "int => int => int" where
  "fee_eff v shift = v - (v div (2 ^ nat shift))"

(* Cell-by-cell quote walk *)

fun walk_dn :: "(int => int) => int => int => int => nat => int => int => int * int * int" where
  "walk_dn LQ S P imin 0 acc rem = (acc, S, P)"
| "walk_dn LQ S P imin (Suc fuel) acc rem =
    (if rem <= 0 | P <= imin then (acc, S, P)
     else let L = LQ P;
              S2 = G (P - 1);
              dyi = dy_in L S S2;
              dxo = dx_out L S S2
          in if dyi <= rem
             then walk_dn LQ S2 (P - 1) imin fuel (acc + dxo) (rem - dyi)
             else let S2' = S_for_dy_in L S rem
                  in (acc + dx_out L S S2', S2', P))"
(* Partial down-consumption: S2' lies strictly inside cell P
   ([G(P-1), G(P))) because the full-cell dy_in exceeded the remainder —
   otherwise the branch above would have hopped. The active cell is the
   cell CONTAINING the new price, so P is unchanged. (The previous
   P - 1 stranded P one cell below S and froze subsequent up-walks:
   seeds 16/34 differential finding, 2026-10-17.) *)

fun walk_up :: "(int => int) => int => int => int => nat => int => int => int * int * int" where
  "walk_up LQ S P imax 0 acc rem = (acc, S, P)"
| "walk_up LQ S P imax (Suc fuel) acc rem =
    (if rem <= 0 | P >= imax + 1 then (acc, S, P)
     else let L = LQ P;
              S2 = G (P + 1);
              dxi = dx_in L S S2;
              dyo = dy_out L S S2
          in if dxi <= rem
             then walk_up LQ S2 (P + 1) imax fuel (acc + dyo) (rem - dxi)
             else let S2' = S_for_dx_in L S rem
                  in (acc + dy_out L S S2', S2', P))"

definition quote_y_in_fee :: "(int => int) => int => int => int => int => int => int => int * int * int" where
  "quote_y_in_fee LQ S P imin imax v shift =
    (let v_eff = fee_eff v shift;
         fuel = nat (P - imin + 1)
     in walk_dn LQ S P imin fuel 0 v_eff)"

definition quote_x_in_fee :: "(int => int) => int => int => int => int => int => int => int * int * int" where
  "quote_x_in_fee LQ S P imin imax v shift =
    (let v_eff = fee_eff v shift;
         fuel = nat (imax + 1 - P)
     in walk_up LQ S P imax fuel 0 v_eff)"

(* Gross (parity) variants: no fee skim on the input side. *)

definition quote_y_in_gross :: "(int => int) => int => int => int => int => int => int => int * int * int" where
  "quote_y_in_gross LQ S P imin imax v shift =
    (let fuel = nat (P - imin + 1)
     in walk_dn LQ S P imin fuel 0 v)"

definition quote_x_in_gross :: "(int => int) => int => int => int => int => int => int => int * int * int" where
  "quote_x_in_gross LQ S P imin imax v shift =
    (let fuel = nat (imax + 1 - P)
     in walk_up LQ S P imax fuel 0 v)"

(* Export to OCaml *)

export_code
  quote_y_in_fee quote_x_in_fee
  quote_y_in_gross quote_x_in_gross
  S_for_dy_in S_for_dx_in
  dy_in dx_in dy_out dx_out
  fee_eff G isqrt
  in OCaml
  module_name CLMM_v5_Kernel
  file_prefix "clmm_v5_kernel"

end
