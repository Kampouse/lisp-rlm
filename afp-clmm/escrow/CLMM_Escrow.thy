theory CLMM_Escrow
  imports "Concentrated_Liquidity_Market_Making_Operations.CLMM_Description"

(* Escrow ledger model for the lisp-rlm CLMM platform shell.
   Provenance: sibling theory files (CLMM_Description et al.) are the AFP entry
   "Concentrated_Liquidity_Market_Making_Operations" port. This file is NEW
   platform work (not part of the AFP entry) and follows its conventions.

   What this theory claims: every token-moving operation of the escrow shell
   preserves the conservation law

       reserve + sum(credits) = inflow - outflow

   i.e. no operation can mint, lose, or strand tokens. This is the property
   Certora verifies for Uniswap v4 Core (P-01 solvency / ERC-6909 backing);
   here it is a total theorem over unbounded sequences, not an SMT-bounded rule.

   Modelling decisions (deliberate):
   - Tokens are exact units (nat). u128 truncation is owned by the generated
     lisp/TS twins + differential harness (port law: truncation never crosses
     into the model; the model proves direction, twins prove bit-parity).
   - SwapIn/SwapOut move value between a user's credit and the reserve WITHIN
     one token-side escrow. A cross-token swap (A -> B) is SwapIn on escrow-A
     composed with SwapOut on escrow-B; EACH escrow is conserved
     independently, and the exchange rate between the sides is the curve's
     business (base_swap / quote_net, formalized and proved in the imported
     CLMM_Description: monotone, closed difference equation, zero at
     grd_min), not the ledger's. `outflow` moves only on Withdraw: actual
     exit of tokens from contract custody.
   - The ledger needs NO curve assumption: roundtrip_neutral holds for ANY
     priced pair (x, y), so whatever the curve computes, custody is
     unaffected. The curve's arithmetic (incl. rounding) enters at the
     generated twins, not this layer.

   Status: proofs landed (phase 1). The former 6-sorry skeleton is fully
   discharged; the session builds WITHOUT quick_and_dirty, so any remaining
   `sorry` fails the build. One statement changed during proving:
   deposit conservation gained the hypothesis `finite (holders e)` (it is
   FALSE without it: with infinitely many holders, both sums are 0 and a
   deposit raises no visible balance), a bug the proof attempt caught.
   Exactly what skeleton-then-prove is for. *)

begin

section \<open>Ledger state\<close>

type_synonym user = nat

record escrow =
  credits :: "user \<Rightarrow> nat"   (* units the contract owes each user *)
  reserve :: nat                (* contract-owned buffer (pool-side inventory) *)
  inflow  :: nat                (* cumulative tokens deposited into escrow *)
  outflow :: nat                (* cumulative tokens paid out of escrow *)

definition holders :: "escrow \<Rightarrow> user set" where
  "holders e = {u. credits e u > 0}"

definition balance :: "escrow \<Rightarrow> nat" where
  "balance e = reserve e + sum (credits e) (holders e)"

definition escrow_ok :: "escrow \<Rightarrow> bool" where
  "escrow_ok e \<longleftrightarrow> finite (holders e) \<and> outflow e \<le> inflow e
     \<and> balance e = inflow e - outflow e"
(* the `outflow <= inflow` conjunct keeps the nat subtraction honest:
   without it, a state paying out more than it ever received could still
   satisfy the equation via truncation at zero *)

lemma holders_finiteI[intro]:
  assumes "finite {u. credits e u > 0}"
  shows "finite (holders e)"
  using assms unfolding holders_def by simp

lemma balance_ok:
  assumes "escrow_ok e"
  shows "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
  using assms unfolding escrow_ok_def balance_def by simp

section \<open>Operations\<close>

datatype escrow_op =
  Deposit  user nat      (* NEP-141 ft_transfer_call received *)
| Withdraw user nat      (* user redemption, paid from credits *)
| SwapIn  user nat       (* user pays for a swap out of escrowed credit *)
| SwapOut user nat       (* swap output credited to user, drawn from reserve *)

fun apply_op :: "escrow_op \<Rightarrow> escrow \<Rightarrow> escrow option" where
  "apply_op (Deposit u a) e =
     (if 0 < a then Some (e \<lparr> credits := (credits e)(u := credits e u + a)
                            , inflow := inflow e + a \<rparr>) else None)"
| "apply_op (Withdraw u a) e =
     (if a \<le> credits e u then
        Some (e \<lparr> credits := (credits e)(u := credits e u - a)
               , outflow := outflow e + a \<rparr>)
      else None)"
| "apply_op (SwapIn u a) e =
     (if a \<le> credits e u then
        Some (e \<lparr> credits := (credits e)(u := credits e u - a)
               , reserve := reserve e + a \<rparr>)
      else None)"
| "apply_op (SwapOut u a) e =
     (if a \<le> reserve e then
        Some (e \<lparr> reserve := reserve e - a
               , credits := (credits e)(u := credits e u + a) \<rparr>)
      else None)"

fun apply_seq :: "escrow_op list \<Rightarrow> escrow \<Rightarrow> escrow option" where
  "apply_seq [] e = Some e"
| "apply_seq (op # ops) e = (case apply_op op e of None \<Rightarrow> None
     | Some e' \<Rightarrow> apply_seq ops e')"

text \<open>Genesis: a fresh deployment starts here, all-zero. Every other
  reachable state arises via \<open>apply_seq\<close>; the conservation theorems
  lift the invariant from this state through any operation sequence.\<close>

definition init :: escrow where
  "init = \<lparr> credits = (\<lambda>_. 0), reserve = 0, inflow = 0, outflow = 0 \<rparr>"

lemma init_ok[simp]: "escrow_ok init"
  unfolding escrow_ok_def holders_def balance_def init_def by auto

section \<open>Bookkeeping: point updates of the credit function\<close>

text \<open>All conservation reasoning reduces to how the holder sum reacts when a
  single user's credit is overwritten: the user either keeps holder status or
  loses it (or joins it). These two shapes are proved once, here.\<close>

lemma holders_upd_finite:
  assumes "finite (holders e)"
  shows "finite (holders (e\<lparr>credits := (credits e)(u := c)\<rparr>))"
proof (rule finite_subset)
  show "holders (e\<lparr>credits := (credits e)(u := c)\<rparr>) \<subseteq> insert u (holders e)"
    by (auto simp: holders_def)
qed (insert assms, simp)

lemma sum_upd_member:
  assumes fin: "finite (holders e)" and mem: "u \<in> holders e"
  shows "sum (credits e) (holders e) - credits e u + c
       = sum ((credits e)(u := c))
             (holders (e\<lparr>credits := (credits e)(u := c)\<rparr>))"
proof (cases "c = 0")
  case True
  have split_sum: "sum (credits e) (holders e)
                 = credits e u + sum (credits e) (holders e - {u})"
    using fin mem by (rule sum.remove)
  have set_eq: "holders (e\<lparr>credits := (credits e)(u := c)\<rparr>) = holders e - {u}"
    using mem True by (auto simp: holders_def)
  have rw: "sum ((credits e)(u := c)) (holders e - {u})
          = sum (credits e) (holders e - {u})"
    by (rule sum.cong) (auto simp: holders_def)
  show ?thesis using split_sum set_eq rw True by simp
next
  case False
  then have cpos: "0 < c" by simp
  have split_sum: "sum (credits e) (holders e)
                 = credits e u + sum (credits e) (holders e - {u})"
    using fin mem by (rule sum.remove)
  have set_eq: "holders (e\<lparr>credits := (credits e)(u := c)\<rparr>) = holders e"
    using mem cpos by (auto simp: holders_def)
  have he: "holders e = insert u (holders e - {u})" using mem by blast
  have rw: "sum ((credits e)(u := c)) (holders e)
          = c + sum (credits e) (holders e - {u})"
  proof -
    have f1: "sum ((credits e)(u := c)) (holders e)
            = ((credits e)(u := c)) u
            + sum ((credits e)(u := c)) (holders e - {u})"
      using fin mem by (rule sum.remove)
    have f2: "((credits e)(u := c)) u = c" by simp
    have f3: "sum ((credits e)(u := c)) (holders e - {u})
            = sum (credits e) (holders e - {u})"
      by (rule sum.cong) (auto simp: holders_def)
    show ?thesis using f1 f2 f3 by simp
  qed
  show ?thesis using split_sum rw set_eq by (simp add: set_eq)
qed

lemma sum_upd_fresh:
  assumes fin: "finite (holders e)" and fresh: "u \<notin> holders e"
  shows "sum (credits e) (holders e) + c
       = sum ((credits e)(u := c))
             (holders (e\<lparr>credits := (credits e)(u := c)\<rparr>))"
proof (cases "c = 0")
  case True
  have c0: "credits e u = 0" using fresh by (auto simp: holders_def)
  have idem: "(credits e)(u := 0) = credits e"
    using c0 by (simp add: fun_upd_idem_iff)
  have rec_eq: "e\<lparr>credits := (credits e)(u := 0)\<rparr> = e" by (simp add: idem)
  show ?thesis using True by (simp add: idem rec_eq)
next
  case False
  then have cpos: "0 < c" by simp
  have set_eq: "holders (e\<lparr>credits := (credits e)(u := c)\<rparr>) = insert u (holders e)"
    using fresh cpos by (auto simp: holders_def)
  have rw: "sum ((credits e)(u := c)) (insert u (holders e))
          = c + sum (credits e) (holders e)"
  proof -
    have f1: "sum ((credits e)(u := c)) (insert u (holders e))
            = ((credits e)(u := c)) u
            + sum ((credits e)(u := c)) (holders e)"
      using fin fresh by (rule sum.insert)
    have f2: "((credits e)(u := c)) u = c" by simp
    have f3: "sum ((credits e)(u := c)) (holders e) = sum (credits e) (holders e)"
    proof (rule sum.cong)
      show "holders e = holders e" by simp
    next
      fix x assume "x \<in> holders e"
      hence "x \<noteq> u" using fresh by (auto simp: holders_def)
      thus "((credits e)(u := c)) x = credits e x" by simp
    qed
    show ?thesis using f1 f2 f3 by simp
  qed
  show ?thesis unfolding set_eq rw by simp
qed

lemma credit_le_balance:
  assumes fin: "finite (holders e)"
  shows "credits e u \<le> balance e"
proof (cases "u \<in> holders e")
  case True
  have "sum (credits e) (holders e)
      = credits e u + sum (credits e) (holders e - {u})"
    using fin True by (rule sum.remove)
  then show ?thesis by (simp add: balance_def)
next
  case False
  then have c0: "credits e u = 0" by (auto simp: holders_def)
  show ?thesis by (simp add: balance_def c0)
qed

section \<open>Raw sum lemmas (exact record shapes of apply_op)\<close>

text \<open>Each lemma is stated over the exact record written by the operation, so
  the conservation theorems can chain them into \<open>arith\<close> without atom
  mismatch. `h0` bridges the holder set across the extra updated field.\<close>

lemma deposit_sum_raw:
  assumes fin: "finite (holders e)" and pos: "0 < a"
  shows "reserve e
       + sum ((credits e)(u := credits e u + a))
             (holders (e\<lparr>credits := (credits e)(u := credits e u + a)
                                   , inflow := inflow e + a\<rparr>))
       = (reserve e + sum (credits e) (holders e)) + a"
proof (cases "u \<in> holders e")
  case True
  have le: "credits e u \<le> sum (credits e) (holders e)"
  proof -
    have "sum (credits e) (holders e)
        = credits e u + sum (credits e) (holders e - {u})"
      using fin True by (rule sum.remove)
    then show ?thesis by simp
  qed
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u + a)
                              , inflow := inflow e + a\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u + a)\<rparr>)"
    by (simp add: holders_def)
  have h1: "sum (credits e) (holders e) - credits e u + (credits e u + a)
          = sum ((credits e)(u := credits e u + a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u + a)\<rparr>))"
    using sum_upd_member[OF fin True] .
  show ?thesis using h0 h1 le by arith
next
  case False
  have c0: "credits e u = 0" using False by (auto simp: holders_def)
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u + a)
                              , inflow := inflow e + a\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := a)\<rparr>)"
    by (simp add: c0 holders_def)
  have h1: "sum (credits e) (holders e) + a
          = sum ((credits e)(u := a))
                (holders (e\<lparr>credits := (credits e)(u := a)\<rparr>))"
    using sum_upd_fresh[OF fin False, of a] .
  have feq2: "(credits e)(u := credits e u + a) = (credits e)(u := a)"
    using c0 by simp
  have h0b: "holders (e\<lparr>credits := (credits e)(u := a)
                                  , inflow := inflow e + a\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := a)\<rparr>)"
    by (simp add: holders_def)
  have h2: "sum ((credits e)(u := credits e u + a))
              (holders (e\<lparr>credits := (credits e)(u := credits e u + a)
                                    , inflow := inflow e + a\<rparr>))
          = sum (credits e) (holders e) + a"
    using h1[symmetric] by (simp only: feq2 h0b)
  show ?thesis using h2 by arith
qed

lemma withdraw_sum_raw:
  assumes fin: "finite (holders e)" and g: "a \<le> credits e u"
  shows "reserve e
       + sum ((credits e)(u := credits e u - a))
             (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                   , outflow := outflow e + a\<rparr>))
       = (reserve e + sum (credits e) (holders e)) - a"
proof (cases "u \<in> holders e")
  case True
  have le: "credits e u \<le> sum (credits e) (holders e)"
  proof -
    have "sum (credits e) (holders e)
        = credits e u + sum (credits e) (holders e - {u})"
      using fin True by (rule sum.remove)
    then show ?thesis by simp
  qed
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , outflow := outflow e + a\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>)"
    by (simp add: holders_def)
  have h1: "sum (credits e) (holders e) - credits e u + (credits e u - a)
          = sum ((credits e)(u := credits e u - a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>))"
    using sum_upd_member[OF fin True, of "credits e u - a"] .
  show ?thesis using h0 h1 le g by arith
next
  case False
  have c0: "credits e u = 0" using False by (auto simp: holders_def)
  have a0: "a = 0" using g c0 by arith
  have feq: "(credits e)(u := credits e u - a) = credits e"
    using c0 a0 by (simp add: fun_upd_idem_iff)
  have rec0: "e\<lparr>credits := (credits e)(u := credits e u - a)
                        , outflow := outflow e + a\<rparr> = e"
    by (simp add: a0 feq)
  then have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , outflow := outflow e + a\<rparr>) = holders e"
    by simp
  show ?thesis using a0 feq h0 by (simp add: a0 feq h0)
qed

lemma swapin_sum_raw:
  assumes fin: "finite (holders e)" and g: "a \<le> credits e u"
  shows "reserve e + a
       + sum ((credits e)(u := credits e u - a))
             (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                   , reserve := reserve e + a\<rparr>))
       = reserve e + sum (credits e) (holders e)"
proof (cases "u \<in> holders e")
  case True
  have le: "credits e u \<le> sum (credits e) (holders e)"
  proof -
    have "sum (credits e) (holders e)
        = credits e u + sum (credits e) (holders e - {u})"
      using fin True by (rule sum.remove)
    then show ?thesis by simp
  qed
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>)"
    by (simp add: holders_def)
  have h1: "sum (credits e) (holders e) - credits e u + (credits e u - a)
          = sum ((credits e)(u := credits e u - a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>))"
    using sum_upd_member[OF fin True, of "credits e u - a"] .
  show ?thesis using h0 h1 le g by arith
next
  case False
  have c0: "credits e u = 0" using False by (auto simp: holders_def)
  have a0: "a = 0" using g c0 by arith
  have feq: "(credits e)(u := credits e u - a) = credits e"
    using c0 a0 by (simp add: fun_upd_idem_iff)
  have rec0: "e\<lparr>credits := (credits e)(u := credits e u - a)
                        , reserve := reserve e + a\<rparr> = e"
    by (simp add: a0 feq)
  then have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a\<rparr>) = holders e"
    by simp
  show ?thesis using a0 feq h0 by (simp add: a0 feq h0)
qed

lemma swapout_sum_raw:
  assumes fin: "finite (holders e)" and g: "a \<le> reserve e"
  shows "reserve e - a
       + sum ((credits e)(u := credits e u + a))
             (holders (e\<lparr>reserve := reserve e - a
                                   , credits := (credits e)(u := credits e u + a)\<rparr>))
       = reserve e + sum (credits e) (holders e)"
proof (cases "u \<in> holders e")
  case True
  have le: "credits e u \<le> sum (credits e) (holders e)"
  proof -
    have "sum (credits e) (holders e)
        = credits e u + sum (credits e) (holders e - {u})"
      using fin True by (rule sum.remove)
    then show ?thesis by simp
  qed
  have h0: "holders (e\<lparr>reserve := reserve e - a
                              , credits := (credits e)(u := credits e u + a)\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u + a)\<rparr>)"
    by (simp add: holders_def)
  have h1: "sum (credits e) (holders e) - credits e u + (credits e u + a)
          = sum ((credits e)(u := credits e u + a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u + a)\<rparr>))"
    using sum_upd_member[OF fin True, of "credits e u + a"] .
  show ?thesis using h0 h1 le g by arith
next
  case False
  have c0: "credits e u = 0" using False by (auto simp: holders_def)
  have upd_eq: "(credits e)(u := credits e u + a) = (credits e)(u := a)"
    using c0 by simp
  have h0: "holders (e\<lparr>reserve := reserve e - a
                              , credits := (credits e)(u := credits e u + a)\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := a)\<rparr>)"
    by (simp add: upd_eq holders_def)
  have h1: "sum (credits e) (holders e) + a
          = sum ((credits e)(u := a))
                (holders (e\<lparr>credits := (credits e)(u := a)\<rparr>))"
    using sum_upd_fresh[OF fin False, of a] .
  have h0b: "holders (e\<lparr>reserve := reserve e - a
                                  , credits := (credits e)(u := a)\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := a)\<rparr>)"
    by (simp add: holders_def)
  have h2: "sum ((credits e)(u := credits e u + a))
              (holders (e\<lparr>reserve := reserve e - a
                                    , credits := (credits e)(u := credits e u + a)\<rparr>))
          = sum (credits e) (holders e) + a"
    using h1[symmetric] by (simp only: upd_eq h0b)
  show ?thesis using h2 g by arith
qed

section \<open>Conservation theorems\<close>

theorem deposit_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (Deposit u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have pos: "0 < a"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u + a)
                          , inflow := inflow e + a\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>credits := (credits e)(u := credits e u + a)
                                  , inflow := inflow e + a\<rparr>))"
    using holders_upd_finite[OF fin] by (simp add: holders_def)
  have le2: "outflow e \<le> inflow e + a" using ok_le by arith
  have b2: "balance (e\<lparr>credits := (credits e)(u := credits e u + a)
                                  , inflow := inflow e + a\<rparr>) = inflow e + a - outflow e"
    using deposit_sum_raw[OF fin pos] ok_bal ok_le by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 le2 b2 by simp
qed

theorem withdraw_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (Withdraw u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have g: "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , outflow := outflow e + a\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have c_le_s: "credits e u \<le> reserve e + sum (credits e) (holders e)"
    using credit_le_balance[OF fin] by (simp add: balance_def)
  have le: "outflow e + a \<le> inflow e" using g c_le_s ok_bal ok_le by arith
  have f2: "finite (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , outflow := outflow e + a\<rparr>))"
    using holders_upd_finite[OF fin] by (simp add: holders_def)
  have b2: "balance (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , outflow := outflow e + a\<rparr>) = inflow e - (outflow e + a)"
    using withdraw_sum_raw[OF fin g] ok_bal le by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 le b2 by simp
qed

theorem swapin_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (SwapIn u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have g: "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , reserve := reserve e + a\<rparr>))"
    using holders_upd_finite[OF fin] by (simp add: holders_def)
  have b2: "balance (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , reserve := reserve e + a\<rparr>) = inflow e - outflow e"
    using swapin_sum_raw[OF fin g] ok_bal by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

theorem swapout_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (SwapOut u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have g: "a \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - a
                          , credits := (credits e)(u := credits e u + a)\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>reserve := reserve e - a
                                  , credits := (credits e)(u := credits e u + a)\<rparr>))"
    using holders_upd_finite[OF fin] by (simp add: holders_def)
  have b2: "balance (e\<lparr>reserve := reserve e - a
                                  , credits := (credits e)(u := credits e u + a)\<rparr>) = inflow e - outflow e"
    using swapout_sum_raw[OF fin g] ok_bal g by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

theorem one_op_ok:
  assumes "escrow_ok e" "apply_op op e = Some e'"
  shows "escrow_ok e'"
  using assms by (cases op; blast intro: deposit_ok withdraw_ok swapin_ok swapout_ok)

theorem seq_ok:
  assumes "escrow_ok e" "apply_seq ops e = Some e'"
  shows "escrow_ok e'"
  using assms
proof (induction ops arbitrary: e)
  case Nil
  then show ?case by simp
next
  case (Cons a ops)
  show ?case
  proof (cases "apply_op a e")
    case None
    hence "apply_seq (a # ops) e = None" by simp
    thus ?thesis using Cons.prems(2) by simp
  next
    case (Some x2)
    hence "apply_seq (a # ops) e = apply_seq ops x2" by simp
    hence "apply_seq ops x2 = Some e'" using Cons.prems(2) by simp
    moreover have "escrow_ok x2" using Cons.prems(1) Some by (rule one_op_ok)
    ultimately show ?thesis using Cons.IH by blast
  qed
qed

text \<open>Headline: the balance is an invariant of every reachable state, and it
  always equals total-in minus total-out: nothing can be minted, lost, or
  stranded.\<close>

theorem conservation:
  assumes "escrow_ok e" "apply_seq ops e = Some e'"
  shows "balance e' = inflow e' - outflow e'"
  using assms seq_ok unfolding escrow_ok_def by blast

section \<open>Reachability: the unconditional statement\<close>

text \<open>The theorems above are conditional on \<open>escrow_ok\<close>. This section
  removes the hypothesis: genesis is the one primordial state, every other
  state arises only via \<open>apply_seq\<close>, so EVERY reachable state conserves --
  with no assumptions on the state at all. This, not the conditional form,
  is the solvency guarantee.\<close>

definition reachable :: "escrow \<Rightarrow> bool" where
  "reachable e \<longleftrightarrow> (\<exists>ops. apply_seq ops init = Some e)"

theorem reachable_ok:
  assumes "reachable e"
  shows "escrow_ok e"
proof -
  obtain ops where ops: "apply_seq ops init = Some e"
    using assms unfolding reachable_def by blast
  show "escrow_ok e" using init_ok ops seq_ok by blast
qed

theorem reachable_conservation:
  assumes "reachable e"
  shows "balance e = inflow e - outflow e \<and> outflow e \<le> inflow e"
  using reachable_ok[OF assms] unfolding escrow_ok_def by blast

section \<open>Swap coupling (composability with the proved curve)\<close>

text \<open>Curve coupling, honestly stated: the ledger layer consumes PRICES as
  opaque naturals. No property of base_swap / quote_net is assumed here --
  roundtrip_neutral quantifies over every (x, y), so any pricing rule,
  including the proved CLMM_Description curve, preserves custody. The
  curve's own theorems (clmm_quote_net_mono, quote_net_diff_eq,
  clmm_quote_net_grd_min: zero net at grd_min) live in the imported session
  and feed the phase-2 twins; nothing here depends on them.\<close>

theorem roundtrip_neutral:
  assumes ok: "escrow_ok e"
      and ap: "apply_seq [SwapIn u x, SwapOut u y] e = Some e'"
  shows "balance e' = balance e"
proof -
  obtain e1 where s1: "apply_op (SwapIn u x) e = Some e1"
    and s2: "apply_op (SwapOut u y) e1 = Some e'"
    using ap by (auto simp: apply_seq.simps split: if_splits)
  have ok1: "escrow_ok e1" using ok s1 by (rule swapin_ok)
  have ok': "escrow_ok e'" using ok1 s2 by (rule swapout_ok)
  have bal1: "balance e1 = inflow e1 - outflow e1"
    using ok1 by (simp add: escrow_ok_def balance_def)
  have bal': "balance e' = inflow e' - outflow e'"
    using ok' by (simp add: escrow_ok_def balance_def)
  have same1: "inflow e1 = inflow e" "outflow e1 = outflow e"
    using s1 by (auto simp: apply_op.simps split: if_splits)
  have same': "inflow e' = inflow e1" "outflow e' = outflow e1"
    using s2 by (auto simp: apply_op.simps split: if_splits)
  have bal_e: "balance e = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  show ?thesis using bal1 bal' same1 same' bal_e by arith
qed

end
