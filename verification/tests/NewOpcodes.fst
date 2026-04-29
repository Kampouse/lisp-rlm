(** New Opcode Step Proofs: PushBuiltin, CallDynamic, RecurDirect

    Proves end-to-end correctness of the 3 new opcodes:
    - PushBuiltin pushes BuiltinFn onto stack
    - CallDynamic dispatches via BuiltinFn or errors
    - RecurDirect pops values and resets PC to 0
    - Roundtrip: PushBuiltin + CallDynamic for multi-arg builtins

    Stack convention for CallDynamic:
      The function reference is on TOP of stack (head of list),
      with arguments below. pop_and_bind reverses arg order so
      the first-pushed arg ends up first in the args list.

    All proofs auto-proven with zero admits.
*)
module NewOpcodes

#set-options "--z3rlimit 5000"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

// Helper: build a fresh closure_vm with given code and nslots
val cvm0 : list opcode -> nat -> closure_vm
let cvm0 code nslots = {
  stack = []; slots = []; pc = 0;
  code = code; ok = true;
  code_table = []; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = [];
}

// ============================================================
// 1. PushBuiltin "list" pushes BuiltinFn "list" onto stack
// ============================================================

val step_push_builtin_list : unit -> Lemma
  (let vm = cvm0 [PushBuiltin "list"; Return] 0 in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | BuiltinFn name :: _ -> name = "list"
    | _ -> false))
let step_push_builtin_list () = ()

// PushBuiltin "car" pushes BuiltinFn "car"
val step_push_builtin_car : unit -> Lemma
  (let vm = cvm0 [PushBuiltin "car"; Return] 0 in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | BuiltinFn name :: _ -> name = "car"
    | _ -> false))
let step_push_builtin_car () = ()

// ============================================================
// 2. CallDynamic 1 with BuiltinFn "car" on top + arg below
//    Stack: [BuiltinFn "car"; List [Num a; Num b]]
//    func = BuiltinFn "car" (top), rest = [List [Num a; Num b]]
//    pop_and_bind 1 rest [] → args = [List [Num a; Num b]]
//    builtin_result "car" [List [Num a; Num b]] = Num a
// ============================================================

val step_call_dynamic_car : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [BuiltinFn "car"; List [Num a; Num b]];
    slots = []; pc = 0;
    code = [CallDynamic 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | Num r :: _ -> r = a
    | _ -> false))
let step_call_dynamic_car a b = ()

// CallDynamic 1 with BuiltinFn "cdr"
val step_call_dynamic_cdr : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [BuiltinFn "cdr"; List [Num a; Num b]];
    slots = []; pc = 0;
    code = [CallDynamic 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | List [Num r] :: _ -> r = b
    | _ -> false))
let step_call_dynamic_cdr a b = ()

// ============================================================
// 3. RecurDirect 2 pops 2 values and resets PC to 0
//    Stack: [Num a; Num b]
//    pop_and_bind 2 → vals = [Num b; Num a], stk = []
//    fill_slots 2 [Num b; Num a] = [Num b; Num a]
// ============================================================

val step_recur_direct_2 : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num a; Num b];
    slots = [Nil; Nil]; pc = 0;
    code = [RecurDirect 2; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 2; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 0 &&
   (match s1.stack with | [] -> true | _ -> false) &&
   (match s1.slots with
    | [Num x; Num y] -> x = b && y = a
    | _ -> false))
let step_recur_direct_2 a b = ()

// RecurDirect 1 pops 1 value and resets PC to 0
val step_recur_direct_1 : a:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num a];
    slots = [Nil]; pc = 0;
    code = [RecurDirect 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 1; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 0 &&
   (match s1.stack with | [] -> true | _ -> false) &&
   (match s1.slots with
    | [Num x] -> x = a
    | _ -> false))
let step_recur_direct_1 a = ()

// ============================================================
// 4. CallDynamic with non-builtin func sets ok=false
// ============================================================

// Num on top (not BuiltinFn) → ok=false
val step_call_dynamic_non_builtin_num : unit -> Lemma
  (let vm : closure_vm = {
    stack = [Num 42; Num 10];
    slots = []; pc = 0;
    code = [CallDynamic 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_call_dynamic_non_builtin_num () = ()

// Bool on top (not BuiltinFn) → ok=false
val step_call_dynamic_non_builtin_bool : unit -> Lemma
  (let vm : closure_vm = {
    stack = [Bool true; Num 1];
    slots = []; pc = 0;
    code = [CallDynamic 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_call_dynamic_non_builtin_bool () = ()

// Empty stack → ok=false
val step_call_dynamic_empty_stack : unit -> Lemma
  (let vm = cvm0 [CallDynamic 1; Return] 0 in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_call_dynamic_empty_stack () = ()

// ============================================================
// 5. Roundtrip: PushBuiltin + CallDynamic for a 2-arg builtin
//    Code: [PushI64 a; PushI64 b; PushBuiltin "list"; CallDynamic 2]
//    Step 1: PushI64 a → [Num a]
//    Step 2: PushI64 b → [Num b; Num a]
//    Step 3: PushBuiltin "list" → [BuiltinFn "list"; Num b; Num a]
//    Step 4: CallDynamic 2 → func=BuiltinFn "list", rest=[Num b; Num a]
//            pop_and_bind 2 [Num b; Num a] [] → remaining=[], args=[Num a; Num b]
//            builtin_result "list" [Num a; Num b] = List [Num a; Num b]
// ============================================================

val roundtrip_push_call_dynamic_list : a:int -> b:int -> Lemma
  (let vm = cvm0 [PushI64 a; PushI64 b; PushBuiltin "list"; CallDynamic 2; Return] 0 in
   let s1 = closure_eval_op vm in
   // s1: [Num a], pc=1
   let s2 = closure_eval_op s1 in
   // s2: [Num b; Num a], pc=2
   let s3 = closure_eval_op s2 in
   // s3: [BuiltinFn "list"; Num b; Num a], pc=3
   let s4 = closure_eval_op s3 in
   // s4: CallDynamic 2 → result = List [Num a; Num b], pc=4
   s4.ok = true && s4.pc = 4 &&
   (match s4.stack with
    | List items :: _ ->
      (match items with
       | Num x :: Num y :: _ -> x = a && y = b
       | _ -> false)
    | _ -> false))
let roundtrip_push_call_dynamic_list a b = ()

// Roundtrip: PushBuiltin "cons" with value + Nil
// Code: [PushI64 a; PushNil; PushBuiltin "cons"; CallDynamic 2]
// Stack after PushI64 a: [Num a]
// Stack after PushNil: [Nil; Num a]
// Stack after PushBuiltin "cons": [BuiltinFn "cons"; Nil; Num a]
// CallDynamic 2: func=BuiltinFn "cons", rest=[Nil; Num a]
//   pop_and_bind 2 [Nil; Num a] [] → remaining=[], args=[Num a; Nil]
//   builtin_result "cons" [Num a; Nil] = List [Num a]
val roundtrip_push_call_dynamic_cons : a:int -> Lemma
  (let vm = cvm0 [PushI64 a; PushNil; PushBuiltin "cons"; CallDynamic 2; Return] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true && s4.pc = 4 &&
   (match s4.stack with
    | List items :: _ ->
      (match items with
       | Num x :: _ -> x = a
       | _ -> false)
    | _ -> false))
let roundtrip_push_call_dynamic_cons a = ()
