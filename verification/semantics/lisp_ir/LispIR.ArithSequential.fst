module LispIR.ArithSequential
(** Sequential Composition for Arith VM — F* Formal Verification

    Proves that the arith VM composes sequentially:
      vm(fuel, c1@c2, s) = vm(fuel', c2, vm(fuel-fuel', c1, s))

    This is the key theorem that makes compiler correctness work.
    The fuel+tuple return model gives SMT constructor axioms to
    chain through induction on c1.

    VM model:
    - fuel: int, decrements on each opcode (matches run_checked in bytecode.rs)
    - Return: (stack, fuel_remaining) — fuel threaded honestly
    - Underflow: returns (stack, 0) — VM halts, composition holds

    Verified: 0 admits
    SMT-proved: APush, AOpAdd, AOpSub, AOpNeg
*)

open FStar.List.Tot
open FStar.Pervasives

// ============================================================
// FUEL-BASED VM (matches run_checked() in bytecode.rs)
// ============================================================

type arith_op =
  | APush of int
  | AOpAdd
  | AOpSub
  | AOpNeg

val arith_vm : fuel:int -> code:list arith_op -> stack:list int ->
  Tot (list int * int) (decreases fuel)
let rec arith_vm fuel code stack =
  if fuel <= 0 then (stack, fuel)
  else match code with
  | [] -> (stack, fuel)
  | APush n :: rest -> arith_vm (fuel - 1) rest (n :: stack)
  | AOpAdd :: rest ->
    (match stack with a :: b :: s' -> arith_vm (fuel - 1) rest ((b + a) :: s') | _ -> (stack, 0))
  | AOpSub :: rest ->
    (match stack with a :: b :: s' -> arith_vm (fuel - 1) rest ((b - a) :: s') | _ -> (stack, 0))
  | AOpNeg :: rest ->
    (match stack with a :: s' -> arith_vm (fuel - 1) rest ((0 - a) :: s') | _ -> (stack, 0))

// ============================================================
// HELPER: run c1 then c2 with threaded fuel
// ============================================================

val run_then : fuel:int -> c1:list arith_op -> s:list int ->
  c2:list arith_op -> Tot (list int * int)
let run_then fuel c1 s c2 =
  let (s1, f1) = arith_vm fuel c1 s in
  arith_vm f1 c2 s1

// ============================================================
// SEQUENTIAL COMPOSITION THEOREM
// vm(fuel, c1@c2, s) = run_then(fuel, c1, s, c2)
//
// Proof: structural induction on c1 (list, decreases c1).
// Each opcode peels one element from c1 and recurses on rest.
// The fuel-1 on both sides matches because each opcode
// consumes exactly 1 fuel in vm.
//
// SMT handles this via constructor axioms on the tuple return:
// - vm returns (s, f) — SMT gets Cons/Pair constructors
// - run_then destructures the tuple — SMT chains through
// ============================================================

val arith_vm_sequential : c1:list arith_op -> fuel:int -> c2:list arith_op -> s:list int ->
  Lemma (ensures arith_vm fuel (c1 @ c2) s = run_then fuel c1 s c2)
  (decreases c1)
let rec arith_vm_sequential c1 fuel c2 s =
  match c1 with
  | [] -> ()
  | APush n :: rest -> arith_vm_sequential rest (fuel - 1) c2 (n :: s)
  | AOpAdd :: rest ->
    (match s with a :: b :: s' -> arith_vm_sequential rest (fuel - 1) c2 ((b + a) :: s') | _ -> ())
  | AOpSub :: rest ->
    (match s with a :: b :: s' -> arith_vm_sequential rest (fuel - 1) c2 ((b - a) :: s') | _ -> ())
  | AOpNeg :: rest ->
    (match s with a :: s' -> arith_vm_sequential rest (fuel - 1) c2 ((0 - a) :: s') | _ -> ())
