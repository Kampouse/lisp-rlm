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

end
