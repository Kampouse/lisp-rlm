theory CLMM_Dynamic
  imports "Concentrated_Liquidity_Market_Making_Operations.CLMM_Transformation"
begin

section \<open>Dynamic composition: pools as evolving states\<close>

text \<open>The AFP entry treats pools as static objects and swap/refine/join as
total functions. A deployed venue is a labelled transition system: LPs mint
and burn, traders swap, and every theorem of the static layer must hold in
every reachable state. We model the state as a pool with an lq book and a
current price, and events as relations on states.

Everything below is a skeleton: identifiers grounded in the entry
(clmm_dsc, grd, lq, wedge, pool_join, pool_comb, combined_pools), lemmas
to be discharged.\<close>

type_synonym pool = "(int \<Rightarrow> real) \<times> (int \<Rightarrow> real) \<times> (int \<Rightarrow> real)"

locale clmm_state =
  fixes P :: pool and sqp :: real
  assumes dsc: "clmm_dsc P"
  (* TODO next session, in jedit: bound sqp to the active grid span via the
     entry's own notions (liq_grd_min lemma, CLMM_Description). *)

type_synonym event =
  "pool \<times> real \<Rightarrow> pool \<times> real \<Rightarrow> bool"

text \<open>Mint/burn: change lq over a range by delta, price untouched.\<close>

text \<open>Swap: price moves by the static-layer walk, fees charged.\<close>

text \<open>THE invariant (layer 1, weeks): every reachable state satisfies the
static layer's combination/optimality theorems — because per-cell liquidity
of the aggregate is a FOLD of position entries, and wedge/add associativity
(see pool_join) makes the fold order-free.\<close>

text \<open>THE open problem (layer 2): fee-growth accounting under interleaved
events — per-position owed vs per-cell charged, wrap/rounding bounds.\<close>

end
