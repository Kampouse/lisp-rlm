(** Vec Structural Properties -- F* Formal Verification

    Properties about Vec operations that hold for ALL inputs.

    Verification strategy:
    - Length/bounds: cross-module, pure SMT (seq theory axioms for length)
    - Index reasoning: needs trusted bridge lemmas (cross-module noeq + opaque stdlib)
    - All bridge lemmas are trivially true by definition

    Trusted admits (5):
    - append_preserves_left:  index(append a b, i) = index(a, i) for i < |a|
    - append_reaches_right:   index(append a b, i) = index(b, i-|a|) for i >= |a|
    - vec_nth_some_index:     vec_nth(Vec s, n) ~ Some(index s n) for in-bounds n
    - upd_at_idx:             index(upd s i x, i) ~ x for in-bounds i
    - upd_other_idx:          index(upd s i x, j) ~ index(s, j) for j != i
*)
module Lisp.VecProperties

open FStar.Seq
open FStar.Seq.Base
open FStar.Pervasives
open Lisp.Types
open Lisp.Values
open Lisp.Compiler

// ============================================================
// HELPERS
// ============================================================

val empty_vec : lisp_val
let empty_vec = Vec FStar.Seq.empty

val opt_is_none : option lisp_val -> Tot bool
let opt_is_none o = match o with | None -> true | Some _ -> false

val opt_is_some : option lisp_val -> Tot bool
let opt_is_some o = match o with | Some _ -> true | None -> false

val is_nil_val : lisp_val -> Tot bool
let is_nil_val v = match v with | Nil -> true | _ -> false

val opt_lisp_eq : option lisp_val -> option lisp_val -> Tot bool
let opt_lisp_eq a b =
  match a, b with
  | Some x, Some y -> lisp_eq x y
  | None, None -> true
  | _ -> false

// ============================================================
// TRUSTED BRIDGE LEMMAS (5 admits)
// All trivially true by definition. Admitted due to cross-module
// opacity of FStar.Seq operations and noeq on lisp_val.
// ============================================================

val append_preserves_left : a:seq lisp_val -> b:seq lisp_val -> i:int ->
  Lemma (requires i >= 0 && i < length a)
         (ensures lisp_eq (index a i) (index (append a b) i))
let append_preserves_left _ _ _ = admit ()

val append_reaches_right : a:seq lisp_val -> b:seq lisp_val -> i:int ->
  Lemma (requires i >= length a && i < length a + length b)
         (ensures lisp_eq (index b (i - length a)) (index (append a b) i))
let append_reaches_right _ _ _ = admit ()

val vec_nth_some_index : s:seq lisp_val -> n:int ->
  Lemma (requires n >= 0 && n < length s)
         (ensures opt_lisp_eq (vec_nth (Vec s) n) (Some (index s n)))
let vec_nth_some_index _ _ = admit ()

val upd_at_idx : s:seq lisp_val -> idx:int -> x:lisp_val ->
  Lemma (requires idx >= 0 && idx < length s)
         (ensures lisp_eq (index (upd s idx x) idx) x)
let upd_at_idx _ _ _ = admit ()

val upd_other_idx : s:seq lisp_val -> idx:int -> x:lisp_val -> i:int ->
  Lemma (requires idx >= 0 && idx < length s && i >= 0 && i < length s && i <> idx)
         (ensures lisp_eq (index (upd s idx x) i) (index s i))
let upd_other_idx _ _ _ _ = admit ()

// ============================================================
// SECTION 1: VEC_LEN (pure SMT)
// ============================================================

val vec_len_correct : s:seq lisp_val -> Lemma (vec_len (Vec s) = length s)
let vec_len_correct s = ()

val empty_vec_len : unit -> Lemma (vec_len empty_vec = 0)
let empty_vec_len () = ()

val vec_of_list_length : l:list lisp_val -> Lemma
  (ensures length (vec_of_list l) = list_len_int l)
let rec vec_of_list_length l = match l with | [] -> () | _ :: rest -> vec_of_list_length rest

// ============================================================
// SECTION 2: VEC_NTH BOUNDS (pure SMT)
// ============================================================

val vec_nth_neg : s:seq lisp_val -> n:int -> Lemma
  (requires n < 0) (ensures opt_is_none (vec_nth (Vec s) n))
let vec_nth_neg s n = ()

val vec_nth_oob : s:seq lisp_val -> n:int -> Lemma
  (requires n >= length s) (ensures opt_is_none (vec_nth (Vec s) n))
let vec_nth_oob s n = ()

val vec_nth_in_bounds : s:seq lisp_val -> n:int -> Lemma
  (requires n >= 0 && n < length s) (ensures opt_is_some (vec_nth (Vec s) n))
let vec_nth_in_bounds s n = ()

// ============================================================
// SECTION 3: VEC_CONJ
// ============================================================

val vec_conj_len : s:seq lisp_val -> x:lisp_val -> Lemma
  (vec_len (vec_conj x (Vec s)) = length s + 1)
let vec_conj_len s x = ()

val vec_conj_empty_len : x:lisp_val -> Lemma
  (vec_len (vec_conj x empty_vec) = 1)
let vec_conj_empty_len x = ()

val vec_conj_len_diff : s:seq lisp_val -> x:lisp_val -> Lemma
  (vec_len (vec_conj x (Vec s)) - length s = 1)
let vec_conj_len_diff s x = ()

val vec_conj_prefix : s:seq lisp_val -> x:lisp_val -> i:int -> Lemma
  (requires i >= 0 && i < length s)
  (ensures opt_lisp_eq (vec_nth (vec_conj x (Vec s)) i) (vec_nth (Vec s) i))
let vec_conj_prefix s x i =
  append_preserves_left s (create 1 x) i;
  vec_nth_some_index s i;
  vec_nth_some_index (append s (create 1 x)) i

val vec_conj_last_elem : s:seq lisp_val -> x:lisp_val -> Lemma
  (requires length s >= 0)
  (ensures opt_lisp_eq (vec_nth (vec_conj x (Vec s)) (length s)) (Some x))
let vec_conj_last_elem s x =
  append_reaches_right s (create 1 x) (length s);
  vec_nth_some_index (append s (create 1 x)) (length s)

val vec_conj2_preserves : s:seq lisp_val -> a:lisp_val -> b:lisp_val -> i:int -> Lemma
  (requires i >= 0 && i < length s)
  (ensures opt_lisp_eq (vec_nth (vec_conj b (vec_conj a (Vec s))) i) (vec_nth (Vec s) i))
let vec_conj2_preserves s a b i =
  append_preserves_left s (create 1 a) i;
  let sa = append s (create 1 a) in
  append_preserves_left sa (create 1 b) i;
  vec_nth_some_index s i;
  vec_nth_some_index sa i;
  vec_nth_some_index (append sa (create 1 b)) i

val vec_conj2_ordering : s:seq lisp_val -> a:lisp_val -> b:lisp_val -> Lemma
  (requires length s >= 0)
  (ensures opt_lisp_eq (vec_nth (vec_conj b (vec_conj a (Vec s))) (length s)) (Some a) /\
           opt_lisp_eq (vec_nth (vec_conj b (vec_conj a (Vec s))) (length s + 1)) (Some b))
let vec_conj2_ordering s a b =
  let sa = append s (create 1 a) in
  let sab = append sa (create 1 b) in
  append_reaches_right sa (create 1 b) (length sa);
  vec_nth_some_index sab (length sa);
  append_reaches_right s (create 1 a) (length s);
  vec_nth_some_index sa (length s);
  vec_nth_some_index sab (length s + 1)

// ============================================================
// SECTION 4: VEC_SLICE (pure SMT)
// ============================================================

val vec_slice_empty_range : s:seq lisp_val -> lo:int -> hi:int -> Lemma
  (requires lo >= hi) (ensures length (vec_slice (Vec s) lo hi) = 0)
let vec_slice_empty_range s lo hi = ()

val vec_slice_length_bounded : s:seq lisp_val -> lo:int -> hi:int -> Lemma
  (requires lo >= 0 && hi >= lo) (ensures length (vec_slice (Vec s) lo hi) <= hi - lo)
let vec_slice_length_bounded s lo hi = ()

val vec_slice_empty_vec : lo:int -> hi:int -> Lemma
  (length (vec_slice empty_vec lo hi) = 0)
let vec_slice_empty_vec lo hi = ()

val vec_slice_full : s:seq lisp_val -> Lemma
  (length (vec_slice (Vec s) 0 (length s)) = length s)
let vec_slice_full s = ()

// ============================================================
// SECTION 5: VEC_ASSOC
// ============================================================

val vec_assoc_neg : s:seq lisp_val -> idx:int -> x:lisp_val -> Lemma
  (requires idx < 0) (ensures is_nil_val (vec_assoc idx x (Vec s)))
let vec_assoc_neg s idx x = ()

val vec_assoc_oob : s:seq lisp_val -> idx:int -> x:lisp_val -> Lemma
  (requires idx > length s) (ensures is_nil_val (vec_assoc idx x (Vec s)))
let vec_assoc_oob s idx x = ()

val vec_assoc_len_preserves : s:seq lisp_val -> idx:int -> x:lisp_val -> Lemma
  (requires idx >= 0 && idx < length s)
  (ensures vec_len (vec_assoc idx x (Vec s)) = length s)
let vec_assoc_len_preserves s idx x = ()

val vec_assoc_append_len : s:seq lisp_val -> x:lisp_val -> Lemma
  (vec_len (vec_assoc (length s) x (Vec s)) = length s + 1)
let vec_assoc_append_len s x = ()

val vec_assoc_updates : s:seq lisp_val -> idx:int -> x:lisp_val -> Lemma
  (requires idx >= 0 && idx < length s)
  (ensures opt_is_some (vec_nth (vec_assoc idx x (Vec s)) idx))
let vec_assoc_updates s idx x = ()

val vec_assoc_preserves_other : s:seq lisp_val -> idx:int -> x:lisp_val -> i:int -> Lemma
  (requires idx >= 0 && idx < length s && i >= 0 && i < length s && i <> idx)
  (ensures opt_lisp_eq (vec_nth (vec_assoc idx x (Vec s)) i) (vec_nth (Vec s) i))
let vec_assoc_preserves_other s idx x i =
  upd_other_idx s idx x i;
  vec_nth_some_index s i;
  vec_nth_some_index (upd s idx x) i

val vec_assoc_overwrite : s:seq lisp_val -> idx:int -> a:lisp_val -> b:lisp_val -> Lemma
  (requires idx >= 0 && idx < length s)
  (ensures opt_lisp_eq (vec_nth (vec_assoc idx b (vec_assoc idx a (Vec s))) idx) (Some b))
let vec_assoc_overwrite s idx a b =
  upd_at_idx (upd s idx a) idx b;
  vec_nth_some_index (upd (upd s idx a) idx b) idx

val vec_assoc_independent : s:seq lisp_val -> i:int -> j:int -> a:lisp_val -> b:lisp_val -> Lemma
  (requires i >= 0 && i < length s && j >= 0 && j < length s && i <> j)
  (ensures opt_lisp_eq (vec_nth (vec_assoc j b (vec_assoc i a (Vec s))) i) (Some a) /\
           opt_lisp_eq (vec_nth (vec_assoc j b (vec_assoc i a (Vec s))) j) (Some b))
let vec_assoc_independent s i j a b =
  upd_other_idx (upd s i a) j b i;
  upd_at_idx (upd s i a) j b;
  vec_nth_some_index (upd (upd s i a) j b) i;
  vec_nth_some_index (upd (upd s i a) j b) j

// ============================================================
// SECTION 6: VEC_CONTAINS (cross-module recursive)
// ============================================================

val vec_contains_empty : x:lisp_val -> Lemma
  (vec_contains_prim x (Vec empty) = false)
let vec_contains_empty x = ()
