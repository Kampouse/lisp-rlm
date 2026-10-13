theory CLMM_Dynamic
  imports "Concentrated_Liquidity_Market_Making_Operations.CLMM_Transformation"
begin

section \<open>Dynamic composition: pools as evolving states\<close>

text \<open>The AFP entry treats pools as static objects and swap/refine/join as
total functions. A deployed venue is a labelled transition system: LPs mint
and burn, traders swap, and every theorem of the static layer must hold in
every reachable state. We model the state as a pool with an lq book and a
current price, and events as relations on states.

The state locale is grounded in the entry's own notions: a dynamic venue is
a discretely-consistent pool whose liquidity book is nonempty and whose
current price lies inside the populated price span
\<^term>\<open>grd_min P \<le> sqp \<and> sqp \<le> grd_max P\<close> (Grid_Information: grd_min/grd_max are
the prices at the outer edges of nz_support (lq P)).\<close>

type_synonym pool = "(int \<Rightarrow> real) \<times> (int \<Rightarrow> real) \<times> (int \<Rightarrow> real)"

locale clmm_state =
  fixes P :: pool and sqp :: real
  assumes dsc: "clmm_dsc P"
  assumes nz:  "nz_support (lq P) \<noteq> {}"
  assumes sqp_span: "grd_min P \<le> sqp \<and> sqp \<le> grd_max P"
begin

text \<open>The support is finite (clmm_dsc), so idx_min/idx_max land inside it
and both edge cells carry liquidity: a venue can never be empty at its
edges. This is the fact burn preconditions will have to preserve.\<close>

lemma sup_fin: "finite (nz_support (lq P))"
  using dsc by (simp add: clmm_dsc_def finite_liq_def)

lemma active_min_nz: "lq P (idx_min (lq P)) \<noteq> 0"
  using nz sup_fin by (rule idx_min_finite_in)

lemma active_max_nz: "lq P (idx_max (lq P)) \<noteq> 0"
  using nz sup_fin by (rule idx_max_finite_in)

text \<open>Span facts: the grounding is non-vacuous. The span is a genuine
interval with positive width, and the price inside it is positive.\<close>

lemma bounds_welldef: "grd_min P = grd P (idx_min (lq P))"
  by (simp add: grd_min_def idx_min_img_def)

lemma bounds_welldef_hi: "grd_max P = grd P (idx_max (lq P) + 1)"
  by (simp add: grd_max_def idx_max_img_def)

lemma bounds_strict: "grd_min P < grd_max P"
  by (simp add: dsc nz liq_grd_min_max)

lemma sqp_pos: "0 < sqp"
proof -
  have "0 < grd_min P" using dsc nz by (simp add: liq_grd_min)
  then show ?thesis using sqp_span by linarith
qed

end

section \<open>Events as relations on states\<close>

text \<open>Mint/burn: change lq over a range by delta, grid and price untouched.
Mint is total (support only grows, so the span can only widen outward).
Burn is guarded: the relation only permits burns that keep the post book
nonnegative, keep the support nonempty, and keep the (unchanged) price
inside the post span. The guard is not decoration:
unguarded_burn_kills_invariant exhibits a burn that satisfies every weaker
natural condition (nonnegative book, nonempty support, liquidity only
removed) and still strands the price outside the span.\<close>

definition mint_rel :: "real \<Rightarrow> int \<Rightarrow> int \<Rightarrow> pool \<times> real \<Rightarrow> pool \<times> real \<Rightarrow> bool" where
"mint_rel d a b Ps Ps' \<longleftrightarrow>
  (let (P, s) = Ps; (P', s') = Ps' in
    d \<ge> 0 \<and> a \<le> b \<and> s' = s \<and>
    grd P' = grd P \<and> fee P' = fee P \<and>
    (\<forall>i. lq P' i = (if a \<le> i \<and> i \<le> b then lq P i + d else lq P i)))"

definition burn_rel :: "real \<Rightarrow> int \<Rightarrow> int \<Rightarrow> pool \<times> real \<Rightarrow> pool \<times> real \<Rightarrow> bool" where
"burn_rel d a b Ps Ps' \<longleftrightarrow>
  mint_rel d a b Ps' Ps \<and>
  nz_support (lq (fst Ps')) \<noteq> {} \<and>
  (\<forall>i. 0 \<le> lq (fst Ps') i) \<and>
  grd_min (fst Ps') \<le> snd Ps \<and> snd Ps \<le> grd_max (fst Ps')"

section \<open>Preservation: liquidity events keep the venue well-formed\<close>

subsection \<open>Support monotonicity arithmetic\<close>

lemma nz_grow:
  fixes f g :: "int \<Rightarrow> real"
  assumes "\<And>i. f i \<le> g i" and "\<And>i. 0 \<le> f i"
  shows "nz_support f \<subseteq> nz_support g"
proof
  fix i :: int
  assume i: "i \<in> nz_support f"
  then have fnz: "f i \<noteq> 0" by (simp add: nz_support_def)
  then have "0 < f i" using assms(2)[of i] fnz by linarith
  then have "0 < g i" using assms(1)[of i] by simp
  then show "i \<in> nz_support g" by (simp add: nz_support_def)
qed

lemma nz_shrink:
  fixes f g :: "int \<Rightarrow> real"
  assumes "\<And>i. g i \<le> f i" and "\<And>i. 0 \<le> g i"
  shows "nz_support g \<subseteq> nz_support f"
proof
  fix i :: int
  assume i: "i \<in> nz_support g"
  then have gnz: "g i \<noteq> 0" by (simp add: nz_support_def)
  then have "0 < g i" using assms(2)[of i] gnz by linarith
  then have "0 < f i" using assms(1)[of i] by simp
  then show "i \<in> nz_support f" by (simp add: nz_support_def)
qed

lemma finite_nz_of_subset_range:
  fixes f f' :: "int \<Rightarrow> real"
  assumes sub: "nz_support f' \<subseteq> nz_support f \<union> {a..b}"
    and fin: "finite (nz_support f)"
  shows "finite (nz_support f')"
  by (rule finite_subset[OF sub]) (simp add: fin)

lemma idx_min_anti:
  fixes f g :: "int \<Rightarrow> real"
  assumes f: "nz_support f \<noteq> {}" "finite (nz_support f)"
    and g: "nz_support g \<noteq> {}" "finite (nz_support g)"
    and sub: "nz_support f \<subseteq> nz_support g"
  shows "idx_min g \<le> idx_min f"
proof -
  have e_g: "idx_min g = Min (nz_support g)" using idx_min_finite g .
  have e_f: "idx_min f = Min (nz_support f)" using idx_min_finite f .
  have m: "Min (nz_support g) \<le> Min (nz_support f)"
    by (rule Min_antimono[OF sub f(1) g(2)])
  show ?thesis unfolding e_g e_f using m .
qed

lemma idx_max_mono:
  fixes f g :: "int \<Rightarrow> real"
  assumes f: "nz_support f \<noteq> {}" "finite (nz_support f)"
    and g: "nz_support g \<noteq> {}" "finite (nz_support g)"
    and sub: "nz_support f \<subseteq> nz_support g"
  shows "idx_max f \<le> idx_max g"
proof -
  have e_f: "idx_max f = Max (nz_support f)" using idx_max_finite f .
  have e_g: "idx_max g = Max (nz_support g)" using idx_max_finite g .
  have m: "Max (nz_support f) \<le> Max (nz_support g)"
    by (rule Max_mono[OF sub f(1) g(2)])
  show ?thesis unfolding e_f e_g using m .
qed

lemma grd_mono:
  assumes "span_grid P"
  shows "mono (grd P)"
  using assms unfolding span_grid_def by (simp add: strict_mono_mono)

subsection \<open>Span transport along grid equality\<close>

text \<open>Proven structurally: each conjunct transports by pointwise rewriting,
so no simplifier search over Archimedean witnesses is ever needed.\<close>

lemma span_grid_grd_cong:
  assumes eq: "grd P = grd Q"
  shows "span_grid P = span_grid Q"
proof -
  have c1: "strict_mono (grd P) = strict_mono (grd Q)" using eq by simp
  have c2: "(\<forall>i. 0 < grd P i) = (\<forall>i. 0 < grd Q i)" using eq by simp
  have c3: "(\<forall>r>0. \<exists>i. grd P i < r) = (\<forall>r>0. \<exists>i. grd Q i < r)" using eq by simp
  have c4: "(\<forall>r. \<exists>i. r < grd P i) = (\<forall>r. \<exists>i. r < grd Q i)" using eq by simp
  show ?thesis by (simp add: span_grid_def c1 c2 c3 c4)
qed

lemma span_of_dsc:
  assumes dsc: "clmm_dsc P" and eq: "grd P' = grd P"
  shows "span_grid P'"
proof -
  have sg: "span_grid P" using dsc by (rule clmm_dsc_span_grid)
  from span_grid_grd_cong[OF eq] sg show ?thesis by simp
qed

subsection \<open>Mint preserves the venue\<close>

theorem mint_preserves:
  assumes st: "clmm_state P s"
    and mint: "mint_rel d a b (P, s) (P', s')"
  shows "clmm_state P' s'"
proof -
  have pre_dsc: "clmm_dsc P" and pre_nz: "nz_support (lq P) \<noteq> {}"
    and pre_span: "grd_min P \<le> s \<and> s \<le> grd_max P"
    using st by (simp_all add: clmm_state_def)
  have pre_supfin: "finite (nz_support (lq P))"
    using pre_dsc by (simp add: clmm_dsc_def finite_liq_def)
  have nzP: "\<And>i. 0 \<le> lq P i" using pre_dsc by (simp add: clmm_dsc_def)
  have fpos: "\<And>i. 0 \<le> fee P i" and flt: "\<And>i. fee P i < 1"
    using pre_dsc by (simp_all add: clmm_dsc_def)

  have ex: "0 \<le> d \<and> s' = s \<and> grd P' = grd P \<and> fee P' = fee P \<and>
    (\<forall>i. lq P' i = (if a \<le> i \<and> i \<le> b then lq P i + d else lq P i))"
    using mint by (simp add: mint_rel_def split: prod.splits)
  then have d0: "0 \<le> d" and ss: "s' = s"
    and g0: "grd P' = grd P" and f0: "fee P' = fee P"
    and lq': "\<forall>i. lq P' i = (if a \<le> i \<and> i \<le> b then lq P i + d else lq P i)"
    by simp_all

  have pt: "\<And>i. lq P i \<le> lq P' i"
  proof -
    fix i :: int
    have "lq P' i = (if a \<le> i \<and> i \<le> b then lq P i + d else lq P i)"
      using lq' by simp
    then show "lq P i \<le> lq P' i" using d0 by (cases "a \<le> i \<and> i \<le> b") auto
  qed

  have span': "span_grid P'" using pre_dsc g0 by (rule span_of_dsc)

  have incl: "nz_support (lq P') \<subseteq> nz_support (lq P) \<union> {a..b}"
    using lq' by (auto simp: nz_support_def split: if_splits)
  have fin': "finite (nz_support (lq P'))"
    by (rule finite_nz_of_subset_range[OF incl pre_supfin])
  have liq': "finite_liq P'" using fin' by (simp add: finite_liq_def nz_support_def)

  have nn': "\<And>i. 0 \<le> lq P' i"
  proof -
    fix i :: int
    show "0 \<le> lq P' i" using lq' nzP d0 by (cases "a \<le> i \<and> i \<le> b") auto
  qed

  have dsc': "clmm_dsc P'"
  proof (unfold clmm_dsc_def, intro conjI)
    show "span_grid P'" using span' .
    show "finite_liq P'" by (rule liq')
    show "\<forall>i. 0 \<le> lq P' i" using nn' by simp
    show "\<forall>i. 0 \<le> fee P' i" using fpos f0 by simp
    show "\<forall>i. fee P' i < 1" using flt f0 by simp
  qed

  have sub_nz: "nz_support (lq P) \<subseteq> nz_support (lq P')"
  proof
    fix i :: int
    assume "i \<in> nz_support (lq P)"
    then have "lq P i \<noteq> 0" by (simp add: nz_support_def)
    then have "0 < lq P i" using nzP[of i] by linarith
    then have "0 < lq P' i" using pt[of i] by simp
    then show "i \<in> nz_support (lq P')" by (simp add: nz_support_def)
  qed
  have nz': "nz_support (lq P') \<noteq> {}" using pre_nz sub_nz by auto

  have sgP: "span_grid P" using pre_dsc by (rule clmm_dsc_span_grid)
  have gm: "mono (grd P)" by (rule grd_mono[OF sgP])
  have idx1: "idx_min (lq P') \<le> idx_min (lq P)"
    by (rule idx_min_anti[OF pre_nz pre_supfin nz' fin' sub_nz])
  have lo_step: "grd P (idx_min (lq P')) \<le> grd P (idx_min (lq P))"
    by (rule monoD[OF gm idx1])
  have gmP': "grd_min P' = grd P (idx_min (lq P'))"
    unfolding grd_min_def idx_min_img_def g0 by simp
  have gmP: "grd_min P = grd P (idx_min (lq P))"
    unfolding grd_min_def idx_min_img_def by simp
  have lo: "grd_min P' \<le> grd_min P"
    unfolding gmP' gmP using lo_step .
  have idx2: "idx_max (lq P) \<le> idx_max (lq P')"
    by (rule idx_max_mono[OF pre_nz pre_supfin nz' fin' sub_nz])
  have idx3: "idx_max (lq P) + 1 \<le> idx_max (lq P') + 1" using idx2 by simp
  have hi_step: "grd P (idx_max (lq P) + 1) \<le> grd P (idx_max (lq P') + 1)"
    by (rule monoD[OF gm idx3])
  have gxP: "grd_max P = grd P (idx_max (lq P) + 1)"
    unfolding grd_max_def idx_max_img_def by simp
  have gxP': "grd_max P' = grd P (idx_max (lq P') + 1)"
    unfolding grd_max_def idx_max_img_def g0 by simp
  have hi: "grd_max P \<le> grd_max P'"
    unfolding gxP gxP' using hi_step .

  have "grd_min P' \<le> s' \<and> s' \<le> grd_max P'" using lo hi pre_span ss by simp
  then show "clmm_state P' s'" by (simp add: clmm_state_def dsc' nz')
qed

subsection \<open>Burn preserves the venue (the guard is sufficient)\<close>

theorem burn_preserves:
  assumes st: "clmm_state P s"
    and burn: "burn_rel d a b (P, s) (P', s')"
  shows "clmm_state P' s'"
proof -
  have pre_dsc: "clmm_dsc P"
    using st by (simp add: clmm_state_def)
  have pre_supfin: "finite (nz_support (lq P))"
    using pre_dsc by (simp add: clmm_dsc_def finite_liq_def)
  have fpos: "\<And>i. 0 \<le> fee P i" and flt: "\<And>i. fee P i < 1"
    using pre_dsc by (simp_all add: clmm_dsc_def)

  from burn have ex: "mint_rel d a b (P', s') (P, s) \<and> nz_support (lq P') \<noteq> {} \<and>
    (\<forall>i. 0 \<le> lq P' i) \<and> grd_min P' \<le> s \<and> s \<le> grd_max P'"
    by (simp add: burn_rel_def)
  then have nz': "nz_support (lq P') \<noteq> {}"
    and nn': "\<forall>i. 0 \<le> lq P' i"
    and sp2: "grd_min P' \<le> s \<and> s \<le> grd_max P'"
    and mint: "mint_rel d a b (P', s') (P, s)"
    by blast+

  have ex2: "0 \<le> d \<and> s' = s \<and> grd P' = grd P \<and> fee P' = fee P \<and>
    (\<forall>i. lq P i = (if a \<le> i \<and> i \<le> b then lq P' i + d else lq P' i))"
    using mint by (simp add: mint_rel_def split: prod.splits)
  then have ss: "s' = s" and g0: "grd P' = grd P" and f0: "fee P' = fee P"
    and lq'': "\<forall>i. lq P i = (if a \<le> i \<and> i \<le> b then lq P' i + d else lq P' i)"
    by simp_all

  have span'': "span_grid P'" using pre_dsc g0 by (rule span_of_dsc)

  have incl: "nz_support (lq P') \<subseteq> nz_support (lq P) \<union> {a..b}"
    using lq'' nn' by (auto simp: nz_support_def split: if_splits)
  have fin'': "finite (nz_support (lq P'))"
    by (rule finite_nz_of_subset_range[OF incl pre_supfin])
  have liq'': "finite_liq P'" using fin'' by (simp add: finite_liq_def nz_support_def)

  have dsc': "clmm_dsc P'"
  proof (unfold clmm_dsc_def, intro conjI)
    show "span_grid P'" using span'' .
    show "finite_liq P'" by (rule liq'')
    show "\<forall>i. 0 \<le> lq P' i" by (rule nn')
    show "\<forall>i. 0 \<le> fee P' i" using fpos f0 by simp
    show "\<forall>i. fee P' i < 1" using flt f0 by simp
  qed

  have sp2': "grd_min P' \<le> s' \<and> s' \<le> grd_max P'" using sp2 ss by simp
  show "clmm_state P' s'" by (simp add: clmm_state_def dsc' nz' sp2')
qed

section \<open>The burn guard is necessary: a concrete invariant-killing burn\<close>

text \<open>A concrete venue on the exponential grid, with liquidity on cells 0
and 1 and price exp 0. Burning cell 0 down to zero keeps the book
nonnegative, keeps the support nonempty (cell 1), and only removes
liquidity — yet the price exp 0 now falls BELOW the new span floor exp 1.
Every naive guard passes; the invariant dies. Only the span conjunct of
burn_rel catches this burn. This is the first attacker-edge theorem: LP
griefing around the active price is structurally rejected by burn_rel, and
by nothing weaker.\<close>

definition grd0 :: "int \<Rightarrow> real" where "grd0 i = exp (real_of_int i)"
definition lq0 :: "int \<Rightarrow> real" where
  "lq0 i = (if i = (0::int) \<or> i = (1::int) then (1::real) else 0)"
definition fee0 :: "int \<Rightarrow> real" where "fee0 i = 0"
definition P0 :: pool where "P0 = (grd0, lq0, fee0)"
definition P1 :: pool where
  "P1 = (grd0, (\<lambda>i. if i = (1::int) then (1::real) else 0), fee0)"

lemma nz_support_lq0: "nz_support lq0 = {0, 1}"
  by (auto simp: lq0_def nz_support_def)

lemma span_grid_P0: "span_grid P0"
proof -
  have sm: "strict_mono (grd P0)"
    by (simp add: P0_def grd_def grd0_def strict_mono_def exp_less_mono)
  have pos: "\<forall>i. 0 < grd P0 i"
    by (simp add: P0_def grd_def grd0_def)
  have lo: "\<forall>r>0. \<exists>i. grd P0 i < r"
  proof (intro allI impI)
    fix r :: real assume r: "0 < r"
    have e1: "real_of_int (ceiling (1 / r)) - 1 < 1 / r \<and>
              1 / r \<le> real_of_int (ceiling (1 / r))"
      using ceiling_correct[of "1 / r"] .
    have eq: "real_of_int (1 + ceiling (1 / r)) =
              1 + real_of_int (ceiling (1 / r))"
      by (simp add: of_int_add)
    have A: "1 / r < real_of_int (1 + ceiling (1 / r))" using e1 eq by linarith
    have B: "1 + real_of_int (1 + ceiling (1 / r))
             \<le> exp (real_of_int (1 + ceiling (1 / r)))"
      by (rule exp_ge_add_one_self)
    have lt: "1 / r < exp (real_of_int (1 + ceiling (1 / r)))"
      using A B by linarith
    have posr: "0 < 1 / r" using r by simp
    have inv: "inverse (exp (real_of_int (1 + ceiling (1 / r))))
               < inverse (1 / r)"
      using lt posr by (rule less_imp_inverse_less)
    have ir: "inverse (1 / r) = r" using r by simp
    have invr: "inverse (exp (real_of_int (1 + ceiling (1 / r)))) < r"
      using inv ir by simp
    have gneg: "exp (real_of_int (- (1 + ceiling (1 / r)))) < r"
      unfolding of_int_minus exp_minus using invr .
    have weq: "grd P0 (- (1 + ceiling (1 / r)))
               = exp (real_of_int (- (1 + ceiling (1 / r))))"
      by (simp add: P0_def grd_def grd0_def)
    from gneg weq show "\<exists>i. grd P0 i < r" by metis
  qed
  have hi: "\<forall>r. \<exists>i. r < grd P0 i"
  proof
    fix r :: real
    have e1: "real_of_int (ceiling r) - 1 < r \<and> r \<le> real_of_int (ceiling r)"
      using ceiling_correct[of r] .
    have eq: "real_of_int (1 + ceiling r) = 1 + real_of_int (ceiling r)"
      by (simp add: of_int_add)
    have A: "r < real_of_int (1 + ceiling r)" using e1 eq by linarith
    have B: "1 + real_of_int (1 + ceiling r)
             \<le> exp (real_of_int (1 + ceiling r))"
      by (rule exp_ge_add_one_self)
    have C: "r < exp (real_of_int (1 + ceiling r))" using A B by linarith
    have weq: "grd P0 (1 + ceiling r) = exp (real_of_int (1 + ceiling r))"
      by (simp add: P0_def grd_def grd0_def)
    from C weq show "\<exists>i. r < grd P0 i" by metis
  qed
  show ?thesis
    using sm pos lo hi
    by (simp add: P0_def span_grid_def grd_def)
qed

lemma dsc_P0: "clmm_dsc P0"
proof (unfold clmm_dsc_def P0_def, intro conjI)
  show "span_grid (grd0, lq0, fee0)"
    using span_grid_P0 by (simp add: P0_def)
  show "finite_liq (grd0, lq0, fee0)"
    by (simp add: finite_liq_def lq_def nz_support_lq0)
  show "\<forall>i. 0 \<le> lq (grd0, lq0, fee0) i"
    by (simp add: lq_def lq0_def)
  show "\<forall>i. 0 \<le> fee (grd0, lq0, fee0) i"
    by (simp add: fee_def fee0_def)
  show "\<forall>i. fee (grd0, lq0, fee0) i < 1"
    by (simp add: fee_def fee0_def)
qed

lemma state_P0: "clmm_state P0 1"
proof (unfold clmm_state_def, intro conjI dsc_P0)
  show "nz_support (lq P0) \<noteq> {}"
    by (simp add: P0_def lq_def nz_support_lq0)
  have sup0: "nz_support (lq P0) = {0, 1}"
    by (simp add: P0_def lq_def nz_support_lq0)
  have mn: "idx_min (lq P0) = 0"
    unfolding idx_min_def using sup0 by (simp add: cInf_eq_Min)
  have mx: "idx_max (lq P0) + 1 = 2"
    unfolding idx_max_def using sup0 by (simp add: cSup_eq_Max)
  have g0: "grd P0 = grd0" by (simp add: P0_def grd_def)
  have gmin: "grd_min P0 = grd0 (idx_min (lq P0))"
    unfolding grd_min_def idx_min_img_def g0 by simp
  have gmin0: "grd_min P0 = 1" using mn gmin by (simp add: grd0_def)
  have gmax: "grd_max P0 = grd0 (idx_max (lq P0) + 1)"
    unfolding grd_max_def idx_max_img_def g0 by simp
  have gmax2: "grd_max P0 = exp 2" using mx gmax by (simp add: grd0_def)
  show "grd_min P0 \<le> 1" using gmin0 by simp
  show "1 \<le> grd_max P0" using gmax2 exp_ge_add_one_self[of "2::real"] by linarith
qed

theorem unguarded_burn_kills_invariant:
  "\<exists>P s P'. clmm_state P s \<and>
    (\<forall>i. lq P' i \<le> lq P i) \<and>
    grd P' = grd P \<and> fee P' = fee P \<and>
    nz_support (lq P') \<noteq> {} \<and> (\<forall>i. 0 \<le> lq P' i) \<and>
    \<not> clmm_state P' s"
proof -
  have g1: "grd P1 = grd0" by (simp add: P1_def grd_def)
  have nz1: "nz_support (lq P1) = {1}"
    by (auto simp: P1_def lq_def nz_support_def)
  have mn1: "idx_min (lq P1) = 1"
    unfolding idx_min_def using nz1 by (simp add: cInf_eq_Min)
  have gmin1: "grd_min P1 = exp 1"
  proof -
    have gm: "grd_min P1 = grd0 (idx_min (lq P1))"
      unfolding grd_min_def idx_min_img_def g1 by simp
    have "grd_min P1 = grd0 1" using gm mn1 by simp
    also have "\<dots> = exp 1" by (simp add: grd0_def)
    finally show ?thesis .
  qed
  have np: "\<not> clmm_state P1 1"
  proof
    assume c: "clmm_state P1 1"
    then have "grd_min P1 \<le> 1" unfolding clmm_state_def by blast
    then show False using gmin1 exp_ge_add_one_self[of "1::real"] by linarith
  qed
  have lq_le: "lq P1 i \<le> lq P0 i" for i :: int
    by (cases "i = (1::int)"; cases "i = (0::int)")
       (simp_all add: P0_def P1_def lq_def lq0_def)
  have nn1: "0 \<le> lq P1 i" for i :: int
    by (cases "i = (1::int)") (simp_all add: P1_def lq_def)
  have f1: "fee P1 = fee0" by (simp add: P1_def fee_def)
  show ?thesis
    apply (rule_tac x = P0 in exI)
    apply (rule_tac x = "1::real" in exI)
    apply (rule_tac x = P1 in exI)
    using state_P0 np lq_le nz1 nn1 g1 f1
    by (simp add: P0_def P1_def grd_def lq_def fee_def clmm_state_def)
qed

text \<open>Swap: price moves by the static-layer walk, fees charged. Next step:
define the walk as the entry's refined swap (CLMM_Transformation) applied
at the active cell, with the post-span condition as guard. Both liquidity
preservation obligations are now discharged: mint_rel and burn_rel keep
clmm_state, so provision/removal can never strand the venue outside the
static layer's validity domain.\<close>

text \<open>THE invariant (layer 1, weeks): every reachable state satisfies the
static layer's combination/optimality theorems — because per-cell liquidity
of the aggregate is a FOLD of position entries, and wedge/add associativity
(see pool_join) makes the fold order-free. With mint/burn preservation
proven, swap preservation is the last event needed before the reachable
set can be defined and inducted on.\<close>

text \<open>THE open problem (layer 2): fee-growth accounting under interleaved
events — per-position owed vs per-cell charged, wrap/rounding bounds.\<close>

end
