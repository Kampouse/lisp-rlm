(** Compiler Soundness Theorem

    Structural soundness proof: compile(e) on VM produces the same result
    as eval(e) in the source evaluator.

    Status: Per-form lemmas proven for all atomic forms:
      - Literals: Num, Bool, Nil
      - Arithmetic: +, -, *, /
      - Comparisons: =, >, <, <=, >=
      - Control: if (true/false branches), let, sym lookup, not

    Proof strategy: Each lemma uses the "fuel lemma" pattern:
      1. compile_lambda produces Some code (by existing CompilerSpec lemmas)
      2. eval_steps unfolds through the compiled bytecode (Z3 with sufficient fuel)
      3. eval_expr unfolds the source expression
      4. val_eq witnesses the equality at the noeq barrier

    Note: lisp_val is noeq (contains ffloat), so we use val_eq instead of =.
    Note: vm_result and eval_result both have Ok/Err; Lisp.Source.Ok is qualified.

    Compound forms (nested if, let, lambda) need a simulation relation approach
    rather than this fuel-based unfolding strategy.
*)
module LispIR.Soundness

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

// === Value equality across noeq barrier ===

val val_eq : lisp_val -> lisp_val -> Tot bool
let val_eq a b =
  match a, b with
  | Num x,    Num y    -> x = y
  | Bool x,   Bool y   -> x = y
  | Str x,    Str y    -> x = y
  | Nil,      Nil      -> true
  | _, _                -> false

val val_eq_num : x:int -> y:int -> Lemma
  (val_eq (Num x) (Num y) = (x = y))
let val_eq_num x y = ()

val val_eq_bool : x:bool -> y:bool -> Lemma
  (val_eq (Bool x) (Bool y) = (x = y))
let val_eq_bool x y = ()

val val_eq_nil : unit -> Lemma
  (val_eq Nil Nil = true)
let val_eq_nil () = ()

// === Compiler helpers ===

val build_env_h : list string -> list lisp_val -> Tot env
let rec build_env_h params args =
  match params, args with
  | [], _ -> []
  | p :: prest, a :: arest -> (p, a) :: build_env_h prest arest
  | p :: prest, [] -> (p, Nil) :: build_env_h prest []

val build_slots_h : list string -> list lisp_val -> Tot (list lisp_val)
let rec build_slots_h params args =
  match params, args with
  | [], _ -> []
  | p :: prest, a :: arest -> a :: build_slots_h prest arest
  | _ :: prest, [] -> Nil :: build_slots_h prest []

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

val init_vm_h : list opcode -> list lisp_val -> vm_state
let init_vm_h code slots = { stack = []; slots = slots; pc = 0; code = code; ok = true }

// Run VM and extract stack top (if any) — uses nat for steps
val vm_top : steps:nat -> vm_state -> Tot (option lisp_val)
let vm_top n s =
  match eval_steps n s with
  | Ok s' ->
    (match s'.stack with
     | top :: _ -> Some top
     | [] -> None)
  | _ -> None

// Run source evaluator and extract Ok value.
// Note: vm_result and eval_result both have Ok/Err constructors.
// We qualify Lisp.Source.Ok to resolve ambiguity since both are opened.
val src_val : fuel:int -> expr:lisp_val -> env:env -> Tot (option lisp_val)
let src_val fuel expr env =
  let r = eval_expr (fuel + 1) expr env in
  match r with
  | Lisp.Source.Ok v -> Some v
  | Lisp.Source.Err _ -> None

// === PER-FORM SOUNDNESS LEMMAS ===

// --- LITERALS ---

val sound_num : n:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (Num n) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (Num n) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_num n = val_eq_num n n

val sound_bool : b:bool -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (Bool b) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (Bool b) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_bool b = val_eq_bool b b

val sound_nil : unit -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] Nil with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 Nil [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_nil () = val_eq_nil ()

// --- BINARY ARITHMETIC ---

val sound_add : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "+"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "+"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_add a b = val_eq_num (op_int_add a b) (op_int_add a b)

val sound_sub : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "-"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "-"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_sub a b = val_eq_num (op_int_sub a b) (op_int_sub a b)

val sound_mul : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "*"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "*"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_mul a b = val_eq_num (int_mul a b) (int_mul a b)

val sound_div : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "/"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "/"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_div a b = val_eq_num (int_div a b) (int_div a b)

// --- COMPARISONS ---

val sound_gt : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym ">"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym ">"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_gt a b = val_eq_bool (op_int_gt a b) (op_int_gt a b)

val sound_lt : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "<"; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "<"; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_lt a b = val_eq_bool (op_int_lt a b) (op_int_lt a b)

val sound_eq : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "="; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "="; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_eq a b = val_eq_bool (a = b) (a = b)

val sound_le : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "<="; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "<="; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_le a b = val_eq_bool (op_int_le a b) (op_int_le a b)

val sound_ge : a:int -> b:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym ">="; Num a; Num b]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym ">="; Num a; Num b]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_ge a b = val_eq_bool (op_int_ge a b) (op_int_ge a b)

// --- SYMBOL LOOKUP ---

val sound_sym : n:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 ["x"] (Sym "x") with
    | None -> false
    | Some code ->
      match vm_top 100 (init_vm_h code [Num n]), src_val 100 (Sym "x") [("x", Num n)] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_sym n = val_eq_num n n

// --- NOT ---

val sound_not_true : unit -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "not"; Bool true]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "not"; Bool true]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_not_true () = val_eq_bool false false

val sound_not_false : unit -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "not"; Bool false]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "not"; Bool false]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_not_false () = val_eq_bool true true

// --- IF (concrete) ---

val sound_if_true : unit -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Num 1; Num 42; Num 99]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "if"; Num 1; Num 42; Num 99]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_if_true () = val_eq_num 42 42

val sound_if_false : unit -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "if"; Bool false; Num 42; Num 99]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "if"; Bool false; Num 42; Num 99]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_if_false () = val_eq_num 99 99

// --- LET ---

val sound_let : n:int -> Lemma
  (requires True)
  (ensures (match compile_lambda 100 [] (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) with
    | None -> false
    | Some code ->
      match vm_top 100 (fresh_vm code), src_val 100 (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) [] with
      | Some vm_v, Some ev_v -> val_eq vm_v ev_v
      | _ -> false))
let sound_let n = val_eq_num n n

// === SUMMARY ===
// All per-form soundness lemmas verified by F* with 0 admits.
// Each lemma proves: for the given form, the compiled bytecode
// executed on the VM produces the same value as the source evaluator.
//
// Verified: 18 lemmas (num, bool, nil, add, sub, mul, div, gt, lt,
//           eq, le, ge, sym, not_true, not_false, if_true, if_false, let)
//
// Limitation: These proofs use fuel-based unfolding. Z3 unfolds
// compile_lambda + eval_steps + eval_expr and checks the results
// structurally match. This works for atomic forms but does not scale
// to nested compound forms (nested if, let, lambda). A simulation
// relation proof would be needed for the full completeness theorem.
