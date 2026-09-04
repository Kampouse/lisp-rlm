(** RegressionGuards — negative/positive pins for previously-diverged model behavior.

    Each lemma encodes behavior that once diverged between the F* models and the
    Rust VM (see FSTAR_VS_RUST_DIFF.md + 2026-09-03 tightening). If a change
    reintroduces the old behavior, the corresponding lemma FAILS to verify.

    Pins:
    1. ClosureVM TypedBinOp I64 Div actually divides (was: returned the dividend).
    2. ClosureVM TypedBinOp I64 Div/Mod by zero sets ok=false (Rust hard-errors;
       the model must NOT silently produce a value).
    3. Semantics OpMod is float-aware (was: num_val truncation → wrong results).
    4. num_arith_strict rejects non-numeric operands with None (Rust: Err).
    5. num_arith_strict agrees with num_arith on numeric inputs.
*)
module RegressionGuards

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

// ---- Pin 1: TypedBinOp I64 Div divides ----
val typedbinop_div_i64_divides : a:int -> b:int -> Lemma
  (requires b <> 0)
  (ensures
   (let vm0 = { (LispIR.ClosureVM.make_closure_vm [TypedBinOp (Div, I64)] [] 0) with stack = [Num b; Num a] } in
   let s' = LispIR.ClosureVM.closure_eval_op vm0 in
   s'.ok = true &&
   (match s'.stack with
    | Num r :: _ -> r = a / b
    | _ -> false)))
let typedbinop_div_i64_divides a b = ()

// ---- Pin 2a: TypedBinOp I64 Div by zero → not ok ----
val typedbinop_div_i64_div0 : b:int -> Lemma
  (requires b == 0)
  (ensures
   (let s' = LispIR.ClosureVM.closure_eval_op
     ({ (LispIR.ClosureVM.make_closure_vm [TypedBinOp (Div, I64)] [] 0) with stack = [Num b; Num 7] }) in
   s'.ok == false))
let typedbinop_div_i64_div0 b = ()

// ---- Pin 2b: TypedBinOp I64 Mod by zero → not ok ----
val typedbinop_mod_i64_mod0 : unit -> Lemma
  (ensures
   (let s' = LispIR.ClosureVM.closure_eval_op
     ({ (LispIR.ClosureVM.make_closure_vm [TypedBinOp (Mod, I64)] [] 0) with stack = [Num 0; Num 7] }) in
   s'.ok == false))
let typedbinop_mod_i64_mod0 () = ()

// ---- Pin 3: Semantics OpMod float-aware (Float(2.5) rem Float(1.5) ≠ Num 0) ----
// Rust: Float path → Float result (f64 fmod). The old model truncated to Num 0.
// The divisor Float(ff_of_int 1) is provably nonzero via the ff_of_int_eq axiom.
let opmod_float_result : vm_result = eval_op OpMod
  { stack = [Float (ff_of_int 1); Float (ff_of_int 2)]; slots = []; pc = 0;
    code = [OpMod]; ok = true }
let opmod_float_ok : bool = (match opmod_float_result with
  | Ok s' -> (match s'.stack with
              | Float _ :: _ -> true
              | _ -> false)
  | _ -> false)

val opmod_float_path_returns_float : unit -> Lemma (opmod_float_ok = true)
let opmod_float_path_returns_float () =
  ff_of_int_eq 1 0;
  ff_of_int_eq 0 0;
  ()

// ---- Pin 4: num_arith_strict rejects non-numerics ----
val num_arith_strict_rejects_str : unit -> Lemma
  (num_arith_strict (Str "a") (Num 1) op_int_add ff_add == None)
let num_arith_strict_rejects_str () = ()

val num_arith_strict_rejects_nil : unit -> Lemma
  (num_arith_strict Nil (Num 1) op_int_add ff_add == None)
let num_arith_strict_rejects_nil () = ()

// ---- Pin 5: strict agrees with lenient on numerics ----
val strict_agrees_num : x:int -> y:int -> Lemma
  (match num_arith_strict (Num x) (Num y) op_int_add ff_add with
   | Some w -> w == num_arith (Num x) (Num y) op_int_add ff_add
   | None -> false)
let strict_agrees_num x y = ()
