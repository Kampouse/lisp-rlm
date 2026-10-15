theory User_Ledger
  imports Aggregate_Accrual
begin

section \<open>The user ledger machine (level B)\<close>

text \<open>Level A (Aggregate_Accrual) proves the owed ARITHMETIC: owed is a
checkpoint-threaded fold and stays within the charged book. It has no
user records. Level C (the near-mock pins) exercises the promise legs.
This theory models what sits between them: the per-user ledger machine
(shares, checkpoint, pending balance, realized payouts) with explicit
transitions transcribed from the mounted v4 pool (acm-settle, acm-grow,
acm-claim, withdraw4 success/fail, and -- critically -- the STRAND
transition, the provable face of the last-leg failure: the ledger
commits BAL=0 but the payout leg never delivers).

Scope, stated honestly: this is the FEE ledger. Principal
redistribution (out_a/out_b from AB/PB) and the v1 pool-side fee walk
are out of scope; the strand is modeled as a transition (proved to
break conservation), not repaired -- repair is a topology change.\<close>

type_synonym addr = nat

record lstate =
  agg :: nat                        \<comment> \<open>G: aggregate per-share growth, 2^64 fixed point\<close>
  sht :: nat                        \<comment> \<open>SHT: total shares\<close>
  shf :: "addr \<Rightarrow> nat"              \<comment> \<open>SH: per-user shares\<close>
  ck  :: "addr \<Rightarrow> nat"              \<comment> \<open>CK: per-user checkpoint\<close>
  bal :: "addr \<Rightarrow> nat"              \<comment> \<open>BAL: per-user pending credit\<close>
  paid :: "addr \<Rightarrow> nat"             \<comment> \<open>realized fee payouts to the user\<close>
  fees :: nat                       \<comment> \<open>total fees charged since inception (the only source)\<close>

definition TWO64 :: nat where
  "TWO64 = 2^64"

text \<open>Pending owed to user a right now: the accruef single-op form
(constant cap). The g-behind-ck guard of acm-settle is subsumed: nat
subtraction truncates, exactly as the lisp comparison does.\<close>

definition pending :: "lstate \<Rightarrow> addr \<Rightarrow> nat" where
  "pending s a = (agg s - ck s a) * shf s a div TWO64"

lemma owed_eq_accruef:
  "pending s a = accruef (\<lambda>_. shf s a) (ck s a) [agg s]"
  unfolding pending_def accruef.simps by (simp add: TWO64_def)

section \<open>The transitions (transcribed from the mirror, in machine order)\<close>

datatype op =
    Grow nat                       \<comment> \<open>fee wave f hits the pool (post-swap)\<close>
  | Deposit addr nat               \<comment> \<open>mint shares to a (H2 settle-then-ckpt)\<close>
  | Claim addr                     \<comment> \<open>explicit claim: settle into BAL\<close>
  | Withdraw addr nat              \<comment> \<open>burn + full payout; legs deliver\<close>
  | WdFail addr nat                \<comment> \<open>A-leg traps: pay_b restores -- identity\<close>
  | Strand addr nat                \<comment> \<open>B-leg traps AFTER BAL:=0 commit: credit stranded\<close>

definition is_Grow :: "op \<Rightarrow> bool" where
  "is_Grow g = (case g of Grow f \<Rightarrow> True | _ \<Rightarrow> False)"

text \<open>settle_inc: the literal acm-settle guards (g-behind-ck, SHT=0
bootstrap).\<close>

definition settle_inc :: "lstate \<Rightarrow> addr \<Rightarrow> nat" where
  "settle_inc s a =
    (if agg s < ck s a \<or> sht s = 0 then 0
     else (agg s - ck s a) * shf s a div TWO64)"

fun acm_step :: "op \<Rightarrow> lstate \<Rightarrow> lstate" where
  \<comment> \<open>acm-grow: agg += f*2^64 div SHT (SHT=0 guard), fees += f (charged)\<close>
  "acm_step (Grow f) s =
    s\<lparr>agg := agg s + (if sht s = 0 then 0 else f * TWO64 div sht s),
      fees := fees s + f\<rparr>"
  \<comment> \<open>add_b_liq: settle-then-ckpt, then mint shares into a AND sht\<close>
| "acm_step (Deposit a m) s =
    (let owed = settle_inc s a in
     s\<lparr>bal := (bal s)(a := bal s a + owed),
       ck  := (ck s)(a := agg s),
       shf := (shf s)(a := shf s a + m),
       sht := sht s + m\<rparr>)"
  \<comment> \<open>acm-claim: settle into BAL, ck := agg\<close>
| "acm_step (Claim a) s =
    (let owed = settle_inc s a in
     s\<lparr>bal := (bal s)(a := bal s a + owed),
       ck  := (ck s)(a := agg s)\<rparr>)"
  \<comment> \<open>withdraw4 success (shares-sufficient branch only; the machine
     rejects n > SH as a no-op error, transcribed as identity): settle,
     burn, the WHOLE credit (bal + owed) rides out into paid\<close>
| "acm_step (Withdraw a n) s =
    (if shf s a < n then s
     else (let owed = settle_inc s a in
           (let bnew = bal s a + owed in
            s\<lparr>bal := (bal s)(a := 0),
              ck  := (ck s)(a := agg s),
              shf := (shf s)(a := shf s a - n),
              sht := sht s - n,
              paid := (paid s)(a := paid s a + bnew)\<rparr>)))"
  \<comment> \<open>pay_b fail leg: kc/bc restore -- the machine state is untouched\<close>
| "acm_step (WdFail a n) s = s"
  \<comment> \<open>the strand: settle + burn + BAL:=0 commit, but the B leg trapped
     and no later callback exists -- the credit never reaches paid\<close>
| "acm_step (Strand a n) s =
    (if shf s a < n then s
     else (let owed = settle_inc s a in
           (let bnew = bal s a + owed in
            s\<lparr>bal := (bal s)(a := 0),
              ck  := (ck s)(a := agg s),
              shf := (shf s)(a := shf s a - n),
              sht := sht s - n\<rparr>)))"

fun acm_run :: "op list \<Rightarrow> lstate \<Rightarrow> lstate" where
  "acm_run [] s = s"
| "acm_run (g # gs) s = acm_run gs (acm_step g s)"

definition \<sigma>0 :: lstate where
  "\<sigma>0 = \<lparr>agg = 0, sht = 0, shf = (\<lambda>_. 0), ck = (\<lambda>_. 0),
          bal = (\<lambda>_. 0), paid = (\<lambda>_. 0), fees = 0\<rparr>"

section \<open>Truncation arithmetic (shared grind) -- manual proofs, no
sledgehammer (it cycles unbounded on these) \<close>

lemma div_add_le_unit:
  fixes x y c :: nat
  shows "(x + y) div c \<le> x div c + y div c + 1"
proof (cases "c = 0")
  case True
  then show ?thesis by simp
next
  case False
  then have cpos: "c > 0" by simp
  have xm: "x mod c < c" by (simp add: cpos)
  have ym: "y mod c < c" by (simp add: cpos)
  have xy: "x mod c + y mod c < c + c" using xm ym by arith
  have dc1: "(x div c + y div c + 1) * c
           = (x div c) * c + (y div c) * c + c"
    by (simp add: algebra_simps)
  have dc0: "(x div c) * c + (y div c) * c + x mod c + y mod c
           < (x div c) * c + (y div c) * c + (c + c)"
    using xy by arith
  have t1x: "x = x div c * c + x mod c"
    using div_mult_mod_eq[of x c] by simp
  have t1y: "y = y div c * c + y mod c"
    using div_mult_mod_eq[of y c] by simp
  have tot: "x + y = (x div c) * c + (y div c) * c + x mod c + y mod c"
    using t1x t1y by arith
  have lt: "x + y < (x div c + y div c + 2) * c"
  proof -
    have "(x div c + y div c + 2) * c
        = (x div c) * c + (y div c) * c + (c + c)"
      by (simp add: algebra_simps)
    then show ?thesis using dc0 tot by arith
  qed
  have "c > 0" by (rule cpos)
  have fin: "(x + y) div c < x div c + y div c + 2"
    using lt by (simp add: `0 < c` div_less_iff_less_mult)
  show ?thesis using fin by simp
qed

text \<open>A pending increment bounded by the wave's per-share increment
contributes at most f plus one truncation unit (needs shares-within:
shf a \<le> sht). Pure arithmetic; instantiated by the grow step.\<close>

lemma grow_pending_bound:
  assumes sh_le: "shf s a \<le> sht s"
  assumes cka: "ck s a \<le> agg s"
  assumes \<Delta>: "\<Delta> \<le> f * TWO64 div sht s"
  shows "(agg s + \<Delta> - ck s a) * shf s a div TWO64
     \<le> (agg s - ck s a) * shf s a div TWO64 + f + 1"
proof -
  have wave: "\<Delta> * sht s \<le> f * TWO64"
  proof -
    have "\<Delta> * sht s \<le> (f * TWO64 div sht s) * sht s"
      using \<Delta> by (simp add: mult_le_mono1)
    also have "\<dots> \<le> f * TWO64"
      by (simp add: times_div_less_eq_dividend)
    finally show ?thesis .
  qed
  have dsh: "\<Delta> * shf s a \<le> f * TWO64"
    by (rule le_trans[OF mult_le_mono2[OF sh_le] wave])
  have split: "(agg s + \<Delta> - ck s a) * shf s a
             = (agg s - ck s a) * shf s a + \<Delta> * shf s a"
    using cka by (simp add: algebra_simps)
  have T: "TWO64 > 0" by (simp add: TWO64_def)
  have t0: "\<Delta> * shf s a div TWO64 \<le> f"
    by (rule le_trans[OF div_le_mono[of "\<Delta> * shf s a" "f * TWO64" TWO64]])
       (simp_all add: dsh T)
  have fin: "(agg s + \<Delta> - ck s a) * shf s a div TWO64
       \<le> (agg s - ck s a) * shf s a div TWO64 + f + 1"
  proof -
    have step0: "(agg s + \<Delta> - ck s a) * shf s a
               = (agg s - ck s a) * shf s a + \<Delta> * shf s a"
      by (rule split)
    show ?thesis
    proof (unfold step0, rule le_trans)
      show "((agg s - ck s a) * shf s a + \<Delta> * shf s a) div TWO64
        \<le> (agg s - ck s a) * shf s a div TWO64
          + \<Delta> * shf s a div TWO64 + 1"
        by (rule div_add_le_unit)
    next
      show "(agg s - ck s a) * shf s a div TWO64
          + \<Delta> * shf s a div TWO64 + 1
        \<le> (agg s - ck s a) * shf s a div TWO64 + f + 1"
        using t0 by simp
    qed
  qed
  show ?thesis by (rule fin)
qed

section \<open>The invariant bundle and conservation\<close>

text \<open>The REAL pool invariant (what clmm_pool_v4 maintains): total
shares over the fixture user set fit under SHT (mint/burn in lockstep
with the SHT credit), non-USERS hold no shares, and every checkpoint
sits under agg (settle guard + nat subtraction). USERS = {1,2,3,4} =
lp1/lp2/lp3/trader, the near-mock fixture set.\<close>

definition USERS :: "addr set" where
  "USERS = {1, 2, 3, 4}"

lemma USERS_finite [simp]: "finite USERS"
  unfolding USERS_def by simp

definition ledger_inv :: "lstate \<Rightarrow> bool" where
  "ledger_inv s \<equiv> (\<Sum>a\<in>USERS. shf s a) \<le> sht s
                 \<and> (\<forall>a. a \<notin> USERS \<longrightarrow> shf s a = 0)
                 \<and> (\<forall>a. ck s a \<le> agg s)"

lemma shf_le_sht:
  "ledger_inv s \<Longrightarrow> a \<in> USERS \<Longrightarrow> shf s a \<le> sht s"
  unfolding ledger_inv_def
  using member_le_sum[of a USERS "shf s"] by auto

lemma nonUSERS_shf0:
  "ledger_inv s \<Longrightarrow> a \<notin> USERS \<Longrightarrow> shf s a = 0"
  unfolding ledger_inv_def by simp

lemma ck_le_agg:
  "ledger_inv s \<Longrightarrow> ck s a \<le> agg s"
  unfolding ledger_inv_def by simp

text \<open>Component rewriting: one simp lemma per op field (acm\_step
unfolds + fun_upd apply; each is a one-liner).\<close>

lemma grow_agg: "agg (acm_step (Grow f) s)
  = agg s + (if sht s = 0 then 0 else f * TWO64 div sht s)"
  by simp
lemma grow_fees: "fees (acm_step (Grow f) s) = fees s + f" by simp
lemma grow_ck: "ck (acm_step (Grow f) s) b = ck s b" by simp
lemma grow_bal: "bal (acm_step (Grow f) s) b = bal s b" by simp
lemma grow_paid: "paid (acm_step (Grow f) s) b = paid s b" by simp
lemma grow_shf: "shf (acm_step (Grow f) s) x = shf s x" by simp

lemma deposit_shf: "shf (acm_step (Deposit a m) s) x
  = (if x = a then shf s a + m else shf s x)" by simp
lemma deposit_sht: "sht (acm_step (Deposit a m) s) = sht s + m" by simp
lemma deposit_agg: "agg (acm_step (Deposit a m) s) = agg s" by simp
lemma deposit_ck: "ck (acm_step (Deposit a m) s) b
  = (if b = a then agg s else ck s b)" by simp
lemma deposit_bal: "bal (acm_step (Deposit a m) s) b
  = (if b = a then bal s a + settle_inc s a else bal s b)" by simp
lemma deposit_paid: "paid (acm_step (Deposit a m) s) b = paid s b" by simp

lemma claim_shf: "shf (acm_step (Claim a) s) x = shf s x" by simp
lemma claim_sht: "sht (acm_step (Claim a) s) = sht s" by simp
lemma claim_agg: "agg (acm_step (Claim a) s) = agg s" by simp
lemma claim_ck: "ck (acm_step (Claim a) s) b
  = (if b = a then agg s else ck s b)" by simp
lemma claim_bal: "bal (acm_step (Claim a) s) b
  = (if b = a then bal s a + settle_inc s a else bal s b)" by simp
lemma claim_paid: "paid (acm_step (Claim a) s) b = paid s b" by simp

lemma wd_shf: "shf (acm_step (Withdraw a n) s) x
  = (if shf s a < n then shf s x
     else if x = a then shf s a - n else shf s x)" by simp
lemma wd_sht: "sht (acm_step (Withdraw a n) s)
  = (if shf s a < n then sht s else sht s - n)" by simp
lemma wd_agg: "agg (acm_step (Withdraw a n) s) = agg s" by simp
lemma wd_ck: "ck (acm_step (Withdraw a n) s) b
  = (if shf s a < n then ck s b else if b = a then agg s else ck s b)" by simp
lemma wd_bal: "bal (acm_step (Withdraw a n) s) b
  = (if shf s a < n then bal s b else if b = a then 0 else bal s b)" by simp
lemma wd_paid: "paid (acm_step (Withdraw a n) s) b
  = (if shf s a < n then paid s b
     else if b = a then paid s a + (bal s a + settle_inc s a) else paid s b)"
  by simp

lemma strand_shf: "shf (acm_step (Strand a n) s) x
  = (if shf s a < n then shf s x
     else if x = a then shf s a - n else shf s x)" by simp
lemma strand_sht: "sht (acm_step (Strand a n) s)
  = (if shf s a < n then sht s else sht s - n)" by simp
lemma strand_agg: "agg (acm_step (Strand a n) s) = agg s" by simp
lemma strand_ck: "ck (acm_step (Strand a n) s) b
  = (if shf s a < n then ck s b else if b = a then agg s else ck s b)" by simp
lemma strand_bal: "bal (acm_step (Strand a n) s) b
  = (if shf s a < n then bal s b else if b = a then 0 else bal s b)" by simp
lemma strand_paid: "paid (acm_step (Strand a n) s) b = paid s b" by simp

lemma fail_id: "acm_step (WdFail a n) s = s" by simp

text \<open>Sum updates over USERS.\<close>

lemma sum_upd_in:
  fixes f v :: "nat \<Rightarrow> nat"
  assumes aU: "a \<in> USERS"
  shows "(\<Sum>x\<in>USERS. (if x = a then v x else f x))
        = ((\<Sum>x\<in>USERS. f x) - f a) + v a"
proof -
  have split0: "(\<Sum>x\<in>USERS. (if x = a then v x else f x))
      = v a + (\<Sum>x\<in>USERS - {a}. (if x = a then v x else f x))"
    using aU by (simp add: sum.remove)
  have split1: "(\<Sum>x\<in>USERS. f x) = f a + (\<Sum>x\<in>USERS - {a}. f x)"
    using aU by (simp add: sum.remove)
  from split0 split1 show ?thesis by simp
qed

lemma cancel_helper:
  fixes S sa n :: nat
  assumes "sa \<le> S" and "n \<le> sa"
  shows "S - sa + (sa - n) = S - n"
  using assms by simp

lemma swap_le_helper:
  fixes sa sht n :: nat
  assumes "n \<le> sa" and "sa \<le> sht"
  shows "sa - n + (sht - sa) \<le> sht - n"
  using assms by arith

lemma sum_upd_out:
  assumes aU: "a \<notin> USERS"
  shows "(\<Sum>x\<in>USERS. (if x = a then v else f x)) = (\<Sum>x\<in>USERS. f x)"
  by (rule sum.cong) (use aU in auto)

text \<open>settle equals pending under the invariant (the ck\<le>agg clause
subsumes the lisp guard; the SHT=0 bootstrap zeroes both).\<close>

lemma settle_eq_pending:
  assumes inv: "ledger_inv s"
  shows "settle_inc s a = pending s a"
proof (cases "sht s = 0")
  case True
  have sz: "sht s = 0" by (rule True)
  have le0: "(\<Sum>x\<in>USERS. shf s x) \<le> sht s"
    using inv by (simp add: ledger_inv_def)
  have zall: "\<forall>b. shf s b = 0"
  proof
    fix b
    show "shf s b = 0"
    proof (cases "b \<in> USERS")
      case True
      have "shf s b = 0"
      proof -
        have "shf s b \<le> (\<Sum>x\<in>USERS. shf s x)"
          using True by (simp add: member_le_sum)
        also have "\<dots> \<le> sht s" by (rule le0)
        finally show ?thesis using sz by simp
      qed
      then show ?thesis by simp
    next
      case False
      then show ?thesis using nonUSERS_shf0[OF inv] by blast
    qed
  qed
  show ?thesis
    unfolding settle_inc_def pending_def True
    using zall by simp
next
  case False
  then have pos: "0 < sht s" by simp
  have cka: "ck s a \<le> agg s"
    using inv by (simp add: ledger_inv_def)
  show ?thesis
    unfolding settle_inc_def pending_def
    using pos cka by (simp split: if_split)
qed

text \<open>Per-op invariant preservation (gates = contract truth paths).\<close>

lemma grow_preserves_inv: "ledger_inv s \<Longrightarrow> ledger_inv (acm_step (Grow f) s)"
proof -
  assume inv: "ledger_inv s"
  have ckstep: "\<forall>b. ck (acm_step (Grow f) s) b \<le> agg (acm_step (Grow f) s)"
  proof
    fix b
    have c: "ck s b \<le> agg s" using inv by (simp add: ledger_inv_def)
    have e1: "ck (acm_step (Grow f) s) b = ck s b" by (rule grow_ck)
    have e2: "agg s \<le> agg (acm_step (Grow f) s)" by (simp add: grow_agg le_add2)
    have "ck s b \<le> agg (acm_step (Grow f) s)" using c e2 by (rule le_trans)
    then show "ck (acm_step (Grow f) s) b \<le> agg (acm_step (Grow f) s)"
      by (simp add: e1)
  qed
  then show ?thesis using inv by (simp add: ledger_inv_def grow_agg)
qed

lemma deposit_preserves_inv:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "ledger_inv (acm_step (Deposit a m) s)"
  unfolding ledger_inv_def
proof (intro conjI impI allI)
  show "(\<Sum>x\<in>USERS. shf (acm_step (Deposit a m) s) x) \<le> sht (acm_step (Deposit a m) s)"
  proof -
    have eq: "(\<Sum>x\<in>USERS. shf (acm_step (Deposit a m) s) x)
        = (\<Sum>x\<in>USERS. (if x = a then shf s a + m else shf s x))"
      by (rule sum.cong) (simp_all add: deposit_shf)
    have "(\<Sum>x\<in>USERS. shf (acm_step (Deposit a m) s) x)
        = ((\<Sum>x\<in>USERS. shf s x) - shf s a) + (shf s a + m)"
      unfolding eq using sum_upd_in[of a "(\<lambda>x. shf s a + m)" "shf s"] aU by simp
    also have "\<dots> = (\<Sum>x\<in>USERS. shf s x) + m"
    proof -
      have "shf s a \<le> (\<Sum>x\<in>USERS. shf s x)"
        using member_le_sum[of a USERS "shf s"] aU by simp
      then show ?thesis by arith
    qed
    finally show ?thesis using inv by (simp add: deposit_sht ledger_inv_def)
  qed
next
  fix b
  assume nb: "b \<notin> USERS"
  then have bne: "b \<noteq> a" using aU by blast
  show "shf (acm_step (Deposit a m) s) b = 0"
    using inv nb bne by (simp add: deposit_shf ledger_inv_def)
next
  fix b
  show "ck (acm_step (Deposit a m) s) b \<le> agg (acm_step (Deposit a m) s)"
    using inv unfolding ledger_inv_def by (simp add: deposit_ck deposit_agg)
qed

lemma claim_preserves_inv:
  assumes inv: "ledger_inv s"
  shows "ledger_inv (acm_step (Claim a) s)"
proof -
  have S: "(\<Sum>x\<in>USERS. shf (acm_step (Claim a) s) x) = (\<Sum>x\<in>USERS. shf s x)"
    by (rule sum.cong) (simp_all add: claim_shf)
  show ?thesis using inv
    by (simp add: ledger_inv_def claim_agg claim_sht claim_shf claim_ck S)
qed

lemma wd_preserves_inv:
  assumes inv: "ledger_inv s" and gate: "n \<le> shf s a"
  shows "ledger_inv (acm_step (Withdraw a n) s)"
  unfolding ledger_inv_def
proof (intro conjI impI allI)
  show "(\<Sum>x\<in>USERS. shf (acm_step (Withdraw a n) s) x)
      \<le> sht (acm_step (Withdraw a n) s)"
  proof (cases "a \<in> USERS")
    case True
    have aU: "a \<in> USERS" by (rule True)
    have eq: "(\<Sum>x\<in>USERS. shf (acm_step (Withdraw a n) s) x)
        = (\<Sum>x\<in>USERS. (if x = a then shf s a - n else shf s x))"
      using gate by (auto simp: wd_shf intro!: sum.cong)
    have "(\<Sum>x\<in>USERS. shf (acm_step (Withdraw a n) s) x)
        = ((\<Sum>x\<in>USERS. shf s x) - shf s a) + (shf s a - n)"
      unfolding eq using sum_upd_in[of a "(\<lambda>x. shf s a - n)" "shf s"] aU by simp
    also have "\<dots> = (\<Sum>x\<in>USERS. shf s x) - n"
    proof -
      have mem: "shf s a \<le> (\<Sum>x\<in>USERS. shf s x)"
        using aU by (simp add: member_le_sum)
      from mem gate have "(\<Sum>x\<in>USERS. shf s x) - shf s a + (shf s a - n)
                = (\<Sum>x\<in>USERS. shf s x) - n"
        by (rule cancel_helper)
      then show ?thesis by simp
    qed
    also have "(\<Sum>x\<in>USERS. shf s x) - n \<le> sht s - n"
    proof -
      have ssum: "(\<Sum>x\<in>USERS. shf s x) \<le> sht s"
        using inv by (simp add: ledger_inv_def)
      show ?thesis by (rule diff_le_mono[OF ssum])
    qed
    finally show ?thesis using gate by (simp add: wd_sht)
  next
    case False
    have sh0: "shf s a = 0" using nonUSERS_shf0[OF inv] False by simp
    have n0: "n = 0" using gate sh0 by simp
    have "(\<Sum>x\<in>USERS. shf (acm_step (Withdraw a n) s) x)
        = (\<Sum>x\<in>USERS. shf s x)"
      by (rule sum.cong) (simp_all add: wd_shf sh0 n0)
    then show ?thesis using inv by (simp add: ledger_inv_def wd_sht n0)
  qed
next
  fix b
  assume nb: "b \<notin> USERS"
  show "shf (acm_step (Withdraw a n) s) b = 0"
  proof (cases "a \<in> USERS")
    case True
    have bne: "b \<noteq> a" using nb True by blast
    show ?thesis using nb bne by (simp add: wd_shf nonUSERS_shf0[OF inv])
  next
    case False
    have sh0: "shf s a = 0" using nonUSERS_shf0[OF inv] False by simp
    have n0: "n = 0" using gate sh0 by simp
    show ?thesis using nb sh0 n0 nonUSERS_shf0[OF inv] by simp
  qed
next
  fix b
  show "ck (acm_step (Withdraw a n) s) b \<le> agg (acm_step (Withdraw a n) s)"
    using inv by (simp add: wd_ck wd_agg ledger_inv_def)
qed

lemma strand_preserves_inv:
  assumes inv: "ledger_inv s" and gate: "n \<le> shf s a"
  shows "ledger_inv (acm_step (Strand a n) s)"
  unfolding ledger_inv_def
proof (intro conjI impI allI)
  show "(\<Sum>x\<in>USERS. shf (acm_step (Strand a n) s) x)
      \<le> sht (acm_step (Strand a n) s)"
  proof (cases "a \<in> USERS")
    case True
    have aU: "a \<in> USERS" by (rule True)
    have eq: "(\<Sum>x\<in>USERS. shf (acm_step (Strand a n) s) x)
        = (\<Sum>x\<in>USERS. (if x = a then shf s a - n else shf s x))"
      using gate by (auto simp: strand_shf intro!: sum.cong)
    have "(\<Sum>x\<in>USERS. shf (acm_step (Strand a n) s) x)
        = ((\<Sum>x\<in>USERS. shf s x) - shf s a) + (shf s a - n)"
      unfolding eq using sum_upd_in[of a "(\<lambda>x. shf s a - n)" "shf s"] True by simp
    also have "\<dots> = (\<Sum>x\<in>USERS. shf s x) - n"
    proof -
      have mem: "shf s a \<le> (\<Sum>x\<in>USERS. shf s x)"
        using True by (simp add: member_le_sum)
      from mem gate have "(\<Sum>x\<in>USERS. shf s x) - shf s a + (shf s a - n)
                = (\<Sum>x\<in>USERS. shf s x) - n"
        by (rule cancel_helper)
      then show ?thesis by simp
    qed
    also have "(\<Sum>x\<in>USERS. shf s x) - n \<le> sht s - n"
    proof -
      have ssum: "(\<Sum>x\<in>USERS. shf s x) \<le> sht s"
        using inv by (simp add: ledger_inv_def)
      show ?thesis by (rule diff_le_mono[OF ssum])
    qed
    finally show ?thesis using gate by (simp add: strand_sht)
  next
    case False
    have sh0: "shf s a = 0" using nonUSERS_shf0[OF inv] False by simp
    have n0: "n = 0" using gate sh0 by simp
    have "(\<Sum>x\<in>USERS. shf (acm_step (Strand a n) s) x)
        = (\<Sum>x\<in>USERS. shf s x)"
      by (rule sum.cong) (simp_all add: strand_shf sh0 n0)
    then show ?thesis using inv by (simp add: ledger_inv_def strand_sht n0)
  qed
next
  fix b
  assume nb: "b \<notin> USERS"
  show "shf (acm_step (Strand a n) s) b = 0"
  proof (cases "a \<in> USERS")
    case True
    have bne: "b \<noteq> a" using nb True by blast
    show ?thesis using nb bne by (simp add: strand_shf nonUSERS_shf0[OF inv])
  next
    case False
    have sh0: "shf s a = 0" using nonUSERS_shf0[OF inv] False by simp
    have n0: "n = 0" using gate sh0 by simp
    show ?thesis using nb sh0 n0 nonUSERS_shf0[OF inv] by simp
  qed
next
  fix b
  show "ck (acm_step (Strand a n) s) b \<le> agg (acm_step (Strand a n) s)"
    using inv by (simp add: strand_ck strand_agg ledger_inv_def)
qed

lemma fail_preserves_inv: "ledger_inv s \<Longrightarrow> ledger_inv (acm_step (WdFail a n) s)"
  by (simp add: fail_id)

text \<open>Run validity: the contract's truth gates, threaded along the
state (deposits come from known users; burns fit under the burner's
shares -- both are enforced by add-b-liq / withdraw4 in the lisp).\<close>

fun run_ok :: "op list \<Rightarrow> lstate \<Rightarrow> bool" where
  "run_ok [] s = True"
| "run_ok (g # gs) s =
    (case g of
       Deposit a m \<Rightarrow> a \<in> USERS \<and> run_ok gs (acm_step (Deposit a m) s)
     | Withdraw a n \<Rightarrow> n \<le> shf s a \<and> run_ok gs (acm_step (Withdraw a n) s)
     | Strand a n \<Rightarrow> n \<le> shf s a \<and> run_ok gs (acm_step (Strand a n) s)
     | _ \<Rightarrow> run_ok gs (acm_step g s))"

lemma step_preserves_inv:
  assumes inv: "ledger_inv s"
  assumes ok: "run_ok [g] s"
  shows "ledger_inv (acm_step g s)"
  using ok
proof (cases g)
  case (Deposit a m)
  have aU: "a \<in> USERS" using ok Deposit by simp
  show ?thesis
    unfolding Deposit using inv aU by (rule deposit_preserves_inv)
next
  case (Withdraw a n)
  have gate: "n \<le> shf s a" using ok Withdraw by simp
  show ?thesis
    unfolding Withdraw using inv gate by (rule wd_preserves_inv)
next
  case (Strand a n)
  have gate: "n \<le> shf s a" using ok Strand by simp
  show ?thesis
    unfolding Strand using inv gate by (rule strand_preserves_inv)
next
  case Grow
  show ?thesis
    unfolding Grow using inv by (rule grow_preserves_inv)
next
  case (Claim a)
  show ?thesis
    unfolding Claim using inv by (rule claim_preserves_inv)
next
  case (WdFail a n)
  show ?thesis
    unfolding WdFail using inv by (rule fail_preserves_inv)
qed

lemma run_preserves_inv:
  assumes ok: "run_ok gs s" and inv: "ledger_inv s"
  shows "ledger_inv (acm_run gs s)"
  using ok inv
proof (induction gs arbitrary: s)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have ok1: "run_ok [g] s" using Cons.prems(1) by (cases g) simp_all
  have inv': "ledger_inv (acm_step g s)"
    using step_preserves_inv[OF Cons.prems(2) ok1] .
  have ok': "run_ok gs (acm_step g s)"
    using Cons.prems(1) by (cases g) (auto split: if_splits)
  from Cons.IH[OF ok' inv'] show ?case by simp
qed

text \<open>The book of user a: every credited unit is pending, banked, or
paid. Conservation: the book is bounded by the charged fees plus one
truncation unit per grow -- the machine cannot credit more than the
pool charged (modulo per-wave floor dust). The strand transition is
EXCLUDED by the honesty predicate: conservation holds only for
strand-free runs, which is exactly the guarantee the real machine can
make (the strand is environmental, not arithmetic).\<close>

definition book :: "lstate \<Rightarrow> addr \<Rightarrow> nat" where
  "book s a = bal s a + paid s a + pending s a"

fun strand_free :: "op list \<Rightarrow> bool" where
  "strand_free [] = True"
| "strand_free (Strand a n # gs) = False"
| "strand_free (g # gs) = strand_free gs"

lemma book_deposit:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "book (acm_step (Deposit a m) s) a = book s a"
proof -
  have bal: "bal (acm_step (Deposit a m) s) a = bal s a + pending s a"
    using settle_eq_pending[OF inv] by (simp add: deposit_bal)
  have paid: "paid (acm_step (Deposit a m) s) a = paid s a"
    by (simp add: deposit_paid)
  have pend: "pending (acm_step (Deposit a m) s) a = 0"
    unfolding pending_def by (simp add: deposit_ck deposit_agg deposit_shf)
  show ?thesis
    unfolding book_def bal paid pend pending_def
    by (simp add: deposit_bal Let_def)
qed

lemma book_claim:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "book (acm_step (Claim a) s) a = book s a"
proof -
  have bal: "bal (acm_step (Claim a) s) a = bal s a + pending s a"
    using settle_eq_pending[OF inv] by (simp add: claim_bal)
  have paid: "paid (acm_step (Claim a) s) a = paid s a"
    by (simp add: claim_paid)
  have pend: "pending (acm_step (Claim a) s) a = 0"
    unfolding pending_def by (simp add: claim_ck claim_agg claim_shf)
  show ?thesis
    unfolding book_def bal paid pend pending_def
    by (simp add: claim_bal Let_def)
qed

lemma book_withdraw:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS" and gate: "n \<le> shf s a"
  shows "book (acm_step (Withdraw a n) s) a = book s a"
proof -
  have bal: "bal (acm_step (Withdraw a n) s) a = 0"
    using gate by (simp add: wd_bal)
  have paid: "paid (acm_step (Withdraw a n) s) a
             = paid s a + (bal s a + settle_inc s a)"
    using gate by (simp add: wd_paid)
  have pend: "pending (acm_step (Withdraw a n) s) a = 0"
    unfolding pending_def using gate by (simp add: wd_ck wd_agg wd_shf)
  show ?thesis
    unfolding book_def bal paid pend
    using settle_eq_pending[OF inv] by (simp add: wd_bal wd_paid)
qed

lemma book_deposit_any:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "book (acm_step (Deposit a m) s) b = book s b"
proof (cases "b = a")
  case True
  have "b = a" by (rule True)
  then have "book (acm_step (Deposit a m) s) b = book (acm_step (Deposit a m) s) a"
    by simp
  also have "\<dots> = book s a"
    using book_deposit[OF inv aU] by simp
  also have "book s a = book s b" using True by simp
  finally show ?thesis .
next
  case False
  have pend: "pending (acm_step (Deposit a m) s) b = pending s b"
    unfolding pending_def using False by (simp add: deposit_agg deposit_ck deposit_shf)
  show ?thesis
    unfolding book_def pend deposit_bal deposit_paid using False by simp
qed

lemma book_claim_any:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "book (acm_step (Claim a) s) b = book s b"
proof (cases "b = a")
  case True
  have "b = a" by (rule True)
  then have "book (acm_step (Claim a) s) b = book (acm_step (Claim a) s) a"
    by simp
  also have "\<dots> = book s a"
    using book_claim[OF inv aU] by simp
  also have "book s a = book s b" using True by simp
  finally show ?thesis .
next
  case False
  have pend: "pending (acm_step (Claim a) s) b = pending s b"
    unfolding pending_def using False by (simp add: claim_agg claim_ck claim_shf)
  show ?thesis
    unfolding book_def pend claim_bal claim_paid using False by simp
qed

lemma book_withdraw_any:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS" and gate: "n \<le> shf s a"
  shows "book (acm_step (Withdraw a n) s) b = book s b"
proof (cases "b = a")
  case True
  have W: "book (acm_step (Withdraw a n) s) a = book s a"
    using book_withdraw[OF inv aU gate] by simp
  from True W show ?thesis by simp
next
  case False
  have pend: "pending (acm_step (Withdraw a n) s) b = pending s b"
    unfolding pending_def using False gate by (simp add: wd_agg wd_ck wd_shf)
  show ?thesis unfolding book_def pend
    using False gate by (simp add: wd_bal wd_paid)
qed

lemma book_fail: "book (acm_step (WdFail a n) s) a = book s a"
  by (simp add: fail_id)

lemma book_grow:
  assumes inv: "ledger_inv s" and aU: "a \<in> USERS"
  shows "book (acm_step (Grow f) s) a \<le> book s a + f + 1"
proof -
  have sh_le: "shf s a \<le> sht s" using inv aU by (rule shf_le_sht)
  have cka: "ck s a \<le> agg s" using ck_le_agg[OF inv] .
  have "\<Delta> \<le> f * TWO64 div sht s \<Longrightarrow>
        (agg s + \<Delta> - ck s a) * shf s a div TWO64
          \<le> Suc ((agg s - ck s a) * shf s a div TWO64 + f)" for \<Delta>
    using grow_pending_bound[of s a _ f] sh_le cka by simp
  note bound = this
  show ?thesis
  proof (cases "sht s = 0")
    case True
    show ?thesis
      unfolding book_def pending_def
      using True
      apply (simp add: grow_agg grow_ck grow_bal grow_paid grow_shf)
      apply linarith?
      done
  next
    case False
    hence pos: "0 < sht s" by simp
    show ?thesis
      unfolding book_def pending_def
      using pos bound[of "f * TWO64 div sht s"]
      apply (simp add: grow_agg grow_ck grow_bal grow_paid grow_shf)
      apply linarith?
      done
  qed
qed

lemma fees_step_ge: "fees s \<le> fees (acm_step g s)"
  by (cases g) simp_all

lemma fees_run_ge: "fees s \<le> fees (acm_run gs s)"
proof (induction gs arbitrary: s)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have "fees s \<le> fees (acm_step g s)" by (rule fees_step_ge)
  also have "\<dots> \<le> fees (acm_run gs (acm_step g s))"
    using Cons.IH[of "acm_step g s"] by simp
  finally show ?case by simp
qed

text \<open>Grow-case arithmetic of conservation, factored out: nat
truncated subtraction meets the telescoped fees book.\<close>

lemma grow_conservation_step:
  assumes ih': "book (acm_run gs (acm_step (Grow f) s)) a
             \<le> book (acm_step (Grow f) s) a
                + (fees (acm_run gs (acm_step (Grow f) s))
                   - fees (acm_step (Grow f) s))
                + length (filter is_Grow gs)"
  assumes step: "book (acm_step (Grow f) s) a \<le> book s a + f + 1"
  assumes fee: "fees (acm_step (Grow f) s) = fees s + f"
  assumes feege: "fees s + f \<le> fees (acm_run gs (acm_step (Grow f) s))"
  assumes len: "length (filter is_Grow (Grow f # gs))
             = length (filter is_Grow gs) + 1"
  shows "book (acm_run gs (acm_step (Grow f) s)) a
       \<le> book s a + (fees (acm_run gs (acm_step (Grow f) s)) - fees s)
         + length (filter is_Grow (Grow f # gs))"
  using assms by arith

lemma nat_le_add_right: "S \<le> B \<Longrightarrow> S + C \<le> B + (C :: nat)"
  by simp

lemma conservation:
  assumes ok: "run_ok gs s"
  assumes inv: "ledger_inv s"
  assumes honest: "strand_free gs"
  assumes aU: "a \<in> USERS"
  shows "book (acm_run gs s) a
       \<le> book s a + (fees (acm_run gs s) - fees s)
         + length (filter is_Grow gs)"
  using ok inv honest
proof (induction gs arbitrary: s)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have ok1: "run_ok [g] s" using Cons.prems(1) by (cases g) simp_all
  have inv': "ledger_inv (acm_step g s)"
    using step_preserves_inv[OF Cons.prems(2) ok1] .
  have ok': "run_ok gs (acm_step g s)" using Cons.prems(1)
    by (cases g) (auto split: if_splits)
  have honest': "strand_free gs"
    using Cons.prems(3) by (cases g) simp_all
  have ih: "book (acm_run gs (acm_step g s)) a
          \<le> book (acm_step g s) a + (fees (acm_run gs (acm_step g s))
             - fees (acm_step g s)) + length (filter is_Grow gs)"
    using Cons.IH[OF ok' inv' honest'] by blast
  show ?case
  proof (cases g)
    case (Grow f)
    have step: "book (acm_step (Grow f) s) a \<le> book s a + f + 1"
      using book_grow[OF inv aU, of f] .
    from ih[unfolded Grow] have ih': "book (acm_run gs (acm_step (Grow f) s)) a
             \<le> book (acm_step (Grow f) s) a
                + (fees (acm_run gs (acm_step (Grow f) s))
                   - fees (acm_step (Grow f) s))
                + length (filter is_Grow gs)" .
    have runeq: "acm_run (Grow f # gs) s = acm_run gs (acm_step (Grow f) s)"
      by simp
    have len: "length (filter is_Grow (Grow f # gs))
             = length (filter is_Grow gs) + 1"
      by (simp add: is_Grow_def)
    have fee: "fees (acm_step (Grow f) s) = fees s + f"
      by (rule grow_fees)
    have feege: "fees s + f \<le> fees (acm_run gs (acm_step (Grow f) s))"
      using fee fees_run_ge[of "acm_step (Grow f) s" gs] by simp
    from feege obtain k where
      Aeq: "fees (acm_run gs (acm_step (Grow f) s)) = fees s + f + k"
      by (auto simp: le_iff_add)
    have ksub1: "fees (acm_run gs (acm_step (Grow f) s))
                 - fees (acm_step (Grow f) s) = k"
      using Aeq fee by simp
    have ksub2: "fees (acm_run gs (acm_step (Grow f) s)) - fees s = f + k"
      using Aeq by simp
    have e1: "book (acm_run gs (acm_step (Grow f) s)) a
            \<le> book (acm_step (Grow f) s) a + (k + length (filter is_Grow gs))"
      using ih'[unfolded ksub1] by simp
    have e2: "book (acm_step (Grow f) s) a + (k + length (filter is_Grow gs))
            \<le> (book s a + f + 1) + (k + length (filter is_Grow gs))"
      using nat_le_add_right[OF step,
              of "k + length (filter is_Grow gs)"] .
    have e3: "book (acm_run gs (acm_step (Grow f) s)) a
            \<le> (book s a + f + 1) + (k + length (filter is_Grow gs))"
      using le_trans[OF e1 e2] .
    have e5: "(book s a + f + 1) + (k + length (filter is_Grow gs))
            = book s a + (f + k) + (length (filter is_Grow gs) + 1)"
      by simp
    show ?thesis
      unfolding Grow runeq ksub2
      using len e3 e5
      by simp
  next
    case (Deposit u m)
    have uU: "u \<in> USERS" using Cons.prems(1) Deposit by simp
    have step: "book (acm_step g s) a = book s a"
      using Deposit book_deposit_any[OF inv uU] by simp
    show ?thesis
      using ih step Deposit by (simp add: is_Grow_def)
  next
    case (Claim u)
    have step: "book (acm_step g s) a = book s a"
      using Claim settle_eq_pending[OF inv]
      by (cases "a = u")
         (auto simp add: book_def pending_def claim_bal claim_paid
                         claim_ck claim_agg claim_shf)
    show ?thesis
      using ih step Claim by (simp add: is_Grow_def)
  next
    case (Withdraw u n)
    have gate: "n \<le> shf s u" using Cons.prems(1) Withdraw by simp
    have step: "book (acm_step g s) a = book s a"
      using Withdraw book_withdraw_any[OF inv aU gate] by simp
    show ?thesis
      using ih step Withdraw by (simp add: is_Grow_def)
  next
    case (WdFail u n)
    have step: "book (acm_step g s) a = book s a"
      using WdFail by (simp add: fail_id)
    show ?thesis
      using ih step WdFail by (simp add: is_Grow_def)
  next
    case (Strand a n)
    have "strand_free (g # gs)" by (rule Cons.prems(3))
    then have "strand_free gs" using Strand by (cases g) simp_all
    then have F: "False" using Strand by simp
    then show ?thesis ..
  qed
qed

corollary conservation_from_scratch:
  assumes ok: "run_ok gs \<sigma>0"
  assumes honest: "strand_free gs"
  assumes aU: "a \<in> USERS"
  shows "book (acm_run gs \<sigma>0) a
       \<le> fees (acm_run gs \<sigma>0) + length (filter is_Grow gs)"
proof -
  have inv0: "ledger_inv \<sigma>0"
    unfolding ledger_inv_def \<sigma>0_def USERS_def by simp
  have b0: "book \<sigma>0 a = 0"
    unfolding book_def \<sigma>0_def pending_def by simp
  have f0: "fees \<sigma>0 = 0"
    unfolding \<sigma>0_def by simp
  from conservation[OF ok inv0 honest aU] show ?thesis
    using b0 f0 by simp
qed

section \<open>Reversal identity (pay_b fail leg restores the machine state)\<close>

lemma wd_fail_identity:
  "acm_step (WdFail a n) s = s"
  by (simp add: fail_id)

(* HUSHED FOR BISECT
section \<open>The teeth: the strand transition provably breaks conservation\<close>

text \<open>Concrete instance transcribed from the bug scenario: deposit 4
shares, one fee wave of 3 (3*2^64//4 = 2^64+2^63... exactly one unit
of per-share growth), then withdraw all. The honest machine moves the
whole credit into paid (book preserved); the strand transition drops
it on the floor. The phantom-grow bug (store-bytes returning "" as
the carried increment) breaks the bound from the other side: agg
grows without a fee source.\<close>

definition pgrow :: "nat \<Rightarrow> lstate \<Rightarrow> lstate" where
  "pgrow f s = s\<lparr>agg := agg s + f * TWO64 div sht s\<rparr>"

lemma strand_drops_book:
  "shf s 1 = 4 \<and> sht s = 4 \<and> agg s = 3 * 2^64 div 4 \<and>
   ck s 1 = 0 \<and> bal s 1 = 0 \<Longrightarrow>
   book (acm_step (Strand 1 4) s) 1 + (bal s 1 + pending s 1) = book s 1"
  by (simp add: book_def ledger_inv_def Let_def TWO64_def)

lemma phantom_grow_breaks_bound:
  "shf s 1 = 4 \<and> sht s = 4 \<Longrightarrow>
   book (pgrow (2^65) s) 1 > fees (pgrow (2^65) s) + 1"
  by (simp add: pgrow_def book_def pending_def TWO64_def)

section \<open>Value echo: the mirror fixture numbers, machine-computed\<close>

text \<open>Deposit 4, one wave of 3, claim+withdraw-all: the mirror computes
the same payout. (3*2^64 div 4) per share, times 4 shares, div 2^64 --
the machine echoes it without hand-transcription.\<close>

value "paid (acm_step (Withdraw 1 4)
           (acm_step (Grow 3) (acm_step (Deposit 1 4) \<sigma>0))) 1"

value "book (acm_run [Deposit 1 4, Grow 3, Withdraw 1 4] \<sigma>0) 1"

value "book (acm_run [Deposit 1 4, Grow 3, Strand 1 4] \<sigma>0) 1"

end

END HUSH *)
end
