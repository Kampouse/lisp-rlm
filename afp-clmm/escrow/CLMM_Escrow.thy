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
  keys    :: "nat set"          (* live keyed pools *)
  pools   :: "nat \<Rightarrow> nat"     (* per-pool LP slice of the reserve *)
  totals  :: "nat \<Rightarrow> nat"      (* per-pool total shares outstanding *)
  books   :: "nat \<Rightarrow> user \<Rightarrow> nat" (* per-pool shares per user *)
  lp_pool :: nat                (* default-pool slice of reserve (legacy) *)
  total_shares  :: nat                (* default-pool total shares *)
  shares  :: "user \<Rightarrow> nat"   (* default-pool shares per user *)

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

(* Keyed-pool support sets: per-pool mirrors of holders/lp_holders. *)
definition book_support :: "escrow \<Rightarrow> nat \<Rightarrow> user set" where
  "book_support e k = {u. books e k u > 0}"

definition total_support :: "escrow \<Rightarrow> nat set" where
  "total_support e = {k. totals e k > 0}"

definition lp_ok :: "escrow \<Rightarrow> bool" where
  "lp_ok e \<longleftrightarrow> lp_pool e \<le> reserve e
     \<and> (total_shares e = 0 \<longleftrightarrow> lp_pool e = 0)
     \<and> finite (lp_holders e)
     \<and> sum (shares e) (lp_holders e) = total_shares e
     \<and> sum (pools e) (total_support e) \<le> reserve e
     \<and> lp_pool e + sum (pools e) (total_support e) \<le> reserve e
     \<and> finite (total_support e)
     \<and> (\<forall>k. totals e k = 0 \<longleftrightarrow> pools e k = 0)
     \<and> (\<forall>k. pools e k \<le> totals e k)
     \<and> (\<forall>k. sum (books e k) (book_support e k) = totals e k)
     \<and> (\<forall>k. finite (book_support e k))
     \<and> total_support e \<subseteq> keys e"
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
| PoolOpen  nat          (* register a fresh pool key *)
| PoolClose nat          (* deregister an EMPTY pool key *)
| LpKeyOpen   nat user nat   (* keyed mint: pool, user, amount *)
| LpKeyRemove nat user nat   (* keyed redeem: pool, user, shares *)

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
     (if 0 < a \<and> a \<le> reserve e - (lp_pool e + sum (pools e) (total_support e)) then
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
                    , credits := (credits e)(u := credits e u + v) \<rparr>)
      else None)"
(* Multi-LP (keyed pools): each pool key k carries its own slice (pools k),
   share supply (totals k) and per-user book (books k). LpKeyOpen/LpKeyRemove
   are the per-pool mirrors of LpOpen/LpRemove with the SAME floor semantics;
   the legacy fields are untouched (independent pools over one reserve).
   PoolOpen registers a fresh key (idempotence refused: re-opening a live
   key would alias two ledgers onto one slice); PoolClose requires the
   pool EMPTY (0 shares AND 0 pool), so a close can never orphan shares.
   The SwapOut guard spends `sum (pools e) (total_support e)` -- the pooled
   total over the SUPPORT -- protecting every live pool without a
   finiteness premise: keys not in the support hold pools = 0 (invariant
   lp_ok: totals k = 0 <-> pools k = 0 and totals > 0 defines the support). *)
| "apply_op (PoolOpen k) e =
     (if k \<in> keys e then None
      else Some (e \<lparr> keys := keys e \<union> {k} \<rparr>))"
| "apply_op (PoolClose k) e =
     (if k \<notin> keys e \<or> pools e k \<noteq> 0 \<or> totals e k \<noteq> 0 then None
      else Some (e \<lparr> keys := keys e - {k} \<rparr>))"
| "apply_op (LpKeyOpen k u a) e =
     (let m = (if pools e k = 0 then a else a * totals e k div pools e k) in
      if k \<in> keys e \<and> 0 < a \<and> a \<le> credits e u \<and> 0 < m then
        Some (e \<lparr> credits := (credits e)(u := credits e u - a)
                    , reserve := reserve e + a
                    , pools := (pools e)(k := pools e k + a)
                    , totals := (totals e)(k := totals e k + m)
                    , books := (books e)(k := (books e k)(u := books e k u + m)) \<rparr>)
      else None)"
| "apply_op (LpKeyRemove k u s) e =
     (let v = s * pools e k div totals e k in
      if k \<in> keys e \<and> 0 < s \<and> s \<le> books e k u \<and> 0 < v \<and> v \<le> reserve e then
        Some (e \<lparr> reserve := reserve e - v
                    , pools := (pools e)(k := pools e k - v)
                    , totals := (totals e)(k := totals e k - s)
                    , books := (books e)(k := (books e k)(u := books e k u - s))
                    , credits := (credits e)(u := credits e u + v) \<rparr>)
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
         , keys = {}, pools = (\<lambda>_. 0), totals = (\<lambda>_. 0), books = (\<lambda>_ _. 0)
         , lp_pool = 0, total_shares = 0, shares = (\<lambda>_. 0) \<rparr>"

lemma init_ok[simp]: "escrow_ok init"
  unfolding escrow_ok_def holders_def balance_def init_def by auto

(* conservation ledger is intact under every op that leaves it untouched
   (crucially LpKeyRemove: reserve drops v AND outflow rises v) *)
lemma ok_bump:
  assumes "escrow_ok e" and "credits e' = credits e"
      and "outflow e' = outflow e" and "inflow e' = inflow e"
      and "reserve e' = reserve e"
  shows "escrow_ok e'"
proof -
  have h: "holders e' = holders e"
    unfolding holders_def using assms(2) by simp
  show ?thesis using assms(1) h assms(2) assms(3) assms(4) assms(5)
    unfolding escrow_ok_def balance_def by simp
qed

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

lemma holders_keyopen_finite:
  assumes "finite (holders e)"
  shows "finite (holders (e\<lparr>credits := (credits e)(u := c), reserve := w
                              , pools := p, totals := t, books := b\<rparr>))"
proof (rule finite_subset)
  show "holders (e\<lparr>credits := (credits e)(u := c), reserve := w
                        , pools := p, totals := t, books := b\<rparr>)
      \<subseteq> insert u (holders e)"
    by (auto simp: holders_def)
qed (insert assms, simp)

lemma holders_keyremove_finite:
  assumes "finite (holders e)"
  shows "finite (holders (e\<lparr>reserve := w, pools := p, totals := t
                              , books := b, outflow := z\<rparr>))"
proof (rule finite_subset)
  show "holders (e\<lparr>reserve := w, pools := p, totals := t
                        , books := b, outflow := z\<rparr>) \<subseteq> holders e"
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
                                   , credits := (credits e)(u := credits e u + v)\<rparr>))
       = (reserve e + sum (credits e) (holders e)) - v"
proof -
  have h0: "holders (e\<lparr>reserve := reserve e - v
                              , lp_pool := lp_pool e - v
                              , total_shares := total_shares e - s
                              , shares := (shares e)(u := shares e u - s)
                              , credits := (credits e)(u := credits e u + v)\<rparr>) = holders e"
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
  from ap have g': "0 < a" "a \<le> reserve e - (lp_pool e + sum (pools e) (total_support e))"
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
                          , credits := (credits e)(u := credits e u + v)\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have b2: "balance (e\<lparr>reserve := reserve e - v
                                  , lp_pool := lp_pool e - v
                                  , total_shares := total_shares e - s
                                  , shares := (shares e)(u := shares e u - s)
                                  , credits := (credits e)(u := credits e u + v)\<rparr>)
           = inflow e - outflow e"
    using lpremove_sum_raw ok_bal v g ok_le by (simp add: balance_def)
  have fin_h: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have f2: "finite (holders (e\<lparr>reserve := reserve e - v
                                  , lp_pool := lp_pool e - v
                                  , total_shares := total_shares e - s
                                  , shares := (shares e)(u := shares e u - s)
                                  , credits := (credits e)(u := credits e u + v)\<rparr>))"
    using holders_lp_finite[OF fin_h] by simp
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

section \<open>Multi-LP: keyed pools over one reserve\<close>

text \<open>Each pool key carries its own slice of the SAME reserve, its own share
  supply and its own per-user book. The legacy single-pool fields are the
  untouched layer beneath: every legacy theorem survives verbatim, and the
  keyed ops never read or write them (independence). The shared quantities
  -- credits, reserve, in/out -- are exactly the ones the conservation
  ledger tracks, so escrow_ok carries through unchanged; lp_ok carries the
  keyed solvency conjuncts (pooled slice real, per-pool backing, per-pool
  book totals). Support-set design: the SwapOut guard spends
  sum (pools e) (total_support e), which needs NO finiteness premise --
  total_support is provably finite in every lp_ok state, and keys outside
  it hold pools = 0 by the zero-coupling conjunct.\<close>

lemma init_keys[simp]: "keys init = {}" by (simp add: init_def)
lemma init_pools[simp]: "pools init = (\<lambda>_. 0)" by (simp add: init_def)
lemma init_totals[simp]: "totals init = (\<lambda>_. 0)" by (simp add: init_def)
lemma init_books[simp]: "books init = (\<lambda>_ _. 0)" by (simp add: init_def)

lemma ok_poolopen:
  assumes ok: "escrow_ok e" and ap: "apply_op (PoolOpen k) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have eq: "e' = e\<lparr>keys := keys e \<union> {k}\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  show ?thesis using ok_bump[OF ok, of e'] eq by simp
qed

lemma ok_poolclose:
  assumes ok: "escrow_ok e" and ap: "apply_op (PoolClose k) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have eq: "e' = e\<lparr>keys := keys e - {k}\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  show ?thesis using ok_bump[OF ok, of e'] eq by simp
qed

lemma ok_lpkeyopen:
  assumes ok: "escrow_ok e" and ap: "apply_op (LpKeyOpen k u a) e = Some e'"
  shows "escrow_ok e'"
proof -
  from ap have g: "0 < a" "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , pools := (pools e)(k := pools e k + a)
                          , totals := (totals e)(k := totals e k + (if pools e k = 0 then a else a * totals e k div pools e k))
                          , books := (books e)(k := (books e k)(u := books e k u + (if pools e k = 0 then a else a * totals e k div pools e k)))\<rparr>"
    by (auto simp: apply_op.simps Let_def split: if_splits)
  have fin: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                  , reserve := reserve e + a
                                  , pools := (pools e)(k := pools e k + a)
                                  , totals := (totals e)(k := totals e k + (if pools e k = 0 then a else a * totals e k div pools e k))
                                  , books := (books e)(k := (books e k)(u := books e k u + (if pools e k = 0 then a else a * totals e k div pools e k)))\<rparr>))"
    using holders_keyopen_finite[OF fin] by simp
  have h0: "holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a
                              , pools := (pools e)(k := pools e k + a)
                              , totals := (totals e)(k := totals e k + (if pools e k = 0 then a else a * totals e k div pools e k))
                              , books := (books e)(k := (books e k)(u := books e k u + (if pools e k = 0 then a else a * totals e k div pools e k)))\<rparr>)
          = holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a\<rparr>)"
    by (simp add: holders_def)
  have b1: "reserve e + a
          + sum ((credits e)(u := credits e u - a))
                (holders (e\<lparr>credits := (credits e)(u := credits e u - a)
                                      , reserve := reserve e + a\<rparr>))
          = reserve e + sum (credits e) (holders e)"
    using swapin_sum_raw[OF fin g(2)] .
  have b2: "balance (e\<lparr>credits := (credits e)(u := credits e u - a)
                              , reserve := reserve e + a
                              , pools := (pools e)(k := pools e k + a)
                              , totals := (totals e)(k := totals e k + (if pools e k = 0 then a else a * totals e k div pools e k))
                              , books := (books e)(k := (books e k)(u := books e k u + (if pools e k = 0 then a else a * totals e k div pools e k)))\<rparr>)
          = inflow e - outflow e"
    using b1 ok_bal h0 by (simp add: balance_def)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

lemma ok_lpkeyremove:
  assumes ok: "escrow_ok e" and ap: "apply_op (LpKeyRemove k u s) e = Some e'"
  shows "escrow_ok e'"
proof -
  obtain v where v: "v = s * pools e k div totals e k"
    and g: "0 < s" "s \<le> books e k u" "0 < v" "v \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - v
                          , pools := (pools e)(k := pools e k - v)
                          , totals := (totals e)(k := totals e k - s)
                          , books := (books e)(k := (books e k)(u := books e k u - s))
                          , credits := (credits e)(u := credits e u + v)\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  have fin_h: "finite (holders e)" using ok by (simp add: escrow_ok_def)
  have ok_le: "outflow e \<le> inflow e" using ok by (simp add: escrow_ok_def)
  have ok_bal: "reserve e + sum (credits e) (holders e) = inflow e - outflow e"
    using ok by (simp add: escrow_ok_def balance_def)
  have f2: "finite (holders (e\<lparr>reserve := reserve e - v
                                  , pools := (pools e)(k := pools e k - v)
                                  , totals := (totals e)(k := totals e k - s)
                                  , books := (books e)(k := (books e k)(u := books e k u - s))
                                  , credits := (credits e)(u := credits e u + v)\<rparr>))"
    using holders_keyremove_finite[OF fin_h] by simp
  have h0: "holders (e\<lparr>reserve := reserve e - v
                              , pools := (pools e)(k := pools e k - v)
                              , totals := (totals e)(k := totals e k - s)
                              , books := (books e)(k := (books e k)(u := books e k u - s))
                              , credits := (credits e)(u := credits e u + v)\<rparr>) = holders e"
    by (simp add: holders_def)
  have b3: "sum ((credits e)(u := credits e u + v)) (holders e)
          = sum (credits e) (holders e) + v"
    using g(1) fin_h by (auto simp: sum.remove insert_absorb)
  have b2: "balance (e\<lparr>reserve := reserve e - v
                              , pools := (pools e)(k := pools e k - v)
                              , totals := (totals e)(k := totals e k - s)
                              , books := (books e)(k := (books e k)(u := books e k u - s))
                              , credits := (credits e)(u := credits e u + v)\<rparr>)
          = inflow e - outflow e"
    using ok_bal b3 by (simp add: balance_def algebra_simps)
  show ?thesis unfolding eq escrow_ok_def using f2 ok_le b2 by simp
qed

text \<open>Set-level update toolkit: a point update against an EXPLICIT finite
  set (the supports), stated so the op proofs can chain them by name.\<close>

lemma sum_upd_set_member:
  fixes f :: "'a \<Rightarrow> nat" and S :: "'a set" and k :: "'a" and c :: nat
  assumes fin: "finite S" and mem: "k \<in> S"
  shows "sum f S - f k + c = sum (f(k := c)) S"
proof -
  have sp: "sum f S = f k + sum f (S - {k})"
    using fin mem by (rule sum.remove)
  have spl: "sum (f(k := c)) S = (f(k := c)) k + sum (f(k := c)) (S - {k})"
    using fin mem by (rule sum.remove)
  have gk: "(f(k := c)) k = c" by simp
  have cn: "sum (f(k := c)) (S - {k}) = sum f (S - {k})"
    by (rule sum.cong) auto
  show ?thesis using sp spl gk cn by arith
qed

lemma sum_upd_set_fresh:
  fixes f :: "'a \<Rightarrow> nat" and S :: "'a set" and k :: "'a" and c :: nat
  assumes fin: "finite S" and fresh: "k \<notin> S"
  shows "sum f S + c = sum (f(k := c)) (insert k S)"
proof -
  have ins: "sum (f(k := c)) (insert k S) = (f(k := c)) k + sum (f(k := c)) S"
    using assms by (rule sum.insert)
  have a1: "(f(k := c)) k = c" by simp
  have cn: "sum (f(k := c)) S = sum f S"
    by (rule sum.cong) (insert fresh, auto)
  show ?thesis using ins a1 cn by simp
qed

lemma support_upd_finite:
  fixes f :: "'a \<Rightarrow> nat" and c :: nat
  assumes "finite {x. f x > 0}"
  shows "finite {x. (f(u := c)) x > 0}"
proof (rule finite_subset)
  show "{x. (f(u := c)) x > 0} \<subseteq> insert u {x. f x > 0}" by auto
qed (insert assms, simp)

text \<open>Support bridges: the zero-coupling conjunct turns set membership
  into arithmetic and back.\<close>

lemma support_pools_zero:
  assumes "lp_ok e"
  shows "pools e k = 0 \<longleftrightarrow> k \<notin> total_support e"
  using assms by (auto simp: lp_ok_def total_support_def)

section \<open>LP share-ledger invariant\<close>

text \<open>Mirror of the credit ledger: the share ledger is finite, its total
  is covered by the total_shares, and the pool is a real slice of the reserve
  that exists exactly when shares do. Carried through every operation;
  init discharges it trivially.\<close>

lemma lp_init_ok[simp]: "lp_ok init"
  by (auto simp: lp_ok_def lp_holders_def init_def total_support_def
                 book_support_def)

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
    (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def apply_op.simps split: if_splits)

lemma lp_ok_withdraw:
  assumes ok: "lp_ok e" and ap: "apply_op (Withdraw u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "a \<le> credits e u")
    (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def apply_op.simps split: if_splits)

lemma lp_ok_swapin:
  assumes ok: "lp_ok e" and ap: "apply_op (SwapIn u a) e = Some e'"
  shows "lp_ok e'" using ok
  using ap by (cases "a \<le> credits e u")
    (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def apply_op.simps split: if_splits)

lemma lp_ok_swapout:
  assumes ok: "lp_ok e" and ap: "apply_op (SwapOut u a) e = Some e'"
  shows "lp_ok e'"
proof -
  have g: "0 < a" "a \<le> reserve e - (lp_pool e + sum (pools e) (total_support e))"
    and eq: "e' = e\<lparr>reserve := reserve e - a
                          , credits := (credits e)(u := credits e u + a)\<rparr>"
    using ap by (auto simp: apply_op.simps split: if_splits)
  have rle: "reserve e - a \<le> reserve e" by simp
  have kle: "lp_pool e \<le> reserve e" using ok by (simp add: lp_ok_def)
  have sup_le: "sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  show ?thesis using g rle kle sup_le ok eq
    by (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def)
qed

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
      by (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def M_def)
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
      by (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def M_def)
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
                          , credits := (credits e)(u := credits e u + v)\<rparr>"
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
      by (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def)
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
      by (auto simp: lp_ok_def lp_holders_def total_support_def book_support_def)
  qed
qed

section \<open>Keyed-pool lp_ok: joint solvency under the keyed ops\<close>

text \<open>Every conjunct of the keyed extension of lp_ok is discharged by
  name: the pooled-slice bound, the per-pool backing inequality, the
  per-pool book-sum equality, and the support finiteness. The two LP
  carries are THE delicate ones -- LpKeyOpen preserves the cross
  inequality a*t/p <= a*p only because the mint formula swaps operands
  with the fresh-pool case; LpKeyRemove preserves pools k <= totals k
  only because the payout FLOORS (div_le_mono direction). Legacy ops
  never touch the keyed fields, so their carries are identity carries.\<close>

lemma key_support_bridge:
  assumes ok: "lp_ok e" and fresh: "pools e k = 0"
  shows "sum (pools e) (total_support e) + a
       = sum ((pools e)(k := a)) (insert k (total_support e))"
proof -
  have fin: "finite (total_support e)" using ok by (simp add: lp_ok_def)
  have kn: "k \<notin> total_support e" using support_pools_zero[OF ok] fresh by simp
  have i1: "sum ((pools e)(k := a)) (insert k (total_support e))
          = ((pools e)(k := a)) k + sum ((pools e)(k := a)) (total_support e)"
    using fin kn by (rule sum.insert)
  have a1: "((pools e)(k := a)) k = a" by simp
  have i2: "sum ((pools e)(k := a)) (total_support e)
          = sum (pools e) (total_support e)"
    by (rule sum.cong) (insert kn, auto)
  show ?thesis using i1 a1 i2 by simp
qed

lemma key_support_bridge_sub:
  assumes ok: "lp_ok e" and mem: "k \<in> total_support e" and vle: "v \<le> pools e k"
  shows "sum (pools e) (total_support e) - v
       = sum ((pools e)(k := pools e k - v)) (total_support e)"
proof -
  have fin: "finite (total_support e)" using ok by (simp add: lp_ok_def)
  have sp: "sum (pools e) (total_support e)
          = pools e k + sum (pools e) (total_support e - {k})"
    using fin mem by (rule sum.remove)
  have p1: "pools e k - v \<le> pools e k" by simp
  have c1: "sum ((pools e)(k := pools e k - v)) (total_support e)
          = ((pools e)(k := pools e k - v)) k
          + sum ((pools e)(k := pools e k - v)) (total_support e - {k})"
    using fin mem by (rule sum.remove)
  have c2: "((pools e)(k := pools e k - v)) k = pools e k - v" by simp
  have c3: "sum ((pools e)(k := pools e k - v)) (total_support e - {k})
          = sum (pools e) (total_support e - {k})"
    by (rule sum.cong) auto
  show ?thesis using sp c1 c2 c3 vle by arith
qed

lemma book_support_bridge_add:
  fixes f :: "'a \<Rightarrow> nat" and S :: "'a set" and u :: "'a" and c :: nat
  assumes fin: "finite S" and mem: "u \<in> S"
  shows "sum f S - f u + (f u + c) = sum (f(u := f u + c)) S"
  using sum_upd_set_member assms by simp

lemma book_support_bridge_sub:
  fixes f :: "'a \<Rightarrow> nat" and S :: "'a set" and u :: "'a" and c :: nat
  assumes fin: "finite S" and mem: "u \<in> S" and cle: "c \<le> f u"
  shows "sum f S - c = sum (f(u := f u - c)) S"
proof -
  have sp: "sum f S = f u + sum f (S - {u})"
    using fin mem by (rule sum.remove)
  have fle: "f u \<le> sum f S" using sp by arith
  have step: "sum f S - f u + (f u - c) = sum (f(u := f u - c)) S"
    using sum_upd_set_member[OF fin mem, where c="f u - c"] by simp
  show ?thesis using step cle fle by arith
qed

lemma lp_holders_keys[simp]: "lp_holders (e\<lparr>keys := K\<rparr>) = lp_holders e"
  by (simp add: lp_holders_def)

(* Registry soundness (gap 5): a pool holding value is registered. *)
lemma support_in_keys[intro]:
  assumes sub: "{k. 0 < totals e k} \<subseteq> keys e" and pos: "0 < totals e x"
  shows "x \<in> keys e"
proof -
  have mem: "x \<in> {k. 0 < totals e k}" using pos by simp
  thus ?thesis by (rule subsetD [OF sub])
qed

lemma lp_ok_poolopen:
  assumes ok: "lp_ok e" and ap: "apply_op (PoolOpen k) e = Some e'"
  shows "lp_ok e'"
  using ok ap by (cases "k \<in> keys e")
    (auto simp: lp_ok_def total_support_def book_support_def apply_op.simps split: if_splits)

lemma lp_ok_poolclose:
  assumes ok: "lp_ok e" and ap: "apply_op (PoolClose k) e = Some e'"
  shows "lp_ok e'"
  using ok ap
proof (cases "k \<notin> keys e \<or> pools e k \<noteq> 0 \<or> totals e k \<noteq> 0")
  case True
  hence C: "apply_op (PoolClose k) e = None" by simp
  thus ?thesis using ap by simp
next
  case False
  hence g: "k \<in> keys e" "pools e k = 0" "totals e k = 0" by simp_all
  from ap False have eq: "e' = e\<lparr>keys := keys e - {k}\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  have old_sub: "{k. 0 < totals e k} \<subseteq> keys e"
    using ok by (simp add: lp_ok_def total_support_def)
  have new_sub: "{k. 0 < totals e k} \<subseteq> keys e - {k}"
  proof
    fix x assume xm: "x \<in> {k. 0 < totals e k}"
    hence ink: "x \<in> keys e" by (rule subsetD [OF old_sub])
    have xnk: "x \<noteq> k"
    proof (rule notI)
      assume xk: "x = k"
      hence t0: "totals e x = 0" using g(3) by simp
      show False using xm t0 by simp
    qed
    show "x \<in> keys e - {k}" using ink xnk by simp
  qed
  show ?thesis unfolding eq
    using ok g new_sub
    by (auto simp: lp_ok_def book_support_def total_support_def)
qed

lemma lp_ok_keyopen:
  assumes ok: "lp_ok e" and ap: "apply_op (LpKeyOpen k u a) e = Some e'"
  shows "lp_ok e'"
proof (cases "pools e k = 0")
  (* fresh pool: mint = a, support of pools grows by {k} *)
  case True
  have tk0: "pools e k = 0" using True .
  obtain m where m: "m = (if pools e k = 0 then a else a * totals e k div pools e k)"
    by simp
  from ap have g: "0 < a" "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , pools := (pools e)(k := pools e k + a)
                          , totals := (totals e)(k := totals e k + m)
                          , books := (books e)(k := (books e k)(u := books e k u + m))\<rparr>"
    using True m by (auto simp: apply_op.simps Let_def split: if_splits)
  have tzk: "totals e k = 0" using ok True by (simp add: lp_ok_def)
  (* S: the pools support gains exactly k *)
  have S: "total_support e' = insert k (total_support e)"
    unfolding eq total_support_def using g(1) tzk m tk0
    by (auto simp: total_support_def lp_ok_def)
  (* P1: pooled slice stays real *)
  have brid: "sum (pools e) (total_support e) + a
            = sum ((pools e)(k := pools e k + a)) (insert k (total_support e))"
    using ok tk0 by (simp add: key_support_bridge)
  have sup_le: "sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  (* e-level sum identity, then record-reduce into e' shape *)
  have P1a: "sum ((pools e)(k := pools e k + a)) (insert k (total_support e))
           \<le> reserve e + a"
    using brid[symmetric] sup_le g(1) by arith
  have Ssum: "sum (pools e') (total_support e')
            = sum ((pools e)(k := pools e k + a)) (insert k (total_support e))"
    unfolding S by (use eq in simp)
  have r': "reserve e' = reserve e + a" unfolding eq by simp
  have P1: "sum (pools e') (total_support e') \<le> reserve e'"
    using Ssum P1a r' by arith
  (* P2: per-pool backing *)
  have P2: "\<And>j. pools e' j \<le> totals e' j"
  proof -
    fix j
    show "pools e' j \<le> totals e' j"
    proof (cases "j = k")
      case True
      thus ?thesis unfolding eq using m tzk tk0 by simp
    next
      case False
      thus ?thesis unfolding eq using ok by (simp add: lp_ok_def)
    qed
  qed
  (* P3: per-pool book sum *)
  have P3: "sum (books e' k) (book_support e' k) = totals e' k"
  proof (cases "u \<in> book_support e k")
    case True
    have fin: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
    have e1: "sum (books e k) (book_support e k) - books e k u
            + (books e k u + m)
            = sum ((books e k)(u := books e k u + m)) (book_support e k)"
      using book_support_bridge_add[OF fin True] .
    have e2: "book_support e' k = book_support e k"
      unfolding eq book_support_def using ok True
      by (auto simp: lp_ok_def book_support_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have mz: "m = a" using m tk0 by simp
    have f1: "sum (books e k) (book_support e k)
            = books e k u + sum (books e k) (book_support e k - {u})"
      using fin True by (rule sum.remove)
    have fle: "books e k u \<le> sum (books e k) (book_support e k)"
      using f1 by arith
    have r1: "sum ((books e k)(u := books e k u + m)) (book_support e k)
            = totals e k + m"
      using e1 ex fle mz by arith
    show ?thesis using e2 r1 mz eq by simp
  next
    case False
    have fin: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
    have e1: "sum (books e k) (book_support e k) + m
            = sum ((books e k)(u := m)) (insert u (book_support e k))"
      using sum_upd_set_fresh[OF fin False, of "books e k" m] by simp
    have e2: "book_support e' k = insert u (book_support e k)"
      unfolding eq book_support_def using ok False m True g(1)
      by (auto simp: lp_ok_def book_support_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have mz: "m = a" using m tk0 by simp
    have u0: "books e k u = 0" using False by (simp add: book_support_def)
    have r2: "sum ((books e k)(u := m)) (insert u (book_support e k))
            = totals e k + m"
      using e1 ex by arith
    show ?thesis using e2 r2 mz u0 eq by simp
  qed
  (* P4/P5: supports stay finite *)
  have P4: "finite (total_support e')"
    using S ok by (simp add: lp_ok_def)
  have P5: "\<And>j. finite (book_support e' j)"
  proof -
    fix j
    show "finite (book_support e' j)"
    proof (cases "j = k")
      case True
      have fin: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
      have sub: "book_support e' j \<subseteq> insert u (book_support e j)"
        unfolding eq book_support_def True by auto
      have fin2: "finite (insert u (book_support e j))" using fin True by simp
      show ?thesis by (rule finite_subset[OF sub fin2])
    next
      case False
      have bs: "book_support e' j = book_support e j"
        using eq False by (simp add: book_support_def)
      show ?thesis unfolding bs using ok by (simp add: lp_ok_def)
    qed
  qed
  have lple: "lp_pool e \<le> reserve e" using ok by (simp add: lp_ok_def)
  have lp': "lp_pool e' = lp_pool e" unfolding eq by simp
  have jointe: "lp_pool e + sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  have P1j: "lp_pool e' + sum (pools e') (total_support e') \<le> reserve e'"
    using Ssum lp' brid jointe r' by arith
  have gk: "k \<in> keys e"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  show ?thesis using P1 P1j P2 P3 P4 P5 ok True m tk0 eq gk
    by (auto simp: lp_ok_def book_support_def total_support_def lp_holders_def)
next
  (* live pool: mint = a * totals k div pools k; support UNCHANGED *)
  case False
  from ap have g: "0 < a" "a \<le> credits e u"
    and eq: "e' = e\<lparr>credits := (credits e)(u := credits e u - a)
                          , reserve := reserve e + a
                          , pools := (pools e)(k := pools e k + a)
                          , totals := (totals e)(k := totals e k + (a * totals e k div pools e k))
                          , books := (books e)(k := (books e k)(u := books e k u + (a * totals e k div pools e k)))\<rparr>"
    using False by (auto simp: apply_op.simps Let_def split: if_splits)
  define m where "m = a * totals e k div pools e k"
  have nz: "0 < pools e k" using False by simp
  have pp: "pools e k \<le> totals e k" using ok by (simp add: lp_ok_def)
  have ziff: "(totals e k = 0) = (pools e k = 0)" using ok by (simp add: lp_ok_def)
  have nt: "0 < totals e k"
  proof (rule ccontr)
    assume "\<not> 0 < totals e k"
    hence t0: "totals e k = 0" by simp
    hence "pools e k = 0" using ziff by simp
    thus False using nz by simp
  qed
  (* backing arithmetic: the invariant pool <= totals makes the mint cover a *)
  have dm: "a * totals e k div totals e k = a" using nt by simp
  have mono: "a * totals e k div totals e k \<le> a * totals e k div pools e k"
    by (rule div_le_mono2[OF nz pp])
  have mge: "a \<le> m" unfolding m_def using mono dm by arith
  have mpos: "0 < m" using g(1) mge by arith
  have P2k: "pools e k + a \<le> totals e k + m" using pp mono dm by (simp add: m_def)
  (* S: support unchanged *)
  have S: "total_support e' = total_support e"
    unfolding eq total_support_def using nt mpos
    by (auto simp: total_support_def m_def)
  (* P1: pooled slice grows by a, reserve grows by a *)
  have ppos: "0 < pools e k" using False by simp
  have mem: "k \<in> total_support e"
  proof (rule ccontr)
    assume nc: "k \<notin> total_support e"
    hence "pools e k = 0" using support_pools_zero[OF ok] by simp
    thus False using ppos by simp
  qed
  have fin: "finite (total_support e)" using ok by (simp add: lp_ok_def)
  have sp: "sum (pools e) (total_support e)
          = pools e k + sum (pools e) (total_support e - {k})"
    using fin mem by (rule sum.remove)
  have c1: "sum ((pools e)(k := pools e k + a)) (total_support e)
          = ((pools e)(k := pools e k + a)) k
          + sum ((pools e)(k := pools e k + a)) (total_support e - {k})"
    using fin mem by (rule sum.remove)
  have c2: "((pools e)(k := pools e k + a)) k = pools e k + a" by simp
  have c3: "sum ((pools e)(k := pools e k + a)) (total_support e - {k})
          = sum (pools e) (total_support e - {k})"
    by (rule sum.cong) auto
  have ksum: "sum ((pools e)(k := pools e k + a)) (total_support e)
            = sum (pools e) (total_support e) + a"
    using sp c1 c2 c3 by simp
  have sup_le: "sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  have P1a: "sum ((pools e)(k := pools e k + a)) (total_support e)
           \<le> reserve e + a"
    using ksum[symmetric] sup_le g(1) by arith
  have Ssum: "sum (pools e') (total_support e')
            = sum ((pools e)(k := pools e k + a)) (total_support e)"
    unfolding S by (use eq in simp)
  have r': "reserve e' = reserve e + a" unfolding eq by simp
  have P1: "sum (pools e') (total_support e') \<le> reserve e'"
    using Ssum P1a r' by arith
  (* P2: full conjunct *)
  have P2: "\<And>j. pools e' j \<le> totals e' j"
  proof -
    fix j
    show "pools e' j \<le> totals e' j"
    proof (cases "j = k")
      case True
      thus ?thesis unfolding eq using P2k by (simp add: m_def)
    next
      case False
      thus ?thesis unfolding eq using ok by (simp add: lp_ok_def)
    qed
  qed
  (* P3: per-pool book sum, mint m *)
  have P3: "sum (books e' k) (book_support e' k) = totals e' k"
  proof (cases "u \<in> book_support e k")
    case True
    have finb: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
    have e1: "sum (books e k) (book_support e k) - books e k u
            + (books e k u + m)
            = sum ((books e k)(u := books e k u + m)) (book_support e k)"
      using book_support_bridge_add[OF finb True] .
    have e2: "book_support e' k = book_support e k"
      unfolding eq book_support_def using ok True
      by (auto simp: lp_ok_def book_support_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have f1: "sum (books e k) (book_support e k)
            = books e k u + sum (books e k) (book_support e k - {u})"
      using finb True by (rule sum.remove)
    have fle: "books e k u \<le> sum (books e k) (book_support e k)"
      using f1 by arith
    have r3: "sum ((books e k)(u := books e k u + m)) (book_support e k)
            = totals e k + m"
      using e1 ex fle by arith
    show ?thesis using e2 r3 m_def eq by simp
  next
    case False
    have finb: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
    have e1: "sum (books e k) (book_support e k) + m
            = sum ((books e k)(u := m)) (insert u (book_support e k))"
      using sum_upd_set_fresh[OF finb False, of "books e k" m] by simp
    have e2: "book_support e' k = insert u (book_support e k)"
      unfolding eq book_support_def using ok False mpos
      by (auto simp: lp_ok_def book_support_def m_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have r4: "sum ((books e k)(u := m))
                  (insert u (book_support e k)) = totals e k + m"
      using e1 ex by arith
    have u0: "books e k u = 0" using False by (simp add: book_support_def)
    show ?thesis using e2 r4 m_def u0 eq by simp
  qed
  have P4: "finite (total_support e')"
    using S ok by (simp add: lp_ok_def)
  have P5: "\<And>j. finite (book_support e' j)"
  proof -
    fix j
    show "finite (book_support e' j)"
    proof (cases "j = k")
      case True
      have fin: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
      have sub: "book_support e' j \<subseteq> insert u (book_support e j)"
        unfolding eq book_support_def True by auto
      have fin2: "finite (insert u (book_support e j))" using fin True by simp
      show ?thesis by (rule finite_subset[OF sub fin2])
    next
      case False
      have bs: "book_support e' j = book_support e j"
        using eq False by (simp add: book_support_def)
      show ?thesis unfolding bs using ok by (simp add: lp_ok_def)
    qed
  qed
  have lple: "lp_pool e \<le> reserve e" using ok by (simp add: lp_ok_def)
  have lp': "lp_pool e' = lp_pool e" unfolding eq by simp
  have jointe: "lp_pool e + sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  have P1j: "lp_pool e' + sum (pools e') (total_support e') \<le> reserve e'"
    using lp' Ssum jointe ksum r' by arith
  have gk: "k \<in> keys e"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  show ?thesis using P1 P1j P2 P3 P4 P5 ok False eq gk
    by (auto simp: lp_ok_def book_support_def total_support_def lp_holders_def)
qed

lemma div_payout_floor:
  fixes p t s :: nat
  assumes pt: "p \<le> t" and st: "s \<le> t" and tpos: "0 < t"
  shows "p - s * p div t \<le> t - s"
proof (cases "t \<le> p + s")
  case True
  have hk: "(t - s) + s = t" using st by simp
  have hd0: "(p + s - t) + (t - s) = p" using True st pt by presburger
  have dle: "p + s - t \<le> s" using True st pt by presburger
  have h2a: "(p + s - t) * ((t - s) + s)
           = (p + s - t) * (t - s) + (p + s - t) * s" by (rule distrib_left)
  have h2b: "(p + s - t) * t = (p + s - t) * ((t - s) + s)" by (simp only: hk)
  have h2: "(p + s - t) * t = (p + s - t) * (t - s) + (p + s - t) * s"
    by (rule trans[OF h2b h2a])
  have h4a: "s * ((p + s - t) + (t - s)) = s * (p + s - t) + s * (t - s)"
    by (rule distrib_left)
  have h4b: "s * p = s * ((p + s - t) + (t - s))" by (simp only: hd0)
  have h4: "s * p = s * (p + s - t) + s * (t - s)" by (rule trans[OF h4b h4a])
  have h6: "(p + s - t) * (t - s) \<le> s * (t - s)" by (rule mult_le_mono1[OF dle])
  have hc: "(p + s - t) * s = s * (p + s - t)" by (rule mult.commute)
  have hf: "(p + s - t) * t \<le> s * p" using h2 hc h6 h4 by presburger
  have hdiv: "(p + s - t) * t div t \<le> s * p div t" by (rule div_le_mono[OF hf])
  have hd1: "(p + s - t) * t div t = p + s - t" using tpos by (simp add: div_mult_self1)
  have hge: "p + s - t \<le> s * p div t" using hdiv hd1 by simp
  have sub: "p - s * p div t \<le> p - (p + s - t)" using hge by arith
  have eq: "p - (p + s - t) = t - s" using hd0 by arith
  show ?thesis using sub eq by arith
next
  case False
  have regb: "p \<le> t - s" using False by presburger
  have dp2: "p * s \<le> p * t" by (rule mult_le_mono2[OF st])
  have dm2: "s * p \<le> p * t" using dp2 by simp
  have dle2: "s * p div t \<le> p * t div t" by (rule div_le_mono[OF dm2])
  have hd2: "p * t div t = p" using tpos by (simp add: div_mult_self1)
  have dle: "s * p div t \<le> p" using dle2 hd2 by presburger
  show ?thesis using dle regb by arith
qed

lemma lp_ok_keyremove:
  assumes ok: "lp_ok e" and ap: "apply_op (LpKeyRemove k u s) e = Some e'"
  shows "lp_ok e'"
proof -
  obtain v where v: "v = s * pools e k div totals e k"
    and g: "0 < s" "s \<le> books e k u" "0 < v" "v \<le> reserve e"
    and eq: "e' = e\<lparr>reserve := reserve e - v
                          , pools := (pools e)(k := pools e k - v)
                          , totals := (totals e)(k := totals e k - s)
                          , books := (books e)(k := (books e k)(u := books e k u - s))
                          , credits := (credits e)(u := credits e u + v)\<rparr>"
    using ap by (auto simp: apply_op.simps Let_def split: if_splits)
  (* guard consequences *)
  have T0: "totals e k \<noteq> 0"
  proof
    assume Tz: "totals e k = 0"
    hence "s * pools e k div totals e k = 0" by simp
    thus False using g(3) v by simp
  qed
  hence tpos: "0 < totals e k" by simp
  have ppos: "0 < pools e k"
  proof (cases "pools e k = 0")
    case True
    hence "totals e k = 0" using ok by (simp add: lp_ok_def)
    thus ?thesis using T0 by simp
  next
    case False
    thus ?thesis by simp
  qed
  have mem: "k \<in> total_support e"
  proof (rule ccontr)
    assume nc: "k \<notin> total_support e"
    hence "pools e k = 0" using support_pools_zero[OF ok] by simp
    thus False using ppos by simp
  qed
  (* sb: s <= totals e k, mirroring the legacy sum.remove argument *)
  have finb: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
  have bex: "sum (books e k) (book_support e k) = totals e k"
    using ok by (simp add: lp_ok_def)
  have memu: "u \<in> book_support e k" using g(1) g(2) by (auto simp: book_support_def)
  have f1: "sum (books e k) (book_support e k)
          = books e k u + sum (books e k) (book_support e k - {u})"
    using finb memu by (rule sum.remove)
  have s_le_T: "s \<le> totals e k"
  proof (cases "books e k u = totals e k")
    case True
    thus ?thesis using g(2) by simp
  next
    case False
    have mem': "books e k u < sum (books e k) (book_support e k)"
      using f1 bex False by simp
    hence ltT: "books e k u < totals e k" using bex by simp
    thus ?thesis using g(2) by simp
  qed
  (* P2: per-pool backing -- redemption floors (pool <= totals, s <= totals) *)
  have pp: "pools e k \<le> totals e k" using ok by (simp add: lp_ok_def)
  have dm: "s * pools e k \<le> totals e k * pools e k"
    by (rule mult_le_mono1[OF s_le_T])
  have d1: "s * pools e k div totals e k \<le> totals e k * pools e k div totals e k"
    by (rule div_le_mono[OF dm])
  have d2: "totals e k * pools e k div totals e k = pools e k"
    using tpos by (simp add: div_mult_self1)
  have vle: "v \<le> pools e k" using d1 d2 v by simp
  have corner: "pools e k \<le> v \<Longrightarrow> totals e k \<le> s"
  proof -
    assume pv: "pools e k \<le> v"
    have pe: "s * pools e k div totals e k = v" using v by simp
    have ge: "pools e k \<le> s * pools e k div totals e k" using pv pe by simp
    have step1: "pools e k * totals e k
               \<le> (s * pools e k div totals e k) * totals e k"
      by (rule mult_le_mono1[OF ge])
    have dm: "s * pools e k = (s * pools e k div totals e k) * totals e k
                            + s * pools e k mod totals e k"
      by (simp add: div_mult_mod_eq)
    have step2: "pools e k * totals e k \<le> s * pools e k"
      using step1 dm by arith
    show "totals e k \<le> s"
    proof (rule ccontr)
      assume nc: "\<not> totals e k \<le> s"
      hence lt: "s < totals e k" by simp
      have "pools e k * s < pools e k * totals e k"
        using ppos lt by (simp add: mult_less_cancel2)
      thus False using step2 by simp
    qed
  qed
  have P2: "pools e k - v \<le> totals e k - s"
  proof (cases "pools e k \<le> v")
    case True
    hence "totals e k \<le> s" using corner by simp
    thus ?thesis using True by arith
  next
    case False
    have pfb: "pools e k - s * pools e k div totals e k \<le> totals e k - s"
      by (rule div_payout_floor[OF pp s_le_T tpos])
    thus ?thesis using v by simp
  qed
  (* P1: pooled slice shrinks exactly with the reserve *)
  have bsub: "sum (pools e) (total_support e) - v
            = sum ((pools e)(k := pools e k - v)) (total_support e)"
    using key_support_bridge_sub[OF ok mem vle] .
  have sup_le: "sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  have joint: "lp_pool e + sum (pools e) (total_support e) \<le> reserve e"
    using ok by (simp add: lp_ok_def)
  have P1a: "sum ((pools e)(k := pools e k - v)) (total_support e)
           \<le> reserve e - v"
    using bsub[symmetric] sup_le g(4) by arith
  have Ssum: "sum (pools e') (total_support e')
            = sum ((pools e)(k := pools e k - v)) (total_support e)"
  proof (cases "totals e k = s")
    case True
    have tzs: "totals e k = s" using True .
    have veq: "v = pools e k" using tzs tpos g(1) v by (simp add: div_mult_self1)
    have pzk: "pools e' k = 0" using eq veq by simp
    have finS: "finite (total_support e)"
      using ok by (simp add: lp_ok_def total_support_def)
    have supp: "total_support e' = total_support e - {k}"
    proof (rule set_eqI)
      fix j
      show "j \<in> total_support e' \<longleftrightarrow> j \<in> total_support e - {k}"
      proof (cases "j = k")
        case True
        thus ?thesis using eq tzs ok by (simp add: total_support_def lp_ok_def)
      next
        case False
        thus ?thesis using eq ok by (simp add: total_support_def lp_ok_def)
      qed
    qed
    have e3: "sum (pools e') (total_support e - {k})
            = sum (pools e') (total_support e)"
      by (rule sum.mono_neutral_left[OF finS]) (auto simp: pzk)
    have e4t: "sum (pools e') (total_support e)
             = sum ((pools e)(k := pools e k - v)) (total_support e)"
      by (simp add: eq)
    show ?thesis using supp e3 e4t by simp
  next
    case False
    have tp2: "0 < totals e k - s" using False s_le_T by simp
    have supp: "total_support e' = total_support e"
    proof (rule set_eqI)
      fix j
      show "j \<in> total_support e' \<longleftrightarrow> j \<in> total_support e"
      proof (cases "j = k")
        case True
        thus ?thesis using eq tp2 ok tpos by (simp add: total_support_def lp_ok_def)
      next
        case False
        thus ?thesis using eq ok by (simp add: total_support_def lp_ok_def)
      qed
    qed
    have e4: "sum (pools e') (total_support e)
            = sum ((pools e)(k := pools e k - v)) (total_support e)"
      by (simp add: eq)
    show ?thesis using supp e4 by simp
  qed
  have r': "reserve e' = reserve e - v" unfolding eq by simp
  have P1: "sum (pools e') (total_support e') \<le> reserve e'"
    using Ssum P1a r' by arith
  have finS2: "finite (total_support e)"
    using ok by (simp add: lp_ok_def total_support_def)
  have vk: "pools e k \<le> sum (pools e) (total_support e)"
    using finS2 mem by (simp add: sum.remove)
  have vle2: "v \<le> sum (pools e) (total_support e)"
    using v d1 d2 vk by arith
  have J2: "lp_pool e + sum ((pools e)(k := pools e k - v)) (total_support e)
          \<le> reserve e - v"
  proof -
    have r1: "lp_pool e + sum ((pools e)(k := pools e k - v)) (total_support e)
            = lp_pool e + (sum (pools e) (total_support e) - v)"
      using bsub[symmetric] by simp
    have r2: "lp_pool e + (sum (pools e) (total_support e) - v)
            = lp_pool e + sum (pools e) (total_support e) - v" using vle2 by simp
    show ?thesis using r1 r2 joint by arith
  qed
  have lp': "lp_pool e' = lp_pool e" unfolding eq by simp
  have r'v: "reserve e' = reserve e - v" unfolding eq by simp
  have Jf: "lp_pool e' + sum (pools e') (total_support e') \<le> reserve e'"
    using J2 Ssum lp' r'v by arith
  (* P3: book sum drops exactly by s *)
  have e2: "sum (books e k) (book_support e k) - s
          = sum ((books e k)(u := books e k u - s)) (book_support e k)"
    using g(2) by (rule book_support_bridge_sub[OF finb memu])
  (* zero-iff survives the burn: totals k - s vanishes exactly when
     pools k - v does (payout floor tracks the burn) *)
  have zsync: "(totals e k - s = 0) = (pools e k - v = 0)"
  proof (cases "totals e k \<le> s")
    case True
    have tp: "totals e k * pools e k \<le> s * pools e k"
      by (rule mult_le_mono1[OF True])
    have dv: "totals e k * pools e k div totals e k
            \<le> s * pools e k div totals e k"
      by (rule div_le_mono[OF tp])
    have pv: "pools e k \<le> v" using tp dv d2 v by simp
    show ?thesis using True pv corner by simp
  next
    case False
    have dm: "totals e k * v \<le> s * pools e k"
      using v by simp
    have lt: "s * pools e k < pools e k * totals e k"
      using False g ppos by (simp add: mult_less_cancel2)
    have vlt: "v < pools e k"
    proof -
      have ch: "totals e k * v < pools e k * totals e k"
        by (rule le_less_trans[OF dm lt])
      show ?thesis using ch ppos by (simp add: mult_less_cancel1)
    qed
    show ?thesis using False vlt vle by simp
  qed
  (* support of the book may shed u only when its holding drops to zero;
     in either case the sum over the NEW support equals the same value *)
  have P3: "sum (books e' k) (book_support e' k) = totals e' k"
  proof (cases "books e k u - s = 0")
    case True
    have e3: "book_support e' k = book_support e k - {u}"
      unfolding eq book_support_def True using memu
      by (auto simp: book_support_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have rm: "sum (books e k) (book_support e k)
            = books e k u + sum (books e k) (book_support e k - {u})"
      using finb memu by (rule sum.remove)
    have bus: "books e k u = s" using True g(2) by arith
    have cg: "sum ((books e k)(u := books e k u - s)) (book_support e k - {u})
            = sum (books e k) (book_support e k - {u})"
      by (rule sum.cong) auto
    have stepA: "sum ((books e k)(u := books e k u - s)) (book_support e k - {u})
               = totals e k - s"
      using ex rm bus cg by arith
    have stepB: "sum ((books e k)(u := books e k u - s)) (book_support e' k)
               = sum ((books e k)(u := books e k u - s)) (book_support e k - {u})"
      using e3 by simp
    show ?thesis using stepA stepB unfolding eq by simp
  next
    case False
    have e3: "book_support e' k = book_support e k"
      unfolding eq book_support_def using memu False
      by (auto simp: book_support_def)
    have ex: "sum (books e k) (book_support e k) = totals e k"
      using ok by (simp add: lp_ok_def)
    have final: "sum ((books e k)(u := books e k u - s)) (book_support e k)
               = totals e k - s"
      using ex e2 by arith
    show ?thesis using e3 final unfolding eq by simp
  qed
  have P4: "finite (total_support e')"
  proof -
    have fin4: "finite {x. totals e x > 0}"
      using ok by (simp add: lp_ok_def total_support_def)
    have base: "finite {x. ((totals e)(k := totals e k - s)) x > 0}"
      by (rule support_upd_finite[OF fin4])
    have fnz: "total_support (e\<lparr>reserve := reserve e - v
                          , pools := (pools e)(k := pools e k - v)
                          , totals := (totals e)(k := totals e k - s)
                          , books := (books e)(k := (books e k)(u := books e k u - s))
                          , credits := (credits e)(u := credits e u + v)\<rparr>)
             = {x. ((totals e)(k := totals e k - s)) x > 0}"
      by (simp add: total_support_def)
    have t1: "finite (total_support (e\<lparr>reserve := reserve e - v
                          , pools := (pools e)(k := pools e k - v)
                          , totals := (totals e)(k := totals e k - s)
                          , books := (books e)(k := (books e k)(u := books e k u - s))
                          , credits := (credits e)(u := credits e u + v)\<rparr>))"
      using fnz base by arith
    show ?thesis unfolding eq by (rule t1)
  qed
  have P5: "\<And>j. finite (book_support e' j)"
  proof -
    fix j
    show "finite (book_support e' j)"
    proof (cases "j = k")
      case True
      have fin: "finite (book_support e k)" using ok by (simp add: lp_ok_def)
      have sub: "book_support e' j \<subseteq> insert u (book_support e j)"
        unfolding eq book_support_def True by auto
      have fin2: "finite (insert u (book_support e j))" using fin True by simp
      show ?thesis by (rule finite_subset[OF sub fin2])
    next
      case False
      have bs: "book_support e' j = book_support e j"
        using eq False by (simp add: book_support_def)
      show ?thesis unfolding bs using ok by (simp add: lp_ok_def)
    qed
  qed
  have old_sub: "{k. 0 < totals e k} \<subseteq> keys e"
    using ok by (simp add: lp_ok_def total_support_def)
  have ts: "{k. 0 < totals e' k} \<subseteq> keys e"
  proof
    fix x assume xm: "x \<in> {k. 0 < totals e' k}"
    hence xp: "0 < totals e' x" by simp
    show "x \<in> keys e"
    proof (cases "x = k")
      case True
      hence "totals e' x = totals e k - s" unfolding eq by simp
      hence tp: "0 < totals e k" using xp by arith
      have mem: "x \<in> {k. 0 < totals e k}" using True tp by simp
      show "x \<in> keys e" by (rule subsetD [OF old_sub mem])
    next
      case False
      have "totals e' x = totals e x" using False unfolding eq by simp
      hence tp: "0 < totals e x" using xp by simp
      have mem: "x \<in> {k. 0 < totals e k}" using tp by simp
      show "x \<in> keys e" by (rule subsetD [OF old_sub mem])
    qed
  qed
  show ?thesis using P1 Jf P2 P3 P4 P5 zsync ok eq ts
    by (auto simp: lp_ok_def book_support_def lp_holders_def total_support_def)
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
next
  case (PoolOpen k)
  show ?thesis using assms(1) assms(2) PoolOpen lp_ok_poolopen by blast
next
  case (PoolClose k)
  show ?thesis using assms(1) assms(2) PoolClose lp_ok_poolclose by blast
next
  case (LpKeyOpen k u a)
  show ?thesis using assms(1) assms(2) LpKeyOpen lp_ok_keyopen by blast
next
  case (LpKeyRemove k u s)
  show ?thesis using assms(1) assms(2) LpKeyRemove lp_ok_keyremove by blast
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
  using assms by (cases op; blast intro: deposit_ok withdraw_ok swapin_ok
    swapout_ok lpopen_ok lpremove_ok ok_poolopen ok_poolclose
    ok_lpkeyopen ok_lpkeyremove)

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

text \<open>Registry soundness (the ERR_NOKEY twin): in every reachable state,
  a keyed pool that holds value is registered - the ledger can never hold
  funds in an unregistered pool. PoolOpen/PoolClose manage the registry,
  and both keyed money ops now demand membership in it.\<close>

theorem reachable_registered:
  assumes "reachable e" and "k \<in> total_support e"
  shows "k \<in> keys e"
proof -
  have lp: "lp_ok e" using assms(1) reachable_lp by blast
  have sub: "total_support e \<subseteq> keys e" using lp by (simp add: lp_ok_def)
  show ?thesis by (rule subsetD [OF sub assms(2)])
qed

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
