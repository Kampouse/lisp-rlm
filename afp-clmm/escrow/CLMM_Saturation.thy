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

end
