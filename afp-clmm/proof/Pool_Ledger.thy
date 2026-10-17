theory Pool_Ledger
  imports Main
begin

section \<open>The pool token ledger (principal walk)\<close>

text \<open>Companion to User_Ledger (the FEE ledger). This is the TOKEN
side: where the pool's actual balance lives and who it is owed to.

Scope, stated honestly: AGGREGATE principal. The pool owes shareholders
collectively; the per-user split (redemption price, skew) is the
redemption walk, still scoped out. Likewise the COUPLED proof (one op
sequence driving both ledgers, with setaside >= fee-book) is future
work -- here the two ledgers are proven sound independently, on
matching op shapes.

Buckets: principal (labeled to shareholders), setaside (labeled to fee
credits), unalloc (collected but not yet labeled), burnt (already paid
out to users). psum = principal + setaside + unalloc + burnt is "every
token the pool ever received, accounted somewhere".\<close>

record pstate =
  principal :: nat              \<comment> \<open>tokens backing outstanding shares\<close>
  setaside :: nat               \<comment> \<open>tokens backing fee credits\<close>
  unalloc :: nat                \<comment> \<open>collected fees not yet allocated\<close>
  burnt :: nat                  \<comment> \<open>total tokens paid out\<close>

definition psum :: "pstate \<Rightarrow> nat" where
  "psum s = principal s + setaside s + unalloc s + burnt s"

datatype pop =
    PDep nat                      \<comment> \<open>m tokens deposited (mint m shares)\<close>
  | PSweep nat                    \<comment> \<open>m collected fees arrive in unalloc\<close>
  | PGrow nat                     \<comment> \<open>label f collected fees as fee credit\<close>
  | PWd nat nat                   \<comment> \<open>pay b principal + f fees out to a user\<close>

fun pdep :: "pop \<Rightarrow> nat" where
  "pdep (PDep m) = m"
| "pdep (PSweep m) = m"
| "pdep (PGrow f) = 0"
| "pdep (PWd b f) = 0"

section \<open>Transitions (token legs of the machine, same op order)\<close>

text \<open>Provenance is a GUARD: Grow can only label fees that were actually
collected (f \<le> unalloc); payouts can only leave their labeled bucket.
Violations are sound no-ops, exactly like WdFail / over-withdrawal in
the fee ledger.\<close>

fun pstep :: "pop \<Rightarrow> pstate \<Rightarrow> pstate" where
  \<comment> \<open>deposit: m tokens arrive, labeled principal (mint m shares in fee ledger)\<close>
  "pstep (PDep m) s =
    s\<lparr>principal := principal s + m\<rparr>"
  \<comment> \<open>sweep: m collected fees arrive in unalloc (token moves in)\<close>
| "pstep (PSweep m) s =
    s\<lparr>unalloc := unalloc s + m\<rparr>"
  \<comment> \<open>grow wave: label f of the collected float as fee credit (no token moves)\<close>
| "pstep (PGrow f) s =
    (if f \<le> unalloc s
     then s\<lparr>unalloc := unalloc s - f, setaside := setaside s + f\<rparr>
     else s)"
  \<comment> \<open>withdraw success: b principal + f fees ride out together\<close>
| "pstep (PWd b f) s =
    (if b \<le> principal s \<and> f \<le> setaside s
     then s\<lparr>principal := principal s - b,
           setaside := setaside s - f,
           burnt := burnt s + b + f\<rparr>
     else s)"

fun prun :: "pop list \<Rightarrow> pstate \<Rightarrow> pstate" where
  "prun [] s = s"
| "prun (g # gs) s = prun gs (pstep g s)"

definition pded :: "pop list \<Rightarrow> nat" where
  "pded gs = sum_list (map pdep gs)"

definition \<sigma>p0 :: pstate where
  "\<sigma>p0 = \<lparr>principal = 0, setaside = 0, unalloc = 0, burnt = 0\<rparr>"

section \<open>Per-op accounting (manual proofs, no sledgehammer)\<close>

lemma psum_PDep:
  "psum (pstep (PDep m) s) = psum s + pdep (PDep m)"
  by (simp add: psum_def)

lemma psum_PGrow:
  "psum (pstep (PGrow f) s) = psum s + pdep (PGrow f)"
proof (cases "f \<le> unalloc s")
  case True
  have u: "unalloc s - f + f = unalloc s"
    using True by linarith
  then show ?thesis by (simp add: psum_def u ac_simps)
next
  case False
  then show ?thesis by (simp add: psum_def)
qed

lemma psum_PSweep:
  "psum (pstep (PSweep m) s) = psum s + pdep (PSweep m)"
  by (simp add: psum_def)
lemma psum_PWd:
  "psum (pstep (PWd b f) s) = psum s + pdep (PWd b f)"
proof (cases "b \<le> principal s \<and> f \<le> setaside s")
  case True
  then have g1: "b \<le> principal s" and g2: "f \<le> setaside s" by simp_all
  have p1: "principal s - b + b = principal s"
    using g1 by linarith
  have p2: "setaside s - f + f = setaside s"
    using g2 by linarith
  then show ?thesis by (simp add: psum_def p1 p2 ac_simps)
next
  case False
  then show ?thesis by (simp add: psum_def)
qed

lemma psum_step:
  "psum (pstep g s) = psum s + pdep g"
proof (cases g)
  case (PDep m)
  then show ?thesis using psum_PDep by simp
next
  case (PSweep m)
  then show ?thesis using psum_PSweep by simp
next
  case (PGrow f)
  then show ?thesis using psum_PGrow by simp
next
  case (PWd b f)
  then show ?thesis using psum_PWd by simp
qed

section \<open>Conservation: every token accounted, always\<close>

lemma pool_conservation:
  "psum (prun gs s) = psum s + pded gs"
proof (induction gs arbitrary: s)
  case Nil
  then show ?case by (simp add: pded_def)
next
  case (Cons g gs)
  have "psum (prun (g # gs) s) = psum (prun gs (pstep g s))" by simp
  also have "... = psum (pstep g s) + pded gs" using Cons.IH by simp
  also have "... = psum s + pdep g + pded gs" using psum_step by simp
  also have "... = psum s + pded (g # gs)" by (simp add: pded_def)
  finally show ?case .
qed

text \<open>The pool-side no-free-money: tokens paid out can never exceed
tokens deposited, and the residual balance is exactly the three labeled
buckets.\<close>

corollary pool_from_scratch:
  "psum (prun gs \<sigma>p0) = pded gs"
proof -
  have "psum (prun gs \<sigma>p0) = psum \<sigma>p0 + pded gs"
    by (rule pool_conservation)
  then show ?thesis by (simp add: \<sigma>p0_def psum_def)
qed

corollary pool_no_vaporize:
  "burnt (prun gs \<sigma>p0) \<le> pded gs"
proof -
  have "psum (prun gs \<sigma>p0) = pded gs" by (rule pool_from_scratch)
  then have "principal (prun gs \<sigma>p0) + setaside (prun gs \<sigma>p0)
             + unalloc (prun gs \<sigma>p0) + burnt (prun gs \<sigma>p0) = pded gs"
    by (simp add: psum_def)
  then show ?thesis by simp
qed

end
