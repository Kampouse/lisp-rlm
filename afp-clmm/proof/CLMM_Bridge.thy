theory CLMM_Bridge
  imports CLMM_Escrow User_Ledger
begin

text \<open>\<paragraph>\<open>Baseline\<close> Lives in the consolidated single-session
proof tree (live imports, no snapshots): escrow side and ledger side are
both verified at their current commits in this folder.\<close>

text \<open>
\<paragraph>\<open>What this theory is\<close> The bridge between the two disjoint machines
of the fee architecture: the escrow shell (state ‹escrow_ok› /
‹lp_ok›, ops ‹escrow_op›) and the per-user fee ledger (state
‹ledger_inv›, ops ‹op›). Both sides export ‹Deposit›
and ‹Withdraw›; the ledger side is aliased to ‹lop› here.

\<paragraph>\<open>What this theory claims\<close>
\begin{itemize}
  \item One receipt = one joint event; both machines accept or the whole
        event rejects (atomic, REJECT = zero state change on both).
  \item Local survival: ‹escrow_ok›, ‹lp_ok› and
        ‹ledger_inv› survive every accepted event
        (‹jstep_ok›) and every run (‹jrun_ok›)
        -- the CLMM_Route pattern, generalized to two different machines.
  \item Fee solvency: the ledger's live claims never exceed the escrow's
        fee float, at every accepted hop, given the per-hop backing
        checks (‹jrun_solvent›).
\end{itemize}

\<paragraph>\<open>What this theory does NOT claim\<close> The fee float oracle
‹feein› is abstract (locale ‹bridge›). Mapping it onto the
concrete escrow ops is deferred to the open obligations O1--O3 at the
bottom of this file. Everything stated here is proven or an explicit
locale assumption -- no sorry.\<close>

section \<open>Joint events\<close>

type_synonym lop = op

text \<open>Three event classes, none silent:
\begin{itemize}
  \item ‹Joint›: one receipt, both machines move. The escrow side
        must return @{term Some} AND the ledger side must fire (its state
        actually changes); a ledger no-op paired with an escrow move is
        ‹EscrOnly›, not ‹Joint›.
  \item ‹EscrOnly›: escrow moves, ledger sees \<open>\<epsilon>\<close>
        (PoolOpen/PoolClose, plain custody deposits, LP lifecycle).
  \item ‹LedgOnly›: ledger moves, escrow sees \<open>\<epsilon>\<close>
        (Grow waves, Claim, and the compensation legs WdFail / StrandPay;
        Strand itself is the B-leg trap -- see O3).
\end{itemize}\<close>

datatype jevent =
    Joint escrow_op lop
  | EscrOnly escrow_op
  | LedgOnly lop

type_synonym jstate = "escrow \<times> lstate"

text \<open>Ledger acceptance: ‹acm_step› is total and uses identity as
its no-op; ``fires'' means the state actually moved.\<close>
definition fires :: "lop \<Rightarrow> lstate \<Rightarrow> bool" where
  "fires g s \<longleftrightarrow> acm_step g s \<noteq> s"

fun jstep :: "jevent \<Rightarrow> jstate \<Rightarrow> jstate option" where
  "jstep (Joint eo lo) (e, l) =
     (case apply_op eo e of
        None \<Rightarrow> None
      | Some e' \<Rightarrow> if fires lo l
                    then Some (e', acm_step lo l)
                    else None)"
| "jstep (EscrOnly eo) (e, l) =
     (case apply_op eo e of
        None \<Rightarrow> None
      | Some e' \<Rightarrow> Some (e', l))"
| "jstep (LedgOnly lo) (e, l) =
     (if fires lo l then Some (e, acm_step lo l) else None)"

fun jrun :: "jevent list \<Rightarrow> jstate \<Rightarrow> jstate option" where
  "jrun [] s = Some s"
| "jrun (ev # evs) s =
     (case jstep ev s of None \<Rightarrow> None | Some s' \<Rightarrow> jrun evs s')"

section \<open>Ledger permissions, threaded per hop\<close>

text \<open>‹run_ok› is state-dependent (Deposit needs \<open>a \<in> USERS\<close>,
Withdraw/Strand need share-coverage AT THE STATE WHERE THEY FIRE), so the
bridge threads it hop by hop: an accepted event's ledger op must be
‹run_ok› at the current ledger state. Rejected events consume no
permission.\<close>

fun event_ledger_op :: "jevent \<Rightarrow> lop option" where
  "event_ledger_op (Joint _ lo) = Some lo"
| "event_ledger_op (LedgOnly lo) = Some lo"
| "event_ledger_op (EscrOnly _) = None"

fun jperm :: "jevent list \<Rightarrow> jstate \<Rightarrow> bool" where
  "jperm [] s = True"
| "jperm (ev # evs) s =
     (case jstep ev s of
        None \<Rightarrow> True
      | Some s' \<Rightarrow>
          (\<forall>lo. event_ledger_op ev = Some lo \<longrightarrow> run_ok [lo] (snd s))
          \<and> jperm evs s')"

section \<open>Local survival\<close>

theorem jstep_ok:
  assumes eo: "escrow_ok e" "lp_ok e"
    and lo: "ledger_inv l"
    and rok: "\<forall>lg. event_ledger_op ev = Some lg \<longrightarrow> run_ok [lg] l"
    and st: "jstep ev (e, l) = Some (e', l')"
  shows "escrow_ok e' \<and> lp_ok e' \<and> ledger_inv l'"
proof (cases ev)
  case (Joint eo' lo')
  from st Joint obtain ex
    where ap: "apply_op eo' e = Some ex"
      and eq: "e' = ex"
      and l': "l' = acm_step lo' l"
    by (auto split: option.splits if_splits)
  have ok1: "escrow_ok ex" using eo(1) ap by (rule one_op_ok)
  have ok2: "lp_ok ex" using eo(2) ap by (rule one_op_lp)
  have rk: "run_ok [lo'] l" using rok Joint by simp
  have inv: "ledger_inv (acm_step lo' l)"
    using step_preserves_inv[OF lo rk] .
  show ?thesis using ok1 ok2 inv eq l' by simp
next
  case (EscrOnly eo')
  from st EscrOnly obtain ex
    where ap: "apply_op eo' e = Some ex"
      and eq: "e' = ex"
      and l': "l' = l"
    by (auto split: option.splits)
  have ok1: "escrow_ok ex" using eo(1) ap by (rule one_op_ok)
  have ok2: "lp_ok ex" using eo(2) ap by (rule one_op_lp)
  show ?thesis using ok1 ok2 eq l' lo by simp
next
  case (LedgOnly lo')
  from st LedgOnly have l': "l' = acm_step lo' l" and e': "e' = e"
    by (auto split: if_splits)
  have rk: "run_ok [lo'] l" using rok LedgOnly by simp
  have inv: "ledger_inv (acm_step lo' l)"
    using step_preserves_inv[OF lo rk] .
  show ?thesis unfolding e' l' using eo(1) eo(2) inv by simp
qed

text \<open>Headline survival, route-style: every reachable joint state
carries all three invariants.\<close>
theorem jrun_ok:
  assumes eo: "escrow_ok e" "lp_ok e"
    and lo: "ledger_inv l"
    and p: "jperm js (e, l)"
    and r: "jrun js (e, l) = Some (e', l')"
  shows "escrow_ok e' \<and> lp_ok e' \<and> ledger_inv l'"
  using eo lo p r
proof (induction js arbitrary: e l e' l')
  case Nil
  from Nil.prems(5) have pr: "(e, l) = (e', l')" by (auto split: option.splits)
  from pr have eq: "e' = e" and eq2: "l' = l" by auto
  then show ?case using Nil.prems(1,2,3) by simp
next
  case (Cons ev evs)
  from Cons.prems(5) obtain e'' l''
    where step: "jstep ev (e, l) = Some (e'', l'')"
    and rest: "jrun evs (e'', l'') = Some (e', l')"
    by (auto split: option.splits)
  from Cons.prems(4) step have perm1:
    "\<forall>lg. event_ledger_op ev = Some lg \<longrightarrow> run_ok [lg] l"
    by simp
  have inv1: "escrow_ok e''" and inv2: "lp_ok e''"
    and inv3: "ledger_inv l''"
    using jstep_ok[OF Cons.prems(1) Cons.prems(2) Cons.prems(3) perm1 step] by simp_all
  from Cons.prems(4) step have pnext: "jperm evs (e'', l'')" by simp
  from Cons.IH[OF inv1 inv2 inv3 pnext rest] show ?case .
qed

section \<open>Fee solvency: claims against the fee float\<close>

text \<open>Live claims = everything the ledger still owes or has parked:
pending, balance, stranded. Realized payouts (‹paid›) have left the
claims jar -- they were delivered.\<close>
definition fee_claims :: "lstate \<Rightarrow> nat" where
  "fee_claims l = (\<Sum>a\<in>USERS. bal l a + pending l a + stranded l a)"

text \<open>The fee float oracle: units the ESCROW custodies that are earmarked
as the fee source. Backed by real custody, and it can only shrink through
escrow outflow (deliveries) -- never by bookkeeping alone.\<close>
locale bridge =
  fixes feein :: "escrow \<Rightarrow> nat"
  assumes feein_backed: "feein e \<le> balance e"
  assumes feein_drain:
    "apply_op eo e = Some e' \<Longrightarrow>
     feein e' + (outflow e' - outflow e) \<ge> feein e"
begin

definition fee_solvent :: "jstate \<Rightarrow> bool" where
  "fee_solvent s \<longleftrightarrow> fee_claims (snd s) \<le> feein (fst s)"

text \<open>Per-hop backing check, threaded like ‹jperm›: every ACCEPTED
hop must leave the joint state solvent. The substance -- WHICH concrete
ops satisfy this (Grow waves against float capture, deliveries shrinking
claims, stranded parked against the float) -- is exactly the O1--O3
discharge surface; this theory only needs the check to hold per hop.\<close>

fun solvent_run :: "jevent list \<Rightarrow> jstate \<Rightarrow> bool" where
  "solvent_run [] s = True"
| "solvent_run (ev # evs) s =
     (case jstep ev s of
        None \<Rightarrow> True
      | Some s' \<Rightarrow> fee_solvent s' \<and> solvent_run evs s')"

theorem jrun_solvent:
  assumes sol0: "fee_solvent (e, l)"
    and sol: "solvent_run js (e, l)"
    and r: "jrun js (e, l) = Some (e', l')"
  shows "fee_solvent (e', l')"
  using sol0 sol r
proof (induction js arbitrary: e l e' l')
  case Nil
  from Nil.prems(3) have pr: "(e, l) = (e', l')" by (auto split: option.splits)
  from pr have eq: "e' = e" and eq2: "l' = l" by auto
  then show ?case using Nil.prems(1) by simp
next
  case (Cons ev evs)
  from Cons.prems(3) obtain e'' l''
    where step: "jstep ev (e, l) = Some (e'', l'')"
    and rest: "jrun evs (e'', l'') = Some (e', l')"
    by (auto split: option.splits)
  from Cons.prems(2) step have sol1: "fee_solvent (e'', l'')" by simp
  from Cons.prems(2) step have sol2: "solvent_run evs (e'', l'')" by simp
  from Cons.IH[OF sol1 sol2 rest] show ?case .
qed

text \<open>Combined headline: solvent runs stay solvent AND survive.\<close>
theorem jrun_sound:
  assumes eo: "escrow_ok e" "lp_ok e" "ledger_inv l" "fee_solvent (e, l)"
    and p: "jperm js (e, l)"
    and sol: "solvent_run js (e, l)"
    and r: "jrun js (e, l) = Some (e', l')"
  shows "escrow_ok e' \<and> lp_ok e' \<and> ledger_inv l' \<and> fee_solvent (e', l')"
  using jrun_ok[OF eo(1) eo(2) eo(3) p r] jrun_solvent[OF eo(4) sol r] by simp

end

text \<open>
\<paragraph>\<open>Open obligations (the review surface)\<close>
\begin{description}
  \item[O1 (slack).] ‹User_Ledger.conservation› admits
        \<open>+ length (filter is_Grow gs)\<close> slack: each Grow wave can leak one
        unit of rounding into the book. Exact solvency
        (‹fee_solvent›) needs either (a) the ledger lemma tightened
        to leak-free growth, or (b) the float oracle padded by one unit per
        wave. Decide before discharging O2; this is a rounding-policy
        question, not a proof gap here.
  \item[O2 (realization).] Define ‹feein› over the concrete escrow
        ops: which leg captures the fee (SwapIn side? explicit earmarked
        Deposit?) and that ‹feein_drain› holds per op.
        Everything in ‹bridge› is parameterized by this; wrong
        choices surface here, not downstream.
  \item[O3 (stranded bracketing).] ‹Strand› parks a credit AFTER
        the escrow-side payout committed (B-leg traps post-commit), so the
        escrow has ALREADY outflowed those units while ‹stranded›
        still counts them as claims. Either (a) the escrow side gains the
        matching WdFail-style restore pairing (trapped value returns to
        the float), or (b) ‹feein› must be defined to retain the
        trapped amount until ‹StrandPay› lands. Pick the witness
        side explicitly -- this is THE clause that makes or breaks the
        joint law, and it must not be resolved by silence.
\end{description}\<close>

end
