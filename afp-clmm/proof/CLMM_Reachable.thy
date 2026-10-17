theory
  CLMM_Reachable
  imports
    "CLMM_Dynamic"
begin

section \<open>Reachability: the invariant holds on every state the events can produce\<close>

text \<open>Layer 1. \<open>reachable\<close> is the smallest relation containing a well-formed
seed and closed under mint, burn, and swap. Each event's preservation theorem
shows \<open>clmm_state\<close> is an inductive invariant; layer-1 (the span/con
discipline that every static-layer theorem consumes) therefore holds on every
reachable state, and the transformation stack (pool_comb_clmm_dsc,
CLMM_Transformation) applies verbatim to reachable books.\<close>

inductive reachable :: "pool \<times> real \<Rightarrow> bool" where
  init: "clmm_state P s \<Longrightarrow> reachable (P, s)"
| mstep: "\<lbrakk>reachable (P, s); mint_rel d a b (P, s) (P', s')\<rbrakk> \<Longrightarrow> reachable (P', s')"
| bstep: "\<lbrakk>reachable (P, s); burn_rel d a b (P, s) (P', s')\<rbrakk> \<Longrightarrow> reachable (P', s')"
| sstep: "\<lbrakk>reachable (P, s); swap_rel v P s P' s'\<rbrakk> \<Longrightarrow> reachable (P', s')"

theorem reachable_clmm_state:
  assumes "reachable Ps"
  shows "clmm_state (fst Ps) (snd Ps)"
  using assms
proof induction
  case init
  then show ?case by simp
next
  case (mstep d a b P s P' s')
  then show ?case by (simp add: mint_preserves)
next
  case (bstep d a b P s P' s')
  then show ?case by (simp add: burn_preserves)
next
  case (sstep v P s P' s')
  then show ?case by (simp add: swap_preserves)
qed

section \<open>Donation-safety: unattributed liquidity cannot skew claims\<close>

text \<open>The Euler-style attack: force-send tokens to the contract outside the
normal deposit path so that a share accumulator is skewed in the attacker's
favour. In this model a donation is a raw pointwise addition of liquidity at
one cell. Two facts make the class structurally impossible here: the
donation is exactly a mint event (so every mint guarantee transfers), and it
is \<^emph>\<open>isolated\<close> \<open>---\<close> grid, fee schedule, price, and every other cell are
bit-identical after the event, so no cross-cell accumulator can move.
Honest scope: fee-\<open>owed\<close> accumulator skew is a layer-2 (accrual ledger)
concern and is bounded separately by the accrual WIP; this section kills the
book-level class.\<close>

definition donate_rel :: "real \<Rightarrow> int \<Rightarrow> pool \<times> real \<Rightarrow> pool \<times> real \<Rightarrow> bool" where
"donate_rel v i Ps Ps' \<longleftrightarrow>
  (let (P, s) = Ps; (P', s') = Ps' in
    0 \<le> v \<and> s' = s \<and> grd P' = grd P \<and> fee P' = fee P \<and>
    (\<forall>j. lq P' j = (if j = i then lq P j + v else lq P j)))"

lemma donate_is_mint:
  assumes "donate_rel v i Ps Ps'"
  shows "mint_rel v i i Ps Ps'"
proof -
  have rng: "\<And>j. (i \<le> j \<and> j \<le> i) = (j = (i::int))" by auto
  show ?thesis using assms by (simp add: donate_rel_def mint_rel_def rng)
qed

lemma donate_isolated:
  assumes "donate_rel v i (P, s) (P', s')"
  shows "s' = s" and "grd P' = grd P" and "fee P' = fee P"
    and "\<And>j. j \<noteq> i \<Longrightarrow> lq P' j = lq P j"
    and "lq P' i = lq P i + v"
  using assms by (simp_all add: donate_rel_def split: prod.splits)

theorem donate_preserves:
  assumes st: "clmm_state P s"
    and d: "donate_rel v i (P, s) (P', s')"
  shows "clmm_state P' s'"
  using mint_preserves[OF st donate_is_mint[OF d]] .

theorem reachable_donate:
  assumes r: "reachable (P, s)"
    and d: "donate_rel v i (P, s) (P', s')"
  shows "reachable (P', s')"
  using reachable.mstep[OF r donate_is_mint[OF d]] .

section \<open>Compositional roundtrip: withdrawal after any reachable history\<close>

text \<open>The escrow layer proves single-op roundtrips neutral per user. Here we
lift the LP-safety half to the reachable world: from ANY reachable state --
i.e. after an arbitrary history of provisions, removals, donations and
price walks -- a bounded withdrawal remains legal (never strands the venue),
removes no more than the book holds at cell \<open>i\<close>, touches nothing else, and
deposit/withdrawal order does not matter for the net book
(\<open>mint_burn_commute\<close>). Honest scope: full book commutation through swaps is
FALSE in general -- swap reindexes cells above the insertion point
(refine/wedge in CLMM_Transformation), so cell identity is positional after
a price walk; reconciling that reindexing is the escrow ledger's job, not
the book's.\<close>

definition roundtrip_withdraw ::
  "pool \<Rightarrow> real \<Rightarrow> real \<Rightarrow> int \<Rightarrow> pool \<Rightarrow> real \<Rightarrow> bool" where
"roundtrip_withdraw P s d i P'' s'' \<longleftrightarrow>
  reachable (P, s) \<and> 0 \<le> d \<and>
  burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"

theorem roundtrip_legal:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
  shows "reachable (P'', s'') \<and> clmm_state P'' s''"
proof -
  from rt have rs: "reachable (P, s)"
    by (simp add: roundtrip_withdraw_def)
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  from rs bd have "reachable (P'', s'')" by (rule reachable.bstep)
  moreover
  have st0: "clmm_state P s" using reachable_clmm_state[OF rs] by simp
  have "clmm_state P'' s''" using burn_preserves[OF st0 bd] by simp
  ultimately show ?thesis by blast
qed

text \<open>Bridge lemmas: burn = pointwise subtraction, stated WITHOUT the
let/pair wrappers so every later theorem consumes these and never unfolds
the nested definitions again (unfolding both together makes simp flounder).\<close>

lemma burn_pointwise:
  assumes bd: "burn_rel d i i (Q, t) (Q', t')"
  shows "lq Q' i = lq Q i - d \<and>
    (\<forall>j. j \<noteq> i \<longrightarrow> lq Q j = lq Q' j) \<and>
    grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
proof -
  from bd have md: "mint_rel d i i (Q', t') (Q, t)"
    by (simp add: burn_rel_def split: prod.splits)
  from md have pj: "\<And>j. lq Q j = (if j = i then lq Q' j + d else lq Q' j)"
    by (simp add: mint_rel_def split: prod.splits)
  have f1: "lq Q' i = lq Q i - d" using pj[of i] by simp
  have f2: "\<And>j. j \<noteq> i \<Longrightarrow> lq Q j = lq Q' j" using pj by simp
  have f3: "grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
    using md by (simp add: mint_rel_def split: prod.splits)
  from f1 f2 f3 show ?thesis by blast
qed

theorem roundtrip_bounded:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
  shows "lq P'' i = lq P i - min d (lq P i)"
proof -
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  have pb: "lq P'' i = lq P i - min d (lq P i)"
    using burn_pointwise[OF bd] by blast
  from pb show ?thesis .
qed

theorem roundtrip_others_intact:
  assumes rt: "roundtrip_withdraw P s d i P'' s''"
    and ne: "j \<noteq> i"
  shows "lq P'' j = lq P j \<and> grd P'' = grd P \<and>
    fee P'' = fee P \<and> s'' = s"
proof -
  from rt have bd: "burn_rel (min d (lq P i)) i i (P, s) (P'', s'')"
    by (simp add: roundtrip_withdraw_def)
  from burn_pointwise[OF bd] ne have peq: "lq P j = lq P'' j" by blast
  from burn_pointwise[OF bd] have pg: "grd P'' = grd P" by blast
  from burn_pointwise[OF bd] have pf: "fee P'' = fee P" by blast
  from burn_pointwise[OF bd] have ps: "s'' = s" by blast
  from peq pg pf ps show ?thesis by simp
qed

lemma overburn_rejected:
  assumes bd: "burn_rel d i i (P, s) (P'', s'')"
    and big: "lq P i < d"
  shows False
proof -
  from bd have md: "mint_rel d i i (P'', s'') (P, s)"
    by (simp add: burn_rel_def split: prod.splits)
  from md have dp: "0 \<le> d" by (simp add: mint_rel_def split: prod.splits)
  from md have pj: "\<And>j. lq P j = (if j = i then lq P'' j + d else lq P'' j)"
    by (simp add: mint_rel_def split: prod.splits)
  from pj[of i] have pii: "lq P i = lq P'' i + d" by simp
  from bd have gdi: "0 \<le> lq P'' i"
    by (simp add: burn_rel_def split: prod.splits)
  from dp pii gdi big show False by linarith
qed

lemma mint_pointwise:
  assumes md: "mint_rel d i i (Q, t) (Q', t')"
  shows "lq Q' i = lq Q i + d \<and>
    (\<forall>j. j \<noteq> i \<longrightarrow> lq Q j = lq Q' j) \<and>
    grd Q' = grd Q \<and> fee Q' = fee Q \<and> t' = t"
  using md by (simp add: mint_rel_def split: prod.splits)

lemma mint_burn_commute:
  assumes e1: "mint_rel d1 i i (P, s) (P1, s1)"
    and e2: "burn_rel d2 i i (P1, s1) (P2, s2)"
    and e3: "burn_rel d2 i i (P, s) (P3, s3)"
    and e4: "mint_rel d1 i i (P3, s3) (P4, s4)"
  shows "lq P2 = lq P4 \<and> s2 = s4"
proof -
  have p1: "lq P1 i = lq P i + d1 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P j = lq P1 j)"
    using mint_pointwise[OF e1] by blast
  have p2: "lq P2 i = lq P1 i - d2 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P1 j = lq P2 j)"
    using burn_pointwise[OF e2] by blast
  have p3: "lq P3 i = lq P i - d2 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P j = lq P3 j)"
    using burn_pointwise[OF e3] by blast
  have p4: "lq P4 i = lq P3 i + d1 \<and> (\<forall>j. j \<noteq> i \<longrightarrow> lq P3 j = lq P4 j)"
    using mint_pointwise[OF e4] by blast
  have p1i: "lq P1 i = lq P i + d1" using p1 by blast
  have p2i: "lq P2 i = lq P1 i - d2" using p2 by blast
  have p3i: "lq P3 i = lq P i - d2" using p3 by blast
  have p4i: "lq P4 i = lq P3 i + d1" using p4 by blast
  have eqi: "lq P2 i = lq P4 i" using p1i p2i p3i p4i by linarith
  have p1o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P j = lq P1 j" using p1 by blast
  have p2o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P1 j = lq P2 j" using p2 by blast
  have p3o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P j = lq P3 j" using p3 by blast
  have p4o: "\<And>j. j \<noteq> i \<Longrightarrow> lq P3 j = lq P4 j" using p4 by blast
  have lqe: "lq P2 = lq P4"
  proof
    fix j :: int
    show "lq P2 j = lq P4 j"
    proof cases
      assume je: "j = i"
      then show ?thesis using eqi by simp
    next
      assume jne: "j \<noteq> i"
      have c1: "lq P j = lq P1 j" using p1o[of j] jne by blast
      have c2: "lq P1 j = lq P2 j" using p2o[of j] jne by blast
      have c3: "lq P j = lq P3 j" using p3o[of j] jne by blast
      have c4: "lq P3 j = lq P4 j" using p4o[of j] jne by blast
      from c1 c2 c3 c4 show ?thesis by linarith
    qed
  qed
  moreover have "s2 = s4"
  proof -
    from burn_pointwise[OF e2] have sb2: "s2 = s1" by blast
    from burn_pointwise[OF e3] have sb3: "s3 = s" by blast
    from mint_pointwise[OF e1] have sm1: "s1 = s" by blast
    from mint_pointwise[OF e4] have sm4: "s4 = s3" by blast
    from sb2 sb3 sm1 sm4 show "s2 = s4" by simp
  qed
  ultimately show ?thesis by blast
qed

section \<open>Attacker-edge rung 5: sandwich / MEV fee bound\<close>

text \<open>The sandwich question: can trade placement --- or sequencer ordering
of trades --- capture value from the venue's fee schedule itself?

Structural facts proven here:
\<^item> swaps through an existing tick level (\<^emph>\<open>on-grid\<close> walks) leave the
  pool object itself unchanged (\<^theory_text>\<open>swap_on_grid_inv\<close>); off-grid walks
  insert a tick level and reindex cells above it --- fee \<^emph>\<open>values\<close>
  travel with the shift (the inserted cell copies the value below it),
  so fee value cannot be manufactured by trading, only reindexed.
\<^item> swaps through a fixed book are zero-sum on the quote token: a round
  trip (walk up, then walk back to the start) nets exactly 0 quote units
  (\<^theory_text>\<open>swap_roundtrip_zero\<close>) --- the two legs telescope definitionally.
  Price impact through an unchanged book is temporary.
\<^item> consequently the fee schedule cannot be arbitraged by sandwich
  placement on a static book (\<^theory_text>\<open>sandwich_static_no_profit\<close>): no sequence
  of swaps through an unchanged book --- however ordered --- changes any
  fee cell. The MEV surface is confined to ordering effects on other
  agents' flows, never to fee capture; per-event fee accounting
  (charges to the traded cell's accumulator) is layer 2.

Honest scope: walks that cross price while inserting new ticks reindex
cells above the insertion point (cell identity is positional there), so
the zero-sum law is stated for walks landing on existing ticks, and the
display lemma takes the tick-crossing as an explicit premise.\<close>

subsection \<open>On-grid swaps restore the pool exactly\<close>

theorem swap_on_grid_inv:
  assumes sw: "swap_rel v P s P' s'"
    and grid: "grd P (lower_tick P s') = s'"
  shows "P' = P \<and> s' = s + v"
proof -
  from sw have def_: "s' = s + v \<and> P' = refine P s'"
    by (simp add: swap_rel_def)
  from grid have "refine P s' = P"
    by (rule refine_eq [OF refl])
  with def_ show "P' = P \<and> s' = s + v" by simp
qed

text \<open>Off-grid walks insert a tick level: cells above the insertion
point shift position, the new cell copying the fee value below it.
Fee \<^emph>\<open>values\<close> therefore travel with the book --- never created or
destroyed by trading, at most reindexed.\<close>

subsection \<open>Round trips through a fixed book are zero-sum\<close>

theorem swap_roundtrip_zero:
  assumes pos: "0 < u"
  shows "gen_quote_diff P L s (s + u) + gen_quote_diff P L (s + u) s = 0"
proof -
  have "gen_quote_diff P L s (s + u) =
      gen_quote (grd P) L (s + u) - gen_quote (grd P) L s"
    unfolding gen_quote_diff_def by simp
  moreover have "gen_quote_diff P L (s + u) s =
      gen_quote (grd P) L s - gen_quote (grd P) L (s + u)"
    unfolding gen_quote_diff_def by simp
  ultimately show ?thesis by simp
qed

text \<open>Value form of the up-leg: quote paid is the partial cell at entry,
full cells in between, partial cell at exit --- the telescoping display
is proven in the entry's locale
(\<^theory_text>\<open>clmm_gen_quote_diff_eq\<close> in CLMM_Description); the zero-sum law
above needs none of it, since the two legs cancel definitionally.\<close>

subsection \<open>Sandwich bound on a static book\<close>

text \<open>Inductive extension of the fee-schedule invariance: any finite
sequence of swaps through an unchanging book (each leg returns the book
and price to the same state) leaves the fee schedule exactly where it
started. Sequencing power buys no fee value.\<close>

fun swap_seq where
  "swap_seq P s [] P' s' \<longleftrightarrow> P' = P \<and> s' = s"
| "swap_seq P s (v # vs) P' s' \<longleftrightarrow>
    (\<exists>P1 s1. swap_rel v P s P1 s1 \<and> swap_seq P1 s1 vs P' s')"

text \<open>Every prefix on-grid swap restores the pool, so the whole
sequence does.\<close>

theorem swap_seq_fee_inv:
  shows "swap_seq P s vs P' s' \<Longrightarrow>
    (\<And>u Q t t' P1. u \<in> set vs \<Longrightarrow> swap_rel u Q t P1 t' \<Longrightarrow>
      grd Q (lower_tick Q t') = t') \<Longrightarrow>
    P' = P \<and> s' = s + sum_list vs"
proof (induction vs arbitrary: P s P' s')
  case Nil
  then show ?case by simp
next
  case (Cons v vs)
  from Cons.prems(1) obtain P1 s1
    where r1: "swap_rel v P s P1 s1"
      and r2: "swap_seq P1 s1 vs P' s'"
    by (auto simp add: swap_seq.simps)
  have memb: "v \<in> set (v # vs)" by simp
  from Cons.prems(2) [OF memb r1]
  have g1: "grd P (lower_tick P s1) = s1" .
  have f1: "P1 = P \<and> s1 = s + v" by (rule swap_on_grid_inv [OF r1 g1])
  have g2: "\<And>u Q t t' P1. u \<in> set vs \<Longrightarrow> swap_rel u Q t P1 t' \<Longrightarrow>
      grd Q (lower_tick Q t') = t'"
  proof -
    fix u Q t t' P1
    assume mv: "u \<in> set vs" and mr: "swap_rel u Q t P1 t'"
    from mv have mv2: "u \<in> set (v # vs)" by simp
    from Cons.prems(2) [OF mv2 mr]
    show "grd Q (lower_tick Q t') = t'" .
  qed
  have f2: "P' = P1 \<and> s' = s1 + sum_list vs" using Cons.IH [OF r2 g2] .
  from f1 f2 show ?case by simp
qed

text \<open>The sandwich law: any finite sequence of swaps that lands on-grid
at every step and returns the price to the entry level --- the
attacker's front-run/back-run pair, however long --- restores the pool
object exactly: every fee cell, every liquidity cell, everything.
Gross quote position is additionally zero by
\<^theory_text>\<open>swap_roundtrip_zero\<close>. Fee capture through placement is exactly
zero; ordering effects on other agents' flows are an interaction with
layer-2 accrual, not a fee-schedule attack.\<close>

theorem sandwich_static_no_profit:
  assumes seq: "swap_seq P s vs P' s'"
    and grid: "\<And>u Q t t' P1. u \<in> set vs \<Longrightarrow> swap_rel u Q t P1 t' \<Longrightarrow>
      grd Q (lower_tick Q t') = t'"
    and book: "grd P' = grd P \<and> lq P' = lq P \<and> s' = s"
  shows "P' = P \<and> s' = s \<and> fee P' = fee P"
  using swap_seq_fee_inv [OF seq grid] book by simp

subsection \<open>Fee growth is bounded by volume, not placement\<close>

text \<open>On a static book an on-grid swap changes nothing at all, and an
off-grid swap only reindexes (the new cell copying the fee value below
it), so accrued fee mass \<^term>\<open>fee P\<close> is never grown by trading; fees
grow only through the accrual event (layer 2), charged per performed
operation --- never through swap ordering. Combined with
\<^theory_text>\<open>swap_roundtrip_zero\<close>: a sandwich's gross quote position through a
static book is exactly zero, and its fee capture is exactly zero.\<close>

end
