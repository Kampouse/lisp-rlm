(* CLMM.Cell — per-cell swap math: capacity soundness.
   Lane fence (2026-10-17):
   - F* (this dir)  : arithmetic KERNEL laws — cell in/out, capacity.
   - Isabelle (afp) : protocol/ledger invariants.
   - Parity harness : limb bit-exactness vs clmm_v5_ref.py (regression).

   Mirrors lib/clmm_pool_v5.lisp / clmm_v5_kernel.lisp closed forms:
     dy(L,S,S2)  = L*(S2-S) >> 64          (move output; dyi = down-face)
     dx(L,S,S2)  = L*(S2-S) // (S*S2)      (move input;  dxo = down-face)
     tstar       = (2^64*(dy+1) - 1) // lq  (exact dy-solver core, p5-tstar!)
     sfdyi       = max(1, S - tstar)       (partial-fill price)
   dy_out IS mulshr64 on the move — monotonicity/dust/le_raw come free
   from CLMM.Fee. The money lemma: tstar is the EXACT capacity — the
   kernel comment's informal "proofs" made theorems here. *)

module CLMM.Cell

open CLMM.Fee

type price = s:int{0 < s /\ s < u128_max}

(* ---------- 1. the two faces of a cell move ---------- *)

(* output of moving price lo -> hi (dy face; dyi = this on the down-walk) *)
let dy_out (lq:liq) (lo hi:price)
  : Pure int (requires (lo <= hi)) (ensures (fun _ -> True))
  = mulshr64 (hi - lo) lq

(* input cost of moving price lo -> hi (dx face) *)
let dx_out (lq:liq) (lo hi:price)
  : Pure int (requires (lo <= hi)) (ensures (fun _ -> True))
  = op_Multiply lq (hi - lo) / op_Multiply lo hi

(* ---------- 2. wellformedness: no negative output ---------- *)

let dy_out_nonneg (lq:liq) (lo hi:price)
  : Lemma (requires (lo <= hi)) (ensures (0 <= dy_out lq lo hi /\ 0 <= dx_out lq lo hi))
  = (mulshr64_nonneg (hi - lo) lq);
    FStar.Math.Lemmas.lemma_div_le 0 (op_Multiply lq (hi - lo)) (op_Multiply lo hi)

(* ---------- 3. THE MONEY LEMMA: tstar is the exact dy-capacity ----------
   tstar lq dy = largest move t with dy_out(lq, s, s+t) <= dy:
     - filling tstar units does NOT exceed dy (no overfill)
     - one unit more DOES exceed (no underfill — it is the max)
   This is the soundness of p5-tstar!/sfdyi: the partial-fill price is
   computed from the exact capacity, not an approximation. *)

let tstar (lq:pos) (dy:amt) : Tot int = (op_Multiply two64 (dy + 1) - 1) / lq

let dy_capacity_sound (lq:liq) (dy:amt) (s:price)
  : Lemma (requires (lq > 0 /\ s + tstar lq dy + 1 < u128_max))
          (ensures (dy_out lq s (s + tstar lq dy) <= dy
                    /\ dy < dy_out lq s (s + tstar lq dy + 1)))
  = // N = 2^64*(dy+1) - 1; tstar = N/lq. Div facts:
    //   L*tstar <= N  and  N < L*(tstar+1)
    FStar.Math.Lemmas.lemma_div_mod (op_Multiply two64 (dy + 1) - 1) lq;
    FStar.Math.Lemmas.lemma_mod_lt (op_Multiply two64 (dy + 1) - 1) lq;
    // upper: L*tstar <= N < 2^64*(dy+1)  =>  floor(L*tstar/2^64) <= dy
    FStar.Math.Lemmas.lemma_div_le (op_Multiply lq (tstar lq dy))
      (op_Multiply two64 (dy + 1) - 1) two64;
    FStar.Math.Lemmas.lemma_div_le (op_Multiply two64 dy)
      (op_Multiply two64 (dy + 1) - 1) two64;
    // lower: 2^64*(dy+1) <= L*(tstar+1)  =>  dy+1 <= floor(L*(tstar+1)/2^64)
    FStar.Math.Lemmas.lemma_div_le (op_Multiply two64 (dy + 1))
      (op_Multiply lq (tstar lq dy + 1)) two64

(* ---------- 4. bridge to the walk: the swarm take is legal ----------
   Swarm branch (p5-swarm-dn/up): if capacity < remaining then take =
   capacity (whole cell) else take = remaining (partial fill, via sfdyi
   which consumes exactly rm). Both faces never exceed the remainder and
   are nonneg — i.e. they inhabit Walk.take_t. Conservation (CLMM.Walk)
   then applies to the real walk. *)

let swarm_take_legal (dyi:int) (rm:amt)
  : Lemma (requires (0 <= dyi))
          (ensures ((if dyi < rm then dyi else rm) <= rm
                    /\ 0 <= (if dyi < rm then dyi else rm)))
  = ()

(* and the down-walk cell capacity IS nonneg — dyi is a dy_out face *)
let dyi_nonneg (lq:liq) (hi lo:price)
  : Lemma (requires (lo <= hi))
          (ensures (0 <= dy_out lq lo hi))
  = mulshr64_nonneg (hi - lo) lq
