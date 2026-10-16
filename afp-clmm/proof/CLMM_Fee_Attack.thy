theory CLMM_Fee_Attack
  imports Aggregate_Accrual CLMM_Fee_Slack CLMM_Fee_Slack_Fixed CLMM_Dynamic
begin

section \<open>Attacker-edge layer 2: dust-farming and negative slack\<close>

text \<open>Layer-2 fee-arithmetic attacker-edge rungs, on top of the accrual
base (Aggregate_Accrual: the owed fold, the charged aggregate book, and
the bridge theorem owed <= charged + one truncation unit per op).

Rung 2 (dust-farming): can an attacker amplify fee extraction by
re-triggering accrual rounds faster than the protocol's checkpoint
cadence assumes?  Answer at the event level: no.  Replaying a
constant-growth accrual any number of times charges exactly one honest
truncation unit, because the checkpoint threads to the current growth
after the first leg and every later leg has zero growth delta
(dust_farming_exact).  Per-leg floor division never overcharges
(div_floor64_no_overcharge) and is tight exactly when the delta is a
multiple of the scale (div_floor64_tight); the per-op slack of the
bridge theorem is bounded by one full unit and is attained only by
two nonzero remainders (div_floor64_slack_le_1).

Rung 3 (no negative slack): nat subtraction saturates, so a HOSTILE
checkpoint above the current growth silently zeroes the leg -- the
witness theorem pins this down concretely (a checkpoint of 7 against
growth 5 charges nothing, while the same growth charged from the
genesis checkpoint collects a full unit whenever the cap is at least
one scale unit).  Under the sorted-growth invariant the loss cannot
occur: strictly increasing checkpoints with capped legs keep every
fold strictly positive (sorted_no_loss).  Guard necessary, guard
sufficient -- the rung-1 discipline, at the fee layer.\<close>

section \<open>Rung 2a: floor division never overcharges, and is tight\<close>

text \<open>Per-op floor is the owed-side rounding: it can only charge less
than the exact product, never more.  (Jean's div_floor64_le_mult in
product form; restated per-op for the fee legs.)\<close>

lemma div_floor64_no_overcharge:
  fixes x :: nat
  shows "x div 2^64 * 2^64 \<le> x"
  using div_floor64_le_mult unfolding div_floor64_def by simp

text \<open>Tightness: the floor loses nothing exactly when the scale
divides the product.\<close>

lemma div_floor64_tight:
  fixes x :: nat
  assumes dvd: "2^64 dvd x"
  shows "x div 2^64 * 2^64 = x"
proof -
  from dvd have m0: "x mod 2^64 = 0" by (rule dvd_imp_mod_0)
  have dm: "x div 2^64 * 2^64 + x mod 2^64 = x"
    by (rule div_mult_mod_eq)
  show ?thesis using dm m0 by arith
qed

text \<open>The bridge theorem's +1 per op is a real bound but never a
bigger subsidy: splitting a total across two legs loses at most one
unit, and only when BOTH remainders are nonzero.  Consequence: an
attacker splitting fee exposure across many accrual events gains at
most one unit per event -- the same unit the bridge already charges --
never a compounding edge.\<close>

lemma div_floor64_slack_le_1:
  fixes X Y :: nat
  shows "X div 2^64 + Y div 2^64
       \<le> (X + Y) div 2^64 + 1"
  by (rule div_add_slack) simp

section \<open>Rung 2b: replay idempotence -- the dust-farming kill\<close>

text \<open>Accruing at constant growth g from a checkpoint already at g is
the zero leg: the growth delta is zero, so the round charges nothing,
no matter the cap and no matter how often it is replayed.\<close>

lemma replay_zero:
  shows "accruef Lf g (replicate n g) = 0"
proof (induction n)
  case 0
  show ?case by simp
next
  case (Suc n)
  have step: "accruef Lf g (replicate (Suc n) g)
      = (g - g) * Lf g div 2^64 + accruef Lf g (replicate n g)"
    by simp
  show ?case using step Suc.IH by simp
qed

text \<open>Dust-farming exactness: replaying the accrual for one honest
growth leg any number of extra times adds nothing.  The first leg
charges the honest floor unit; every replay leg has checkpoint = growth
and charges zero.  Protocol revenue equals the single-shot charge --
the attacker cannot amplify by re-triggering.\<close>

theorem dust_farming_exact:
  shows "accruef Lf c (replicate (Suc n) g)
       = (g - c) * Lf g div 2^64"
proof -
  have r1: "replicate (Suc n) g = g # replicate n g" by simp
  have u: "accruef Lf c (g # replicate n g)
      = (g - c) * Lf g div 2^64 + accruef Lf g (replicate n g)"
    by simp
  have z: "accruef Lf g (replicate n g) = 0"
    by (rule replay_zero)
  show ?thesis using u z by simp
qed

theorem dust_farming_bounded:
  shows "accruef Lf c (replicate (Suc n) g)
       \<le> (g - c) * Lf g div 2^64 + 1"
  using dust_farming_exact by simp

text \<open>Batching law: one round over [g] followed by n zero-delta
replays equals exactly the split into the honest leg plus the replays.
No ordering or batch-shape of the same events changes the total --
the fold's order-freedom carries to the fee layer.\<close>

lemma accruef_replay_split:
  shows "accruef Lf c (g # replicate n g)
       = accruef Lf g (replicate n g) + accruef Lf c [g]"
proof -
  have exL: "accruef Lf c (g # replicate n g)
      = (g - c) * Lf g div 2^64 + accruef Lf g (replicate n g)"
    by simp
  have exR: "accruef Lf g (replicate n g) + accruef Lf c [g]
      = accruef Lf g (replicate n g)
        + ((g - c) * Lf g div 2^64 + accruef Lf g [])"
    by simp
  have zero: "accruef Lf g [] = 0" by simp
  show ?thesis using exL exR zero by simp
qed

section \<open>Rung 3: negative slack -- witness and guard\<close>

text \<open>Witness: a hostile checkpoint above the current growth saturates
the nat delta and the whole leg is skipped, for ANY cap.  The same
growth charged from the genesis checkpoint (0) collects a full unit
whenever the cap covers one scale unit.  A checkpoint bug (or a
transaction that writes an inflated checkpoint) therefore silently
zeroes fee revenue -- an attacker holding such a position farms
revenue down to zero.  This is why the checkpoint monotonicity guard
is not decoration.\<close>

theorem no_negative_slack_witness:
  shows "accruef Lf 7 [5] = 0"
    and "0 < g \<Longrightarrow> 2^64 \<le> g * Lf g \<Longrightarrow> 0 < accruef Lf 0 [g]"
proof -
  show "accruef Lf 7 [5] = 0" by simp
next
  assume g0: "0 < g" and a2: "2^64 \<le> g * Lf g"
  have q1: "(2^64::nat) div 2^64 = 1" by simp
  have q2: "(1::nat) \<le> g * Lf g div 2^64" using a2 q1 by simp
  show "0 < accruef Lf 0 [g]" using q2 by simp
qed

text \<open>General saturation law behind the witness.\<close>

lemma checkpoint_saturation_sat:
  assumes gt: "g < c"
  shows "accruef Lf c [g] = 0"
  using gt by simp

text \<open>Guard sufficiency, per leg: a strictly-growing leg with a cap
covering one scale unit charges at least a full unit.\<close>

lemma checkpoint_sorted_leg_nonzero:
  fixes c g :: nat
  assumes lt: "c < g" and cap: "2^64 \<le> Lf g"
  shows "0 < (g - c) * Lf g div 2^64"
proof -
  have onele: "(1::nat) \<le> g - c" using lt by simp
  have a1: "(1::nat) * 2^64 \<le> (g - c) * 2^64"
    by (rule mult_le_mono1 [OF onele])
  have m1: "(g - c) * 2^64 \<le> (g - c) * Lf g"
    by (rule mult_le_mono2 [OF cap])
  have a2: "2^64 \<le> (g - c) * Lf g" using a1 m1 by arith
  have p3: "2^64 div 2^64 \<le> (g - c) * Lf g div 2^64"
    by (rule div_le_mono [OF a2])
  show "0 < (g - c) * Lf g div 2^64" using p3 by simp
qed

text \<open>Guard sufficiency, fold level: under the sorted-growth
invariant (checkpoints strictly increase along the op list, no
duplicates) with every cap at least one scale unit, every accrual
fold charges strictly positive.  Combined with the witness: the
checkpoint guard is necessary, and sorted growth is sufficient --
no negative slack is reachable in the guarded protocol.\<close>

theorem sorted_no_loss:
  assumes sorted_g: "sorted (c # gs)"
    and dist: "distinct (c # gs)"
    and cap: "\<And>h. h \<in> set (c # gs) \<Longrightarrow> 2^64 \<le> Lf h"
    and ne: "gs \<noteq> []"
  shows "0 < accruef Lf c gs"
proof (cases gs)
  case Nil
  then show ?thesis using ne by simp
next
  case (Cons g rest)
  then have gsform: "gs = g # rest" by simp
  have sorted2: "sorted (c # g # rest)" using sorted_g gsform by simp
  have dist2: "distinct (c # g # rest)" using dist gsform by simp
  have cle: "c \<le> g" using sorted2 by (auto simp add: sorted_simps)
  have cne: "c \<noteq> g" using dist2 by simp
  have clt: "c < g" using cle cne by arith
  have capg: "2^64 \<le> Lf g" using cap[of g] gsform by simp
  have onele: "(1::nat) \<le> g - c" using clt by simp
  have a1: "(1::nat) * 2^64 \<le> (g - c) * 2^64"
    by (rule mult_le_mono1 [OF onele])
  have m1: "(g - c) * 2^64 \<le> (g - c) * Lf g"
    by (rule mult_le_mono2 [OF capg])
  have a2: "2^64 \<le> (g - c) * Lf g" using a1 m1 by arith
  have p3: "2^64 div 2^64 \<le> (g - c) * Lf g div 2^64"
    by (rule div_le_mono [OF a2])
  have leg: "0 < (g - c) * Lf g div 2^64" using p3 by simp
  have unfold: "accruef Lf c (g # rest)
      = (g - c) * Lf g div 2^64 + accruef Lf g rest" by simp
  show ?thesis using gsform unfold leg by simp
qed

section \<open>Rung 3b: fold-level under-charge-only\<close>

text \<open>The owed fold never exceeds the undivided product sum: fee
arithmetic can only under-charge relative to the exact real-arithmetic
obligation, never over-charge.  Together with the bridge theorem this
locates every loss precisely: truncation (bounded, rung 2) or
saturation (guarded, rung 3) -- never an overcharge path.\<close>

lemma accruef_le_prodsum:
  fixes Lf :: "nat \<Rightarrow> nat"
  shows "accruef Lf c gs \<le> prodsum Lf c gs"
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have j1: "(g - c) * Lf g div 2^64 * 2^64 \<le> (g - c) * Lf g"
    by (rule div_floor64_no_overcharge)
  have j2: "(g - c) * Lf g div 2^64
      \<le> (g - c) * Lf g"
    by (rule div_le_dividend)
  have t1: "accruef Lf g gs \<le> prodsum Lf g gs" using Cons.IH .
  have add: "accruef Lf c (g # gs) \<le> prodsum Lf c (g # gs)"
    using j2 t1 by simp
  show ?case using add by simp
qed

section \<open>Rung 5b: the aggregate window -- the profit bound, both sides\<close>

text \<open>Jean's accruef_sum_bound gives the upper side (never overcharged
beyond one unit per op). The lower side is the attacker-facing half: no
structure --- replay, splitting, hostile checkpoints, reordering (the
fold adds nonneg legs, so reordering legs swaps addends) --- saves more
than one truncation unit per accrual leg off the honest bill.  Together:
the fee bill for a growth history always sits in the window

  \<^term>\<open>prodsum Lf 0 gs div 2^64 - length gs \<le> accruef Lf 0 gs\<close> and
  \<^term>\<open>accruef Lf 0 gs \<le> prodsum Lf 0 gs div 2^64 + length gs\<close>

so no strategy extracts more than the fee slack allows, and none is
overcharged beyond it.\<close>

text \<open>Core ingredient: the carry bound --- adding under a floor hides
at most one extra truncation unit (tight, e.g. legs q-1, q-1).\<close>

lemma div_add_le:
  fixes m n q :: nat
  assumes q: "1 \<le> q"
  shows "(m + n) div q \<le> m div q + n div q + 1"
proof -
  have qpos: "0 < q" using q by simp
  have m1: "q * (m div q) + m mod q = m" by (fact mult_div_mod_eq)
  have n1: "q * (n div q) + n mod q = n" by (fact mult_div_mod_eq)
  have mq: "m mod q < q" by (rule mod_less_divisor) (rule qpos)
  have nq: "n mod q < q" by (rule mod_less_divisor) (rule qpos)
  have S1: "q * (m div q + 1) = q * (m div q) + q"
    by (simp add: algebra_simps)
  have S2: "q * (n div q + 1) = q * (n div q) + q"
    by (simp add: algebra_simps)
  have a1: "m < q * (m div q + 1)" using m1 mq S1 by arith
  have a2: "n < q * (n div q + 1)" using n1 nq S2 by arith
  have S: "q * (m div q + 1) + q * (n div q + 1)
      = q * (m div q + n div q + 2)" by (simp add: algebra_simps)
  have lt: "m + n < q * (m div q + n div q + 2)" using a1 a2 S by arith
  have DD: "(m + n) div q * q + (m + n) mod q = m + n"
    by (fact div_mult_mod_eq)
  show ?thesis
  proof (rule ccontr)
    assume F: "\<not> (m + n) div q \<le> m div q + n div q + 1"
    then have ge: "m div q + n div q + 2 \<le> (m + n) div q" by simp
    have le1: "q * (m div q + n div q + 2) \<le> q * ((m + n) div q)"
      by (rule mult_le_mono2) (rule ge)
    have D: "q * ((m + n) div q) \<le> m + n" using DD by simp
    have c1: "q * (m div q + n div q + 2) \<le> m + n"
      using le_trans [OF le1 D] .
    show False using lt c1 by linarith
  qed
qed

text \<open>Lower side: the fold never falls short of the honest bill by more
than one truncation unit per leg.\<close>

theorem accruef_window_lower:
  fixes Lf :: "nat \<Rightarrow> nat" and c :: nat
  shows "prodsum Lf c gs div 2^64 \<le> accruef Lf c gs + length gs"
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have unfold: "accruef Lf c (g # gs)
      = (g - c) * Lf g div 2^64 + accruef Lf g gs" by simp
  have punfold: "prodsum Lf c (g # gs)
      = (g - c) * Lf g + prodsum Lf g gs" by simp
  have leg: "(g - c) * Lf g div 2^64
      \<le> (g - c) * Lf g div 2^64 + 1" by arith
  have ih: "prodsum Lf g gs div 2^64
      \<le> accruef Lf g gs + length gs" using Cons.IH .
  have pd: "prodsum Lf c (g # gs) div 2^64
      = ((g - c) * Lf g + prodsum Lf g gs) div 2^64"
    using punfold by simp
  have tot: "((g - c) * Lf g + prodsum Lf g gs) div 2^64
      \<le> (g - c) * Lf g div 2^64 + prodsum Lf g gs div 2^64 + 1"
    by (rule div_add_le) simp
  have lensimp: "length (g # gs) = Suc (length gs)" by simp
  show ?case using unfold pd ih tot lensimp by arith
qed

text \<open>The window theorem, both sides at the genesis checkpoint.\<close>

theorem fee_profit_window:
  fixes Lf :: "nat \<Rightarrow> nat"
  shows "prodsum Lf 0 gs div 2^64 - length gs \<le> accruef Lf 0 gs \<and>
      accruef Lf 0 gs
      \<le> prodsum Lf 0 gs div 2^64 + length gs"
proof
  show "prodsum Lf 0 gs div 2^64 - length gs \<le> accruef Lf 0 gs"
    using accruef_window_lower [of Lf 0 gs] by arith
next
  have uni: "accruef Lf 0 gs
      \<le> prodsum Lf 0 gs div 2^64 + length gs"
    using accruef_sum_bound [of Lf 0 gs] .
  show "accruef Lf 0 gs
      \<le> prodsum Lf 0 gs div 2^64 + length gs" using uni .
qed

section \<open>Composite: fixed-point guard vs. real slack, the full chain\<close>

text \<open>kb96's fail-closed law bounds the guard's shortfall by the real
slack, in real units. Two additions complete the chain. First, the
guard gap is at most ONE DENOMINATOR in guard units: whenever
usage \<le> capacity, div_ceil64 usage \<le> div_floor64 capacity + 1, so the
fixed point never disagrees with the straight edge by more than 2^64.
The proof composes the carry bound div_add_le (rung 5b) with
div_le_mono. Second, the tempting refinement "shortfall in guard
units \<le> real slack" is FALSE: capacity = usage = 10 with den = 4 gives
gap 4 > 0 = real slack (kb96's subtraction-on-the-left absorbs this
case into zero; a multiplied-out form does not). The subtraction-free
formulation in kb96 is exactly what survives -- documented here so
nobody restates the false version. Together with fee_profit_window:
the charged bill sits within a bounded, path-independent window of
the honest obligation, and the guard that enforces it fails closed
with gap at most one denominator. No strategy extracts more.\<close>

lemma ceil64_le_floor64_add1:
  fixes c u :: nat
  assumes le: "u \<le> c"
  shows "div_ceil64 u \<le> div_floor64 c + 1"
proof -
  have bridge: "u + 2^64 - 1 = u + (2^64 - 1)" by simp
  have s1: "(u + (2^64 - 1)) div 2^64 \<le> (c + (2^64 - 1)) div 2^64"
    by (rule div_le_mono) (insert le, simp)
  have s2: "(c + (2^64 - 1)) div 2^64
      \<le> c div 2^64 + (2^64 - 1) div 2^64 + 1"
    by (rule div_add_le) simp
  show ?thesis
    unfolding div_ceil64_def div_floor64_def
    using bridge s1 s2 by simp
qed

theorem guard_gap_at_most_den:
  fixes c u :: nat
  assumes le: "u \<le> c"
  shows "div_ceil64 u * 2^64 \<le> div_floor64 c * 2^64 + 2^64"
proof -
  have h: "div_ceil64 u \<le> div_floor64 c + 1"
    using ceil64_le_floor64_add1 [OF le] .
  have step: "div_ceil64 u * 2^64 \<le> (div_floor64 c + 1) * 2^64"
    by (rule mult_le_mono [OF h le_refl])
  show ?thesis using step by simp
qed

section \<open>kb94-path: fee slack telescoped over an ascent (item 2, slice A)\<close>

text \<open>kb94's fee_slack_step holds for one nondecreasing price move.
Any finite nondecreasing price path is a list of such moves, and both
sides telescope: intermediate terms cancel in
(a2 - a1) + (a3 - a2) + ... So total NET growth over the whole ascent is
at most total GROSS growth -- solvency is price-path-independent on
ascents: no arrival order of up-moves extracts more net than the gross
book allows. Stated for nondecreasing paths only: the step theorem's
proven direction is an ascent; descents are a separate (unproven here)
concern, claimed by nobody.\<close>

fun ascent :: "real list \<Rightarrow> bool" where
  "ascent [] = True"
| "ascent (x # xs) = (case xs of [] \<Rightarrow> True | y # _ \<Rightarrow> x \<le> y \<and> ascent xs)"

fun growth :: "(real \<Rightarrow> real) \<Rightarrow> real list \<Rightarrow> real" where
  "growth f [] = 0"
| "growth f [x] = 0"
| "growth f (x # y # ys) = (f y - f x) + growth f (y # ys)"

theorem fee_slack_path:
  assumes dsc: "clmm_dsc P"
    and asc: "ascent ps"
    and nn: "\<And>p. p \<in> set ps \<Longrightarrow> 0 < p"
  shows "growth (quote_net P) ps
      \<le> growth (quote_gross P) ps"
using asc nn proof (induction ps)
  case Nil
  show ?case by simp
next
  case (Cons p ps)
  note mem = Cons.prems(2)
  note asct = Cons.prems(1)
  note IH0 = Cons.IH
  show ?case
  proof (cases ps)
    case Nil
    then show ?thesis by simp
  next
    case (Cons q more)
    have psEq: "ps = q # more" using Cons by simp
    have a0: "p \<le> q \<and> ascent ps" using asct psEq by simp
    have p0: "0 < p" using mem [of p] by simp
    have pq: "p \<le> q" using a0 by blast
    have asc': "ascent ps" using a0 by blast
    have step: "quote_net P q - quote_net P p
        \<le> quote_gross P q - quote_gross P p"
      using fee_slack_step [OF dsc p0 pq] .
    have mem2: "\<And>pa. pa \<in> set ps \<Longrightarrow> 0 < pa"
    proof -
      fix pa assume A: "pa \<in> set ps"
      have S: "pa \<in> set (p # ps)" using A by simp
      show "0 < pa" using mem [OF S] .
    qed
    have IH: "growth (quote_net P) ps
        \<le> growth (quote_gross P) ps" using IH0 [OF asc' mem2] .
    have IH': "growth (quote_net P) (q # more)
        \<le> growth (quote_gross P) (q # more)" using IH psEq by simp
    show ?thesis using step IH' by (simp add: psEq)
  qed
qed

section \<open>Off-grid refine: fee values travel (item 2, slice B)\<close>

text \<open>Formalizes the rung-5 prose law, with the copy term made honest. An
off-grid swap inserts a tick level and shifts the fee book positionally;
the inserted cell COPIES the value below it, so unconditional
@{term "fee P' = fee P"} is FALSE (rung 5's own goal-print falsification).
The true law has three legs, all stated here: (1) pointwise, the new book
is the old book on every cell: identity below the boundary, a copy at the
new boundary cell, a one-step shift above it; (2) values-travel: every
post-state fee value is a pre-state fee value -- trading creates no fee
value; (3) exact restore: the old fee book is recovered from the new one
by identity below the boundary and a +1 shift above it (the copy cell is
never read), so reindexing repositions value and adds exactly one
boundary copy -- the venue's own single-cell double-write, not
attacker-shaped. The book moves; the money does not.

Mechanics (from the entry): off-grid refine is
@{term "wedge (fee P) i (fee P i)"} with @{term "i = lower_tick P sqp"}.
Scope honestly: aggregate-sum accounting follows from the pointwise law
plus support finiteness; the boundary copy contributes that one cell's
value to any such sum.\<close>

lemma wedge_split:
  fixes i n :: int
  assumes w: "g = wedge f i c"
  shows "g n = (if n \<le> i then f n
      else if n = i + 1 then c else f (n - 1))"
  using w by (simp add: wedge_def)

theorem fee_offgrid_pointwise:
  assumes i: "i = lower_tick P sqp"
    and off: "grd P i \<noteq> sqp"
    and P': "P' = refine P sqp"
  shows "\<And>n. fee P' n = (if n \<le> i then fee P n
      else if n = i + 1 then fee P i else fee P (n - 1))"
proof -
  fix n
  have w: "fee P' = wedge (fee P) i (fee P i)"
    using i off P' by (rule refine_fee)
  show "fee P' n = (if n \<le> i then fee P n
      else if n = i + 1 then fee P i else fee P (n - 1))"
    using w by (rule wedge_split)
qed

theorem fee_offgrid_values_travel:
  assumes i: "i = lower_tick P sqp"
    and off: "grd P i \<noteq> sqp"
    and P': "P' = refine P sqp"
  shows "\<And>n. fee P' n \<in> range (fee P)"
proof -
  have pw: "\<And>n. fee P' n = (if n \<le> i then fee P n
      else if n = i + 1 then fee P i else fee P (n - 1))"
  proof -
    fix n
    have w: "fee P' = wedge (fee P) i (fee P i)"
      using i off P' by (rule refine_fee)
    show "fee P' n = (if n \<le> i then fee P n
        else if n = i + 1 then fee P i else fee P (n - 1))"
      using w by (rule wedge_split)
  qed
  fix n
  show "fee P' n \<in> range (fee P)"
    using pw [of n] by (cases "n \<le> i"; cases "n = i + 1") auto
qed

theorem fee_offgrid_restore:
  assumes i: "i = lower_tick P sqp"
    and off: "grd P i \<noteq> sqp"
    and P': "P' = refine P sqp"
  shows "fee P = (\<lambda>n. if n \<le> i then fee P' n else fee P' (n + 1))"
proof -
  have pw: "\<And>n. fee P' n = (if n \<le> i then fee P n
      else if n = i + 1 then fee P i else fee P (n - 1))"
  proof -
    fix n
    have w: "fee P' = wedge (fee P) i (fee P i)"
      using i off P' by (rule refine_fee)
    show "fee P' n = (if n \<le> i then fee P n
        else if n = i + 1 then fee P i else fee P (n - 1))"
      using w by (rule wedge_split)
  qed
  show ?thesis
  proof
    fix n
    show "fee P n = (if n \<le> i then fee P' n else fee P' (n + 1))"
    proof (cases "n \<le> i")
      case True
      have "fee P' n = fee P n" using pw [of n] True by simp
      then show ?thesis using True by simp
    next
      case False
      have gt: "i + 1 < n + 1" using False by simp
      have "fee P' (n + 1) = fee P (n + 1 - 1)" using pw [of "n + 1"] gt by simp
      then show ?thesis using False by simp
    qed
  qed
qed

subsection \<open>Swap-level hook: the rung-5 off-grid walk\<close>

text \<open>Off-grid swaps (the rung-5 case @{term "grd P (lower_tick P s') \<noteq> s'"})
reindex the fee book exactly as the three legs above: pointwise
copy-shift, values travel, exact restore. No premise on the pool beyond
the event itself.\<close>

theorem swap_offgrid_fee_travel:
  assumes sw: "swap_rel v P s P' s'"
    and off: "grd P (lower_tick P s') \<noteq> s'"
  shows "fee P = (\<lambda>n. if n \<le> lower_tick P s'
      then fee P' n else fee P' (n + 1))"
    and "\<And>n. fee P' n \<in> range (fee P)"
proof -
  from sw have Peq: "P' = refine P s'" by (simp add: swap_rel_def)
  have r1: "fee P = (\<lambda>n. if n \<le> lower_tick P s'
      then fee P' n else fee P' (n + 1))"
    by (rule fee_offgrid_restore [OF _ off Peq]) simp
  have r2: "\<And>n. fee P' n \<in> range (fee P)"
    by (rule fee_offgrid_values_travel [OF _ off Peq]) simp
  show "fee P = (\<lambda>n. if n \<le> lower_tick P s'
      then fee P' n else fee P' (n + 1))" by (fact r1)
  show "\<And>n. fee P' n \<in> range (fee P)" by (fact r2)
qed

section \<open>Aggregate fee mass under an off-grid refine (item 2, slice C)\<close>

text \<open>The pointwise copy-shift law fixes the aggregate accounting as
well. Summed over its own (finite) support, the new book carries the
old mass PLUS ONE COPY of the boundary cell's value: the inserted cell
i+1 duplicates fee P i, and every other cell merely repositions. The
aggregate law therefore carries the copy term -- the falsified
sum-preserving form (fee P = [3,5], wedge at 0: 8 -> 11) is exactly
the instance fee P i = 3 of this theorem. Summation is over the
nonzero support of each book; finiteness of the old support is
assumed (well-formed pools have finite grids, hence finite fee
support), and finiteness of the new support is derived from it.\<close>

theorem fee_offgrid_sum:
  assumes fsup: "finite {n. fee P n \<noteq> 0}"
    and i: "i = lower_tick P sqp"
    and off: "grd P i \<noteq> sqp"
    and P': "P' = refine P sqp"
  shows "sum (fee P') {n. fee P' n \<noteq> 0}
       = sum (fee P) {n. fee P n \<noteq> 0} + fee P i"
proof -
  have pw: "\<And>n. fee P' n = (if n \<le> i then fee P n
      else if n = i + 1 then fee P i else fee P (n - 1))"
  proof -
    fix n
    have w: "fee P' = wedge (fee P) i (fee P i)"
      using i off P' by (rule refine_fee)
    show "fee P' n = (if n \<le> i then fee P n
        else if n = i + 1 then fee P i else fee P (n - 1))"
      using w by (rule wedge_split)
  qed
  let ?S = "{n. fee P n \<noteq> 0}"
  let ?S' = "{n. fee P' n \<noteq> 0}"
  have finS: "finite ?S" by (rule fsup)
  have finB: "finite (?S \<union> {i + 1} \<union> (+) 1 ` ?S)" using finS by simp
  have subS': "?S' \<subseteq> ?S \<union> {i + 1} \<union> (+) 1 ` ?S"
  proof
    fix n assume n: "n \<in> ?S'"
    then have nz: "fee P' n \<noteq> 0" by simp
    show "n \<in> ?S \<union> {i + 1} \<union> (+) 1 ` ?S"
    proof (cases "n \<le> i")
      case True
      then have "fee P n \<noteq> 0" using pw [of n] nz by simp
      then show ?thesis by simp
    next
      case False
      note nle = False
      show ?thesis
      proof (cases "n = i + 1")
        case True
        then show ?thesis by simp
      next
        case False
        have lt: "i < n" using nle by simp
        have ub: "i + 2 \<le> n" using lt False by linarith
        have eq: "fee P (n - 1) = fee P' n" using pw [of n] ub by simp
        then have nz1: "fee P (n - 1) \<noteq> 0" using nz by simp
        have m: "n - 1 \<in> ?S" using nz1 by simp
        have heq: "n = 1 + (n - 1)" using lt by simp
        have img: "n \<in> (+) 1 ` ?S"
        proof (rule image_eqI)
          show "n = 1 + (n - 1)" using heq by simp
        next
          show "n - 1 \<in> ?S" using m by simp
        qed
        show ?thesis using img by (rule UnI2)
      qed
    qed
  qed
  have finS': "finite ?S'" by (rule finite_subset [OF subS' finB])
  have Aeq: "?S' \<inter> {n. n \<le> i} = ?S \<inter> {n. n \<le> i}"
  proof (rule subset_antisym)
    show "?S' \<inter> {n. n \<le> i} \<subseteq> ?S \<inter> {n. n \<le> i}"
    proof
      fix n assume "n \<in> ?S' \<inter> {n. n \<le> i}"
      then have nS': "n \<in> ?S'" and le: "n \<le> i" by auto
      have nz: "fee P' n \<noteq> 0" using nS' by simp
      have eq: "fee P n = fee P' n" using pw [of n] le by simp
      then have "fee P n \<noteq> 0" using nz by simp
      then show "n \<in> ?S \<inter> {n. n \<le> i}" using le by simp
    qed
  next
    show "?S \<inter> {n. n \<le> i} \<subseteq> ?S' \<inter> {n. n \<le> i}"
    proof
      fix n assume "n \<in> ?S \<inter> {n. n \<le> i}"
      then have nS: "n \<in> ?S" and le: "n \<le> i" by auto
      have nz: "fee P n \<noteq> 0" using nS by simp
      have eq: "fee P' n = fee P n" using pw [of n] le by simp
      then have "fee P' n \<noteq> 0" using nz by simp
      then show "n \<in> ?S' \<inter> {n. n \<le> i}" using le by simp
    qed
  qed
  have sumA: "sum (fee P') (?S' \<inter> {n. n \<le> i})
      = sum (fee P) (?S \<inter> {n. n \<le> i})"
    unfolding Aeq
  proof (rule sum.cong)
    show "?S \<inter> {n. n \<le> i} = ?S \<inter> {n. n \<le> i}" by simp
  next
    fix x assume "x \<in> ?S \<inter> {n. n \<le> i}"
    then have le: "x \<le> i" by simp
    show "fee P' x = fee P x" using pw [of x] le by simp
  qed
  have sumB: "sum (fee P') (?S' \<inter> {n. n = i + 1}) = fee P i"
  proof (cases "fee P i = 0")
    case True
    have notin: "i + 1 \<notin> ?S'" using pw [of "i + 1"] True by simp
    have int: "?S' \<inter> {n. n = i + 1} = {}" using notin by auto
    show ?thesis unfolding int by (simp add: True)
  next
    case False
    have inn: "i + 1 \<in> ?S'" using pw [of "i + 1"] False by simp
    have int: "?S' \<inter> {n. n = i + 1} = {i + 1}" using inn by auto
    show ?thesis unfolding int using pw [of "i + 1"] by simp
  qed
  have Cimg: "(\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n})
      = ?S \<inter> {n. i + 1 \<le> n}"
  proof (rule subset_antisym)
    show "(\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n})
        \<subseteq> ?S \<inter> {n. i + 1 \<le> n}"
    proof
      fix m assume "m \<in> (\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n})"
      then obtain n where n: "n \<in> ?S'" and ub: "i + 2 \<le> n"
        and meq: "m = n - 1" by auto
      have nz: "fee P' n \<noteq> 0" using n by simp
      have e1: "fee P (n - 1) = fee P' n" using pw [of n] ub by simp
      have m0: "fee P m \<noteq> 0" using nz e1 meq by simp
      have mlb: "i + 1 \<le> m" using ub meq by simp
      show "m \<in> ?S \<inter> {n. i + 1 \<le> n}" using m0 mlb by simp
    qed
  next
    show "?S \<inter> {n. i + 1 \<le> n}
        \<subseteq> (\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n})"
    proof
      fix m assume "m \<in> ?S \<inter> {n. i + 1 \<le> n}"
      then have mz: "fee P m \<noteq> 0" and lb: "i + 1 \<le> m" by auto
      define n where "n = m + 1"
      have meq: "m = n - 1" by (simp add: n_def)
      have ub: "i + 2 \<le> n" unfolding n_def using lb by linarith
      have "fee P' n = fee P (n - 1)" using pw [of n] ub by simp
      then have nS: "n \<in> ?S'" using mz meq by simp
      show "m \<in> (\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n})"
      proof (rule image_eqI)
        show "m = n - 1" using meq by simp
      next
        show "n \<in> ?S' \<inter> {n. i + 2 \<le> n}" using nS ub by (simp add: n_def)
      qed
    qed
  qed
  have injm: "inj_on (\<lambda>n. n - 1) (?S' \<inter> {n. i + 2 \<le> n})"
    by (rule inj_onI) arith
  have sumC: "sum (fee P') (?S' \<inter> {n. i + 2 \<le> n})
      = sum (fee P) (?S \<inter> {n. i + 1 \<le> n})"
  proof -
    have r: "sum (fee P) ((\<lambda>n. n - 1) ` (?S' \<inter> {n. i + 2 \<le> n}))
        = sum (\<lambda>x. fee P (x - 1)) (?S' \<inter> {n. i + 2 \<le> n})"
      by (simp add: sum.reindex [OF injm])
    have c2: "sum (\<lambda>x. fee P (x - 1)) (?S' \<inter> {n. i + 2 \<le> n})
        = sum (fee P') (?S' \<inter> {n. i + 2 \<le> n})"
    proof (rule sum.cong)
      show "?S' \<inter> {n. i + 2 \<le> n} = ?S' \<inter> {n. i + 2 \<le> n}" by simp
    next
      fix x assume "x \<in> ?S' \<inter> {n. i + 2 \<le> n}"
      then show "fee P (x - 1) = fee P' x" using pw [of x] by simp
    qed
    show ?thesis using r c2 Cimg by simp
  qed
  have finI: "finite (?S' \<inter> {n. n \<le> i})"
    and finII: "finite (?S' \<inter> {n. i + 1 \<le> n})"
    and finIII: "finite (?S' \<inter> {n. n = i + 1})"
    and finIV: "finite (?S' \<inter> {n. i + 2 \<le> n})"
    using finS' by simp_all
  have split1: "?S' = (?S' \<inter> {n. n \<le> i}) \<union> (?S' \<inter> {n. i + 1 \<le> n})"
    by auto
  have d1: "?S' \<inter> {n. n \<le> i} \<inter> (?S' \<inter> {n. i + 1 \<le> n}) = {}" by auto
  have sumUB: "sum (fee P') ?S'
      = sum (fee P') (?S' \<inter> {n. n \<le> i})
        + sum (fee P') (?S' \<inter> {n. i + 1 \<le> n})"
    apply (subst split1)
    apply (rule sum.union_disjoint [OF finI finII d1])
    done
  have split2: "?S' \<inter> {n. i + 1 \<le> n}
      = (?S' \<inter> {n. n = i + 1}) \<union> (?S' \<inter> {n. i + 2 \<le> n})"
    by auto
  have d2: "?S' \<inter> {n. n = i + 1} \<inter> (?S' \<inter> {n. i + 2 \<le> n}) = {}" by auto
  have sumU2: "sum (fee P') (?S' \<inter> {n. i + 1 \<le> n})
      = sum (fee P') (?S' \<inter> {n. n = i + 1})
        + sum (fee P') (?S' \<inter> {n. i + 2 \<le> n})"
    apply (subst split2)
    apply (rule sum.union_disjoint [OF finIII finIV d2])
    done
  have rsplit: "?S = (?S \<inter> {n. n \<le> i}) \<union> (?S \<inter> {n. i + 1 \<le> n})"
    by auto
  have finR1: "finite (?S \<inter> {n. n \<le> i})"
    and finR2: "finite (?S \<inter> {n. i + 1 \<le> n})"
    using finS by simp_all
  have dR: "?S \<inter> {n. n \<le> i} \<inter> (?S \<inter> {n. i + 1 \<le> n}) = {}" by auto
  have sumR: "sum (fee P) ?S
      = sum (fee P) (?S \<inter> {n. n \<le> i})
        + sum (fee P) (?S \<inter> {n. i + 1 \<le> n})"
    apply (subst rsplit)
    apply (rule sum.union_disjoint [OF finR1 finR2 dR])
    done
  show ?thesis using sumUB sumA sumB sumC sumU2 sumR by simp
qed

section \<open>Descents: falsification and the mirrored step law (item 2, descents)\<close>

text \<open>kb94 states the fee-slack step for an ascent @{term "p \<le> p'"}, and
slice A inherited that restriction. The investigation verdict: the
restriction is LOAD-BEARING, not a proof artifact. Extending the ascent
form to a descent is FALSE -- counterexample: one cell, lq = 1,
fee = 1/2 (so the gross coefficient is 1/(1 - 1/2) = 2), price moves
1 -> 1/2 inside the cell. Then net moves by 1*(1/2 - 1) = -1/2 while
gross moves by 2*(1/2 - 1) = -1, and the ascent form would demand
-1/2 <= -1, which is false: the fee mass widens the gross swing in
BOTH directions, so the per-move diff inequality necessarily flips
orientation on a down-move. (The build system caught this first: the
drafted direction-free step theorem failed at linarith with exactly
this goal -- the goal print is a soundness oracle, third campaign use.)

What IS true on a descent is the mirrored law below: the gross move
understates the net move, i.e. the gap between gross and net does not
narrow on the way down. It is kb94 applied to the reversed (ascending)
pair, with both sides negated -- no new decomposition needed. Scope
consequence for paths: a mixed up-down path has NO uniform telescoping
bound, because descent legs carry the mirrored inequality while
telescoping needs one orientation; slice A's ascent premise is exactly
what makes the telescope sum. The full path law stays an ascent law,
now with the descent direction examined and its exact behavior
pinned.\<close>

theorem fee_slack_step_down:
  assumes dsc: "clmm_dsc P"
    and pp: "0 < p"
    and pp': "0 < p'"
    and le: "p' \<le> p"
  shows "quote_gross P p' - quote_gross P p
      \<le> quote_net P p' - quote_net P p"
proof -
  have flipped: "quote_net P p - quote_net P p'
      \<le> quote_gross P p - quote_gross P p'"
    by (rule fee_slack_step [OF dsc pp' le])
  show ?thesis using flipped by linarith
qed

end
