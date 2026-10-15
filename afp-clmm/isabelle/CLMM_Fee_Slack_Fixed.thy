theory CLMM_Fee_Slack_Fixed
  imports Aggregate_Accrual
begin

(* Provenance: probe ~/dev/kb96, 2026-10-12, EXIT:0 (transcript
   ~/dev/kb96.log.r6; core lemma + 64-pinned corollary proved on Main in 5s).
   Promoted into session CLMM_Dynamic per the kb90 recipe: theory renamed,
   import flipped to unqualified in-session Aggregate_Accrual.
   Deterministic tactics only: div_mult_mod_eq (Rings.thy:1739) and
   mod_less_divisor (Euclidean_Rings.thy:1071) atoms + arith/linarith;
   one bounded 2-case split. No subtraction in the derivation; the
   truncated-slack goal takes a single C*den <= F*den case test.
   Direction (fail-closed): the fixed-point (floor capacity / ceil usage)
   slack UNDERSTATES the real slack. The transposed form
   (C*den - F*den <= capacity - usage) is FALSE -- Python counterexample
   den=785866, usage=120558013, capacity=120569449 (fixed slack = den,
   real slack = 11436).

   Review verified 2026-10-13: counterexample recomputed exactly
   (C*den-F*den = 785866 > 11436 = capacity-usage: transposed False,
   proven direction True); compiles green in CLMM_Dynamic; zero
   downstream users yet (parked theorem). Instantiation order checked:
   fixes sequence is den F C with capacity usage fixed inside the
   subscripts -- the [of "2^64" capacity usage ...] application matches. *)

lemma slack_fixed_le_real:
  fixes capacity usage den F C :: nat
  assumes denpos: "den > 0"
      and le: "usage \<le> capacity"
      and Fdef: "F = capacity div den"
      and Cdef: "C = (usage + den - 1) div den"
    shows "F * den - C * den \<le> capacity - usage"
proof -
  have qC: "F * den + capacity mod den = capacity"
    using Fdef div_mult_mod_eq[of capacity den] by simp
  have mC: "capacity mod den < den" using denpos by simp
  have Fle: "F * den \<le> capacity" using qC mC by arith
  have qU: "C * den + (usage + den - 1) mod den = usage + den - 1"
    using Cdef div_mult_mod_eq[of "usage + den - 1" den] by simp
  have mU: "(usage + den - 1) mod den < den" using denpos by simp
  have rU: "usage \<le> C * den" using qU mU by arith
  have H: "F * den + usage \<le> capacity + C * den" using Fle rU by linarith
  show ?thesis
  proof (cases "C * den \<le> F * den")
    case True
    show ?thesis using H True le by arith
  next
    case False
    have F0: "F * den - C * den = 0" using False by arith
    show ?thesis by (simp add: F0)
  qed
qed

(* Compile-time twin: div_floor64/div_ceil64 (Aggregate_Accrual, den = 2^64)
   inherit the bound. Folding the div-forms through div_floor64_eq /
   div_ceil64_eq yields exactly the pinned-atom instance of the core. *)
lemma slack_fixed_le_real64:
  fixes capacity usage :: nat
  assumes le: "usage \<le> capacity"
    shows "div_floor64 capacity * 2^64 - div_ceil64 usage * 2^64
           \<le> capacity - usage"
  using slack_fixed_le_real[of "2^64" capacity usage
        "div_floor64 capacity" "div_ceil64 usage"] le
  by (simp add: div_floor64_eq div_ceil64_eq)

end
