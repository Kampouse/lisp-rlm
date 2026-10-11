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

lemma sat_div_add_bound:
  fixes a g :: nat
  shows "(a + g) div 2^64 \<le> a div 2^64 + g div 2^64 + 1"
  by (arith | simp | fastforce)

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
proof (rule ccontr)
  have d1: "g - c + c = g" using assms(1) by simp
  have t1: "(g - c) * L + c * L = (g - c + c) * L"
    by (simp add: distrib_right [symmetric])
  have tot: "(g - c) * L + c * L = g * L" using t1 d1 by simp
  have r1: "(g - c) * L div s * s + (g - c) * L mod s = (g - c) * L" by simp
  have r2: "c * L div s * s + c * L mod s = c * L" by simp
  have r3: "g * L div s * s + g * L mod s = g * L" by simp
  have m1: "(g - c) * L mod s < s" using assms(2) by simp
  have m2: "c * L mod s < s" using assms(2) by simp
  have m3: "g * L mod s < s" using assms(2) by simp
  have eq: "(g - c) * L div s * s + c * L div s * s
        + ((g - c) * L mod s + c * L mod s)
      = g * L div s * s + g * L mod s"
    using r1 r2 r3 tot by arith
  have up: "(g - c) * L div s * s + c * L div s * s
        + ((g - c) * L mod s + c * L mod s)
      < g * L div s * s + 2 * s"
    using eq m3 by arith
  have up2: "(g - c) * L div s * s + c * L div s * s
      < g * L div s * s + 2 * s"
    using up m1 m2 by arith
  assume "\<not> (g - c) * L div s + c * L div s \<le> g * L div s + 1"
  then have cc: "g * L div s + 2 \<le> (g - c) * L div s + c * L div s" by arith
  have step1: "(g * L div s + 2) * s
      \<le> ((g - c) * L div s + c * L div s) * s"
    by (rule mult_le_mono [OF cc le_refl])
  have cc2: "g * L div s * s + 2 * s
      \<le> (g - c) * L div s * s + c * L div s * s"
    using step1 by (simp add: distrib_right)
  show False using cc2 up2 by arith
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

end
