(** Dict VM correctness tests *)
module DictOps

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

// DictGet existing key
val vm_dict_get_found : unit -> Lemma
  (match eval_steps 100 {
    stack = [Str "x";
            Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictGet];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
let vm_dict_get_found () = ()

// DictGet missing key -> Nil
val vm_dict_get_missing : unit -> Lemma
  (match eval_steps 100 {
    stack = [Str "z";
            Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictGet];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Nil :: _ -> true
     | _ -> false)
   | _ -> false)
let vm_dict_get_missing () = ()

// DictSet new key: check we get a Dict with 2 entries
val vm_dict_set_new : unit -> Lemma
  (match eval_steps 100 {
    stack = [Num 99; Str "y"; Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictSet];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Dict entries :: _ ->
       (match entries with
        | [(ky, Num vy); (kx, Num vx)] -> ky = "y" && vy = 99 && kx = "x" && vx = 42
        | _ -> false)
     | _ -> false)
   | _ -> false)
let vm_dict_set_new () = ()

// DictSet overwrite existing key
val vm_dict_set_overwrite : unit -> Lemma
  (match eval_steps 100 {
    stack = [Num 7; Str "x"; Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictSet];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Dict entries :: _ ->
       (match entries with
        | [(k, Num v)] -> k = "x" && v = 7
        | _ -> false)
     | _ -> false)
   | _ -> false)
let vm_dict_set_overwrite () = ()

// DictGet on empty dict -> Nil
val vm_dict_get_empty : unit -> Lemma
  (match eval_steps 100 {
    stack = [Str "x"; Dict []];
    slots = [];
    pc = 0;
    code = [DictGet];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Nil :: _ -> true
     | _ -> false)
   | _ -> false)
let vm_dict_get_empty () = ()
