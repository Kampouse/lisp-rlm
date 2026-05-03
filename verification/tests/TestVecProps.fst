module TestVecProps

open FStar.Seq
open FStar.Seq.Base
open Lisp.Types
open Lisp.Values
open Lisp.Compiler

val empty_vec : lisp_val
let empty_vec = Vec FStar.Seq.empty

val opt_is_none : option lisp_val -> Tot bool
let opt_is_none o = match o with | None -> true | Some _ -> false

val opt_is_some : option lisp_val -> Tot bool
let opt_is_some o = match o with | Some _ -> true | None -> false

val is_nil_val : lisp_val -> Tot bool
let is_nil_val v = match v with | Nil -> true | _ -> false

val vec_len_correct : s:seq lisp_val -> Lemma
  (vec_len (Vec s) = length s)
let vec_len_correct s = ()

val empty_vec_len : unit -> Lemma
  (vec_len empty_vec = 0)
let empty_vec_len () = ()

val vec_of_list_length : l:list lisp_val -> Lemma
  (ensures length (vec_of_list l) = list_len_int l)
let rec vec_of_list_length l =
  match l with
  | [] -> ()
  | _ :: rest -> vec_of_list_length rest

val vec_nth_neg : s:seq lisp_val -> n:int -> Lemma
  (requires n < 0)
  (ensures opt_is_none (vec_nth (Vec s) n))
let vec_nth_neg s n = ()

val vec_nth_oob : s:seq lisp_val -> n:int -> Lemma
  (requires n >= length s)
  (ensures opt_is_none (vec_nth (Vec s) n))
let vec_nth_oob s n = ()

val vec_nth_in_bounds : s:seq lisp_val -> n:int -> Lemma
  (requires n >= 0 && n < length s)
  (ensures opt_is_some (vec_nth (Vec s) n))
let vec_nth_in_bounds s n = ()
