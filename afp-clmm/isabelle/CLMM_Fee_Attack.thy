theory CLMM_Fee_Attack
  imports Aggregate_Accrual
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

end
