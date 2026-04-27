(** Closure application tests *)
module ClosureTest

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Closure
open LispIR.Semantics

// ((fn [x] (+ x 1)) 5) = 6
val apply_identity_inc : unit -> Lemma
  (match apply_lambda 100 ["x"] (List [Sym "+"; Sym "x"; Num 1]) [Num 5] [] with
   | Some (Num r) -> r = 6
   | _ -> false)
let apply_identity_inc () = ()

// ((fn [x y] (+ x y)) 3 4) = 7
val apply_two_params : unit -> Lemma
  (match apply_lambda 100 ["x"; "y"] (List [Sym "+"; Sym "x"; Sym "y"]) [Num 3; Num 4] [] with
   | Some (Num r) -> r = 7
   | _ -> false)
let apply_two_params () = ()

// Closure with captured env:
// (let [n 42] (fn [] n)) → closure with env [("n", 42)]
// ((fn [] n)) with env [("n", 42)] → 42
val apply_captured : unit -> Lemma
  (match apply_lambda 100 [] (Sym "n") [] [("n", Num 42)] with
   | Some (Num r) -> r = 42
   | _ -> false)
let apply_captured () = ()

// Closure with if: ((fn [x] (if (> x 0) x (- 0 x))) 5) = 5
val apply_if_positive : unit -> Lemma
  (match apply_lambda 100 ["x"]
    (List [Sym "if"; List [Sym ">"; Sym "x"; Num 0]; Sym "x"; List [Sym "-"; Num 0; Sym "x"]])
    [Num 5] [] with
   | Some (Num r) -> r = 5
   | _ -> false)
let apply_if_positive () = ()

// Same with negative input: ((fn [x] (if (> x 0) x (- 0 x))) (-3)) = 3
val apply_if_negative : unit -> Lemma
  (match apply_lambda 100 ["x"]
    (List [Sym "if"; List [Sym ">"; Sym "x"; Num 0]; Sym "x"; List [Sym "-"; Num 0; Sym "x"]])
    [Num (-3)] [] with
   | Some (Num r) -> r = 3
   | _ -> false)
let apply_if_negative () = ()

// Closure with let: ((fn [x] (let [y (+ x 1)] y)) 5) = 6
val apply_with_let : unit -> Lemma
  (match apply_lambda 100 ["x"]
    (List [Sym "let"; List [List [Sym "y"; List [Sym "+"; Sym "x"; Num 1]]]; Sym "y"])
    [Num 5] [] with
   | Some (Num r) -> r = 6
   | _ -> false)
let apply_with_let () = ()
