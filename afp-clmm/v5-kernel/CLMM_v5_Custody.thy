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
   Scratch_Custody (parent HOL-Library), builds 1-11, EXIT:0 2026-10-18.
   Promoted into session CLMM_v5_Kernel same day; scope boundary
   (burnc owed stacking vs float) documented in the tail comment.
 *)
theory CLMM_v5_Custody
  imports "HOL-Library.Code_Target_Numeral"
begin

type_synonym acct = string

record cust =
  esc    :: "acct \<Rightarrow> nat"
  book   :: "acct \<Rightarrow> nat"
  owed   :: "acct \<Rightarrow> nat"
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
| "cstep (EClaim v) s0 =
    (if v = 0 \<or> owed s0 ''E'' < v \<or> esc s0 ''E'' < v \<or> held s0 < v then s0
     else s0 \<lparr> esc := osub (esc s0) ''E'' v,
               owed := osub (owed s0) ''E'' v,
               held := held s0 - v, paid := paid s0 + v \<rparr>)"
| "cstep (ECancel w v) s0 =
    (if v = 0 \<or> pend s0 w < v \<or> esc s0 ''E'' < v \<or> held s0 < v then s0
     else s0 \<lparr> esc := osub (esc s0) ''E'' v,
               pend := osub (pend s0) w v,
               held := held s0 - v, paid := paid s0 + v \<rparr>)"
| "cstep (EBurn cx cy) s0 =
    (if esc s0 ''E'' < cx + cy then s0
     else s0 \<lparr> owed := oadd (owed s0) ''E'' (cx + cy) \<rparr>)"

fun crun :: "cust \<Rightarrow> ev list \<Rightarrow> cust" where
  "crun s [] = s"
| "crun s (e # es) = crun (cstep e s) es"

definition esc_1 :: "cust \<Rightarrow> nat" where "esc_1 s = esc s ''E''"
definition book_1 :: "cust \<Rightarrow> nat" where "book_1 s = book s ''E''"

definition czero :: cust where
  "czero = \<lparr> esc = \<lambda>_. 0, book = \<lambda>_. 0, owed = \<lambda>_. 0,
              pend = \<lambda>_. 0, held = 0, intake = 0, paid = 0 \<rparr>"

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
  proof (cases "v = 0 \<or> owed s ''E'' < v \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using EClaim True fund by (simp add: esc_1_def book_1_def)
  next
    case False
    have vle: "v \<le> esc s ''E''" using False by simp
    have eh: "esc s ''E'' \<le> held s" using fund by (simp add: esc_1_def book_1_def)
    show ?thesis using EClaim False fund vle eh
      by (simp add: esc_1_def book_1_def)
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
  proof (cases "v = 0 \<or> owed s ''E'' < v \<or> esc s ''E'' < v \<or> held s < v")
    case True show ?thesis using EClaim True hp by simp
  next
    case False
    have vle: "v \<le> held s" using False by simp
    show ?thesis using EClaim False hp vle by simp
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

(* Scope boundary (2026-10-18): owed is capped per-burnc at the CURRENT
   float (p5-req-cap), so back-to-back burnc calls can stack owed past
   esc; an underfunded claim then traps (li-sub borrow = wasm
   Unreachable = rollback) and retries until the float refills.
   Conservation and funding are unaffected (the trap is token-neutral);
   a liveness fix belongs in the contract: track the per-side owed
   total and cap new owed at esc minus that total. *)

end
