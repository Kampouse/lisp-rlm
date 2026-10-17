theory Aggregate_Accrual
  imports Aggregate_Book
begin

section \<open>Fixed-point division helpers (64-bit scale)\<close>

text \<open>Every truncating division in the fee layers goes through one of
these two helpers. The floor form is for owed/capacity quantities
(rounding down can only charge less); the ceil form is for
usage/debit quantities (fail-closed: rounding up can only charge
more, never less). The 2^64 denominator is baked in here so all
layers share one scale constant -- the same-literal-everywhere
discipline that keeps fixed-point contracts consistent.\<close>

definition div_floor64 :: "nat \<Rightarrow> nat" where
  "div_floor64 x = x div 2^64"

definition div_ceil64 :: "nat \<Rightarrow> nat" where
  "div_ceil64 x = (x + 2^64 - 1) div 2^64"

lemma div_floor64_eq:
  "div_floor64 x = x div 2^64"
  by (simp add: div_floor64_def)

lemma div_ceil64_eq:
  "div_ceil64 x = (x + 2^64 - 1) div 2^64"
  by (simp add: div_ceil64_def)

text \<open>Direction laws: floor never exceeds, ceil never falls short.\<close>

lemma div_floor64_le_mult:
  "div_floor64 x * 2^64 \<le> x"
proof -
  have dm: "x = div_floor64 x * 2^64 + x mod 2^64"
    by (simp add: div_floor64_def div_mult_mod_eq)
  have lt: "x mod 2^64 < 2^64" by simp
  show "div_floor64 x * 2^64 \<le> x" using dm lt by arith
qed

lemma div_ceil64_ge_mult:
  "x \<le> div_ceil64 x * 2^64"
proof -
  have dm: "(x + 2^64 - 1) div 2^64 * 2^64
      + (x + 2^64 - 1) mod 2^64 = x + 2^64 - 1"
    by (rule div_mult_mod_eq)
  have lt: "(x + 2^64 - 1) mod 2^64 < 2^64" by simp
  show "x \<le> div_ceil64 x * 2^64"
    unfolding div_ceil64_def using dm lt by arith
qed

section \<open>Fee-growth accrual within the charged aggregate book\<close>

text \<open>Gap-1 bridge (layer 1 meets layer 2): per-position OWED is the
accrue fold over an op sequence (accruef, checkpoint-threaded); per-cell
CHARGED is the aggregate book of per-op growth deltas (Aggregate_Book).
The bridge theorem composes them: owed stays within the charged book
(under a uniform liquidity cap) plus one truncation unit per op --
the machine-arithmetic form of the fee-growth question sketched in
CLMM_Dynamic.thy. Order-free: no sortedness assumption, matching the
Aggregate_Book fold cornerstone.

Accrual engine mirrored from escrow/CLMM_Saturation (commits 74d1dd72,
55ebb5bd, cfb98934) to keep the CLMM_Dynamic session decoupled from the
escrow STRICT session. Deterministic chains throughout.\<close>

section \<open>The accrual fold and the bridge theorem\<close>

text \<open>Accrual engine mirrored from escrow/CLMM_Saturation (commits
74d1dd72, 55ebb5bd, cfb98934) so the CLMM_Dynamic session stays
decoupled from the escrow STRICT session. Deterministic chains only.\<close>

section \<open>The bridge: owed stays within the charged book\<close>

text \<open>Positions in the charged book are the per-op growth deltas: op i
with delta Dg charges Dg * Lmax into the cell. The owed fold (accruef)
stays within that book plus one truncation unit per op.\<close>

fun accruef :: "(nat \<Rightarrow> nat) \<Rightarrow> nat \<Rightarrow> nat list \<Rightarrow> nat" where
  "accruef Lf _ [] = 0"
| "accruef Lf c (g # gs) = (g - c) * Lf g div 2^64 + accruef Lf g gs"

fun prodsum :: "(nat \<Rightarrow> nat) \<Rightarrow> nat \<Rightarrow> nat list \<Rightarrow> nat" where
  "prodsum Lf _ [] = 0"
| "prodsum Lf c (g # gs) = (g - c) * Lf g + prodsum Lf g gs"

fun deltas :: "nat \<Rightarrow> nat list \<Rightarrow> nat list" where
  "deltas _ [] = []"
| "deltas c (g # gs) = (g - c) # deltas g gs"

text \<open>Caps monotonicity: per-op liquidity caps lift to the fold.\<close>
lemma accruef_caps_mono:
  assumes "\<And>g. g \<in> set gs \<Longrightarrow> caps g \<le> Lmax"
  shows "accruef caps c gs \<le> accruef (\<lambda>_. Lmax) c gs"
  using assms
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have cap: "caps g \<le> Lmax" using Cons.prems by simp
  have tail: "\<And>h. h \<in> set gs \<Longrightarrow> caps h \<le> Lmax" using Cons.prems by simp
  have ih: "accruef caps g gs \<le> accruef (\<lambda>_. Lmax) g gs"
    using Cons.IH [OF tail] .
  have prod: "(g - c) * caps g \<le> (g - c) * Lmax"
    by (rule mult_le_mono2 [OF cap])
  have div: "(g - c) * caps g div 2^64 \<le> (g - c) * Lmax div 2^64"
    by (rule div_le_mono [OF prod])
  show ?case using div ih by simp
qed

text \<open>Split workhorse (mirrored from escrow CLMM_Saturation).\<close>
lemma div_add_slack:
  fixes X Y s :: nat
  assumes "1 \<le> s"
  shows "X div s + Y div s \<le> (X + Y) div s + 1"
proof (rule ccontr)
  have r1: "X div s * s + X mod s = X" by simp
  have r2: "Y div s * s + Y mod s = Y" by simp
  have r3: "(X + Y) div s * s + (X + Y) mod s = X + Y" by simp
  have m1: "X mod s < s" using assms by simp
  have m2: "Y mod s < s" using assms by simp
  have m3: "(X + Y) mod s < s" using assms by simp
  have eq: "X div s * s + Y div s * s + (X mod s + Y mod s)
      = (X + Y) div s * s + (X + Y) mod s"
    using r1 r2 r3 by arith
  have up: "X div s * s + Y div s * s + (X mod s + Y mod s)
      < (X + Y) div s * s + 2 * s"
    using eq m3 by arith
  have up2: "X div s * s + Y div s * s < (X + Y) div s * s + 2 * s"
    using up m1 m2 by arith
  assume "\<not> X div s + Y div s \<le> (X + Y) div s + 1"
  then have cc: "(X + Y) div s + 2 \<le> X div s + Y div s" by arith
  have step1: "((X + Y) div s + 2) * s \<le> (X div s + Y div s) * s"
    by (rule mult_le_mono [OF cc le_refl])
  have cc2: "(X + Y) div s * s + 2 * s \<le> X div s * s + Y div s * s"
    using step1 by (simp add: distrib_right)
  show False using cc2 up2 by arith
qed

text \<open>Sum form: the owed fold is at most the undivided product sum
divided once, plus one truncation unit per op.\<close>
lemma accruef_sum_bound:
  fixes Lf :: "nat \<Rightarrow> nat"
  shows "accruef Lf c gs \<le> prodsum Lf c gs div 2^64 + length gs"
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have unfold: "accruef Lf c (g # gs)
      = (g - c) * Lf g div 2^64 + accruef Lf g gs" by simp
  have ih: "accruef Lf g gs \<le> prodsum Lf g gs div 2^64 + length gs"
    using Cons.IH .
  have punfold: "prodsum Lf c (g # gs)
      = (g - c) * Lf g + prodsum Lf g gs" by simp
  have work: "(g - c) * Lf g div 2^64 + prodsum Lf g gs div 2^64
      \<le> ((g - c) * Lf g + prodsum Lf g gs) div 2^64 + 1"
  proof (rule div_add_slack)
    show "1 \<le> (2^64::nat)" by simp
  qed
  have lensimp: "length (g # gs) = Suc (length gs)" by simp
  have t2: "accruef Lf c (g # gs)
      \<le> (g - c) * Lf g div 2^64 + prodsum Lf g gs div 2^64 + length gs"
    using unfold ih by simp
  have t3: "accruef Lf c (g # gs)
      \<le> ((g - c) * Lf g + prodsum Lf g gs) div 2^64 + 1 + length gs"
    using t2 work by arith
  show ?case using t3 punfold lensimp by simp
qed

lemma sorted_hd_le_last:
  fixes xs :: "nat list"
  assumes "sorted (x # xs)"
  shows "x \<le> last (x # xs)"
  using assms
proof (induction xs arbitrary: x)
  case Nil
  then show ?case by simp
next
  case (Cons y ys)
  have xy: "x \<le> y" using Cons.prems by (auto simp add: sorted_simps)
  have srt: "sorted (y # ys)" using Cons.prems by (auto simp add: sorted_simps)
  have lasteq: "last (x # y # ys) = last (y # ys)" by (simp add: last_ConsR)
  show ?case using xy Cons.IH [OF srt] lasteq by arith
qed

text \<open>Uniform liquidity telescopes: product sum = last growth times L
(along a sorted checkpoint list).\<close>
lemma prodsum_const_sorted:
  assumes "sorted (c # gs)"
  shows "prodsum (\<lambda>_. L) c gs = (last (c # gs) - c) * L"
  using assms
proof (induction gs arbitrary: c)
  case Nil
  then show ?case by simp
next
  case (Cons g gs)
  have c1: "c \<le> g" using Cons.prems by (auto simp add: sorted_simps)
  have s2: "sorted (g # gs)" using Cons.prems by (auto simp add: sorted_simps)
  have ih: "prodsum (\<lambda>_. L) g gs = (last (g # gs) - g) * L"
    using Cons.IH [OF s2] .
  have last2: "g \<le> last (g # gs)" using s2 by (rule sorted_hd_le_last)
  have unfold: "prodsum (\<lambda>_. L) c (g # gs)
      = (g - c) * L + (last (g # gs) - g) * L"
    using ih by simp
  have lasteq: "last (c # g # gs) = last (g # gs)" by (simp add: last_ConsR)
  have dsum: "(g - c) + (last (g # gs) - g) = last (g # gs) - c"
    using c1 last2 by arith
  have regroup: "(g - c) * L + (last (g # gs) - g) * L
      = ((g - c) + (last (g # gs) - g)) * L"
    by (simp add: distrib_right [symmetric])
  have diffsplit: "(g - c) * L + (last (g # gs) - g) * L
      = (last (c # g # gs) - c) * L"
    using regroup dsum lasteq by simp
  show ?case using unfold diffsplit by simp
qed

text \<open>The charged book over deltas is exactly the product sum.\<close>
lemma book_deltas_eq_prodsum:
  shows "aggregate_list (\<lambda>d _. d * L) (deltas c gs) 0
       = prodsum (\<lambda>_. L) c gs"
  by (induction gs arbitrary: c)
    (simp_all add: aggregate_list_def)

text \<open>Main bridge theorem.\<close>
theorem accrue_within_charged_book:
  assumes "\<And>g. g \<in> set gs \<Longrightarrow> caps g \<le> Lmax"
  shows "accruef caps 0 gs
       \<le> aggregate_list (\<lambda>d _. d * Lmax) (deltas 0 gs) 0 div 2^64
          + length gs"
proof -
  have uni: "accruef caps 0 gs \<le> accruef (\<lambda>_. Lmax) 0 gs"
    using assms by (rule accruef_caps_mono)
  have sumform: "accruef (\<lambda>_. Lmax) 0 gs
      \<le> prodsum (\<lambda>_. Lmax) 0 gs div 2^64 + length gs"
    by (rule accruef_sum_bound)
  have book: "aggregate_list (\<lambda>d _. d * Lmax) (deltas 0 gs) 0
      = prodsum (\<lambda>_. Lmax) 0 gs"
    by (rule book_deltas_eq_prodsum)
  have t1: "accruef caps 0 gs
      \<le> prodsum (\<lambda>_. Lmax) 0 gs div 2^64 + length gs"
    by (rule le_trans [OF uni sumform])
  show ?thesis using t1 by (simp add: book)
qed

end
