(* Promoted from probe /tmp/kb94, 2026-10-12 (EXIT:0, ~3s build).
   Deterministic tactics only: rule/simp/linarith/blast/metis. *)
theory CLMM_Fee_Slack
  imports "Concentrated_Liquidity_Market_Making_Operations.CLMM_Description"
begin

section \<open>kb94: fee-slack on the real curve (net diff \<le> gross diff)\<close>

theorem fee_slack_step:
  assumes dsc: "clmm_dsc P"
    and pp: "0 < p"
    and le: "p \<le> p'"
  shows "quote_net P p' - quote_net P p \<le> quote_gross P p' - quote_gross P p"
proof -
  have fliq: "finite_liq P" using dsc clmm_dsc_liq(1) by simp
  have f1: "finite (nz_support (lq P))" using fliq finite_liq_def by simp
  interpret L: finite_nz_support "lq P"
    by unfold_locales (rule f1)
  interpret Pfin: finite_liq_pool P by unfold_locales (rule fliq)
  have f2: "finite (nz_support (gross_fct (lq P) (fee P)))"
    using Pfin.finite_liq_gross_fct by (simp add: nz_support_def)
  interpret G: finite_nz_support "gross_fct (lq P) (fee P)"
    by unfold_locales (rule f2)
  define j where "j = lower_tick P p"
  define k where "k = lower_tick P p'"
  have ppos: "0 < p'" using pp le by simp
  have g_j: "grd P j \<le> p" unfolding j_def using lower_tick_geq[OF dsc pp] .
  have G_ub: "grd P k \<le> p'" unfolding k_def using lower_tick_geq[OF dsc ppos] .
  have jk: "j \<le> k"
    unfolding j_def k_def
    by (rule lower_tick_mono[OF dsc pp refl refl le])
  have fee_lt: "\<And>i. fee P i < 1" using dsc clmm_dsc_fees by simp
  have fee_ge: "\<And>i. 0 \<le> fee P i" using dsc clmm_dsc_fees by simp
  have onepos: "\<And>i. 0 < 1 - fee P i" using fee_lt by simp
  have nonneg_lq: "\<And>i. 0 \<le> lq P i" using dsc clmm_dsc_liq(2) by simp
  have pmul: "\<And>a b c::real. a \<le> b \<Longrightarrow> 0 \<le> c \<Longrightarrow> a * c \<le> b * c"
    by (rule mult_right_mono)
  have dom: "\<And>i. lq P i \<le> gross_fct (lq P) (fee P) i"
  proof -
    fix i
    have hp: "0 < 1 - fee P i" using onepos[of i] by linarith
    have hf: "0 \<le> fee P i" using fee_ge[of i] by linarith
    have nn_i: "0 \<le> lq P i" using nonneg_lq[of i] by linarith
    have le1: "1 - fee P i \<le> 1" using hf by linarith
    have lmul: "lq P i * (1 - fee P i) \<le> lq P i * 1"
      by (rule mult_left_mono[OF le1 nn_i])
    have base: "lq P i * (1 - fee P i) \<le> lq P i"
      using lmul by linarith
    have hp2: "0 \<le> 1 - fee P i" using hp by linarith
    have "lq P i * (1 - fee P i) / (1 - fee P i) \<le> lq P i / (1 - fee P i)"
      by (rule divide_right_mono[OF base hp2])
    thus "lq P i \<le> gross_fct (lq P) (fee P) i"
      using hp unfolding gross_fct_def one_cpl_def by simp
  qed
  have grd_ge: "\<And>i. 0 \<le> grd P (i+1) - grd P i"
  proof -
    fix i
    have "grd P i < grd P (i+1)" using clmm_dsc_grd_Suc[OF dsc] .
    thus "0 \<le> grd P (i+1) - grd P i" by linarith
  qed
  have nzG: "\<And>i. lq P i \<noteq> 0 \<Longrightarrow> gross_fct (lq P) (fee P) i \<noteq> 0"
  proof -
    fix i
    assume "lq P i \<noteq> 0"
    have "fee P i \<noteq> 1" using fee_lt[of i] by linarith
    hence "(lq P i = 0) = (gross_fct (lq P) (fee P) i = 0)"
      by (rule gross_fct_nz_eq)
    thus "gross_fct (lq P) (fee P) i \<noteq> 0" using \<open>lq P i \<noteq> 0\<close> by simp
  qed
  show ?thesis
  proof (cases "j = k")
    case True
    have a: "quote_net P p'
          = lq P k * (p' - grd P k)
            + sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i)) {i. lq P i \<noteq> 0 \<and> i < k}"
      unfolding quote_net_def k_def
      by (rule L.clmm_gen_quote_sum[OF dsc ppos]) simp
    have b: "quote_net P p
          = lq P j * (p - grd P j)
            + sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i)) {i. lq P i \<noteq> 0 \<and> i < j}"
      unfolding quote_net_def j_def
      by (rule L.clmm_gen_quote_sum[OF dsc pp]) simp
    have s1: "quote_net P p' - quote_net P p
        = lq P k * (p' - grd P k) - (lq P k * (p - grd P k))"
      using a b True by simp
    have ga: "quote_gross P p'
          = gross_fct (lq P) (fee P) k * (p' - grd P k)
            + sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i))
                   {i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> i < k}"
      unfolding quote_gross_def k_def
      by (rule G.clmm_gen_quote_sum[OF dsc ppos]) simp
    have gb: "quote_gross P p
          = gross_fct (lq P) (fee P) j * (p - grd P j)
            + sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i))
                   {i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> i < j}"
      unfolding quote_gross_def j_def
      by (rule G.clmm_gen_quote_sum[OF dsc pp]) simp
    have s2: "quote_gross P p' - quote_gross P p
        = gross_fct (lq P) (fee P) k * (p' - grd P k)
        - gross_fct (lq P) (fee P) k * (p - grd P k)"
      using ga gb True by simp
    have e1: "quote_net P p' - quote_net P p = lq P k * (p' - p)"
      using s1 by (simp add: algebra_simps)
    have e2: "quote_gross P p' - quote_gross P p
        = gross_fct (lq P) (fee P) k * (p' - p)"
      using s2 by (simp add: algebra_simps)
    have pp0: "0 \<le> p' - p" using le by linarith
    show ?thesis
      unfolding e1 e2
      by (rule pmul[OF dom pp0])
  next
    case False
    have jlt: "j < k" using jk False by simp
    have jeq: "j = lower_tick P p" by (simp add: j_def)
    have keq: "k = lower_tick P p'" by (simp add: k_def)
    have qd: "gen_quote (grd P) (lq P) p' - gen_quote (grd P) (lq P) p
        = lq P k * (p' - grd P k)
          + sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i))
                 {i. lq P i \<noteq> 0 \<and> j < i \<and> i < k}
          + lq P j * (grd P (j+1) - p)"
      by (rule L.clmm_gen_quote_diff_eq[OF dsc jeq keq pp le jlt, unfolded gen_quote_diff_def])
    have qdj: "quote_net P p' - quote_net P p
        = lq P k * (p' - grd P k)
          + sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i)) {i. lq P i \<noteq> 0 \<and> j < i \<and> i < k}
          + lq P j * (grd P (j+1) - p)"
      using qd by (simp add: quote_net_def j_def k_def)
    have gd: "gen_quote (grd P) (gross_fct (lq P) (fee P)) p' - gen_quote (grd P) (gross_fct (lq P) (fee P)) p
        = gross_fct (lq P) (fee P) k * (p' - grd P k)
          + sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i))
                 {i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> j < i \<and> i < k}
          + gross_fct (lq P) (fee P) j * (grd P (j+1) - p)"
      by (rule G.clmm_gen_quote_diff_eq[OF dsc jeq keq pp le jlt, unfolded gen_quote_diff_def])
    have gdj: "quote_gross P p' - quote_gross P p
        = gross_fct (lq P) (fee P) k * (p' - grd P k)
          + sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i))
                 {i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> j < i \<and> i < k}
          + gross_fct (lq P) (fee P) j * (grd P (j+1) - p)"
      using gd by (simp add: quote_gross_def j_def k_def)
    have ppk: "0 \<le> p' - grd P k" using G_ub by linarith
    have t1: "lq P k * (p' - grd P k) \<le> gross_fct (lq P) (fee P) k * (p' - grd P k)"
      by (rule pmul[OF dom ppk])
    have lt_ub: "p < grd P (j+1)" by (rule lower_tick_ubound[OF dsc jeq])
    have gjp: "0 \<le> grd P (j+1) - p" using lt_ub by linarith
    have t2: "lq P j * (grd P (j+1) - p) \<le> gross_fct (lq P) (fee P) j * (grd P (j+1) - p)"
      by (rule pmul[OF dom gjp])
    have t3: "sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i))
                {i. lq P i \<noteq> 0 \<and> j < i \<and> i < k}
            \<le> sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i))
                {i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> j < i \<and> i < k}"
    proof -
      let ?S = "{i. lq P i \<noteq> 0 \<and> j < i \<and> i < k}"
      let ?S' = "{i. gross_fct (lq P) (fee P) i \<noteq> 0 \<and> j < i \<and> i < k}"
      have finS': "finite ?S'"
      proof (rule finite_subset[where B = "{i. gross_fct (lq P) (fee P) i \<noteq> 0}"])
        show "?S' \<subseteq> {i. gross_fct (lq P) (fee P) i \<noteq> 0}"
        proof
          fix i assume "i \<in> ?S'"
          thus "i \<in> {i. gross_fct (lq P) (fee P) i \<noteq> 0}" by simp
        qed
      next
        show "finite {i. gross_fct (lq P) (fee P) i \<noteq> 0}"
          by (metis G.fin_nz_sup nz_support_def)
      qed
      have m1: "sum (\<lambda>i. lq P i * (grd P (i+1) - grd P i)) ?S
          \<le> sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)) ?S"
      proof (rule sum_mono)
        fix i assume "i \<in> ?S"
        have step_i: "0 \<le> grd P (i+1) - grd P i"
          using clmm_dsc_grd_Suc[OF dsc, of i] by linarith
        show "lq P i * (grd P (i+1) - grd P i)
            \<le> gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)"
          by (rule pmul[OF dom step_i])
      qed
      have sub: "?S \<subseteq> ?S'"
      proof
        fix i assume "i \<in> ?S"
        thus "i \<in> ?S'" using nzG by simp
      qed
      have m2: "sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)) ?S
          \<le> sum (\<lambda>i. gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)) ?S'"
      proof (rule sum_mono2)
        show "finite ?S'" by (rule finS')
        show "?S \<subseteq> ?S'" by (rule sub)
        fix i assume "i \<in> ?S' - ?S"
        have g0: "0 \<le> gross_fct (lq P) (fee P) i"
          by (rule order_trans[OF nonneg_lq dom])
        have step_i: "0 \<le> grd P (i+1) - grd P i"
          using clmm_dsc_grd_Suc[OF dsc, of i] by linarith
        have "0 * (grd P (i+1) - grd P i)
            \<le> gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)"
          by (rule pmul[OF g0 step_i])
        thus "0 \<le> gross_fct (lq P) (fee P) i * (grd P (i+1) - grd P i)" by simp
      qed
      show ?thesis by (rule order_trans[OF m1 m2])
    qed
    show ?thesis using qdj gdj t1 t2 t3 by linarith
  qed
qed

end
