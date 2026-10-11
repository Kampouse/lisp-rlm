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
   Exactly what skeleton-then-prove is for.

   Phase A (LP lifecycle): lp_pool/total_shares/shares are bookkeeping INSIDE
   the same reserve — LpOpen moves credits into reserve and mints shares
   (floor ratio, refund on zero), LpRemove burns shares and moves
   reserve straight to outflow (floor payout s*pool/total_shares, refund on
   zero, excess stays pooled as dust). balance/escrow_ok are UNCHANGED,
   so every conservation proof survives verbatim; swap_out's guard is
   strengthened to spare the LP slice (a <= reserve - lp_pool). New
   solvency theorems pin the LP side: shares are backed 1:1 and payout
   never exceeds the claim. *)

begin

section \<open>Ledger state\<close>

type_synonym user = nat

record escrow =
  credits :: "user \<Rightarrow> nat"   (* units the contract owes each user *)
  reserve :: nat                (* contract-owned buffer (pool-side inventory) *)
  inflow  :: nat                (* cumulative tokens deposited into escrow *)
  outflow :: nat                (* cumulative tokens paid out of escrow *)
  lp_pool :: nat                (* slice of reserve backing LP shares *)
  total_shares  :: nat                (* total LP shares outstanding *)
  shares  :: "user \<Rightarrow> nat"   (* LP shares per user *)

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

definition lp_holders :: "escrow \<Rightarrow> user set" where
  "lp_holders e = {u. shares e u > 0}"

definition lp_ok :: "escrow \<Rightarrow> bool" where
  "lp_ok e \<longleftrightarrow> lp_pool e \<le> reserve e
     \<and> (total_shares e = 0 \<longleftrightarrow> lp_pool e = 0)
     \<and> finite (lp_holders e)
     \<and> sum (shares e) (lp_holders e) = total_shares e"
(* LP solvency: the pool is a real slice of the reserve; shares exist only
   when the pool does; the share ledger is finite and its total EQUALS the
   total_shares (no unbacked shares, no lost shares). Equality is the honest
   invariant: it holds at genesis (0 = 0) and both LP ops add/subtract from
   both sides, so a weaker bound could never be restored. The equality is what makes the
   per-user redemption bound provable: s <= shares u <= sum <= total_shares. *)

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
| LpOpen  user nat       (* user converts credits to LP shares (mint) *)
| LpRemove user nat      (* user burns LP shares for reserve units (redeem) *)

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
     (if 0 < a \<and> a \<le> reserve e - lp_pool e then
        Some (e \<lparr> reserve := reserve e - a
               , credits := (credits e)(u := credits e u + a) \<rparr>)
      else None)"
(* LpOpen: user converts a credits into LP shares at the current ratio.
   Guard 0 < a & a <= credits e u - the debit must SUCCEED (mirrors
   SwapIn): a nat-truncating debit of missing credits would mint reserve
   from nothing and break conservation. mint = floor(a * total_shares / pool)
   when the pool is live (pool > 0, total_shares > 0); on a fresh pool
   (pool = 0) it is exactly a. Shares = pro-rata claim on the pool:
   floor keeps the ledger integral, the discarded remainder stays in the
   user's credits (refund, not loss). A zero mint would strand the
   deposit (claim below rounding), so it aborts - state unchanged,
   credits kept. *)
| "apply_op (LpOpen u a) e =
     (let m = (if lp_pool e = 0 then a else a * total_shares e div lp_pool e) in
      if 0 < a \<and> a \<le> credits e u \<and> 0 < m then
        Some (e \<lparr> credits := (credits e)(u := credits e u - a)
                    , reserve := reserve e + a
                    , lp_pool := lp_pool e + a
                    , total_shares := total_shares e + m
                    , shares := (shares e)(u := shares e u + m) \<rparr>)
      else None)"
(* LpRemove: burn s shares, pay out v = floor(s * pool / total_shares) from the
   reserve slice straight to outflow (physical redemption). Guard
   0 < s & s <= shares e u & 0 < v & v <= reserve e: the prover rejected
   the weaker form - escrow_ok alone does not bound v, so the payout
   cap is part of the op's success semantics, not an assumption. The
   floor leaves the excess-share remainder in the pool (dust stays
   pooled, never paid twice); a zero payout would burn the shares for
   nothing, so it aborts - state unchanged, shares kept. *)
| "apply_op (LpRemove u s) e =
     (let v = s * lp_pool e div total_shares e in
      if 0 < s \<and> s \<le> shares e u \<and> 0 < v \<and> v \<le> reserve e then
        Some (e \<lparr> reserve := reserve e - v
                    , lp_pool := lp_pool e - v
                    , total_shares := total_shares e - s
                    , shares := (shares e)(u := shares e u - s)
                    , outflow := outflow e + v \<rparr>)
      else None)"

fun apply_seq :: "escrow_op list \<Rightarrow> escrow \<Rightarrow> escrow option" where
  "apply_seq [] e = Some e"
| "apply_seq (op # ops) e = (case apply_op op e of None \<Rightarrow> None
     | Some e' \<Rightarrow> apply_seq ops e')"

text \<open>Genesis: a fresh deployment starts here, all-zero. Every other
  reachable state arises via \<open>apply_seq\<close>; the conservation theorems
  lift the invariant from this state through any operation sequence.\<close>

definition init :: escrow where
  "init = \<lparr> credits = (\<lambda>_. 0), reserve = 0, inflow = 0, outflow = 0
         , lp_pool = 0, total_shares = 0, shares = (\<lambda>_. 0) \<rparr>"

lemma init_ok[simp]: "escrow_ok init"
  unfolding escrow_ok_def holders_def balance_def init_def by auto

section \<open>Bookkeeping: point updates of the credit function\<close>

text \<open>All conservation reasoning reduces to how the holder sum reacts when a
  single user's credit is overwritten: the user either keeps holder status or
  loses it (or joins it). These two shapes are proved once, here.\<close>

lemma holders_upd_finite:
  assumes "finite (holders r)"
  shows "finite (holders (r\<lparr>credits := (credits r)(u := c)\<rparr>))"
proof (rule finite_subset)
  show "holders (r\<lparr>credits := (credits r)(u := c)\<rparr>) \<subseteq> insert u (holders r)"
    by (auto simp: holders_def)
qed (insert assms, simp)

lemma holders_lp_finite:
  assumes "finite (holders e)"
  shows "finite (holders (e\<lparr>reserve := w, lp_pool := x, total_shares := y
                              , shares := sh, outflow := z\<rparr>))"
proof (rule finite_subset)
  show "holders (e\<lparr>reserve := w, lp_pool := x, total_shares := y
                      , shares := sh, outflow := z\<rparr>) \<subseteq> holders e"
    by (auto simp: holders_def)
qed (insert assms, simp)

lemma holders_full_finite:
  assumes "finite (holders e)"
  shows "finite (holders (e\<lparr>credits := (credits e)(u := c), reserve := w
                              , lp_pool := x, total_shares := y, shares := sh\<rparr>))"
proof (rule finite_subset)
  show "holders (e\<lparr>credits := (credits e)(u := c), reserve := w
                        , lp_pool := x, total_shares := y, shares := sh\<rparr>)
      \<subseteq> insert u (holders e)"
    by (auto simp: holders_def)
qed (insert assms, simp)

lemma sum_upd_gen_member:
  fixes f :: "'a \<Rightarrow> nat" and c :: nat
  assumes fin: "finite {x. f x > 0}" and mem: "u \<in> {x. f x > 0}"
  shows "sum f {x. f x > 0} - f u + c
       = sum (f(u := c)) {x. (f(u := c)) x > 0}"
proof (cases "c = 0")
  case True
  have split_sum: "sum f {x. f x > 0} = f u + sum f ({x. f x > 0} - {u})"
    using fin mem by (rule sum.remove)
  have ptwise: "\<And>x. (f(u := 0)) x > 0 \<longleftrightarrow> x \<noteq> u \<and> f x > 0"
    by auto
  have set_eq: "{x. (f(u := 0)) x > 0} = {x. f x > 0} - {u}"
    using ptwise by auto
  have rw: "sum (f(u := 0)) ({x. f x > 0} - {u}) = sum f ({x. f x > 0} - {u})"
    by (rule sum.cong) auto
  have upd0: "f(u := 0) = f(u := c)" using True by simp
  have s1: "sum f {x. f x > 0} - f u + c = sum f ({x. f x > 0} - {u})"
    using True split_sum by simp
  have s2: "\<dots> = sum (f(u := 0)) ({x. f x > 0} - {u})"
    by (rule rw[symmetric])
  have s3: "\<dots> = sum (f(u := 0)) {x. (f(u := 0)) x > 0}"
    by (simp only: set_eq[symmetric])
  have s4: "\<dots> = sum (f(u := c)) {x. (f(u := c)) x > 0}"
    by (simp only: upd0)
  show ?thesis by (simp only: s1 s2 s3 s4)
next
  case False
  then have cpos: "0 < c" by simp
  have split_sum: "sum f {x. f x > 0} = f u + sum f ({x. f x > 0} - {u})"
    using fin mem by (rule sum.remove)
  have set_eq: "{x. (f(u := c)) x > 0} = {x. f x > 0}"
    using mem cpos by auto
  have rw: "sum (f(u := c)) {x. f x > 0} = c + sum f ({x. f x > 0} - {u})"
  proof -
    have f1: "sum (f(u := c)) {x. f x > 0}
            = ((f(u := c)) u) + sum (f(u := c)) ({x. f x > 0} - {u})"
      using fin mem by (rule sum.remove)
    have f2: "((f(u := c)) u) = c" by simp
    have f3: "sum (f(u := c)) ({x. f x > 0} - {u})
            = sum f ({x. f x > 0} - {u})"
      by (rule sum.cong) auto
    show ?thesis using f1 f2 f3 by simp
  qed
  show ?thesis
  proof -
    have g1: "sum f {x. f x > 0} - f u + c = c + sum f ({x. f x > 0} - {u})"
      using split_sum by simp
    have g2: "\<dots> = sum (f(u := c)) {x. f x > 0}"
      by (rule rw[symmetric])
    have g3: "\<dots> = sum (f(u := c)) {x. (f(u := c)) x > 0}"
      by (simp only: set_eq[symmetric])
    show ?thesis by (simp only: g1 g2 g3)
  qed
qed

lemma sum_upd_gen_fresh:
  fixes f :: "'a \<Rightarrow> nat" and c :: nat
  assumes fin: "finite {x. f x > 0}" and fresh: "u \<notin> {x. f x > 0}"
  shows "sum f {x. f x > 0} + c
       = sum (f(u := c)) {x. (f(u := c)) x > 0}"
proof (cases "c = 0")
  case True
  have c0: "f u = 0" using fresh by (cases "f u") auto
  have idem: "f(u := 0) = f" by (simp add: fun_upd_idem_iff c0)
  show ?thesis by (simp add: idem True)
next
  case False
  then have cpos: "0 < c" by simp
  have set_eq: "{x. (f(u := c)) x > 0} = insert u {x. f x > 0}"
    using fresh cpos by auto
  have rw: "sum (f(u := c)) (insert u {x. f x > 0})
          = c + sum f {x. f x > 0}"
  proof -
    have f1: "sum (f(u := c)) (insert u {x. f x > 0})
            = ((f(u := c)) u) + sum (f(u := c)) {x. f x > 0}"
      using fin fresh by (rule sum.insert)
    have f2: "((f(u := c)) u) = c" by simp
    have f3: "sum (f(u := c)) {x. f x > 0} = sum f {x. f x > 0}"
    proof (rule sum.cong)
      show "{x. f x > 0} = {x. f x > 0}" by simp
    next
      fix x assume "x \<in> {x. f x > 0}"
      hence "x \<noteq> u" using fresh by auto
      thus "(f(u := c)) x = f x" by simp
    qed
    show ?thesis using f1 f2 f3 by simp
  qed
  show ?thesis unfolding set_eq rw by simp
qed

lemma sum_upd_member:
  assumes fin: "finite (holders e)" and mem: "u \<in> holders e"
  shows "sum (credits e) (holders e) - credits e u + c
       = sum ((credits e)(u := c))
             (holders (e\<lparr>credits := (credits e)(u := c)\<rparr>))"
  using sum_upd_gen_member[of "credits e" u c] assms
  unfolding holders_def by simp

lemma sum_upd_fresh:
  assumes fin: "finite (holders e)" and fresh: "u \<notin> holders e"
  shows "sum (credits e) (holders e) + c
       = sum ((credits e)(u := c))
             (holders (e\<lparr>credits := (credits e)(u := c)\<rparr>))"
  using sum_upd_gen_fresh[of "credits e" u c] assms
  unfolding holders_def by simp

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

text \<open>LP bookkeeping rides on the same credit-sum mechanics: LpOpen
  debits credits exactly like SwapIn (identical raw shape, extra updated
  fields are invisible to holders); LpRemove leaves credits untouched,
  so its raw lemma is pure record shape.\<close>

lemma lpopen_sum_raw:
  assumes fin: "finite (holders e)" and g: "a \<le> credits e u"
  shows "reserve e + a
       + sum ((credits e)(u := credits e u - a))
             (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                   , reserve := reserve e + a
                                   , lp_pool := lp_pool e + a
                                   , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                                   , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>))
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
                              , reserve := reserve e + a
                              , lp_pool := lp_pool e + a
                              , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                              , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>)"
    by (simp add: holders_def)
  have h1: "sum (credits e) (holders e) - credits e u + (credits e u - a)
          = sum ((credits e)(u := credits e u - a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>))"
    using sum_upd_member[OF fin True, of "credits e u - a"] .
  have step: "reserve e + a + sum ((credits e)(u := credits e u - a))
             (holders (e\<lparr>credits := (credits e)(u := credits e u - a)\<rparr>))
          = reserve e + sum (credits e) (holders e)"
    using h1[symmetric] le g by arith
  show ?thesis using h0 step by simp
next
  case False
  have c0: "credits e u = 0" using False by (auto simp: holders_def)
  have a0: "a = 0" using g c0 by arith
  have feq: "(credits e)(u := credits e u - a) = credits e"
    using c0 a0 by (simp add: fun_upd_idem_iff)
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a
                              , lp_pool := lp_pool e + a
                              , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                              , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>)
          = holders e"
    by (simp add: feq holders_def)
  show ?thesis using a0 feq h0 by simp
qed

lemma lpremove_sum_raw:
  assumes le: "v \<le> reserve e"
  shows "reserve e - v
       + sum (credits e)
             (holders (e\<lparr>reserve := reserve e - v
                                   , lp_pool := lp_pool e - v
                                   , total_shares := total_shares e - s
                                   , shares := (shares e)(u := shares e u - s)
                                   , outflow := outflow e + v\<rparr>))
       = (reserve e + sum (credits e) (holders e)) - v"
proof -
  have h0: "holders (e\<lparr>reserve := reserve e - v
                              , lp_pool := lp_pool e - v
                              , total_shares := total_shares e - s
                              , shares := (shares e)(u := shares e u - s)
                              , outflow := outflow e + v\<rparr>) = holders e"
    by (simp add: holders_def)
  have s1: "(reserve e + sum (credits e) (holders e)) - v
          = (reserve e - v) + sum (credits e) (holders e)"
    using le by arith
  show ?thesis using h0 s1 by simp
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
  from ap have g': "0 < a" "a \<le> reserve e - lp_pool e"
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
  have r_ge: "a \<le> reserve e" using g'(2) by simp
  have b2: "balance (e\<lparr>reserve := reserve e - a
                                  , credits := (credits e)(u := credits e u + a)\<rparr>) = inflow e - outflow e"
    using swapout_sum_raw[OF fin r_ge] ok_bal g'(2) by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

theorem lpopen_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (LpOpen u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  have g: "0 < a" "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , lp_pool := lp_pool e + a
                          , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                          , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , reserve := reserve e + a
                                  , lp_pool := lp_pool e + a
                                  , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                                  , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>))"
    using holders_full_finite[OF fin] by simp
  have b2: "balance (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , reserve := reserve e + a
                                  , lp_pool := lp_pool e + a
                                  , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                                  , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>) = inflow e - outflow e"
    using lpopen_sum_raw[OF fin g(2)] ok_bal by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

theorem lpremove_ok:
  assumes ok: "escrow_ok e" and ap: "apply_op (LpRemove u s) e = Some e'"
  shows "escrow_ok e'"
proof -
  have g: "0 < s" "s \<le> shares e u"
    and v: "0 < s * lp_pool e div total_shares e" "s * lp_pool e div total_shares e \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - (s * lp_pool e div total_shares e)
                          , lp_pool := lp_pool e - (s * lp_pool e div total_shares e)
                          , total_shares := total_shares e - s
                          , shares := (shares e)(u := shares e u - s)
                          , outflow := outflow e + (s * lp_pool e div total_shares e)\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  then obtain v where v: "v = s * lp_pool e div total_shares e"
    and g: "0 < s" "s \<le> shares e u" "0 < v" "v \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - v
                          , lp_pool := lp_pool e - v
                          , total_shares := total_shares e - s
                          , shares := (shares e)(u := shares e u - s)
                          , outflow := outflow e + v\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have b2: "balance (e\<lparr>reserve := reserve e - v
                                  , lp_pool := lp_pool e - v
                                  , total_shares := total_shares e - s
                                  , shares := (shares e)(u := shares e u - s)
                                  , outflow := outflow e + v\<rparr>)
           = inflow e - (outflow e + v)"
    using lpremove_sum_raw ok_bal v g ok_le by (simp add: balance_def)
  have le: "outflow e + v \<le> inflow e"
    using ok_le ok_bal g(4) by arith
  have fin_h: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have f2: "finite (holders (e\<lparr>reserve := reserve e - v
                                  , lp_pool := lp_pool e - v
                                  , total_shares := total_shares e - s
                                  , shares := (shares e)(u := shares e u - s)
                                  , outflow := outflow e + v\<rparr>))"
    using holders_lp_finite[OF fin_h] by simp
  show ?thesis unfolding eq escrow_ok_def using f2 le b2 by simp
qed

section \<open>LP share-ledger invariant\<close>

text \<open>Mirror of the credit ledger: the share ledger is finite, its total
  is covered by the total_shares, and the pool is a real slice of the reserve
  that exists exactly when shares do. Carried through every operation;
  init discharges it trivially.\<close>

lemma lp_init_ok[simp]: "lp_ok init"
  by (auto simp: lp_ok_def lp_holders_def init_def)

text \<open>Shares are a mirror of credits: the same two point-update shapes,
  proved once for the share ledger. Note a zero mint can never occur
  (the op guard rejects it), so the share update always moves its user
  into lp_holders.\<close>

lemma sum_sh_upd_member:
  assumes fin: "finite (lp_holders e)" and mem: "u \<in> lp_holders e"
  shows "sum (shares e) (lp_holders e) - shares e u + c
       = sum ((shares e)(u := c))
             (lp_holders (e\<lparr>shares := (shares e)(u := c)\<rparr>))"
  using sum_upd_gen_member[of "shares e" u c] assms
  unfolding lp_holders_def by simp

lemma sum_sh_upd_fresh:
  assumes fin: "finite (lp_holders e)" and fresh: "u \<notin> lp_holders e" and c0: "0 < c"
  shows "sum (shares e) (lp_holders e) + c
       = sum ((shares e)(u := c))
             (lp_holders (e\<lparr>shares := (shares e)(u := c)\<rparr>))"
  using sum_upd_gen_fresh[of "shares e" u c] assms
  unfolding lp_holders_def by simp

text \<open>Share-ledger twins of the raw lemmas: how the share total reacts
  to a single point update, over the exact records the LP ops write.\<close>

lemma lpopen_sh_sum_raw:
  assumes fin: "finite (lp_holders e)" and mem: "u \<in> lp_holders e" and upd: "0 < c"
  shows "sum (shares e) (lp_holders e) + c
       = sum ((shares e)(u := shares e u + c))
             (lp_holders (e\<lparr>shares := (shares e)(u := shares e u + c)\<rparr>))"
proof -
  have le0: "shares e u \<le> sum (shares e) (lp_holders e)"
  proof -
    have "sum (shares e) (lp_holders e) = shares e u + sum (shares e) (lp_holders e - {u})"
      using fin mem by (rule sum.remove)
    thus ?thesis by simp
  qed
  have "sum (shares e) (lp_holders e) - shares e u + (shares e u + c)
      = sum ((shares e)(u := shares e u + c))
            (lp_holders (e\<lparr>shares := (shares e)(u := shares e u + c)\<rparr>))"
    using sum_sh_upd_member[OF fin mem] .
  thus ?thesis using le0 upd by arith
qed

lemma lpremove_sh_sum_raw:
  assumes fin: "finite (lp_holders e)" and g: "0 < s" "s \<le> shares e u"
  shows "sum (shares e) (lp_holders e) - s
       = sum ((shares e)(u := shares e u - s))
             (lp_holders (e\<lparr>shares := (shares e)(u := shares e u - s)\<rparr>))"
proof -
  have mem: "u \<in> lp_holders e" using g by (auto simp: lp_holders_def)
  have eq1: "sum (shares e) (lp_holders e) - shares e u + (shares e u - s)
           = sum ((shares e)(u := shares e u - s))
                 (lp_holders (e\<lparr>shares := (shares e)(u := shares e u - s)\<rparr>))"
    using sum_sh_upd_member[OF fin mem] .
  have f1: "sum (shares e) (lp_holders e)
          = shares e u + sum (shares e) (lp_holders e - {u})"
    using fin mem by (rule sum.remove)
  show ?thesis using eq1 f1 g(2) by arith
qed

text \<open>LP-neutral ops (no LP fields touched): lp_ok is inherited.\<close>

lemma lp_ok_deposit:
  assumes ok: "lp_ok e" and ap: "apply_op (Deposit u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "0 < a")
    (auto simp: lp_ok_def lp_holders_def apply_op.simps split: if_splits)

lemma lp_ok_withdraw:
  assumes ok: "lp_ok e" and ap: "apply_op (Withdraw u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "a \<le> credits e u")
    (auto simp: lp_ok_def lp_holders_def apply_op.simps split: if_splits)

lemma lp_ok_swapin:
  assumes ok: "lp_ok e" and ap: "apply_op (SwapIn u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "a \<le> credits e u")
    (auto simp: lp_ok_def lp_holders_def apply_op.simps split: if_splits)

lemma lp_ok_swapout:
  assumes ok: "lp_ok e" and ap: "apply_op (SwapOut u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "0 < a \<and> a \<le> reserve e - lp_pool e")
    (auto simp: lp_ok_def lp_holders_def apply_op.simps split: if_splits)

lemma lp_ok_lpopen:
  assumes ok: "lp_ok e" and ap: "apply_op (LpOpen u a) e = Some e'"
  shows "lp_ok e'"
proof -
  have g: "0 < a" "a \<le> credits e u" "0 < (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , lp_pool := lp_pool e + a
                          , total_shares := total_shares e + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)
                          , shares := (shares e)(u := shares e u + (if lp_pool e = 0 then a else a * total_shares e div lp_pool e))\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  define M where "M = (if lp_pool e = 0 then a else a * total_shares e div lp_pool e)"
  have gM: "0 < M" using g(3) M_def by simp
  have eq2: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , lp_pool := lp_pool e + a
                          , total_shares := total_shares e + M
                          , shares := (shares e)(u := shares e u + M)\<rparr>"
    using eq by (simp add: M_def)
  have fin: "finite (lp_holders e)" using ok by (simp add: lp_ok_def)
  show ?thesis
  proof (cases "lp_pool e = 0")
    case True
    have mM: "M = a" using M_def True by simp
    have t0: "total_shares e = 0" using ok True by (simp add: lp_ok_def)
    have sume0: "sum (shares e) (lp_holders e) = 0" using ok t0 by (simp add: lp_ok_def)
    have ufresh: "u \<notin> lp_holders e"
    proof
      assume mem: "u \<in> lp_holders e"
      have f1: "sum (shares e) (lp_holders e)
              = shares e u + sum (shares e) (lp_holders e - {u})"
        using fin mem by (rule sum.remove)
      hence "shares e u \<le> sum (shares e) (lp_holders e)" by simp
      thus False using sume0 mem by (simp add: lp_holders_def)
    qed
    have su0: "shares e u = 0" using ufresh by (simp add: lp_holders_def)
    have sumeq: "sum (shares e) (lp_holders e) + M = sum (shares e') (lp_holders e')"
      using sum_sh_upd_fresh[OF fin ufresh gM] eq2 su0
      by (simp add: lp_holders_def)
    show ?thesis using eq2 ok True sumeq gM mM
      by (auto simp: lp_ok_def lp_holders_def M_def)
  next
    case False
    then have p_pos: "0 < lp_pool e" by simp
    have sumeq: "sum (shares e) (lp_holders e) + M = sum (shares e') (lp_holders e')"
    proof (cases "u \<in> lp_holders e")
      case True
      have mpos: "0 < shares e u" using True by (simp add: lp_holders_def)
      show ?thesis using lpopen_sh_sum_raw[OF fin True gM] eq2 mpos
        by (simp add: lp_holders_def)
    next
      case False
      have su0: "shares e u = 0" using False by (simp add: lp_holders_def)
      show ?thesis using sum_sh_upd_fresh[OF fin False gM] eq2 su0
        by (simp add: lp_holders_def)
    qed
    have exact: "sum (shares e) (lp_holders e) = total_shares e"
      using ok by (simp add: lp_ok_def)
    have bound: "sum (shares e') (lp_holders e') = total_shares e + M"
      using sumeq exact by simp
    have finS: "finite {ua. ua \<noteq> u \<longrightarrow> 0 < shares e ua}"
    proof -
      have "{ua. ua \<noteq> u \<longrightarrow> 0 < shares e ua} \<subseteq> insert u {ua. 0 < shares e ua}" by auto
      from finite_subset[OF this]
      show ?thesis using fin lp_holders_def by simp
    qed
    show ?thesis using eq2 ok bound p_pos gM finS
      by (auto simp: lp_ok_def lp_holders_def M_def)
  qed
qed

lemma lp_ok_lpremove:
  assumes ok: "lp_ok e" and ap: "apply_op (LpRemove u s) e = Some e'"
  shows "lp_ok e'"
proof -
  have g0: "0 < s" "s \<le> shares e u" "0 < s * lp_pool e div total_shares e"
    "s * lp_pool e div total_shares e \<le> reserve e"
    and eq0: "e' = e\<lparr>reserve := reserve e - (s * lp_pool e div total_shares e)
                          , lp_pool := lp_pool e - (s * lp_pool e div total_shares e)
                          , total_shares := total_shares e - s
                          , shares := (shares e)(u := shares e u - s)
                          , outflow := outflow e + (s * lp_pool e div total_shares e)\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  then obtain v where v: "v = s * lp_pool e div total_shares e"
    and g: "0 < s" "s \<le> shares e u" "0 < v" "v \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - v
                          , lp_pool := lp_pool e - v
                          , total_shares := total_shares e - s
                          , shares := (shares e)(u := shares e u - s)
                          , outflow := outflow e + v\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have mem0: "0 < shares e u" by (rule less_le_trans[OF g(1) g(2)])
  hence mem: "u \<in> lp_holders e" by (simp add: lp_holders_def)
  have fin: "finite (lp_holders e)" using ok by (simp add: lp_ok_def)
  have sumeq: "sum (shares e) (lp_holders e) - s = sum (shares e') (lp_holders e')"
    using lpremove_sh_sum_raw[OF fin g(1) g(2)] eq v
    by (simp add: lp_holders_def)
  have exact: "sum (shares e) (lp_holders e) = total_shares e"
    using ok by (simp add: lp_ok_def)
  have f1: "sum (shares e) (lp_holders e)
          = shares e u + sum (shares e) (lp_holders e - {u})"
    using fin mem by (rule sum.remove)
  have s_le_T: "s \<le> total_shares e"
  proof (cases "shares e u = total_shares e")
    case True
    thus ?thesis using g(2) by simp
  next
    case False
    have mem': "shares e u < sum (shares e) (lp_holders e)"
      using f1 exact False by simp
    thus ?thesis using exact g(2) by simp
  qed
  have sumeq': "sum (shares e') (lp_holders e') = total_shares e - s"
    using sumeq exact by simp
  have T0: "total_shares e \<noteq> 0"
  proof
    assume Tz: "total_shares e = 0"
    hence "s * lp_pool e div total_shares e = 0" by simp
    thus False using g(3) v by simp
  qed
  have pool_ne: "lp_pool e \<noteq> 0"
  proof
    assume pz: "lp_pool e = 0"
    hence "total_shares e = 0" using ok by (simp add: lp_ok_def)
    thus False using T0 by simp
  qed
  have pool0: "0 < lp_pool e" using pool_ne by simp
  have v_le_pool: "v \<le> lp_pool e"
  proof -
    have "s * lp_pool e \<le> total_shares e * lp_pool e" using s_le_T by (rule mult_le_mono1)
    hence "s * lp_pool e div total_shares e \<le> total_shares e * lp_pool e div total_shares e"
      by (rule div_le_mono)
    thus ?thesis using v T0 by simp
  qed
  have T_le_s: "lp_pool e \<le> v \<Longrightarrow> total_shares e \<le> s"
  proof -
    assume pv: "lp_pool e \<le> v"
    have pe: "s * lp_pool e div total_shares e = v" using v by simp
    have ge: "lp_pool e \<le> s * lp_pool e div total_shares e" using pv pe by simp
    have step1: "lp_pool e * total_shares e \<le> (s * lp_pool e div total_shares e) * total_shares e"
      using ge T0 by simp
    have dm: "s * lp_pool e = (s * lp_pool e div total_shares e) * total_shares e
                            + s * lp_pool e mod total_shares e"
      by simp
    have step2: "lp_pool e * total_shares e \<le> s * lp_pool e"
      using step1 dm by arith
    have step3: "lp_pool e * total_shares e \<le> lp_pool e * s"
      using step2 by simp
    show "total_shares e \<le> s"
    proof (rule ccontr)
      assume nc: "\<not> total_shares e \<le> s"
      hence lt: "s < total_shares e" by simp
      have "lp_pool e * s < lp_pool e * total_shares e"
        using pool0 lt by (simp add: mult_less_cancel2)
      thus False using step3 by simp
    qed
  qed
  have sT_eq: "s = total_shares e \<Longrightarrow> v = lp_pool e"
  proof -
    assume sT: "s = total_shares e"
    have "s * lp_pool e div total_shares e = total_shares e * lp_pool e div total_shares e"
      using sT by simp
    thus "v = lp_pool e" using v T0 by (simp add: div_mult_self1)
  qed
  have bound: "sum (shares e') (lp_holders e') = total_shares e - s"
    using sumeq exact by simp
  show ?thesis
  proof (cases "s = total_shares e")
    case True
    have vp: "v = lp_pool e" using sT_eq True by simp
    have use: "shares e u = s"
      using exact f1 g(2) True by arith
    have rest0: "sum (shares e) (lp_holders e - {u}) = 0"
      using f1 exact use True by arith
    have finR: "finite (lp_holders e - {u})" using fin by simp
    have oz: "\<And>x. x \<noteq> u \<Longrightarrow> shares e x = 0"
    proof -
      fix x assume xu: "x \<noteq> u"
      show "shares e x = 0"
      proof (cases "x \<in> lp_holders e - {u}")
        case True
        have "sum (shares e) (lp_holders e - {u})
            = shares e x + sum (shares e) ((lp_holders e - {u}) - {x})"
          using finR True by (rule sum.remove)
        hence "shares e x \<le> sum (shares e) (lp_holders e - {u})" by simp
        thus "shares e x = 0" using rest0 by simp
      next
        case False
        thus "shares e x = 0" using xu by (simp add: lp_holders_def)
      qed
    qed
    have finE': "finite (lp_holders e')"
    proof (rule finite_subset)
      show "lp_holders e' \<subseteq> insert u (lp_holders e)"
        using eq by (auto simp: lp_holders_def)
    qed (simp add: fin)
    show ?thesis using eq ok vp use bound True oz finE'
      by (auto simp: lp_ok_def lp_holders_def)
  next
    case False
    have sT: "s \<le> total_shares e"
    proof -
      have ub: "shares e u \<le> sum (shares e) (lp_holders e)"
        using f1 by simp
      thus ?thesis using g(2) exact by simp
    qed
    have s_lt: "s < total_shares e" using sT False by simp
    have tp: "0 < total_shares e - s" using s_lt by simp
    have nb: "lp_pool e - v \<noteq> 0"
    proof
      assume z: "lp_pool e - v = 0"
      hence ple: "lp_pool e \<le> v" by simp
      have "total_shares e \<le> s" using T_le_s ple by simp
      thus False using s_lt by simp
    qed
    have finE': "finite (lp_holders e')"
    proof (rule finite_subset)
      show "lp_holders e' \<subseteq> insert u (lp_holders e)"
        using eq by (auto simp: lp_holders_def)
    qed (simp add: fin)
    show ?thesis using eq ok sumeq' v_le_pool T0 tp nb finE'
      by (auto simp: lp_ok_def lp_holders_def)
  qed
qed

theorem one_op_lp:
  assumes "lp_ok e" "apply_op op e = Some e'"
  shows "lp_ok e'"
  using assms
proof (cases op)
  case (Deposit x1 x2)
  show ?thesis using assms(1) assms(2) Deposit lp_ok_deposit by blast
next
  case (Withdraw x1 x2)
  show ?thesis using assms(1) assms(2) Withdraw lp_ok_withdraw by blast
next
  case (SwapIn x1 x2)
  show ?thesis using assms(1) assms(2) SwapIn lp_ok_swapin by blast
next
  case (SwapOut x1 x2)
  show ?thesis using assms(1) assms(2) SwapOut lp_ok_swapout by blast
next
  case (LpOpen u a)
  show ?thesis using assms(1) assms(2) LpOpen lp_ok_lpopen by blast
next
  case (LpRemove u s)
  show ?thesis using assms(1) assms(2) LpRemove lp_ok_lpremove by blast
qed

text \<open>Sequence lift for the share ledger, mirroring seq_ok.\<close>

theorem seq_lp:
  assumes "lp_ok e" "apply_seq ops e = Some e'"
  shows "lp_ok e'"
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
    moreover have "lp_ok x2" using Cons.prems(1) Some by (rule one_op_lp)
    ultimately show ?thesis using Cons.IH by blast
  qed
qed

theorem one_op_ok:
  assumes "escrow_ok e" "apply_op op e = Some e'"
  shows "escrow_ok e'"
  using assms by (cases op; blast intro: deposit_ok withdraw_ok swapin_ok swapout_ok lpopen_ok lpremove_ok)

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
  shows "escrow_ok e \<and> lp_ok e"
proof -
  obtain ops where ops: "apply_seq ops init = Some e"
    using assms unfolding reachable_def by blast
  show ?thesis using init_ok lp_init_ok ops seq_ok seq_lp by blast
qed

theorem reachable_conservation:
  assumes "reachable e"
  shows "balance e = inflow e - outflow e \<and> outflow e \<le> inflow e"
  using reachable_ok[OF assms] unfolding escrow_ok_def by blast

section \<open>LP solvency: the unconditional share-side statement\<close>

text \<open>Second headline: in every reachable state the LP share ledger is
  fully backed - the pool is real reserve value, shares exist only while
  the pool does, and the share ledger never exceeds the minted total_shares.
  This is the share-side analog of conservation.\<close>

theorem reachable_lp:
  assumes "reachable e"
  shows "lp_ok e"
  using reachable_ok[OF assms] by blast

text \<open>Redemption honesty: in any reachable state with an LP present, a
  user's per-share payout bound holds - floor(s * pool / total_shares) is at
  most the pool they participate in. Shares are capped by the total_shares
  (reachable_lp), so the pro-rata product can never round above the
  shared pool: no redemption path pays value the pool does not hold.\<close>

theorem redeem_within_pool:
  assumes "reachable e" "u \<in> lp_holders e"
  shows "shares e u * lp_pool e div total_shares e \<le> lp_pool e"
proof -
  have fin: "finite (lp_holders e)"
    and sum_le: "sum (shares e) (lp_holders e) \<le> total_shares e"
    using assms(1) reachable_lp by (simp_all add: lp_ok_def)
  have sh: "sum (shares e) (lp_holders e)
          = shares e u + sum (shares e) (lp_holders e - {u})"
    using fin assms(2) by (rule sum.remove)
  hence su: "shares e u \<le> sum (shares e) (lp_holders e)" by simp
  hence sne: "shares e u \<le> total_shares e" using sum_le by arith
  have u_pos: "0 < shares e u" using assms(2) by (simp add: lp_holders_def)
  hence npos: "0 < total_shares e" using sne by arith
  have "shares e u * lp_pool e \<le> total_shares e * lp_pool e"
    using sne by (rule mult_le_mono1)
  hence d1: "shares e u * lp_pool e div total_shares e
           \<le> total_shares e * lp_pool e div total_shares e"
    by (rule div_le_mono)
  hence d2: "total_shares e * lp_pool e div total_shares e = lp_pool e"
    using npos by (simp add: div_mult_self1)
  show ?thesis using d1 d2 by arith
qed

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
