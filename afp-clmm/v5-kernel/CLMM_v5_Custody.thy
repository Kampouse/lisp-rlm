(* Title: CLMM v5 custody layer - exact token-conservation machine
   Author: Hermes agent session, 2026-10-18

   Per-side (X/Y symmetric) token machine for lib/clmm_pool_v5.lisp.
   Buckets: esc (6|ESCX/Y custody float), book (6|X/Y input books),
   owed (6|OW burnc withdrawals), pend (6|PM pending-mint slice),
   held (raw ft2 pool balance), intake (settled kept amts), paid
   (delivered out-legs). Guard failure = identity: a rejected op either
   traps (limb-sub borrow = wasm Unreachable = rollback) or refunds via
   resolve/callback - both token-neutral. Payout legs guard on BOTH the
   esc float (accounting cap, p5-req-cap / p5-leg-decide) and held (an
   ft2 transfer above the pool balance fails at the token contract).

   Laws (all machine-verified, deterministic tactics: simp/cases/
   linarith - nothing admitted, no unbounded tactics):
     run_funded:  esc + book <= held            (funded-ledger safety)
     run_hp:      held + paid = intake          (exact conservation)

   Probe provenance: /tmp/custody-probe/Scratch_Custody.thy, session
   Scratch_Custody (parent HOL-Library), builds 1-17, EXIT:0 2026-10-18
   (builds 12-17 add the owed-total cap: otot bucket, ERR_FLOAT room
   guard on burnc, claim drain; laws 3/4 = owed-eq + burn-room).
   Promoted into session CLMM_v5_Kernel same day.
 *)
theory CLMM_v5_Custody
  imports "HOL-Library.Code_Target_Numeral"
begin

type_synonym acct = string

record cust =
  esc    :: "acct \<Rightarrow> nat"
  book   :: "acct \<Rightarrow> nat"
  owed   :: "acct \<Rightarrow> nat"
  otot   :: nat                          (* 6|OWT: per-side owed total *)
  pend   :: "acct \<Rightarrow> nat"
  held   :: nat
  intake :: nat
  paid   :: nat

datatype src = SDon | SSeed | SMint1 | SMint2 nat | SSwap nat nat

datatype ev =
    EIn src nat
  | ERefund nat
  | EClaim nat
  | ECancel acct nat
  | EBurn nat nat

fun oadd :: "(acct \<Rightarrow> nat) \<Rightarrow> acct \<Rightarrow> nat \<Rightarrow> acct \<Rightarrow> nat" where
  "oadd f k v k' = (if k = k' then f k' + v else f k')"

fun osub :: "(acct \<Rightarrow> nat) \<Rightarrow> acct \<Rightarrow> nat \<Rightarrow> acct \<Rightarrow> nat" where
  "osub f k v k' = (if k = k' then f k' - v else f k')"

(* The token machine, one side (X/Y symmetric). Guard failure = identity:
   a rejected op either traps (limb-sub borrow policy = wasm Unreachable
   = full rollback) or refunds via resolve/callback; both are
   token-neutral. Payout legs guard on BOTH the esc float (accounting
   cap, p5-req-cap / leg-decide) and the raw balance (an ft2 transfer
   above the pool balance fails at the token contract; the callback
   reversal makes it identity). *)

fun cstep :: "ev \<Rightarrow> cust \<Rightarrow> cust" where
  "cstep (EIn SDon amt) s0 =
    (if amt = 0 then s0
     else s0 \<lparr> held := held s0 + amt, intake := intake s0 + amt \<rparr>)"
| "cstep (EIn SSeed amt) s0 =
    (if amt = 0 then s0
     else s0 \<lparr> esc := oadd (esc s0) ''E'' amt,
               held := held s0 + amt, intake := intake s0 + amt \<rparr>)"
| "cstep (EIn SMint1 amt) s0 =
    (if amt = 0 then s0
     else s0 \<lparr> esc := oadd (esc s0) ''E'' amt,
               pend := oadd (pend s0) ''P'' amt,
               held := held s0 + amt, intake := intake s0 + amt \<rparr>)"
| "cstep (EIn (SMint2 req) amt) s0 =
    (if amt = 0 \<or> amt < req then s0
     else s0 \<lparr> esc := oadd (esc s0) ''E'' req,
               held := held s0 + amt, intake := intake s0 + amt \<rparr>)"
| "cstep (EIn (SSwap g f) amt) s0 =
    (if amt = 0 \<or> g + f \<noteq> amt then s0
     else s0 \<lparr> esc := oadd (esc s0) ''E'' f,
               book := oadd (book s0) ''E'' g,
               held := held s0 + amt, intake := intake s0 + amt \<rparr>)"
| "cstep (ERefund v) s0 =
    (if v = 0 \<or> esc s0 ''E'' < v \<or> held s0 < v then s0
     else s0 \<lparr> esc := osub (esc s0) ''E'' v,
               held := held s0 - v, paid := paid s0 + v \<rparr>)"
(* Claim payout: stock = esc + book (one pool-side token stock; a swap
   out-leg moves it esc-to-book). Reclassify book-to-esc first if the
   float alone cannot cover, then drain esc: the exact two-case
   sequence the contract runs. *)
| "cstep (EClaim v) s0 =
    (if v = 0 \<or> owed s0 ''E'' < v \<or> esc s0 ''E'' + book s0 ''E'' < v
        \<or> held s0 < v \<or> otot s0 < v then s0
     else if esc s0 ''E'' < v
     then s0 \<lparr> esc := osub (esc s0) ''E'' (esc s0 ''E''),
               book := osub (book s0) ''E'' (v - esc s0 ''E''),
               owed := osub (owed s0) ''E'' v,
               otot := otot s0 - v,
               held := held s0 - v, paid := paid s0 + v \<rparr>
     else s0 \<lparr> esc := osub (esc s0) ''E'' v,
               owed := osub (owed s0) ''E'' v,
               otot := otot s0 - v,
               held := held s0 - v, paid := paid s0 + v \<rparr>)"
| "cstep (ECancel w v) s0 =
    (if v = 0 \<or> pend s0 w < v \<or> esc s0 ''E'' < v \<or> held s0 < v then s0
     else s0 \<lparr> esc := osub (esc s0) ''E'' v,
               pend := osub (pend s0) w v,
               held := held s0 - v, paid := paid s0 + v \<rparr>)"
| "cstep (EBurn cx cy) s0 =
    (if esc s0 ''E'' < otot s0 + cx + cy then s0
     else s0 \<lparr> owed := oadd (owed s0) ''E'' (cx + cy),
               otot := otot s0 + cx + cy \<rparr>)"

fun crun :: "cust \<Rightarrow> ev list \<Rightarrow> cust" where
  "crun s [] = s"
| "crun s (e # es) = crun (cstep e s) es"

definition esc_1 :: "cust \<Rightarrow> nat" where "esc_1 s = esc s ''E''"
definition book_1 :: "cust \<Rightarrow> nat" where "book_1 s = book s ''E''"

definition czero :: cust where
  "czero = \<lparr> esc = \<lambda>_. 0, book = \<lambda>_. 0, owed = \<lambda>_. 0,
              otot = 0, pend = \<lambda>_. 0,
              held = 0, intake = 0, paid = 0 \<rparr>"

(* Law 1 (funded ledgers): pool-side buckets never exceed the raw
   balance. Intake adds held together with its bucket; payout legs
   debit both under one guard; burnc adds no bucket. *)

theorem step_funded:
  assumes fund: "esc_1 s + book_1 s \<le> held s"
  shows "esc_1 (cstep e s) + book_1 (cstep e s) \<le> held (cstep e s)"
  using fund
proof (cases e)
  case (EIn src amt)
  then show ?thesis using fund by (cases src) (simp_all add: esc_1_def book_1_def)
next
  case (ERefund v)
  show ?thesis
  proof (cases "v = 0 \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using ERefund True fund by (simp add: esc_1_def book_1_def)
  next
    case False
    have vle: "v \<le> esc s ''E''" using False by simp
    have eh: "esc s ''E'' \<le> held s" using fund by (simp add: esc_1_def book_1_def)
    show ?thesis using ERefund False fund vle eh
      by (simp add: esc_1_def book_1_def)
  qed
next
  case (EClaim v)
  show ?thesis
  proof (cases "v = 0 \<or> owed s ''E'' < v \<or> esc s ''E'' + book s ''E'' < v
                  \<or> held s < v \<or> otot s < v")
    case True show ?thesis using EClaim True fund by (simp add: esc_1_def book_1_def)
  next
    case False
    have stock: "v \<le> esc s ''E'' + book s ''E''" using False by simp
    have eh: "esc s ''E'' \<le> held s" using fund by (simp add: esc_1_def book_1_def)
    have bh: "book s ''E'' \<le> held s" using fund by (simp add: esc_1_def book_1_def)
    show ?thesis
    proof (cases "esc s ''E'' < v")
      case True
      show ?thesis using EClaim True fund stock eh bh
        by (simp add: esc_1_def book_1_def) linarith
    next
      case False
      show ?thesis using EClaim False fund stock eh
        by (simp add: esc_1_def book_1_def) linarith
    qed
  qed
next
  case (ECancel w v)
  show ?thesis
  proof (cases "v = 0 \<or> pend s w < v \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using ECancel True fund by (simp add: esc_1_def book_1_def)
  next
    case False
    have vle: "v \<le> esc s ''E''" using False by simp
    have eh: "esc s ''E'' \<le> held s" using fund by (simp add: esc_1_def book_1_def)
    show ?thesis using ECancel False fund vle eh
      by (simp add: esc_1_def book_1_def)
  qed
next
  case (EBurn cx cy)
  show ?thesis
  proof (cases "esc s ''E'' < cx + cy")
    case True show ?thesis using EBurn True fund by (simp add: esc_1_def book_1_def)
  next
    case False show ?thesis using EBurn False fund by (simp add: esc_1_def book_1_def)
  qed
qed

theorem run_funded:
  assumes fund: "esc_1 s + book_1 s \<le> held s"
  shows "esc_1 (crun s es) + book_1 (crun s es) \<le> held (crun s es)"
  using fund
proof (induction es arbitrary: s)
  case Nil show ?case using Nil.prems by simp
next
  case (Cons e es)
  have step: "esc_1 (cstep e s) + book_1 (cstep e s) \<le> held (cstep e s)"
    using step_funded[OF Cons.prems] .
  show ?case using step Cons by simp
qed

theorem czero_funded:
  "esc_1 (crun czero es) + book_1 (crun czero es) \<le> held (crun czero es)"
  by (rule run_funded) (simp add: czero_def esc_1_def book_1_def)

(* Law 2 (exact conservation): held + paid = intake is invariant.
   Payout legs move v from held to paid under guards that make the
   nat subtraction exact; intake moves only with held. *)

theorem step_hp:
  assumes hp: "held s + paid s = intake s"
  shows "held (cstep e s) + paid (cstep e s) = intake (cstep e s)"
  using hp
proof (cases e)
  case (EIn src amt)
  then show ?thesis using hp by (cases src) simp_all
next
  case (ERefund v)
  show ?thesis
  proof (cases "v = 0 \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using ERefund True hp by simp
  next
    case False
    have vle: "v \<le> held s" using False by simp
    show ?thesis using ERefund False hp vle by simp
  qed
next
  case (EClaim v)
  show ?thesis
  proof (cases "v = 0 \<or> owed s ''E'' < v \<or> esc s ''E'' + book s ''E'' < v
                  \<or> held s < v \<or> otot s < v")
    case True show ?thesis using EClaim True hp by simp
  next
    case False
    have vle: "v \<le> held s" using False by simp
    show ?thesis
    proof (cases "esc s ''E'' < v")
      case True show ?thesis using EClaim True False hp vle by simp
    next
      case False show ?thesis using EClaim False False hp vle by simp
    qed
  qed
next
  case (ECancel w v)
  show ?thesis
  proof (cases "v = 0 \<or> pend s w < v \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using ECancel True hp by simp
  next
    case False
    have vle: "v \<le> held s" using False by simp
    show ?thesis using ECancel False hp vle by simp
  qed
next
  case (EBurn cx cy)
  show ?thesis
  proof (cases "esc s ''E'' < cx + cy")
    case True show ?thesis using EBurn True hp by simp
  next
    case False show ?thesis using EBurn False hp by simp
  qed
qed

theorem run_hp:
  assumes hp: "held s + paid s = intake s"
  shows "held (crun s es) + paid (crun s es) = intake (crun s es)"
  using hp
proof (induction es arbitrary: s)
  case Nil show ?case using Nil.prems by simp
next
  case (Cons e es)
  have step: "held (cstep e s) + paid (cstep e s) = intake (cstep e s)"
    using step_hp[OF Cons.prems] .
  show ?case using step Cons by simp
qed

theorem czero_hp:
  "held (crun czero es) + paid (crun czero es) = intake (crun czero es)"
  by (rule run_hp) (simp add: czero_def)

(* Law 3 (owed ledger = owed total): the aggregate owed ledger and the
   per-side owed total move together on every op (burnc up, claim
   down), so they never diverge. *)

theorem step_owed_eq:
  assumes eq: "owed s ''E'' = otot s"
  shows "owed (cstep e s) ''E'' = otot (cstep e s)"
proof (cases e)
  case (EIn src amt)
  then show ?thesis using eq by (cases src) simp_all
next
  case ERefund show ?thesis using ERefund eq by simp
next
  case (EClaim v)
  show ?thesis
  proof (cases "v = 0 \<or> owed s ''E'' < v \<or> esc s ''E'' + book s ''E'' < v
                  \<or> held s < v \<or> otot s < v")
    case True show ?thesis using EClaim True eq by simp
  next
    case False
    show ?thesis using EClaim False eq by simp
  qed
next
  case ECancel show ?thesis using ECancel eq by simp
next
  case (EBurn cx cy)
  show ?thesis
  proof (cases "esc s ''E'' < otot s + cx + cy")
    case True show ?thesis using EBurn True eq by simp
  next
    case False
    show ?thesis using EBurn False eq by simp
  qed
qed

theorem run_owed_eq:
  assumes eq: "owed s ''E'' = otot s"
  shows "owed (crun s es) ''E'' = otot (crun s es)"
  using eq
proof (induction es arbitrary: s)
  case Nil show ?case using Nil.prems by simp
next
  case (Cons e es)
  have step: "owed (cstep e s) ''E'' = otot (cstep e s)"
    using step_owed_eq[OF Cons.prems] .
  show ?case using step Cons by simp
qed

theorem run_owed_eq_zero:
  "owed (crun czero es) ''E'' = otot (crun czero es)"
  by (rule run_owed_eq) (simp add: czero_def)

(* Law 4 (burn-time room): every SETTLED burnc leaves the owed total
   within the float - the ERR_FLOAT cap as a theorem. Not claimed
   globally: later refund/cancel legs may dip esc below otot
   (counterexample: seed 10, EBurn 10 0, ERefund 3). *)

theorem burn_room:
  "cstep (EBurn cx cy) s = s
   \<or> otot (cstep (EBurn cx cy) s) \<le> esc (cstep (EBurn cx cy) s) ''E''"
proof (cases "esc s ''E'' < otot s + cx + cy")
  case True show ?thesis using True by simp
next
  case False
  have lt: "otot s + cx + cy \<le> esc s ''E''" using False by simp
  show ?thesis using False lt by simp
qed

(* The 2026-10-18 owed-total cap (contract 6|OWTX/6|OWTY, ERR_FLOAT)
   closes the stacking class this theory previously documented as a
   scope boundary: burnc now requires otot + (cx + cy) \<le> esc and
   claims drain the total, so a settled burnc is provably covered at
   burn time (law 4) with the aggregate ledger in lockstep (law 3). *)


(* ─── Swap-with-refund composition (2026-10-18, contract b535cae4) ───

A committed partial-fill swap is TWO machine events: the intake
[EIn (SSwap g f) amt] (g = walked amount, f = fee + rem) followed by
ERefund rem. leg-decide releases exactly the refund slice from the
intake escrow (esc += amt at the arm, esc -= rem at the commit), so
the composite nets: esc +fee, book +walked, held +(amt - rem), paid
+rem. Full-keep is the rem = 0 corner. Uncommitted/degenerate legs and
mo-guard aborts (out < mo ⇒ p5-fail on the whole receipt) are ZERO
events - guard failure = identity, the model's stated policy. *)

definition swap_events :: "nat \<Rightarrow> nat \<Rightarrow> nat \<Rightarrow> ev list" where
  "swap_events amt fee rem =
     [EIn (SSwap (amt - fee - rem) (fee + rem)) amt, ERefund rem]"

theorem swap_net:
  assumes le: "fee + rem \<le> amt"
  shows "crun s (swap_events amt fee rem) =
    s\<lparr>esc := oadd (esc s) ''E'' fee,
            book := oadd (book s) ''E'' (amt - fee - rem),
            held := held s + (amt - rem),
            intake := intake s + amt,
            paid := paid s + rem\<rparr>"
proof (cases "amt = 0")
  case True
  then have z: "fee + rem = 0" using le by simp
  then show ?thesis using True by (simp add: swap_events_def ext)
next
  case False
  have g: "amt - fee - rem + (fee + rem) = amt" using le by simp
  have guard: "\<not> (amt = 0 \<or> amt - fee - rem + (fee + rem) \<noteq> amt)"
    using False g by simp
  have hr: "rem \<le> esc s ''E'' + (fee + rem)" by simp
  have hh: "rem \<le> held s + amt" using le by simp
  have hf1: "\<not> (esc s ''E'' + (fee + rem) < rem)" by simp
  have hf2: "\<not> (held s + amt < rem)" using le by simp
  show ?thesis using guard hf1 hf2 by (simp add: swap_events_def ext)
qed

theorem swap_run_funded:
  assumes fund: "esc_1 s + book_1 s \<le> held s" and le: "fee + rem \<le> amt"
  shows "esc_1 (crun s (swap_events amt fee rem))
         + book_1 (crun s (swap_events amt fee rem))
         \<le> held (crun s (swap_events amt fee rem))"
proof -
  from le have g: "amt - fee - rem + fee = amt - rem" by simp
  have t1: "esc s ''E'' + fee + (book s ''E'' + (amt - fee - rem))
          = esc s ''E'' + book s ''E'' + (amt - rem)" using g by linarith
  show ?thesis using fund swap_net[OF le] t1
    by (simp add: esc_1_def book_1_def)
qed

theorem swap_run_hp:
  assumes hp: "held s + paid s = intake s" and le: "fee + rem \<le> amt"
  shows "held (crun s (swap_events amt fee rem))
         + paid (crun s (swap_events amt fee rem))
         = intake (crun s (swap_events amt fee rem))"
proof -
  from le have g: "amt - rem + rem = amt" by simp
  show ?thesis using hp swap_net[OF le] g by simp
qed

theorem swap_run_owed_eq:
  assumes eq: "owed s ''E'' = otot s" and le: "fee + rem \<le> amt"
  shows "owed (crun s (swap_events amt fee rem)) ''E''
         = otot (crun s (swap_events amt fee rem))"
  using eq swap_net[OF le] by simp

(* Full-keep corner (rem = 0): the refund leg is a no-op, so the run
   reduces to the intake alone - the pre-b535cae4 model case. *)
lemma swap_full_keep_run:
  "crun s (swap_events amt fee 0) = cstep (EIn (SSwap (amt - fee) fee) amt) s"
  by (simp add: swap_events_def)

end