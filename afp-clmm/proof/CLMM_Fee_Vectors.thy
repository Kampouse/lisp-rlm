theory CLMM_Fee_Vectors
  imports Aggregate_Accrual "HOL-Library.Code_Target_Numeral"
begin

section \<open>Machine-checked accrual test vectors (transcription bridge)\<close>

text \<open>The port twins (port/contracts/accrual) are regression-pinned to
scenarios; the guarantee lives in the theorems. This theory narrows the
gap from the theory side, without touching any port file: it pins the
accrual fold's inputs and outputs as PROVED equations (each vector is a
lemma with no sorry, build-gated), and exports the exact definitions
through Isabelle's own code generator, so downstream fixtures are
computed by the same equations the theorems quantify over -- not by a
hand transcription.

Two example liquidity functions: the constant-one cap (charges one
unit per full-scale growth step) and the identity cap (charges growth
times growth -- the square that reaches one scale unit exactly at
2^32). The pin lemmas come in two shapes: STRUCTURAL (the fold equals
its one-step descriptive expansion; true by the fold equations and the
cap definition alone, immune to arithmetic occlusion) and NUMERIC
(concrete values, simp-closed). The exported SML module plus the JSON
runner under vectors-gen/ produce the fixture file consumed by the
twins' smoke scenarios; the JSON is cross-checked against an
independent Python mirror of the fold before use.

Scope kept honest: this pins the FOLD the theorems bound. It does not
verify the twins' call-path wiring -- the transcription proof remains
open work in the port lane.\<close>

subsection \<open>Example caps\<close>

definition Lf_one :: "nat \<Rightarrow> nat" where
  "Lf_one _ = 1"

definition Lf_g :: "nat \<Rightarrow> nat" where
  "Lf_g g = g"

subsection \<open>Structural pins (fold = its descriptive expansion)\<close>

lemma vec_empty_struct:
  "accruef Lf_one 0 [] = 0"
  by simp

lemma vec_one_leg_struct:
  "accruef Lf_one 0 [10]
     = (10 - 0) * Lf_one 10 div 2^64 + accruef Lf_one 10 []"
  by simp

lemma vec_two_leg_struct:
  "accruef Lf_one 3 [10, 20]
     = (10 - 3) * Lf_one 10 div 2^64
       + ((20 - 10) * Lf_one 20 div 2^64 + accruef Lf_one 20 [])"
  by simp

lemma vec_sat_struct:
  "accruef Lf_one 20 [10]
     = (10 - 20) * Lf_one 10 div 2^64 + accruef Lf_one 10 []"
  by simp

subsection \<open>Numeric pins (concrete values, simp-closed)\<close>

lemma vec_scale_exact_one:
  "accruef Lf_one 0 [2^64] = 1"
  unfolding Lf_one_def by simp

lemma vec_square_exact_one:
  "accruef Lf_g 0 [2^32] = 1"
  unfolding Lf_g_def by simp

lemma vec_big_cap:
  "accruef (\<lambda>_. 2^64) 0 [3] = 3"
  by simp

lemma vec_saturation_zero:
  "accruef Lf_one 7 [5] = 0"
  unfolding Lf_one_def by simp

lemma vec_prodsum_square:
  "prodsum Lf_g 0 [2^32, 2^32] = 2^64"
  unfolding Lf_g_def by simp

subsection \<open>Code export: the fixture generator's arithmetic core\<close>

text \<open>The export below is Isabelle's own code generator emitting the
VERY definitions the pins quantify over. The JSON runner under
vectors-gen/ evaluates this module; the fixture file is committed
alongside and cross-checked by an independent Python mirror.\<close>

export_code accruef prodsum deltas Lf_one Lf_g integer_of_nat nat_of_integer in SML
  module_name Accruef
  file "/Users/asil/dev/lisp-rlm/afp-clmm/vectors-gen/Accruef.sml"

end
