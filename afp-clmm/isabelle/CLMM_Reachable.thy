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

section \<open>Compositional roundtrip: withdrawal after any reachable history\<close>

text \<open>The escrow layer proves single-op roundtrips neutral per user. Here we
lift the LP-safety half to the reachable world: from ANY reachable state --
i.e. after an arbitrary history of provisions, removals, donations and
price walks -- a bounded withdrawal remains legal (never strands the venue),
removes no more than the book holds at cell \<open>i\<close>, touches nothing else, and
deposit/withdrawal order does not matter for the net book
(\<open>mint_burn_commute\<close>). Honest scope: full book commutation through swaps is
FALSE in general -- swap reindexes cells above the insertion point
(refine/wedge in CLMM_Transformation), so cell identity is positional after
a price walk; reconciling that reindexing is the escrow ledger's job, not
the book's.\<close>

definition roundtrip_withdraw ::
  "pool \<Rightarrow> real \<Rightarrow> real \<Rightarrow> int \<Rightarrow> pool \<Rightarrow> real \<Rightarrow> bool" where
"roundtrip_withdraw P s d i P'' s'' \<longleftrightarrow>
  reachable (P, s) \<and> 0 \<le> d \<and>
  burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"

theorem roundtrip_legal:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
  shows "reachable (P'', s'') \<and> clmm_state P'' s''"
proof -
  from rt have rs: "reachable (P, s)"
    by (simp add: roundtrip_withdraw_def)
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  from rs bd have "reachable (P'', s'')" by (rule reachable.bstep)
  moreover
  have st0: "clmm_state P s" using reachable_clmm_state[OF rs] by simp
  have "clmm_state P'' s''" using burn_preserves[OF st0 bd] by simp
  ultimately show ?thesis by blast
qed

text \<open>Bridge lemmas: burn = pointwise subtraction, stated WITHOUT the
let/pair wrappers so every later theorem consumes these and never unfolds
the nested definitions again (unfolding both together makes simp flounder).\<close>

lemma burn_pointwise:
  assumes bd: "burn_rel d i i (Q, t) (Q', t')"
  shows "lq Q' i = lq Q i - d \<and>
    (\<forall>j. j \<noteq> i \<longrightarrow> lq Q j = lq Q' j) \<and>
    grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
proof -
  from bd have md: "mint_rel d i i (Q', t') (Q, t)"
    by (simp add: burn_rel_def split: prod.splits)
  from md have pj: "\<And>j. lq Q j = (if j = i then lq Q' j + d else lq Q' j)"
    by (simp add: mint_rel_def split: prod.splits)
  have f1: "lq Q' i = lq Q i - d" using pj[of i] by simp
  have f2: "\<And>j. j \<noteq> i \<Longrightarrow> lq Q j = lq Q' j" using pj by simp
  have f3: "grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
    using md by (simp add: mint_rel_def split: prod.splits)
  from f1 f2 f3 show ?thesis by blast
qed

theorem roundtrip_bounded:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
  shows "lq P'' i = lq P i - min d (lq P i)"
proof -
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  have pb: "lq P'' i = lq P i - min d (lq P i)"
    using burn_pointwise[OF bd] by blast
  from pb show ?thesis .
qed

theorem roundtrip_others_intact:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
    and ne: "j \<noteq> i"
  shows "lq P'' j = lq P j \<and> grd P'' = grd P \<and>
    fee P'' = fee P \<and> s'' = s"
proof -
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  from burn_pointwise[OF bd] ne have peq: "lq P j = lq P'' j" by blast
  from burn_pointwise[OF bd] have pg: "grd P'' = grd P" by blast
  from burn_pointwise[OF bd] have pf: "fee P'' = fee P" by blast
  from burn_pointwise[OF bd] have ps: "s'' = s" by blast
  from peq pg pf ps show ?thesis by simp
qed

lemma overburn_rejected:
  assumes bd: "burn_rel d i i (P, s) (P'', s'')"
    and big: "lq P i < d"
  shows False
proof -
  from bd have md: "mint_rel d i i (P'', s'') (P, s)"
    by (simp add: burn_rel_def split: prod.splits)
  from md have dp: "0 \<le> d" by (simp add: mint_rel_def split: prod.splits)
  from md have pj: "\<And>j. lq P j = (if j = i then lq P'' j + d else lq P'' j)"
    by (simp add: mint_rel_def split: prod.splits)
  from pj[of i] have pii: "lq P i = lq P'' i + d" by simp
  from bd have gdi: "0 \<le> lq P'' i"
    by (simp add: burn_rel_def split: prod.splits)
  from dp pii gdi big show False by linarith
qed

lemma mint_pointwise:
  assumes md: "mint_rel d i i (Q, t) (Q', t')"
  shows "lq Q' i = lq Q i + d \<and>
    (\<forall>j. j \<noteq> i \<longrightarrow> lq Q j = lq Q' j) \<and>
    grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
  using md by (simp add: mint_rel_def split: prod.splits)

lemma mint_burn_commute:
  assumes e1: "mint_rel d1 i i (P, s) (P1, s1)"
    and e2: "burn_rel d2 i i (P1, s1) (P2, s2)"
    and e3: "burn_rel d2 i i (P, s) (P3, s3)"
    and e4: "mint_rel d1 i i (P3, s3) (P4, s4)"
  shows "lq P2 = lq P4 \<and> s2 = s4"
proof -
  have p1: "lq P1 i = lq P i + d1 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P j = lq P1 j)"
    using mint_pointwise[OF e1] by blast
  have p2: "lq P2 i = lq P1 i - d2 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P1 j = lq P2 j)"
    using burn_pointwise[OF e2] by blast
  have p3: "lq P3 i = lq P i - d2 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P j = lq P3 j)"
    using burn_pointwise[OF e3] by blast
  have p4: "lq P4 i = lq P3 i + d1 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P3 j = lq P4 j)"
    using mint_pointwise[OF e4] by blast
  have p1i: "lq P1 i = lq P i + d1" using p1 by blast
  have p2i: "lq P2 i = lq P1 i - d2" using p2 by blast
  have p3i: "lq P3 i = lq P i - d2" using p3 by blast
  have p4i: "lq P4 i = lq P3 i + d1" using p4 by blast
  have eqi: "lq P2 i = lq P4 i" using p1i p2i p3i p4i by linarith
  have p1o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P j = lq P1 j" using p1 by blast
  have p2o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P1 j = lq P2 j" using p2 by blast
  have p3o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P j = lq P3 j" using p3 by blast
  have p4o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P3 j = lq P4 j" using p4 by blast
  have lqe: "lq P2 = lq P4"
  proof
    fix j :: int
    show "lq P2 j = lq P4 j"
    proof cases
      assume je: "j = i"
      then show ?thesis using eqi by simp
    next
      assume jne: "j \<noteq> i"
      have c1: "lq P j = lq P1 j" using p1o[of j] jne by blast
      have c2: "lq P1 j = lq P2 j" using p2o[of j] jne by blast
      have c3: "lq P j = lq P3 j" using p3o[of j] jne by blast
      have c4: "lq P3 j = lq P4 j" using p4o[of j] jne by blast
      from c1 c2 c3 c4 show ?thesis by linarith
    qed
  qed
  moreover have "s2 = s4"
  proof -
    from burn_pointwise[OF e2] have sb2: "s2 = s1" by blast
    from burn_pointwise[OF e3] have sb3: "s3 = s" by blast
    from mint_pointwise[OF e1] have sm1: "s1 = s" by blast
    from mint_pointwise[OF e4] have sm4: "s4 = s3" by blast
    from sb2 sb3 sm1 sm4 show "s2 = s4" by simp
  qed
  ultimately show ?thesis by blast
qed

end
