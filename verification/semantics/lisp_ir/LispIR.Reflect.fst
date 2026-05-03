(** Reflection-Based Soundness Proofs

    Case-split-then-normalize pattern for branching forms.
    
    Key insight: the F* normalizer cannot evaluate is_truthy on symbolic Bool.
    Solution: case-split on SOURCE-level condition (Z3 decides),
    then compile and execute with CONCRETE test values (normalizer handles it).
    
    Architecture:
    - Concrete lemmas (sound_if_true, sound_if_false, etc.) use assert_norm
      with concrete values — the normalizer evaluates the VM to completion.
    - Compositional lemmas (sound_if_concrete, sound_if_num, etc.) case-split
      on the source condition and dispatch to concrete lemmas — Z3 connects them.
    - Case-split abs lemmas (sound_abs_true, sound_abs_false) take source-level
      preconditions (x > 0, x <= 0) and compile only the taken branch.
    
    No admitted lemmas — all proofs use assert_norm.
*)
module LispIR.Reflect

open Lisp.Types
open Lisp.Values
open FStar.List.Tot
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val val_eq : lisp_val -> lisp_val -> Tot bool
let val_eq a b = match a, b with
  | Num x,    Num y    -> x = y
  | Bool x,   Bool y   -> x = y
  | Str x,    Str y    -> x = y
  | Nil,      Nil      -> true
  | _, _                -> false

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

val vm_top : steps:nat -> vm_state -> Tot (option lisp_val)
let vm_top n s = match eval_steps n s with
  | Ok s' -> (match s'.stack with top :: _ -> Some top | [] -> None)
  | _ -> None

val src_val : fuel:int -> expr:lisp_val -> env:env -> Tot (option lisp_val)
let src_val fuel expr env =
  let r = eval_expr (fuel + 1) expr env in
  match r with | Lisp.Source.Ok v -> Some v | Lisp.Source.Err _ -> None

// ============================================================
// Arithmetic (same pattern as Soundness.fst — straight-line code)
// ============================================================

val sound_add : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "+"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "+"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_add a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "+"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "+"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_sub : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "-"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "-"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_sub a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "-"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "-"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_mul : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "*"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "*"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_mul a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "*"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "*"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_div : a:int -> b:int -> Lemma
  (requires b <> 0)
  (ensures (match compile_lambda 100 [] (List [Sym "/"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "/"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_div a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "/"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "/"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

// ============================================================
// Comparisons
// ============================================================

val sound_gt : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym ">"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym ">"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_gt a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym ">"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym ">"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_lt : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "<"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "<"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_lt a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "<"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "<"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_eq : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "="; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "="; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_eq a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "="; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "="; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

// ============================================================
// Let binding and symbol lookup
// ============================================================

val sound_let : n:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_let n =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false)

val sound_sym : n:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 ["x"] (Sym "x") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num n]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num n)
      | None -> false))
let sound_sym n =
  assert_norm (
    match compile_lambda 100 ["x"] (Sym "x") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num n]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num n)
      | None -> false)

// ============================================================
// If-expression concrete proofs (normalizer resolves concrete Bool)
// ============================================================

val sound_if_true : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool true; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false))
let sound_if_true a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Bool true; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false)

val sound_if_false : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool false; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false))
let sound_if_false a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Bool false; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false)

// Compositional: case-split on test, dispatch to concrete proofs
val sound_if_concrete : test:bool -> a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool test; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num (if test then a else b))
      | None -> false))
let sound_if_concrete test a b =
  if test then sound_if_true a b
  else sound_if_false a b

// ============================================================
// If with truthy/falsy test values
// ============================================================

// Num is always truthy (wildcard match in is_truthy)
val sound_if_num : n:int -> a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Num n; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false))
let sound_if_num n a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Num n; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false)

// Nil is always falsy
val sound_if_nil : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Nil; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false))
let sound_if_nil a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Nil; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false)

// Str is always truthy (symbolic s — compiler emits PushStr s directly)
val sound_if_str : s:string -> a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Str s; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false))
let sound_if_str s a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Str s; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false)

// ============================================================
// Nested if: (if true (if true 42 99) 0)
// ============================================================

val sound_if_nested_tt : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool true; List [Sym "if"; Bool true; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false))
let sound_if_nested_tt a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Bool true; List [Sym "if"; Bool true; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false)

val sound_if_nested_tf : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool true; List [Sym "if"; Bool false; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false))
let sound_if_nested_tf a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Bool true; List [Sym "if"; Bool false; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num b)
      | None -> false)

val sound_if_nested_ff : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool false; List [Sym "if"; Bool true; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num 0)
      | None -> false))
let sound_if_nested_ff a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Bool false; List [Sym "if"; Bool true; Num a; Num b]; Num 0]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num 0)
      | None -> false)

// ============================================================
// Case-split: Abs function branches
// (if (> x 0) (+ x 1) (- 0 x))
//
// Cannot compile the full if with symbolic x — normalizer can't
// decide is_truthy on the comparison result Bool (x > 0).
// Z3 knows x > 0 from requires, but normalizer doesn't see SMT assumptions.
//
// Solution: case-split on SOURCE condition, compile only taken branch.
// ============================================================

// Branch 1: x > 0, result is x + 1
val sound_abs_true : x:int -> Lemma
  (requires x > 0)
  (ensures (match compile_lambda 100 ["x"] (List [Sym "+"; Sym "x"; Num 1]) with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num (x + 1))
      | None -> false))
let sound_abs_true x =
  assert_norm (
    match compile_lambda 100 ["x"] (List [Sym "+"; Sym "x"; Num 1]) with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num (x + 1))
      | None -> false)

// Branch 2: x <= 0, result is 0 - x
val sound_abs_false : x:int -> Lemma
  (requires x <= 0)
  (ensures (match compile_lambda 100 ["x"] (List [Sym "-"; Num 0; Sym "x"]) with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num (0 - x))
      | None -> false))
let sound_abs_false x =
  assert_norm (
    match compile_lambda 100 ["x"] (List [Sym "-"; Num 0; Sym "x"]) with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num (0 - x))
      | None -> false)

// ============================================================
// Case-split: Max function branches
// (if (>= x y) x y)
// ============================================================

// Branch 1: x >= y, result is x
val sound_max_ge : x:int -> y:int -> Lemma
  (requires x >= y)
  (ensures (match compile_lambda 100 ["x"; "y"] (Sym "x") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x; Num y]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num x)
      | None -> false))
let sound_max_ge x y =
  assert_norm (
    match compile_lambda 100 ["x"; "y"] (Sym "x") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x; Num y]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num x)
      | None -> false)

// Branch 2: x < y, result is y
val sound_max_lt : x:int -> y:int -> Lemma
  (requires x < y)
  (ensures (match compile_lambda 100 ["x"; "y"] (Sym "y") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x; Num y]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num y)
      | None -> false))
let sound_max_lt x y =
  assert_norm (
    match compile_lambda 100 ["x"; "y"] (Sym "y") with
    | None -> false
    | Some code ->
      match vm_top 100 { stack = []; slots = [Num x; Num y]; pc = 0; code = code; ok = true } with
      | Some vm_v -> val_eq vm_v (Num y)
      | None -> false)

// ============================================================
// Float: is_truthy (Float _) = true -> then-branch
// ============================================================
val sound_if_float : f:ffloat -> a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Float f; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false))
let sound_if_float f a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "if"; Float f; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num a)
      | None -> false)

// ============================================================
// sound_if_compile_time: unified if-soundness for all
// compile-time pushable lisp_val constructors.
//
// Proves: for any pushable test value, the VM takes the same
// branch as the source evaluator's is_truthy would dictate.
//
// Covers 5 of 12 constructors (the ones the compiler emits
// directly via push instructions):
//   Num, Float, Bool, Nil, Str
//
// NOT covered (require let-binding or runtime construction):
//   Sym, List, Pair, Dict, Lambda, BuiltinFn, Tagged
// ============================================================
val sound_if_compile_time : test:lisp_val -> a:int -> b:int -> Lemma
  (requires (match test with
    | Num _ | Float _ | Bool _ | Nil | Str _ -> true
    | _ -> false))
  (ensures (match compile_lambda 100 [] (List [Sym "if"; test; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code) with
      | Some vm_v -> val_eq vm_v (Num (if is_truthy test then a else b))
      | None -> false))
let sound_if_compile_time test a b =
  match test with
  | Num n   -> sound_if_num n a b
  | Float f -> sound_if_float f a b
  | Bool bv -> sound_if_concrete bv a b
  | Nil     -> sound_if_nil a b
  | Str s   -> sound_if_str s a b
  | _       -> ()
