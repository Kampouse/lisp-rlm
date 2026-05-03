module TestNorm

open FStar.Seq
open FStar.Seq.Base
open Lisp.Types
open Lisp.Values
open Lisp.Compiler

// Test 1: does normalizer unfold vec_len (Vec (create 1 (Num 0)))?
val test1 : unit -> Lemma
  (vec_len (Vec (create 1 (Num 0))) = 1)
let test1 () =
  assert_norm (vec_len (Vec (create 1 (Num 0))) = 1)

// Test 2: does normalizer unfold length (create 1 (Num 0))?
val test2 : unit -> Lemma
  (length (create 1 (Num 0)) = 1)
let test2 () =
  assert_norm (length (create 1 (Num 0)) = 1)

// Test 3: does normalizer unfold length (append (create 1 (Num 0)) (create 1 (Num 1)))?
val test3 : unit -> Lemma
  (length (append (create 1 (Num 0)) (create 1 (Num 1))) = 2)
let test3 () =
  assert_norm (length (append (create 1 (Num 0)) (create 1 (Num 1))) = 2)

// Test 4: does SMT prove length (append s (create 1 x)) = length s + 1?
val test4 : s:seq lisp_val -> x:lisp_val -> Lemma
  (length (append s (create 1 x)) = length s + 1)
let test4 s x = ()
