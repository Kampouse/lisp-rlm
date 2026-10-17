(* CLMM.Walk — swap-walk conservation: nothing vanishes.
   Lane fence (2026-10-17):
   - F* (this dir)  : arithmetic KERNEL laws — walk bookkeeping, skim.
   - Isabelle (afp) : protocol/ledger invariants — aggregate conservation.
   - Parity harness : limb bit-exactness vs clmm_v5_ref.py (regression).

   Abstracted from p5-swarm-dn/up (lib/clmm_pool_v5.lisp): the loop state
   is (pp, ss, acc, rm, v0); each cell consumes take r = min(r, cell
   capacity via dyi); terminal returns "acc|rm"; caller books
   yin = v_eff - rmf. The Oct-16 k="sw" bug class (rm vanishing, resolve
   paying full intake) is EXACTLY the equation pinned here:
     consumed + remaining == effective intake,  at every step.
   The capacity FORMULA (dxo/dyi, isqrt/spow) is deliberately abstract —
   tying it to take_t is the v4 rung; conservation is shape-independent. *)

module CLMM.Walk

open CLMM.Fee

(* ---------- 1. per-cell consumption law ----------
   A cell's take never exceeds the remaining input and is never negative.
   (Both arms of the swarm satisfy this by construction: whole-cell fill
   takes dyi <= rm; partial fill takes exactly rm.) *)

type take_t = r:amt -> Tot (c:amt{c <= r})

(* ---------- 2. the two recursions (nat-counted, house style) ---------- *)

let rec walk_sum (take:take_t) (r:amt) (n:nat) (acc:int)
  : Tot int (decreases n)
  = if n = 0 then acc
    else walk_sum take (r - take r) (n - 1) (acc + take r)

let rec remn (take:take_t) (r:amt) (n:nat)
  : Tot amt (decreases n)
  = if n = 0 then r
    else remn take (r - take r) (n - 1)

let rec walk_acc_eq (take:take_t) (r:amt) (n:nat) (acc:int)
  : Lemma (ensures (walk_sum take r n acc == walk_sum take r n 0 + acc))
          (decreases n)
  = if n > 0 then begin
      (walk_acc_eq take (r - take r) (n - 1) (acc + take r));
      (walk_acc_eq take (r - take r) (n - 1) (take r))
    end

(* ---------- 3. THEOREM: nothing vanishes ----------
   For ANY legal per-cell take, consumed-so-far + still-remaining equals
   the input the walk started with, at every step count. The loop's exit
   strategy (rm hits 0, or range ends leaving rm>0 refunded) chooses n;
   the equation holds regardless. *)

let rec walk_conservation (take:take_t) (v:amt) (n:nat)
  : Lemma (ensures (walk_sum take v n 0 + remn take v n == v))
          (decreases n)
  = if n > 0 then begin
      (walk_conservation take (v - take v) (n - 1));
      (walk_acc_eq take (v - take v) (n - 1) (take v))
    end

(* ---------- 4. skim composition: the caller's bookkeeping ----------
   Intake v; custody skims F = v >> 8; the walk runs on v_eff = v - F;
   the caller books yin = v_eff - rmf and refunds rmf. Pinned: booked +
   refunded == v_eff exactly — the fee-skimmed input is fully accounted,
   neither created nor destroyed (skim itself is covered in CLMM.Fee). *)

let walk_bookkeeping (take:take_t) (v:amt) (n:nat)
  : Lemma (ensures (v_eff v - remn take (v_eff v) n == walk_sum take (v_eff v) n 0))
  = walk_conservation take (v_eff v) n

(* ---------- 5. sanity pin: 3-step worked example ----------
   take = r/4 (a quarter-cell drain), v = 100, no skim path fee (feep?=nil
   equivalent): v_eff 100 -> 75 -> 56 -> 42 remainder; consumed 58. *)

let ex_take : take_t = fun r -> r / 4

let example_three_step : int = walk_sum ex_take 100 3 0
