theory
  CLMM_Reachable
  imports
    "CLMM_Dynamic"
begin

section \<open>Reachability: the invariant holds on every state the events can produce\<close>

text \<open>Layer 1. \<open>reachable\<close> is the smallest relation containing a well-formed
seed and closed under mint, burn, and swap. Each event's preservation theorem
shows \<open>clmm_state\<close> is an inductive invariant; layer-1 (the span/con
discipline that every static-layer theorem consumes) therefore holds on every
reachable state, and the transformation stack (pool_comb_clmm_dsc,
CLMM_Transformation) applies verbatim to reachable books.\<close>

inductive reachable :: "pool \<times> real \<Rightarrow> bool" where
  init: "clmm_state P s \<Longrightarrow> reachable (P, s)"
| mstep: "\<lbrakk>reachable (P, s); mint_rel d a b (P, s) (P', s')\<rbrakk> \<Longrightarrow> reachable (P', s')"
| bstep: "\<lbrakk>reachable (P, s); burn_rel d a b (P, s) (P', s')\<rbrakk> \<Longrightarrow> reachable (P', s')"
| sstep: "\<lbrakk>reachable (P, s); swap_rel v P s P' s'\<rbrakk> \<Longrightarrow> reachable (P', s')"

theorem reachable_clmm_state:
  assumes "reachable Ps"
  shows "clmm_state (fst Ps) (snd Ps)"
  using assms
proof induction
  case init
  then show ?case by simp
next
  case (mstep d a b P s P' s')
  then show ?case by (simp add: mint_preserves)
next
  case (bstep d a b P s P' s')
  then show ?case by (simp add: burn_preserves)
next
  case (sstep v P s P' s')
  then show ?case by (simp add: swap_preserves)
qed

section \<open>Donation-safety: unattributed liquidity cannot skew claims\<close>

text \<open>The Euler-style attack: force-send tokens to the contract outside the
normal deposit path so that a share accumulator is skewed in the attacker's
favour. In this model a donation is a raw pointwise addition of liquidity at
one cell. Two facts make the class structurally impossible here: the
donation is exactly a mint event (so every mint guarantee transfers), and it
is \<^emph>\<open>isolated\<close> \<open>---\<close> grid, fee schedule, price, and every other cell are
bit-identical after the event, so no cross-cell accumulator can move.
Honest scope: fee-\<open>owed\<close> accumulator skew is a layer-2 (accrual ledger)
concern and is bounded separately by the accrual WIP; this section kills the
book-level class.\<close>

definition donate_rel :: "real \<Rightarrow> int \<Rightarrow> pool \<times> real \<Rightarrow> pool \<times> real \<Rightarrow> bool" where
"donate_rel v i Ps Ps' \<longleftrightarrow>
  (let (P, s) = Ps; (P', s') = Ps' in
    0 \<le> v \<and> s' = s \<and> grd P' = grd P \<and> fee P' = fee P \<and>
    (\<forall>j. lq P' j = (if j = i then lq P j + v else lq P j)))"

lemma donate_is_mint:
  assumes "donate_rel v i Ps Ps'"
  shows "mint_rel v i i Ps Ps'"
proof -
  have rng: "\<And>j. (i \<le> j \<and> j \<le> i) = (j = (i::int))" by auto
  show ?thesis using assms by (simp add: donate_rel_def mint_rel_def rng)
qed

lemma donate_isolated:
  assumes "donate_rel v i (P, s) (P', s')"
  shows "s' = s" and "grd P' = grd P" and "fee P' = fee P"
    and "\<And>j. j \<noteq> i \<Longrightarrow> lq P' j = lq P j"
    and "lq P' i = lq P i + v"
  using assms by (simp_all add: donate_rel_def split: prod.splits)

theorem donate_preserves:
  assumes st: "clmm_state P s"
    and d: "donate_rel v i (P, s) (P', s')"
  shows "clmm_state P' s'"
  using mint_preserves[OF st donate_is_mint[OF d]] .

theorem reachable_donate:
  assumes r: "reachable (P, s)"
    and d: "donate_rel v i (P, s) (P', s')"
  shows "reachable (P', s')"
  using reachable.mstep[OF r donate_is_mint[OF d]] .

end
