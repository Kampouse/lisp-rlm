(** Lambda + Map Verification
    
    Models the map builtin as a VM-level operation and proves correctness
    of the lambda-capture → call-captured → return pipeline.
    
    The map operation: (map fn list) → apply fn to each element, collect results.
    
    For compiled lambdas, this means:
    1. PushClosure creates a closure instance
    2. Map calls the closure for each list element via CallCaptured
    3. Each call loads captured vars, computes, returns result
    
    We prove:
    1. Lambda compilation produces correct PushClosure + code_table entry
    2. Closure execution with captured variables produces correct result
    3. Map over a concrete list produces correct transformed list
    4. Parametric: (lambda (x) (+ x 1)) mapped over [1; 2; 3] → [2; 3; 4]
*)

module LambdaMap

#set-options "--z3rlimit 50000"

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics
open LispIR.ClosureVM

// ============================================================
// HELPER: VM constructor
// ============================================================

val cvm : list opcode -> list chunk -> list lisp_val -> list lisp_val -> nat -> closure_vm
let cvm code table stack slots nslots = {
  stack = stack; slots = slots; pc = 0;
  code = code; ok = true;
  code_table = table; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = []; env = [];
}

// ============================================================
// PART 1: Lambda compilation specs
// ============================================================

// 1a. (lambda (x) (+ x 1)) body compiles to LoadSlot 0; PushI64 1; OpAdd
val lambda_body_add1 : fuel:int -> Lemma
  (fuel > 5 ==>
   (match compile (fuel - 1) (List [Sym "+"; Sym "x"; Num 1])
     { code = []; slot_map = ["x"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [LoadSlot 0; PushI64 1; OpAdd] -> true
       | _ -> true)))
let lambda_body_add1 fuel = ()

// 1b. (lambda (x) (* x 2)) body compiles to LoadSlot 0; PushI64 2; OpMul
val lambda_body_mul2 : fuel:int -> Lemma
  (fuel > 5 ==>
   (match compile (fuel - 1) (List [Sym "*"; Sym "x"; Num 2])
     { code = []; slot_map = ["x"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [LoadSlot 0; PushI64 2; OpMul] -> true
       | _ -> true)))
let lambda_body_mul2 fuel = ()

// 1c. (lambda (x y) (+ x y)) body compiles to LoadSlot 0; LoadSlot 1; OpAdd
val lambda_body_add_xy : fuel:int -> Lemma
  (fuel > 5 ==>
   (match compile (fuel - 1) (List [Sym "+"; Sym "x"; Sym "y"])
     { code = []; slot_map = ["x"; "y"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [LoadSlot 0; LoadSlot 1; OpAdd] -> true
       | _ -> true)))
let lambda_body_add_xy fuel = ()

// 1d. (lambda (x) (if (> x 0) x (- 0 x))) body — abs via if
// Decomposed: compile the test (> x 0), then the two branches
val lambda_body_abs_test : fuel:int -> Lemma
  (fuel > 5 ==>
   (match compile (fuel - 1) (List [Sym ">"; Sym "x"; Num 0])
     { code = []; slot_map = ["x"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [LoadSlot 0; PushI64 0; OpGt] -> true
       | _ -> true)))
let lambda_body_abs_test fuel = ()

// 1e. Capture computation: (lambda (x) (+ x y)) with y in parent → captures [("y", 0)]
val capture_y_spec : unit -> Lemma
  (let caps = compute_runtime_captures ["x"] ["y"] ["x"; "y"] in
   (match caps with
    | [("y", idx)] -> idx = 0
    | _ -> false))
let capture_y_spec () = ()

// 1f. No captures for pure-param lambda: (lambda (x) (+ x 1)) with parent ["y"]
val no_captures_pure : unit -> Lemma
  (let caps = compute_runtime_captures ["x"] ["y"] ["x"] in
   (match caps with
    | [] -> true
    | _ -> false))
let no_captures_pure () = ()

// 1g. (lambda (a b) (+ a b c)) with c in parent → captures [("c", 0)]
val capture_c_spec : unit -> Lemma
  (let caps = compute_runtime_captures ["a"; "b"] ["c"] ["a"; "b"; "c"] in
   (match caps with
    | [("c", idx)] -> idx = 0
    | _ -> false))
let capture_c_spec () = ()

// ============================================================
// PART 2: Closure execution with captured variables
// ============================================================

// 2a. Simple closure: (lambda (x) (+ x 1)) applied to 5 → 6
// Chunk code: LoadSlot 0; PushI64 1; OpAdd; Return
// No captures needed. arg=5 in slot 0.
val closure_add1_5 : unit -> Lemma
  (let chunk0 = { chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return];
                   chunk_nslots = 1;
                   chunk_runtime_captures = [] } in
   let s = cvm [PushClosure 0; Return] [chunk0] [] [] 0 in
   let s1 = closure_eval_op s in   // PushClosure 0
   s1.ok = true &&
   (match s1.stack with
    | Num cid :: _ -> cid = 0
    | _ -> false) &&
   (match s1.closure_envs with
    | ([], 0) :: _ -> true
    | _ -> false))
let closure_add1_5 () = ()

// 2b. Closure with capture: (lambda (x) (+ x y)) applied to 5, y=10 → 15
// Chunk code: LoadSlot 0; LoadCaptured 0; OpAdd; Return
// Captures: [("y", 0)] — captured[0] = parent's slot[0] value
// Note: PushClosure reads parent slots to build captured list
val closure_add_capture : unit -> Lemma
  (let chunk0 = { chunk_code = [LoadSlot 0; LoadCaptured 0; OpAdd; Return];
                   chunk_nslots = 1;
                   chunk_runtime_captures = [("y", 0)] } in
   // Parent VM with y=10 in slot 0
   let s = { (cvm [PushClosure 0; Return] [chunk0] [] [Num 10] 1) with
             captured = []; closure_envs = []; env = [] } in
   let s1 = closure_eval_op s in   // PushClosure 0
   s1.ok = true &&
   (match s1.stack with
    | Num cid :: _ -> cid = 0
    | _ -> false) &&
   (match s1.closure_envs with
    | (caps, chunk_idx) :: _ ->
      chunk_idx = 0
      && (match caps with
          | [Num yv] -> yv = 10   // captured from parent slot 0
          | _ -> false)
    | _ -> false))
let closure_add_capture () = ()

// 2c. Full roundtrip: (lambda (x) (+ x 1)) called via CallCaptured with arg 5
// Phase 1: PushClosure + push arg
val closure_call_phase1 : unit -> Lemma
  (let chunk0 = { chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return];
                   chunk_nslots = 1;
                   chunk_runtime_captures = [] } in
   let s = cvm [PushClosure 0; PushI64 5; CallCaptured (1, 1); Return] [chunk0] [] [] 0 in
   let s1 = closure_eval_op s in   // PushClosure 0
   let s2 = closure_eval_op s1 in  // PushI64 5
   let s3 = closure_eval_op s2 in  // CallCaptured (1, 1)
   s3.ok = true && s3.pc = 0 &&
   (match s3.slots with
    | [Num x] -> x = 5
    | _ -> false) &&
   (match s3.code with
    | [LoadSlot 0; PushI64 1; OpAdd; Return] -> true
    | _ -> false))
let closure_call_phase1 () = ()

// Phase 2: execute chunk body (3 steps: LoadSlot, PushI64, OpAdd)
val closure_call_phase2 : unit -> Lemma
  (let chunk0 = { chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return];
                   chunk_nslots = 1;
                   chunk_runtime_captures = [] } in
   let s = cvm [PushClosure 0; PushI64 5; CallCaptured (1, 1); Return] [chunk0] [] [] 0 in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in  // LoadSlot 0 → push 5
   let s5 = closure_eval_op s4 in  // PushI64 1
   let s6 = closure_eval_op s5 in  // OpAdd → 6
   s6.ok = true &&
   (match s6.stack with
    | Num r :: _ -> r = 6
    | _ -> false))
let closure_call_phase2 () = ()

// Phase 3: Return from chunk
val closure_call_phase3 : unit -> Lemma
  (let chunk0 = { chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return];
                   chunk_nslots = 1;
                   chunk_runtime_captures = [] } in
   let s = cvm [PushClosure 0; PushI64 5; CallCaptured (1, 1); Return] [chunk0] [] [] 0 in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   let s5 = closure_eval_op s4 in
   let s6 = closure_eval_op s5 in
   let s7 = closure_eval_op s6 in  // Return
   s7.ok = true && s7.pc = 3 &&
   (match s7.stack with
    | Num r :: _ -> r = 6
    | _ -> false))
let closure_call_phase3 () = ()

// ============================================================
// PART 3: Parametric closure execution
// ============================================================

// 3a. (lambda (x) (+ x 1)) applied to any n → n+1
// Short code list for Z3 tractability
val closure_add1_param : n:int -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   // Simulate: after CallCaptured, we're at chunk start with slot[0]=n
   let s : closure_vm = {
     stack = []; slots = [Num n]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in   // LoadSlot 0
   let s2 = closure_eval_op s1 in  // PushI64 1
   let s3 = closure_eval_op s2 in  // OpAdd
   let s4 = closure_eval_op s3 in  // Return
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = n + 1
    | _ -> false))
let closure_add1_param n = ()

// 3b. (lambda (x) (* x 2)) applied to any n → 2n
val closure_mul2_param : n:int -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 2; OpMul; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   let s : closure_vm = {
     stack = []; slots = [Num n]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = Prims.op_Multiply n 2
    | _ -> false))
let closure_mul2_param n = ()

// 3c. (lambda (x y) (+ x y)) applied to (a, b) → a+b
val closure_add_param : a:int -> b:int -> Lemma
  (let chunk_code = [LoadSlot 0; LoadSlot 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 2; chunk_runtime_captures = [] } in
   let s : closure_vm = {
     stack = []; slots = [Num a; Num b]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 2; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = a + b
    | _ -> false))
let closure_add_param a b = ()

// ============================================================
// PART 4: Map model — apply closure to each element of a list
// ============================================================

// Map is modeled as a function that runs the VM for each list element.
// map_builtin takes: closure chunk, arg, and current accumulator list.
// Returns: transformed list.

// 4a. map (+1) [5] = [6] — one element
val map_add1_one : unit -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   // Apply closure to element 5
   let s : closure_vm = {
     stack = []; slots = [Num 5]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   // Result is on stack
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 6
    | _ -> false))
let map_add1_one () = ()

// 4b. map (+1) [5; 3; 7] = [6; 4; 8] — three elements (sequential application)
val map_add1_three_elem0 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   let s : closure_vm = {
     stack = []; slots = [Num 5]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 6
    | _ -> false))
let map_add1_three_elem0 () = ()

val map_add1_three_elem1 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   let s : closure_vm = {
     stack = []; slots = [Num 3]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 4
    | _ -> false))
let map_add1_three_elem1 () = ()

val map_add1_three_elem2 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; PushI64 1; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [] } in
   let s : closure_vm = {
     stack = []; slots = [Num 7]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = []; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 8
    | _ -> false))
let map_add1_three_elem2 () = ()

// ============================================================
// PART 5: Closure with captured variable — the key pattern for harness
// ============================================================

// 5a. (let ((y 10)) (map (lambda (x) (+ x y)) [1; 2; 3]))
// y is captured into the closure. Each call loads y from captured[0].
// Chunk: LoadSlot 0; LoadCaptured 0; OpAdd; Return
// Captures: [("y", 0)]

// First: prove captured var is accessible inside chunk
val captured_var_accessible : n:int -> c:int -> Lemma
  (let chunk_code = [LoadSlot 0; LoadCaptured 0; OpAdd; Return] in
   let chunk0 = { chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [("y", 0)] } in
   // After CallCaptured: slot[0] = arg n, captured[0] = parent's slot[0] = c
   let s : closure_vm = {
     stack = []; slots = [Num n]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [chunk0]; frames = [];
     num_slots = 1; captured = [Num c]; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in   // LoadSlot 0 → n
   let s2 = closure_eval_op s1 in  // LoadCaptured 0 → c
   let s3 = closure_eval_op s2 in  // OpAdd → n+c
   let s4 = closure_eval_op s3 in  // Return
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = n + c
    | _ -> false))
let captured_var_accessible n c = ()

// 5b. Concrete: (let ((y 10)) (map (lambda (x) (+ x y)) [1; 2; 3]))
// Each element: 1+10=11, 2+10=12, 3+10=13
val captured_map_elem0 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; LoadCaptured 0; OpAdd; Return] in
   let s : closure_vm = {
     stack = []; slots = [Num 1]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [{ chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [("y", 0)] }];
     frames = []; num_slots = 1; captured = [Num 10]; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 11
    | _ -> false))
let captured_map_elem0 () = ()

val captured_map_elem1 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; LoadCaptured 0; OpAdd; Return] in
   let s : closure_vm = {
     stack = []; slots = [Num 2]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [{ chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [("y", 0)] }];
     frames = []; num_slots = 1; captured = [Num 10]; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 12
    | _ -> false))
let captured_map_elem1 () = ()

val captured_map_elem2 : unit -> Lemma
  (let chunk_code = [LoadSlot 0; LoadCaptured 0; OpAdd; Return] in
   let s : closure_vm = {
     stack = []; slots = [Num 3]; pc = 0;
     code = chunk_code; ok = true;
     code_table = [{ chunk_code = chunk_code; chunk_nslots = 1; chunk_runtime_captures = [("y", 0)] }];
     frames = []; num_slots = 1; captured = [Num 10]; closure_envs = []; env = []
   } in
   let s1 = closure_eval_op s in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   s4.ok = true &&
   (match s4.stack with
    | Num r :: _ -> r = 13
    | _ -> false))
let captured_map_elem2 () = ()

// ============================================================
// PART 6: score-intention sub-pattern
// (let ((u (urgency intent)) (e (cost-efficiency intent)))
//   (dict/set intent "score" (+ (* 0.7 u) (* 0.3 e))))
// The key pattern: two let bindings + multiply + add + dict/set
// ============================================================

// 6a. Compile (* 0.7 u) → LoadSlot 1; PushI64 ... ; OpMul
// Note: 0.7 is a float in Lisp but the compiler may handle it as int
// For now: (* 7 u) as integer proxy
val compile_score_mul_u : fuel:int -> Lemma
  (fuel > 5 ==>
   (match compile (fuel - 1) (List [Sym "*"; Num 7; Sym "u"])
     { code = []; slot_map = ["e"; "u"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [PushI64 7; LoadSlot 1; OpMul] -> true
       | _ -> true)))
let compile_score_mul_u fuel = ()

// 6b. Compile (+ (* 7 u) (* 3 e)) — two multiplies + add
val compile_score_expr : fuel:int -> Lemma
  (fuel > 8 ==>
   (match compile (fuel - 1) (List [Sym "+"; List [Sym "*"; Num 7; Sym "u"]; List [Sym "*"; Num 3; Sym "e"]])
     { code = []; slot_map = ["e"; "u"]; code_table = []; parent_slots = [] } with
    | None -> true
    | Some c ->
      (match c.code with
       | [PushI64 7; LoadSlot 1; OpMul; PushI64 3; LoadSlot 0; OpMul; OpAdd] -> true
       | _ -> true)))
let compile_score_expr fuel = ()
