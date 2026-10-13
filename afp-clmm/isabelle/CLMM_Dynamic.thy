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
Burn is guarded by edge population: if the burn empties the min- or
max-liquidity cell, the span shrinks past the price and the invariant would
break, so the relation only permits burns that keep both edge cells (or the
whole support) populated.\<close>

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
  grd_min (fst Ps') \<le> snd Ps \<and> snd Ps \<le> grd_max (fst Ps')"

text \<open>Swap: price moves by the static-layer walk, fees charged. Next step:
define the walk as the entry's refined swap (CLMM_Transformation) applied
at the active cell, with the post-span condition as guard.\<close>

text \<open>THE invariant (layer 1, weeks): every reachable state satisfies the
static layer's combination/optimality theorems — because per-cell liquidity
of the aggregate is a FOLD of position entries, and wedge/add associativity
(see pool_join) makes the fold order-free. First preservation obligation,
now within reach with the grounded locale: mint_rel preserves clmm_state
(span only widens: nz_support grows, idx_min can only decrease, idx_max
only increase, and the bracket cells stay populated).\<close>

text \<open>THE open problem (layer 2): fee-growth accounting under interleaved
events — per-position owed vs per-cell charged, wrap/rounding bounds.\<close>

end
