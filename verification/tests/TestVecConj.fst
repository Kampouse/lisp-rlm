module TestVecConj

open FStar.Seq
open FStar.Seq.Base
open Lisp.Types
open Lisp.Values
open Lisp.Compiler

val opt_is_none : option lisp_val -> Tot bool
let opt_is_none o = match o with | None -> true | Some _ -> false

val opt_is_some : option lisp_val -> Tot bool
let opt_is_some o = match o with | Some _ -> true | None -> false

val my_conj : lisp_val -> lisp_val -> Tot lisp_val
let my_conj x v =
  match v with
  | Vec s -> Vec (append s (create 1 x))
  | _ -> Vec (create 1 x)

val my_len : lisp_val -> Tot int
let my_len v =
  match v with
  | Vec s -> length s
  | _ -> 0

// Soundness: local mirrors agree with Lisp.Values for Vec inputs
val conj_sound : s:seq lisp_val -> x:lisp_val -> Lemma
  (match my_conj x (Vec s), vec_conj x (Vec s) with
   | Vec a, Vec b -> length a = length b
   | _ -> true)
let conj_sound s x =
  assert_norm (match my_conj x (Vec s), vec_conj x (Vec s) with
               | Vec a, Vec b -> length a = length b
               | _ -> true)

val len_sound : v:lisp_val -> Lemma
  (match v with
   | Vec s -> my_len v = vec_len v
   | _ -> true)
let len_sound v =
  assert_norm (my_len v = vec_len v)

// vec_conj increases length by 1
val vec_conj_len : s:seq lisp_val -> x:lisp_val -> Lemma
  (vec_len (vec_conj (Vec s) x) = length s + 1)
let vec_conj_len s x =
  conj_sound s x;
  len_sound (my_conj x (Vec s));
  ()
