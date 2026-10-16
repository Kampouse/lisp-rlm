(* Promoted from probe kb90 (2026-10-11): /tmp/kb90, EXIT:0. Locale conversion
   (curve_ctx) for kb92: 2026-10-11. Proofs are deterministic
   (simp/linarith/arith/rule) -- no auto_sledgehammer. *)
(* Promoted from probe kb90 (2026-10-11): /tmp/kb90, EXIT:0. Proofs are
   deterministic (simp/linarith/arith/rule) -- no auto_sledgehammer. *)
theory CLMM_Curve
  imports Main
begin

section \<open>kb90: fee-slack law on the fixed-point curve (v1: nat core)\<close>

text \<open>Provenance: AFP CLMM_Description quote_net/quote_gross (gen_token =
  infsum of per-cell books; gross_fct f fee = f / (1 - fee)). The nat curve
  below mirrors the generated walk (gen.py): books are NET, cell i lives on
  the grid interval (G i, G (i+1)], fee is taken on the in side so the
  GROSS book is the fee-inflated book. Fee-slack law (additive, nat-safe):
  moving the price point up from p to p' adds at least as much GROSS as NET --

    nqnet p' + nqgross p \<le> nqgross p' + nqnet p        (p \<le> p')

  i.e. the fee slack (gross - net) is monotone in the price point. This is
  the engine of the per-leg roundtrip bound: buy through the gross book,
  sell back through the net book; the shortfall is exactly accumulated fees
  -- never negative. v1: two cells, uniform fee 997/1000 (mirror of gen.py
  3e15/1e18), grid edge G1 with G0 = 0.\<close>

locale curve_ctx =
  fixes Q0 Q1 :: nat                \<comment> \<open>net books per cell (gen.py Q pat)\<close>
    and Qg0 Qg1 :: nat              \<comment> \<open>gross books (fee-inflated, ceil via muldiv)\<close>
    and G1 :: nat                   \<comment> \<open>inner grid edge; G0 = 0\<close>
  assumes books0: "Q0 * 1000 div 997 \<le> Qg0"
    and books1: "Q1 * 1000 div 997 \<le> Qg1"
begin

definition nqnet :: "nat \<Rightarrow> nat" where
  "nqnet p = Q0 * min p G1 + (if G1 < p then Q1 * (p - G1) else 0)"

definition nqgross :: "nat \<Rightarrow> nat" where
  "nqgross p = Qg0 * min p G1 + (if G1 < p then Qg1 * (p - G1) else 0)"

text \<open>The gross book dominates the net book (ceil by the fee ratio).\<close>

lemma book0_le: "Q0 \<le> Qg0"
proof -
  have "Q0 * 997 \<le> Q0 * 1000" by (rule mult_le_mono) simp_all
  hence "Q0 * 997 div 997 \<le> Qg0" using books0 by (rule le_trans [OF div_le_mono])
  thus ?thesis by simp
qed

lemma book1_le: "Q1 \<le> Qg1"
proof -
  have "Q1 * 997 \<le> Q1 * 1000" by (rule mult_le_mono) simp_all
  hence "Q1 * 997 div 997 \<le> Qg1" using books1 by (rule le_trans [OF div_le_mono])
  thus ?thesis by simp
qed

text \<open>One-unit step: crossing into the next unit releases the live cell's
  book; the GROSS release dominates the NET release.\<close>

lemma step_slack:
  "nqnet (Suc p) + nqgross p \<le> nqgross (Suc p) + nqnet p"
proof (cases "p < G1")
  case True
  show ?thesis
    unfolding nqnet_def nqgross_def using True book0_le by simp
next
  case False
  hence ng1: "\<not> p < G1" by simp
  show ?thesis
  proof (cases "p = G1")
    case True
    show ?thesis
      unfolding nqnet_def nqgross_def using True book1_le by simp
  next
    case False
    hence g1p: "G1 < p" using ng1 by simp
    have h1: "G1 < Suc p" using g1p by simp
    have u: "Suc p - G1 = Suc (p - G1)" using g1p by simp
    have net: "nqnet (Suc p) = Q0 * G1 + Q1 * Suc (p - G1)"
      unfolding nqnet_def using h1 u by simp
    have netto: "nqnet p = Q0 * G1 + Q1 * (p - G1)"
      unfolding nqnet_def using g1p by simp
    have gro: "nqgross (Suc p) = Qg0 * G1 + Qg1 * Suc (p - G1)"
      unfolding nqgross_def using h1 u by simp
    have groto: "nqgross p = Qg0 * G1 + Qg1 * (p - G1)"
      unfolding nqgross_def using g1p by simp
    have e1: "Q1 * Suc (p - G1) = Q1 * (p - G1) + Q1" by simp
    have e2: "Qg1 * Suc (p - G1) = Qg1 * (p - G1) + Qg1" by simp
    have cell: "Q1 * Suc (p - G1) + Qg1 * (p - G1)
              \<le> Qg1 * Suc (p - G1) + Q1 * (p - G1)"
      using e1 e2 book1_le by linarith
    show ?thesis using net netto gro groto cell by linarith
  qed
qed

text \<open>The fee-slack law: gross gain dominates net gain on every move up.\<close>

lemma slack_mono:
  assumes le: "p \<le> p'"
  shows "nqnet p' + nqgross p \<le> nqgross p' + nqnet p"
  using le
proof (induction p' arbitrary: p)
  case 0
  then show ?case by simp
next
  case (Suc p')
  show ?case
  proof (cases "p = Suc p'")
    case True
    then show ?thesis by (simp add: step_slack)
  next
    case False
    hence ple: "p \<le> p'" using Suc.prems by simp
    have ih: "nqnet p' + nqgross p \<le> nqgross p' + nqnet p"
      using Suc.IH ple by simp
    have st: "nqnet (Suc p') + nqgross p' \<le> nqgross (Suc p') + nqnet p'"
      by (rule step_slack)
    show ?thesis using ih st by arith
  qed
qed

end
end
