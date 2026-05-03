(** HOF (Higher-Order Function) Properties -- F* Formal Verification

    Properties about map and filter that hold for ALL inputs.
    Universal theorems proved by structural induction.

    Architecture:
    - Local mirrors of map/filter that SMT can unfold
    - Length-based properties (avoids noeq on lisp_val)
    - Higher-order quantifiers can overwhelm SMT with noeq types;
      trivial corollaries and seq-level operations omitted

    NOTE: FStar.Seq.create requires eqtype elements, so seq-level
    map/filter over lisp_val cannot be defined. List-level only.
*)
module Lisp.HOFProperties

open FStar.List.Tot
open FStar.Pervasives
open Lisp.Types

// ============================================================
// LOCAL HELPERS
// ============================================================

val llist_length : list 'a -> int
let rec llist_length = function | [] -> 0 | _ :: rest -> 1 + llist_length rest

// ============================================================
// LOCAL MIRRORS (SMT can unfold these)
// ============================================================

val lmap : (lisp_val -> lisp_val) -> list lisp_val -> Tot (list lisp_val)
let rec lmap f = function
  | [] -> []
  | x :: xs -> f x :: lmap f xs

val lfilter : (lisp_val -> bool) -> list lisp_val -> Tot (list lisp_val)
let rec lfilter p = function
  | [] -> []
  | x :: xs -> if p x then x :: lfilter p xs else lfilter p xs

// ============================================================
// SECTION 1: MAP LENGTH PROPERTIES
// ============================================================

// Theorem: map preserves length for ALL f and ALL lists.
val map_preserves_length : f:(lisp_val -> lisp_val) -> l:list lisp_val -> Lemma
  (ensures llist_length (lmap f l) = llist_length l)
let rec map_preserves_length f l =
  match l with
  | [] -> ()
  | _ :: rest -> map_preserves_length f rest

// Theorem: |map f (map g l)| = |map (f o g) l|
val map_compose_length : f:(lisp_val -> lisp_val) -> g:(lisp_val -> lisp_val) -> l:list lisp_val -> Lemma
  (ensures llist_length (lmap f (lmap g l)) = llist_length (lmap (fun x -> f (g x)) l))
let rec map_compose_length f g l =
  match l with
  | [] -> ()
  | _ :: rest -> map_compose_length f g rest

// Theorem: |map id l| = |l|
val map_id_length : l:list lisp_val -> Lemma
  (llist_length (lmap (fun x -> x) l) = llist_length l)
let rec map_id_length l =
  match l with
  | [] -> ()
  | _ :: rest -> map_id_length rest

// ============================================================
// SECTION 2: FILTER LENGTH PROPERTIES
// ============================================================

// Theorem: filter never increases length for ALL p and ALL lists.
val filter_length_bound : p:(lisp_val -> bool) -> l:list lisp_val -> Lemma
  (ensures llist_length (lfilter p l) <= llist_length l)
let rec filter_length_bound p l =
  match l with
  | [] -> ()
  | x :: xs ->
    filter_length_bound p xs;
    if p x then () else ()

// ============================================================
// SECTION 3: MAP + FILTER COMPOSITION
// ============================================================

// |filter p (map f xs)| <= |xs|
val filter_map_length : p:(lisp_val -> bool) -> f:(lisp_val -> lisp_val) -> l:list lisp_val -> Lemma
  (ensures llist_length (lfilter p (lmap f l)) <= llist_length l)
let rec filter_map_length p f l =
  match l with
  | [] -> ()
  | x :: xs ->
    filter_map_length p f xs;
    if p (f x) then () else ()

// |map f (filter p xs)| <= |xs|
val map_filter_length : f:(lisp_val -> lisp_val) -> p:(lisp_val -> bool) -> l:list lisp_val -> Lemma
  (ensures llist_length (lmap f (lfilter p l)) <= llist_length l)
let rec map_filter_length f p l =
  match l with
  | [] -> ()
  | x :: xs ->
    map_filter_length f p xs;
    if p x then () else ()

// |filter p (filter q xs)| <= |xs|
val filter_filter_length : p:(lisp_val -> bool) -> q:(lisp_val -> bool) -> l:list lisp_val -> Lemma
  (ensures llist_length (lfilter p (lfilter q l)) <= llist_length l)
let rec filter_filter_length p q l =
  match l with
  | [] -> ()
  | x :: xs ->
    filter_filter_length p q xs;
    if q x then (if p x then () else ()) else ()
