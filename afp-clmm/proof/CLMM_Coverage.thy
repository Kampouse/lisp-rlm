theory CLMM_Coverage
  imports CLMM_Bridge
begin

text \<open>\<paragraph>\<open>What this file is\<close> The machine-checkable coverage
statement: what is proven, what is assumed, what remains open. Gaps are
first-class citizens here -- named, typed, and where possible stated as
provable conditionals, so progress means converting an entry in this file
into a discharged theorem, and the hole count is countable rather than
narrated.\<close>

section \<open>A. The assumptions ledger\<close>

text \<open>Every theorem in the bridge rests on exactly two non-standard
assumptions, both visible in locale ‹bridge›:

  A1  feein_backed : the fee float never exceeds escrow custody
                     (‹feein e \<le> balance e›)
  A2  feein_drain  : the float only shrinks through escrow outflow

Everything else (escrow_ok, lp_ok, ledger_inv survival, per-hop
solvent runs) is unconditional over the modeled event space. The
diagnostic certificates, for any auditor:

    thm bridge_def                      (* the two axioms, verbatim *)
    print_dependencies  (in bridge) jrun_sound
    print_axioms jrun_sound             (* transitive assumption closure *)

Empirical-only bucket (provable nowhere, pinned by near-mock tests):
reject-purity of the wasm artifact, keeper punctuality, oracle-free
liveness of lazy settle legs.\<close>

section \<open>B. O1 -- rounding slack, clean case discharged\<close>

text \<open>‹conservation› admits one rounding unit per Grow wave. The
clean case -- no Grow ops in the run -- is exact, proven here. The
remaining O1 work is a rounding-policy choice, not a proof gap:
(a) tighten the ledger lemma to leak-free growth, or (b) pad ‹feein›
by one unit per wave (a padded solvent_run variant).\<close>

lemma conservation_growless:
  assumes ok: "run_ok gs s"
    and inv: "ledger_inv s"
    and aU: "a \<in> USERS"
    and ng: "filter is_Grow gs = []"
  shows "book (acm_run gs s) a \<le> book s a + (fees (acm_run gs s) - fees s)"
  using conservation[OF ok inv aU] by (simp add: ng)

section \<open>C. O2 -- realization template, consistency witness\<close>

text \<open>The bridge is parameterized by ‹feein›; realizing it over
concrete escrow ops means: pick ‹f›, prove the two axioms for ‹f›,
interpret, inherit every bridge theorem. The trivial float proves the
locale is consistent (satisfiable, no vacuous soundness) and exhibits
the template end-to-end. The REAL O2 discharge -- which leg captures
fees, ‹feein_drain› per concrete op -- replaces this witness.\<close>

definition trivial_feein :: "escrow \<Rightarrow> nat" where
  "trivial_feein e = 0"

text \<open>The two obligations any concrete realization must prove:\<close>
lemma trivial_backed: "trivial_feein e \<le> balance e"
  by (simp add: trivial_feein_def)

lemma trivial_drain:
  "apply_op eo e = Some e' \<Longrightarrow>
   trivial_feein e' + (outflow e' - outflow e) \<ge> trivial_feein e"
  by (simp add: trivial_feein_def)

text \<open>Locale consistency: the axioms are satisfiable, hence
‹jrun_sound› et al. are not vacuous.\<close>
interpretation cover: bridge trivial_feein
  by (unfold_locales; simp add: trivial_feein_def)

text \<open>O2 progress protocol: swap ‹trivial_feein› for the concrete
capture map, reprove A1/A2 per op, re-interpret. All bridge theorems
transfer mechanically; nothing downstream changes.\<close>

section \<open>D. O3 -- stranded bracketing, stated as the missing invariant\<close>

text \<open>‹Strand› parks credit after the escrow-side payout committed.
The joint law holds iff the stranded jar alone stays inside the float
throughout the bracket [payout .. StrandPay/timeout]. That clause is
the missing invariant; the two witness-side policies differ only in
WHICH step must restore backing.\<close>

context bridge
begin

definition strand_backed :: "jstate \<Rightarrow> bool" where
  "strand_backed s \<longleftrightarrow> (\<Sum>a\<in>USERS. stranded (snd s) a) \<le> feein (fst s)"

lemma strand_backed_initial:
  "(\<Sum>a\<in>USERS. stranded (snd s) a) = 0 \<Longrightarrow> strand_backed s"
  by (simp add: strand_backed_def)

text \<open>O3 target (open): ‹strand_backed› is preserved by every
accepted run -- under policy (a) the escrow restore leg supplies the
backing at Strand time; under policy (b) ‹feein› retained the units
since payout and the invariant is inherited from A2. Pick the side,
discharge, delete this paragraph.\<close>

end

section \<open>E. Coverage matrix\<close>

text \<open>
\begin{tabular}{lll}
Invariant & Scope & Status \\
\hline
escrow_ok / lp_ok survival & accepted events + runs & Proven (jstep_ok, jrun_ok) \\
ledger_inv survival & accepted events + runs & Proven (jstep_ok, jrun_ok) \\
claims \<le> float, per-hop & modeled runs & Proven (jrun_solvent, A1+A2) \\
conservation, Grow-free & runs without Grow & Proven (conservation_growless) \\
conservation, exact & runs with Grow & Open O1(a) -- rounding policy \\
solvency under Grow waves & padded variant & Open O1(b) -- rounding policy \\
fee float realization & concrete capture map & Open O2 -- template proven here \\
stranded jar backing & bracket [payout..StrandPay] & Open O3 -- invariant stated here \\
locale bridge consistency & any feein & Proven (cover interpretation) \\
reject purity (wasm) & artifact & Empirical-only (near-mock pins) \\
keeper punctuality & artifact + ops & Empirical-only \\
\end{tabular}\<close>

end
