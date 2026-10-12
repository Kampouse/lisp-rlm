(* Provenance: probe kb91 + kb92 + kb93 (2026-10-11, /tmp/kb91 /tmp/kb92
   /tmp/kb93, all EXIT:0). All proofs deterministic
   (rule/blast/linarith/simp/arith) -- no auto_sledgehammer. *)
theory CLMM_Route
  imports CLMM_Escrow CLMM_Curve
begin

section \<open>Routes: multi-token composition of escrow operations (gap 2, rung B)\<close>

text \<open>Provenance: probe kb91 (2026-10-11), promoted after EXIT:0. A route
  composes per-token escrows: each token lives in its own CLMM_Escrow
  ledger, and a cross-token swap = SwapIn on escrow-A composed with SwapOut
  on escrow-B (the composition the escrow header describes). v1 threads the
  SAME operation through every escrow of a portfolio in parallel; the
  per-hop amount/pool binding arrives with the curve twins (ops carrying a
  target pool). What is proved here is the composition law of gap 2:

    every escrow of the portfolio stays individually sound
    (routes_ok), and the PORTFOLIO VALUE is exact bookkeeping:
    total balance = total inflow - total outflow (route_conservation),
    hence a route starting from genesis mints nothing (route_genesis_zero_sum).

  A route can move value BETWEEN escrows (profit or loss across tokens) --
  conservation is per-ledger; the composition law says the ledgers jointly
  never create value from nothing. That is the honest multi-token promise;
  the pricing-leak bound is the single-pool curve twin (CLMM_Curve).\<close>

type_synonym portfolio = "escrow \<times> escrow"

fun apply_route :: "escrow_op \<Rightarrow> portfolio \<Rightarrow> portfolio option" where
  "apply_route op (a, b) =
     (case apply_op op a of
        None \<Rightarrow> None
      | Some a' \<Rightarrow> (case apply_op op b of
           None \<Rightarrow> None
         | Some b' \<Rightarrow> Some (a', b')))"

fun apply_routes :: "escrow_op list \<Rightarrow> portfolio \<Rightarrow> portfolio option" where
  "apply_routes [] p = Some p"
| "apply_routes (op # ops) p =
     (case apply_route op p of None \<Rightarrow> None | Some p' \<Rightarrow> apply_routes ops p')"

text \<open>Per-ledger soundness survives every hop of the route.\<close>

theorem route_ok:
  assumes oka: "escrow_ok a" and okb: "escrow_ok b"
      and ap: "apply_route op (a, b) = Some (a', b')"
  shows "escrow_ok a' \<and> escrow_ok b'"
proof -
  from ap have s1: "apply_op op a = Some a'"
    and s2: "apply_op op b = Some b'"
    by (auto simp: apply_route.simps split: option.splits)
  have "escrow_ok a'" using oka s1 by (rule one_op_ok)
  moreover have "escrow_ok b'" using okb s2 by (rule one_op_ok)
  ultimately show ?thesis by simp
qed

theorem routes_ok:
  assumes oka: "escrow_ok a" and okb: "escrow_ok b"
      and ap: "apply_routes ops (a, b) = Some (a', b')"
  shows "escrow_ok a' \<and> escrow_ok b'"
  using assms
proof (induction ops arbitrary: a b)
  case Nil
  then show ?case by simp
next
  case (Cons op ops)
  obtain a1 b1 where step: "apply_route op (a, b) = Some (a1, b1)"
    and rest: "apply_routes ops (a1, b1) = Some (a', b')"
    using Cons.prems(3) by (auto simp: apply_route.simps split: option.splits)
  have step_ok: "escrow_ok a1 \<and> escrow_ok b1"
    using Cons.prems(1,2) step by (rule route_ok)
  then show ?case using Cons.IH rest by blast
qed

text \<open>The composition law: portfolio value is exact bookkeeping over the
  two ledgers jointly -- a route reshuffles value between tokens, it never
  mints it. This is the gap-2 lift of conservation to composed ledgers.\<close>

theorem route_conservation:
  assumes oka: "escrow_ok a" and okb: "escrow_ok b"
      and ap: "apply_routes ops (a, b) = Some (a', b')"
  shows "balance a' + balance b'
       = (inflow a' + inflow b') - (outflow a' + outflow b')"
proof -
  have ok: "escrow_ok a' \<and> escrow_ok b'" using assms by (rule routes_ok)
  from ok have la: "outflow a' \<le> inflow a'"
    and ha: "balance a' = inflow a' - outflow a'"
    and lb: "outflow b' \<le> inflow b'"
    and hb: "balance b' = inflow b' - outflow b'"
    unfolding escrow_ok_def by blast+
  show ?thesis using ha hb la lb by linarith
qed

text \<open>Unconditional form: a route from genesis ends with portfolio value
  exactly equal to net deposits -- nothing minted, nothing lost.\<close>

theorem route_genesis_zero_sum:
  assumes ap: "apply_routes ops (init, init) = Some (a', b')"
  shows "balance a' + balance b'
       = (inflow a' + inflow b') - (outflow a' + outflow b')"
  using route_conservation init_ok init_ok ap by blast


section \<open>kb92: per-hop amount binding -- the curve consumes the trade\<close>

text \<open>A forward swap pays the GROSS book: moving the price point from p to
  p+d costs nqgross (p+d) - nqgross p. The reverse swap is credited at the
  NET book: it returns nqnet (p+d) - nqnet p. The fee-slack law (kb90)
  rearranges exactly to: the reverse never returns more than the forward
  paid -- the shortfall is the accumulated fee. This is cycle_loses, the
  nat twin of the AFP roundtrip story (quote_net/quote_gross).\<close>

context curve_ctx
begin

lemma net_le_gross: "nqnet p \<le> nqgross p"
proof (cases "p \<le> G1")
  case True
  show ?thesis
    unfolding nqnet_def nqgross_def using True book0_le by simp
next
  case False
  hence g1p: "G1 < p" by simp
  have m0: "Q0 * G1 \<le> Qg0 * G1" by (rule mult_le_mono) (simp_all add: book0_le)
  have m1: "Q1 * (p - G1) \<le> Qg1 * (p - G1)"
    by (rule mult_le_mono) (simp_all add: book1_le)
  have n: "nqnet p = Q0 * G1 + Q1 * (p - G1)"
    unfolding nqnet_def using g1p by simp
  have g: "nqgross p = Qg0 * G1 + Qg1 * (p - G1)"
    unfolding nqgross_def using g1p by simp
  show ?thesis using n g m0 m1 by linarith
qed

definition swapfwd :: "nat \<Rightarrow> nat \<Rightarrow> nat" where
  "swapfwd p d = nqgross (p + d) - nqgross p"

definition swapback :: "nat \<Rightarrow> nat \<Rightarrow> nat" where
  "swapback p d = nqnet (p + d) - nqnet p"

text \<open>Roundtrip through one pool: buy d, sell d back immediately; the
  return is at most the payment.\<close>

theorem cycle_loses:
  assumes le: "p \<le> p'"
  shows "nqnet p' - nqnet p \<le> nqgross p' - nqgross p"
proof -
  have S: "nqnet p' + nqgross p \<le> nqgross p' + nqnet p"
    by (rule slack_mono [OF le])
  have B0: "nqnet p \<le> nqgross p" by (rule net_le_gross)
  have B1: "nqnet p' \<le> nqgross p'" by (rule net_le_gross)
  show ?thesis using S B0 B1 by arith
qed

corollary roundtrip_bound: "swapback p d \<le> swapfwd p d"
  unfolding swapback_def swapfwd_def
  by (rule cycle_loses) simp

end

section \<open>kb93: the roundtrip on the ledger -- amounts bound to books\<close>

text \<open>The curve quantities become escrow amounts: the forward leg pays
  swapfwd p d out of the swapper's credits (SwapIn), the reverse leg
  credits swapback p d back (SwapOut). The books then record exactly the
  fee: the reserve grows by swapfwd - swapback and the swapper's credits
  drop by the same, while inflow/outflow never move -- the conservation
  ledger is untouched by a roundtrip. cycle_loses is thereby not merely
  a curve inequality: the escrow state carries it. Honest scope: one
  pool, nat twin; cross-token routes stay under the conservation law of
  kb91.\<close>

context curve_ctx
begin

theorem roundtrip_ledger:
  assumes fwd: "apply_seq [SwapIn u (swapfwd p d), SwapOut u (swapback p d)] e = Some e''"
      and aff: "swapfwd p d \<le> credits e u"
  shows "reserve e'' = reserve e + (swapfwd p d - swapback p d)"
    and "credits e'' u = credits e u - (swapfwd p d - swapback p d)"
    and "inflow e'' = inflow e \<and> outflow e'' = outflow e"
proof -
  obtain e1 where s1: "apply_op (SwapIn u (swapfwd p d)) e = Some e1"
      and s2: "apply_op (SwapOut u (swapback p d)) e1 = Some e''"
    using fwd by (auto simp: apply_seq.simps split: option.splits)
  from s1 have eq1: "e1 = e\<lparr>credits := (credits e)(u := credits e u - swapfwd p d)
                               , reserve := reserve e + swapfwd p d\<rparr>"
    by (auto simp: apply_op.simps split: if_splits)
  from s2 have g2: "0 < swapback p d"
      and eq2: "e'' = e1\<lparr>reserve := reserve e1 - swapback p d
                            , credits := (credits e1)(u := credits e1 u + swapback p d)\<rparr>"
    by (auto simp: apply_op.simps eq1 split: if_splits)
  have sbf: "swapback p d \<le> swapfwd p d" by (rule roundtrip_bound)
  have r1: "reserve e1 = reserve e + swapfwd p d" using eq1 by simp
  have r2: "reserve e'' = reserve e1 - swapback p d" using eq2 by simp
  have c1: "credits e1 u = credits e u - swapfwd p d" using eq1 by simp
  have c2: "credits e'' u = credits e1 u + swapback p d" using eq2 by simp
  show "reserve e'' = reserve e + (swapfwd p d - swapback p d)"
    using r1 r2 sbf by linarith
  show "credits e'' u = credits e u - (swapfwd p d - swapback p d)"
    using c1 c2 sbf aff by linarith
  have i1: "inflow e'' = inflow e1" using eq2 by simp
  have i2: "inflow e1 = inflow e" using eq1 by simp
  have o1: "outflow e'' = outflow e1" using eq2 by simp
  have o2: "outflow e1 = outflow e" using eq1 by simp
  show "inflow e'' = inflow e \<and> outflow e'' = outflow e"
    using i1 i2 o1 o2 by simp
qed

corollary roundtrip_mono:
  assumes fwd: "apply_seq [SwapIn u (swapfwd p d), SwapOut u (swapback p d)] e = Some e''"
      and aff: "swapfwd p d \<le> credits e u"
  shows "credits e'' u \<le> credits e u"
    and "reserve e \<le> reserve e''"
proof -
  have rl: "reserve e'' = reserve e + (swapfwd p d - swapback p d)"
       and cu: "credits e'' u = credits e u - (swapfwd p d - swapback p d)"
    using roundtrip_ledger[OF fwd aff] by simp_all
  have sbf: "swapback p d \<le> swapfwd p d" by (rule roundtrip_bound)
  show "credits e'' u \<le> credits e u" using cu sbf by simp
  show "reserve e \<le> reserve e''" using rl sbf by simp
qed

end
end
