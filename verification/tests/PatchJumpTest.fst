(** Patch jump + manual if code construction *)
module PatchJumpTest

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler

// patch_jump replaces JumpIfFalse at index 1 with target 4
val patch_jf_test : unit -> Lemma
  (match patch_jump [PushI64 1; JumpIfFalse 0; PushI64 42; Jump 0; PushI64 99] 1 4 with
   | [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 0; PushI64 99] -> true
   | _ -> false)
let patch_jf_test () = ()

// Second patch: replace Jump at index 3 with target 5
val patch_jmp_test : unit -> Lemma
  (match patch_jump [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 0; PushI64 99] 3 5 with
   | [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99] -> true
   | _ -> false)
let patch_jmp_test () = ()
