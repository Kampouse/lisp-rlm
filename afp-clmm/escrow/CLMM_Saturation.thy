theory CLMM_Saturation
  imports "Concentrated_Liquidity_Market_Making_Operations.CLMM_Description"
begin

section \<open>u64/u128 machine-division saturation laws\<close>

text \<open>Truncation laws for the machine-arithmetic layer (nat with div
2^64 / div 2^128 modelling u64/u128 wrapping). These are
the seed base for refining ideal-math statements (fold, routes, flow-cap)
to saturated semantics.

Provenance: closed by auto_sledgehammer probing (probes kb69--kb74), then
de-SH'd: every proof here is a plain deterministic tactic, so this file
adds no session dependencies and no SH nondeterminism to the STRICT
escrow build.\<close>

lemma sat_div_mono:
  fixes a b :: nat
  shows "a \<le> b \<Longrightarrow> a div 2^64 \<le> b div 2^64"
  by (arith | simp | fastforce)

lemma sat_div_compose:
  fixes x :: nat
  shows "x div 2^64 div 2^64 = x div 2^128"
  by (arith | simp | fastforce)

lemma sat_div_zero:
  fixes x :: nat
  shows "x < 2^64 \<Longrightarrow> x div 2^64 = 0"
  by (arith | simp | fastforce)

lemma sat_div_guard_mono:
  fixes deposits keyed_in :: nat
  assumes "keyed_in \<le> deposits"
  shows "(deposits - keyed_in) div 2^64 \<le> deposits div 2^64"
  by (arith | simp | fastforce)

lemma sat_trunc_hi:
  fixes x :: nat
  shows "x mod 2^128 div 2^64 div 2^64 = 0"
  by (arith | simp | fastforce)

lemma sat_mul_div_mono:
  fixes a b f :: nat
  shows "a \<le> b \<Longrightarrow> a * f div 2^64 \<le> b * f div 2^64"
proof -
  assume "a \<le> b"
  then have "a * f \<le> b * f" by (rule mult_le_mono1)
  then show ?thesis by (rule div_le_mono)
qed

text \<open>Canonical split workhorse: dividing X + Y across s costs at
most one scale unit over dividing the parts separately. All other
split lemmas (growth_add, accrue_step_slack) derive from this one.\<close>
lemma div_add_slack:
  fixes X Y s :: nat
  assumes "1 \<le> s"
  shows "X div s + Y div s \<le> (X + Y) div s + 1"
proof (rule ccontr)
  have r1: "X div s * s + X mod s = X" by simp
  have r2: "Y div s * s + Y mod s = Y" by simp
  have r3: "(X + Y) div s * s + (X + Y) mod s = X + Y" by simp
  have m1: "X mod s < s" using assms by simp
  have m2: "Y mod s < s" using assms by simp
  have m3: "(X + Y) mod s < s" using assms by simp
  have eq: "X div s * s + Y div s * s + (X mod s + Y mod s)
      = (X + Y) div s * s + (X + Y) mod s"
    using r1 r2 r3 by arith
  have up: "X div s * s + Y div s * s + (X mod s + Y mod s)
      < (X + Y) div s * s + 2 * s"
    using eq m3 by arith
  have up2: "X div s * s + Y div s * s < (X + Y) div s * s + 2 * s"
    using up m1 m2 by arith
  assume "\<not> X div s + Y div s \<le> (X + Y) div s + 1"
  then have cc: "(X + Y) div s + 2 \<le> X div s + Y div s" by arith
  have step1: "((X + Y) div s + 2) * s \<le> (X div s + Y div s) * s"
    by (rule mult_le_mono [OF cc le_refl])
  have cc2: "(X + Y) div s * s + 2 * s \<le> X div s * s + Y div s * s"
    using step1 by (simp add: distrib_right)
  show False using cc2 up2 by arith
qed
lemma sat_div_add_bound:
  fixes a g :: nat
  shows "(a + g) div 2^64 \<le> a div 2^64 + g div 2^64 + 1"
proof -
  have one: "1 \<le> (2^64::nat)" by simp
  then have "a div 2^64 + g div 2^64 \<le> (a + g) div 2^64 + 1"
    by (rule div_add_slack [where X = a and Y = g and s = "2^64::nat"])
  then show ?thesis by simp
qed

lemma sat_low_word:
  fixes x :: nat
  shows "x mod 2^64 div 2^64 = 0"
  by (arith | simp | fastforce)

section \<open>FlowX-law statements (fee growth over machine arithmetic)\<close>

text \<open>Shapes from the FlowX ledger kernels (position accrual
owed += (g - c) * L div 2^64 with wrapping g - c, escaped in the
naturals port as wrap_diff; fee = ceil(in * b / s)). Deterministic
linear chains throughout: truncation slack stays explicit (the +1)
so interleaved-event accounting can compose these directly.\<close>

text \<open>A1: owed since checkpoint c is at most owed total.\<close>
lemma growth_mono:
  fixes g c L :: nat
  assumes "c \<le> g"
  shows "(g - c) * L div 2^64 \<le> g * L div 2^64"
proof -
  have "(g - c) * L \<le> g * L" using assms by (simp add: mult_le_mono1)
  then show ?thesis by (rule div_le_mono)
qed

text \<open>A2: owed since c plus owed before c is at most owed total plus one
(truncation slack). Linear chain over div/mod atoms.\<close>
lemma growth_add:
  fixes g c L s :: nat
  assumes "c \<le> g" and "1 \<le> s"
  shows "(g - c) * L div s + c * L div s \<le> g * L div s + 1"
proof -
  have sum: "(g - c) * L + c * L = g * L"
    using assms(1) by (simp add: distrib_right [symmetric])
  have work: "(g - c) * L div s + c * L div s
      \<le> ((g - c) * L + c * L) div s + 1"
    using assms(2) by (rule div_add_slack)
  show ?thesis using work sum by simp
qed

text \<open>A3: ceil-fee rounding costs at most one scale unit (pool-favored
fee law).\<close>
lemma fee_scale_bound:
  fixes iin b s :: nat
  assumes "1 \<le> s"
  shows "(iin * b + s - 1) div s * s \<le> iin * b + s"
proof -
  have r: "(iin * b + s - 1) div s * s + (iin * b + s - 1) mod s
      = iin * b + s - 1" by simp
  have m: "(iin * b + s - 1) mod s < s" using assms(1) by simp
  show ?thesis using r m by arith
qed

text \<open>A4: wrap-diff escape hatch (naturals + re-seed port of FlowX
wrapping subtraction); through-the-wrap owed is at most zero-checkpoint
owed over a doubled counter.\<close>
definition wrap_diff :: "nat \<Rightarrow> nat \<Rightarrow> nat" where
  "wrap_diff g c = (if c \<le> g then g - c else 2^128 - c + g)"

lemma wrap_diff_le:
  fixes g c :: nat
  assumes "g < 2^128" and "c < 2^128"
  shows "wrap_diff g c \<le> g + 2^128"
  using assms by (cases "c \<le> g") (simp_all add: wrap_diff_def)

lemma wrap_diff_bound:
  fixes g c L :: nat
  assumes "g < 2^128" and "c < 2^128"
  shows "wrap_diff g c * L div 2^64 \<le> (g + 2^128) * L div 2^64"
proof -
  have "wrap_diff g c \<le> g + 2^128" using assms by (rule wrap_diff_le)
  then have "wrap_diff g c * L \<le> (g + 2^128) * L" by (rule mult_le_mono1)
  then show ?thesis by (rule div_le_mono)
qed

section \<open>Layer 2: fee-growth accrual over op sequences\<close>

text \<open>The accrue fold threads the checkpoint through a sorted op list:\
 each op adds (g_i - prev_checkpoint) * L div 2^64 (FlowX owed kernel).\
 The sequence bound composes growth_add across ops: per-position owed\
 over a sequence is at most the total-growth accrual plus one\
 truncation unit per op -- the machine-arithmetic form of the layer-2\
 fee-growth question sketched in CLMM_Dynamic.thy.\<close>

fun accrue :: "nat \<Rightarrow> nat \<Rightarrow> nat list \<Rightarrow> nat" where
  "accrue L _ [] = 0"
| "accrue L c (g # gs) = (g - c) * L div 2^64 + accrue L g gs"

text \<open>Workhorse: splitting a quantity across div costs at most one
scale unit.\<close>

lemma accrue_step_slack:
  fixes L g g' c s :: nat
  assumes "c \<le> g" and "g \<le> g'" and "1 \<le> s"
  shows "(g - c) * L div s + (g' - g) * L div s \<le> (g' - c) * L div s + 1"
proof -
  have sum: "(g - c) * L + (g' - g) * L = (g' - c) * L"
    using assms by (simp add: distrib_right [symmetric])
  have work: "(g - c) * L div s + (g' - g) * L div s
      \<le> ((g - c) * L + (g' - g) * L) div s + 1"
    using assms(3) by (rule div_add_slack)
  show ?thesis using work sum by simp
qed

lemma sorted_hd_le_last:
  fixes xs :: "nat list"
  assumes "sorted (x # xs)"
  shows "x \<le> last (x # xs)"
  using assms
proof (induction xs arbitrary: x)
  case Nil
  then show ?case by simp
next
  case (Cons y ys)
  have xy: "x \<le> y" using Cons.prems by (auto simp add: sorted_simps)
  have srt: "sorted (y # ys)" using Cons.prems by (auto simp add: sorted_simps)
  have lasteq: "last (x # y # ys) = last (y # ys)" by (simp add: last_ConsR)
  show ?case using xy Cons.IH [OF srt] lasteq by arith
qed

text \<open>Main bound: no tail case split.\<close>
lemma accrue_bound:
  "sorted (c # gs) \<Longrightarrow>
    accrue L c gs \<le> (last (c # gs) - c) * L div 2^64 + length gs"
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have c1: "c \<le> g" using Cons.prems by (auto simp add: sorted_simps)
  have s2: "sorted (g # gs)" using Cons.prems by (auto simp add: sorted_simps)
  have g2: "g \<le> last (g # gs)" using s2 by (rule sorted_hd_le_last)
  have ih: "accrue L g gs \<le> (last (g # gs) - g) * L div 2^64 + length gs"
    using Cons.IH [OF s2] .
  have unfold: "accrue L c (g # gs)
      = (g - c) * L div 2^64 + accrue L g gs" by simp
  have lasteq: "last (c # g # gs) = last (g # gs)" by (simp add: last_ConsR)
  have step: "(g - c) * L div 2^64 + (last (g # gs) - g) * L div 2^64
      \<le> (last (g # gs) - c) * L div 2^64 + 1"
  proof (rule accrue_step_slack
      [where L = L and g = g and g' = "last (g # gs)" and c = c and s = "2^64::nat"])
    show "c \<le> g" using c1 .
    show "g \<le> last (g # gs)" using g2 .
    show "1 \<le> (2^64::nat)" by simp
  qed
  have lensimp: "length (g # gs) = Suc (length gs)" by simp
  have t2: "accrue L c (g # gs)
      \<le> (g - c) * L div 2^64
        + (last (g # gs) - g) * L div 2^64 + length gs"
    using unfold ih by simp
  have t3: "accrue L c (g # gs)
      \<le> (last (g # gs) - c) * L div 2^64 + 1 + length gs"
    using t2 step by arith
  show ?case using t3 lasteq lensimp by simp
qed

end
