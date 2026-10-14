theory Aggregate_Book
  imports Main
begin

section \<open>Order-free aggregate books (fold cornerstone for CLMM_Dynamic)\<close>

text \<open>Jean's CLMM_Dynamic program (layer 1) needs: per-cell liquidity of an
AGGREGATE is a fold of position entries, and the fold is order-free.
This theory delivers that cornerstone polymorphically over positions
'p and cells 'c, using only plain HOL.

Bridge to the AFP entry (Concentrated_Liquidity_Market_Making_Operations):
`pool_join P1 P2 = (grd P1, \<lambda>i. lq P1 i + lq P2 i, \<lambda>i. pool_fee_join P1 P2 i)`
composes pools by POINTWISE ADDITION of per-cell books. An aggregate book
(`aggregate` below) is exactly that pointwise sum over a set of
positions, and `aggregate_pair` makes the mapping literal: the aggregate
of two positions is the pointwise add `bk p c + bk q c` -- the gross
additivity precedent of the ported fee-join (QBK = QA + QC, integer add).

Order-freedom comes in the shape folds actually consume: list folds
(`aggregate_list`) are invariant under appending batches in either
order (`aggregate_list_append`, `aggregate_list_rev`) and under
swapping adjacent entries (`aggregate_list_adjacent_swap`; adjacent
transpositions generate the full symmetric group, so arbitrary batch
reorderings preserve the aggregate). Set-level folds commute with
bijective relabeling of positions (`aggregate_reindex`).

Proof-style note: every closing tactic below is a bounded rewrite
(explicit rule application, or \<^verbatim>\<open>simp\<close> over arithmetic-only goals with no
chained fact sets and no function-update terms -- the species that made an
unbounded simp oscillate forever in CLMM_Escrow's sum_upd_gen_member).\<close>

type_synonym ('p, 'c) book = "'p \<Rightarrow> 'c \<Rightarrow> nat"

definition support :: "('p, 'c) book \<Rightarrow> 'p \<Rightarrow> 'c set" where
  "support bk p = {c. bk p c > 0}"

definition aggregate :: "('p, 'c) book \<Rightarrow> 'p set \<Rightarrow> 'c \<Rightarrow> nat" where
  "aggregate bk ps = (\<lambda>c. sum (\<lambda>p. bk p c) ps)"

definition aggregate_list :: "('p, 'c) book \<Rightarrow> 'p list \<Rightarrow> 'c \<Rightarrow> nat" where
  "aggregate_list bk xs = (\<lambda>c. sum_list (map (\<lambda>p. bk p c) xs))"

lemma aggregate_pair:
  assumes "p \<noteq> q"
  shows "aggregate bk {p, q} = (\<lambda>c. bk p c + bk q c)"
  using assms unfolding aggregate_def by simp

lemma aggregate_empty[simp]:
  shows "aggregate bk {} = (\<lambda>_. 0)"
  unfolding aggregate_def by simp

lemma aggregate_singleton[simp]:
  shows "aggregate bk {p} = bk p"
  unfolding aggregate_def by simp

lemma aggregate_insert:
  assumes "finite ps" "p \<notin> ps"
  shows "aggregate bk (insert p ps) = (\<lambda>c. bk p c + aggregate bk ps c)"
proof -
  have "\<And>c. sum (\<lambda>q. bk q c) (insert p ps)
           = bk p c + sum (\<lambda>q. bk q c) ps"
    by (rule sum.insert) (use assms in simp_all)
  then show ?thesis unfolding aggregate_def by simp
qed

lemma aggregate_disjoint_union:
  assumes "finite A" "finite B" "A \<inter> B = {}"
  shows "aggregate bk (A \<union> B)
       = (\<lambda>c. aggregate bk A c + aggregate bk B c)"
proof -
  have "\<And>c. sum (\<lambda>p. bk p c) (A \<union> B)
           = sum (\<lambda>p. bk p c) A + sum (\<lambda>p. bk p c) B"
    by (rule sum.union_disjoint) (use assms in simp_all)
  then show ?thesis unfolding aggregate_def by simp
qed

lemma aggregate_positive:
  assumes "aggregate bk ps c > 0"
  shows "\<exists>p\<in>ps. bk p c > 0"
proof -
  from assms have nz: "sum (\<lambda>p. bk p c) ps \<noteq> 0"
    unfolding aggregate_def by simp
  have zero: "sum (\<lambda>p. bk p c) ps = 0"
    if no: "\<And>p. p \<in> ps \<Longrightarrow> bk p c = 0"
    using that by (cases "finite ps") (auto simp: sum.neutral sum.infinite)
  show ?thesis
  proof (rule ccontr)
    assume "\<not> (\<exists>p\<in>ps. 0 < bk p c)"
    then have all0: "\<And>p. p \<in> ps \<Longrightarrow> bk p c = 0" by auto
    from zero[OF all0] nz show False by simp
  qed
qed

lemma support_aggregate_finite:
  assumes "finite ps" "\<And>p. p \<in> ps \<Longrightarrow> finite (support bk p)"
  shows "finite {c. aggregate bk ps c > 0}"
proof -
  have sub: "{c. aggregate bk ps c > 0}
           \<subseteq> (\<Union>p\<in>ps. support bk p)"
    using aggregate_positive[of bk ps] unfolding support_def
    by (blast intro: assms(2))
  show ?thesis
    by (rule finite_subset[OF sub]) (use assms in auto)
qed

lemma aggregate_reindex:
  assumes "bij_betw \<pi> ps qs"
  shows "aggregate (\<lambda>p. bk (\<pi> p)) ps = aggregate bk qs"
proof -
  have "\<And>c. sum (\<lambda>p. bk (\<pi> p) c) ps = sum (\<lambda>p. bk p c) qs"
    by (rule sum.reindex_bij_betw[OF assms])
  then show ?thesis unfolding aggregate_def ..
qed

lemma aggregate_list_append:
  shows "aggregate_list bk (xs @ ys)
       = (\<lambda>c. aggregate_list bk xs c + aggregate_list bk ys c)"
  unfolding aggregate_list_def by (simp add: sum_list_append)

lemma aggregate_list_singleton[simp]:
  shows "aggregate_list bk [p] = bk p"
  unfolding aggregate_list_def by simp

lemma aggregate_list_Cons:
  shows "aggregate_list bk (p # ys) = (\<lambda>c. aggregate_list bk ys c + bk p c)"
  unfolding aggregate_list_def by (simp add: add.commute)

lemma aggregate_list_rev:
  shows "aggregate_list bk (rev xs) = aggregate_list bk xs"
proof (induction xs)
  case Nil
  show ?case unfolding aggregate_list_def by simp
next
  case (Cons a xs)
  have step: "\<And>ys. aggregate_list bk (a # ys)
            = (\<lambda>c. aggregate_list bk ys c + bk a c)"
    unfolding aggregate_list_def by (simp add: add.commute)
  have lhs: "aggregate_list bk (rev (a # xs))
           = (\<lambda>c. aggregate_list bk xs c + bk a c)"
    by (simp add: aggregate_list_append Cons.IH add.commute)
  have rhs: "aggregate_list bk (a # xs)
           = (\<lambda>c. aggregate_list bk xs c + bk a c)"
    by (rule step[of xs])
  show ?case by (simp only: lhs rhs)
qed

lemma aggregate_list_swap2:
  shows "aggregate_list bk (p # q # ys) = aggregate_list bk (q # p # ys)"
  by (rule ext) (simp add: aggregate_list_def add.commute)

lemma aggregate_list_adjacent_swap:
  shows "aggregate_list bk (xs @ p # q # ys)
       = aggregate_list bk (xs @ q # p # ys)"
proof (induction xs)
  case Nil
  show ?case by (simp add: aggregate_list_swap2)
next
  case (Cons r xs)
  have exL: "aggregate_list bk (r # (xs @ p # q # ys))
           = (\<lambda>c. bk r c + aggregate_list bk (xs @ p # q # ys) c)"
    unfolding aggregate_list_def by (simp add: add.commute)
  have exR: "aggregate_list bk (r # (xs @ q # p # ys))
           = (\<lambda>c. bk r c + aggregate_list bk (xs @ q # p # ys) c)"
    unfolding aggregate_list_def by (simp add: add.commute)
  show ?case by (simp only: append_Cons exL exR Cons.IH)
qed

end
