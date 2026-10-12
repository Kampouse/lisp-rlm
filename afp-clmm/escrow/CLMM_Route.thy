(* Provenance: probe kb91 (2026-10-11, /tmp/kb91 EXIT:0), promoted. All proofs
   deterministic (rule/blast/linarith/simp) -- no auto_sledgehammer. *)
theory CLMM_Route
  imports CLMM_Escrow
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

end
