(** VM correctness for known if/let/not code sequences *)
module VmIfTest

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// VM on [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]
// truthy test → takes then-branch → result = 42
val vm_if_true : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
let vm_if_true () = ()

// falsy test → takes else-branch → result = 99
val vm_if_false : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool false; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 99
     | _ -> false)
   | _ -> false)
let vm_if_false () = ()

// nil test → takes else-branch → result = 99
val vm_if_nil : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 99
     | _ -> false)
   | _ -> false)
let vm_if_nil () = ()

// let: [PushI64 n; StoreSlot 0; LoadSlot 0; Return]
val vm_let : n:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 n; StoreSlot 0; LoadSlot 0; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = n
     | _ -> false)
   | _ -> false)
let vm_let n = ()

// not true: [PushBool true; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]
// truthy test → doesn't take else branch → result = false
val vm_not_true : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool true; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Bool r :: _ -> r = false
     | _ -> false)
   | _ -> false)
let vm_not_true () = ()

// not false: takes else branch → true
val vm_not_false : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool false; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Bool r :: _ -> r = true
     | _ -> false)
   | _ -> false)
let vm_not_false () = ()

// not nil: takes else branch → true (is_truthy Nil = false, so JumpIfFalse taken)
val vm_not_nil : unit -> Lemma
  (let s0 = fresh_vm [PushNil; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return] in
   // Step 1: PushNil → stack=[Nil], pc=1
   let s1 = eval_steps 1 s0 in
   (match s1 with
    | LispIR.Semantics.Ok s1' ->
      (match s1'.stack with
       | Nil :: _ ->
         // Step 2: JumpIfFalse 4 → Nil falsy → pc=6
         let s2 = eval_steps 1 s1' in
         (match s2 with
          | LispIR.Semantics.Ok s2' ->
            // Steps 3-4: PushBool true + Return → stack=[Bool true]
            let s4 = eval_steps 2 s2' in
            (match s4 with
             | LispIR.Semantics.Ok s4' ->
               (match s4'.stack with | Bool r :: _ -> r = true | _ -> false)
             | _ -> false)
          | _ -> false)
       | _ -> false)
    | _ -> false))
let vm_not_nil () = ()
