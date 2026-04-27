(** dict_set computation test *)
module DictSetTest

open Lisp.Types
open Lisp.Values

val test_set_new : unit -> Lemma
  (match dict_set "y" (Num 99) [("x", Num 42)] with
   | [("y", Num vy); ("x", Num vx)] -> vy = 99 && vx = 42
   | _ -> false)
let test_set_new () = ()

val test_set_overwrite : unit -> Lemma
  (match dict_set "x" (Num 7) [("x", Num 42)] with
   | [("x", Num v)] -> v = 7
   | _ -> false)
let test_set_overwrite () = ()
