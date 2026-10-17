(* CLMM.Fee — arithmetic laws for the pool v5 fee path.
   Lane fence (2026-10-17):
   - F* (this dir)  : arithmetic KERNEL laws only — skim, mulshr64, feefold sums.
   - Isabelle (afp) : protocol/ledger invariants — aggregate conservation, custody.
   - Parity harness : limb-level bit-exactness vs clmm_v5_ref.py (regression).
   Model: unbounded ints refined to u128 range. The sum only type-checks
   under the growth-monotonicity invariant (FS <= FG per point) — the sane-
   state precondition is structural, not a runtime check.
   NOTE: op_Multiply per house convention (this build's parser reserves *
   for product types in term position; see tests/TestFalseComp.fst et al). *)

module CLMM.Fee

let u128_max : int = 0x100000000000000000000000000000000
let two64    : pos = 0x10000000000000000

type amt    = v:int{0 <= v /\ v < u128_max}
type liq    = d:int{0 <= d /\ d < u128_max}
type growth = g:int{0 <= g /\ g < u128_max}
type snap   = s:int{0 <= s /\ s < u128_max}

(* ---------- 1. custody intake skim: F = v >> 8 ---------- *)

let fee_skim (v:amt) : amt = v / 256
let v_eff   (v:amt) : amt = v - fee_skim v

let skim_complete (v:amt)
  : Lemma (ensures (fee_skim v + v_eff v == v /\ fee_skim v <= v /\ v_eff v <= v))
  = ()

let skim_floor (v:amt)
  : Lemma (ensures (op_Multiply 256 (fee_skim v) <= v /\ v < op_Multiply 256 (fee_skim v) + 256))
  = ()

let skim_zero_small (v:amt)
  : Lemma (requires (v < 256)) (ensures (fee_skim v == 0 /\ v_eff v == v))
  = ()

let skim_effective_lower (v:amt)
  : Lemma (ensures (op_Multiply 256 (v_eff v) >= op_Multiply 255 v))
  = FStar.Math.Lemmas.lemma_div_mod v 256;
    FStar.Math.Lemmas.lemma_mod_lt v 256

(* ---------- 2. mulshr64: (x * d) >> 64 (Q64.64 scaling) ---------- *)

let mulshr64 (x:amt) (d:liq) : int = op_Multiply x d / two64

let mulshr64_zero (x:amt) (d:liq)
  : Lemma (ensures (mulshr64 0 d == 0 /\ mulshr64 x 0 == 0))
  = ()

// truncation loss per term is < 2^64 (the sub-unit dust):
// p - two64 * (p/two64) == p % two64 < two64
let mulshr64_dust (x:amt) (d:liq)
  : Lemma (ensures (op_Multiply x d - op_Multiply two64 (mulshr64 x d) < two64))
  = FStar.Math.Lemmas.lemma_div_mod (op_Multiply x d) two64;
    FStar.Math.Lemmas.lemma_mod_lt (op_Multiply x d) two64

// upper bound by scaled identity: term never exceeds the raw product
let mulshr64_le_raw (x:amt) (d:liq)
  : Lemma (ensures (op_Multiply two64 (mulshr64 x d) <= op_Multiply x d))
  = let p = op_Multiply x d in
    FStar.Math.Lemmas.lemma_div_mod p two64;
    FStar.Math.Lemmas.lemma_mod_lt p two64

// monotone in both arguments
let mulshr64_mono (x1 x2:amt) (d1 d2:liq)
  : Lemma (requires (x1 <= x2 /\ d1 <= d2))
          (ensures (mulshr64 x1 d1 <= mulshr64 x2 d2))
  = FStar.Math.Lemmas.lemma_mult_le_right d1 x1 x2;
    FStar.Math.Lemmas.lemma_mult_le_left x2 d1 d2;
    FStar.Math.Lemmas.lemma_div_le (op_Multiply x1 d1) (op_Multiply x2 d2) two64

(* ---------- 3. feefold: s = Σ_{i=a..b} mulshr64 (FG_i - FS_i) d ----------
   Read-before-write discipline (2026-10-17 scope-trap class): each term
   consumes the PRE-reset snapshot — structural here, since the model is
   pure and the delta input is the snapshot value by construction. *)

let term (g:growth) (s:snap) (d:liq) : Pure int (requires s <= g) (ensures fun _ -> True) = mulshr64 (g - s) d

// nat-counted recursion over an abstract nonnegative-delta function dl.
// All growth-monotonicity quantifier reasoning lives in ONE type-check at
// feefold_sum; the recursion itself carries no quantified precondition
// (this build's SMT encoding rejects that shape; see git log).
let rec sum_n (dl: int -> amt) (d:liq) (a:int) (n:nat) (acc:int)
  : Tot int (decreases n)
  = if n = 0 then acc
    else sum_n dl d a (n - 1) (acc + mulshr64 (dl (a + n - 1)) d)

let sumat (dl: int -> amt) (d:liq) (a:int) (n:nat) : Tot int = sum_n dl d a n 0

// accumulator generalized: acc always comes out additively
let rec sum_n_acc_eq (dl: int -> amt) (d:liq) (a:int) (n:nat) (acc:int)
  : Lemma (ensures (sum_n dl d a n acc == sum_n dl d a n 0 + acc))
          (decreases n)
  = if n > 0 then begin
      (sum_n_acc_eq dl d a (n - 1) (acc + mulshr64 (dl (a + n - 1)) d));
      (sum_n_acc_eq dl d a (n - 1) (mulshr64 (dl (a + n - 1)) d))
    end

// definitional split: one point peels off
let sum_n_split (dl: int -> amt) (d:liq) (a:int) (n:nat)
  : Lemma (requires (n > 0))
          (ensures (sumat dl d a n == sumat dl d a (n - 1)
                    + mulshr64 (dl (a + n - 1)) d))
  = sum_n_acc_eq dl d a (n - 1) (mulshr64 (dl (a + n - 1)) d)

// every term nonneg: mulshr64 of nonnegs is nonneg
let mulshr64_nonneg (x:amt) (d:liq)
  : Lemma (ensures (0 <= mulshr64 x d))
  = FStar.Math.Lemmas.lemma_div_le 0 (op_Multiply x d) two64;
    (assert (0 / two64 == 0))

// fold nonneg: sum of nonneg terms
let sumat_nonneg (dl: int -> amt) (d:liq) (a:int) (n:nat)
  : Lemma (ensures (0 <= sumat dl d a n))
  = let rec aux (k:nat)
      : Lemma (ensures (0 <= sumat dl d a k)) (decreases k)
      = if k > 0 then begin
          (aux (k - 1));
          (mulshr64_nonneg (dl (a + k - 1)) d);
          (sum_n_split dl d a k)
        end
    in aux n

// more points only add: monotone in the range length
let sumat_len_mono (dl: int -> amt) (d:liq) (a:int) (m:nat) (k:nat)
  : Lemma (ensures (sumat dl d a m <= sumat dl d a (m + k)))
  = let rec aux (j:nat)
      : Lemma (ensures (sumat dl d a m <= sumat dl d a (m + j))) (decreases j)
      = if j > 0 then begin
          (aux (j - 1));
          (sum_n_split dl d a (m + j))
        end
    in aux k

// the feefold itself: deltas from (FG, FS) snapshots; the growth-
// monotonicity invariant is discharged HERE, once, at the type level.
let feefold_sum (fg: int -> growth) (fs: int -> snap) (d:liq) (a b:int)
  : Pure int
      (requires (a <= b + 1 /\ (forall i. fs i <= fg i)))
      (ensures (fun _ -> True))
  = sum_n (fun i -> fg i - fs i) d a (b - a + 1) 0

// feefold is nonneg and range-monotone under its invariant
let feefold_nonneg (fg: int -> growth) (fs: int -> snap) (d:liq) (a b:int)
  : Lemma (requires (a <= b + 1 /\ (forall i. fs i <= fg i)))
          (ensures (0 <= feefold_sum fg fs d a b))
  = sumat_nonneg (fun i -> fg i - fs i) d a (b - a + 1)

let feefold_range_mono (fg: int -> growth) (fs: int -> snap) (d:liq) (a b1 b2:int)
  : Lemma (requires (a <= b1 /\ b1 <= b2 /\ (forall i. fs i <= fg i)))
          (ensures (feefold_sum fg fs d a b1 <= feefold_sum fg fs d a b2))
  = sumat_len_mono (fun i -> fg i - fs i) d a (b1 - a + 1) (b2 - b1)

// v2 target (tracked): raw-delta upper bound  fold <= (Σ raw deltas)*d/2^64
// then overflow safety when Σ(fg−fs)*d < 2^128*2^64. Needs a Σ bookkeeping
// lemma over the fold — deferred until v1 pins are green.

(* ---------- 4. sanity pin: two-point worked example ----------
   Position d=1e18 over points [5,6]:
     FG_5=100, FS_5=40 (delta 60); FG_6=0, FS_6=0 (delta 0)
   s = mulshr64 60 1e18 = floor(60e18 / 2^64) — hand-checkable,
   corpus-checkable against clmm_v5_ref.py. *)

let ex_liq : liq = 1000000000000000000

let example_two_point : int =
  feefold_sum (fun (i:int) -> if i = 5 then 100 else 0)
              (fun (i:int) -> if i = 5 then 40 else 0)
              ex_liq 5 6

let example_expected : int = op_Multiply 60 1000000000000000000 / two64

let example_pins () : Lemma (example_two_point == example_expected) = ()
