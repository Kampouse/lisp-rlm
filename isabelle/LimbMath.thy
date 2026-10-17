theory LimbMath
  imports Main
begin

(* Mirror of lib/limb_math.lisp: limbs are natural numbers < 2^61 *)
type_synonym limb = nat

definition LIMB_BITS :: nat where "LIMB_BITS = 61"
definition limb_max :: nat where "limb_max = 2 ^ LIMB_BITS - 1"

definition valid_limb :: "limb \<Rightarrow> bool" where
  "valid_limb x \<longleftrightarrow> x \<le> limb_max"

(* carry-split: (hi, lo) such that x = hi * 2^61 + lo, both valid *)
definition split61 :: "nat \<Rightarrow> limb \<times> limb" where
  "split61 x = (x div 2^LIMB_BITS, x mod 2^LIMB_BITS)"

lemma split61_valid:
  assumes "valid_limb x"
  shows "valid_limb (fst (split61 x)) \<and> valid_limb (snd (split61 x))"
proof -
  have hm: "x < 2 ^ LIMB_BITS"
    using assms by (simp add: valid_limb_def limb_max_def LIMB_BITS_def)
  then have "x div 2 ^ LIMB_BITS = 0" by (simp add: div_less)
  moreover have "x mod 2 ^ LIMB_BITS < 2 ^ LIMB_BITS" using hm by simp
  ultimately show ?thesis
    unfolding split61_def valid_limb_def limb_max_def
    by (simp add: LIMB_BITS_def)
qed

(* fee: out = (amt * bps + 9999) div 10000 <= amt  when bps <= 10000 *)
lemma fee_mono:
  fixes amt bps :: nat
  assumes "bps \<le> 10000"
  shows "(amt * bps + 9999) div 10000 \<le> amt"
proof (cases "amt = 0")
  case True
  then show ?thesis by simp
next
  case False
  have "amt * bps \<le> amt * 10000"
    using assms by (simp add: mult_le_cancel_left)
  then have lt: "amt * bps + 9999 < amt * 10000 + 10000" by linarith
  then have "(amt * bps + 9999) div 10000 < amt + 1"
    using div_less[of "amt * bps + 9999" "amt * 10000 + 10000"] by simp
  then show ?thesis by simp
qed

end
