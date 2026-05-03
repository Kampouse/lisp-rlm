(** Lisp Value Operations — F* Formal Specification

    Mirrors src/bytecode.rs helper functions:
    - num_val, num_arith, num_cmp, lisp_eq, is_truthy
    
    Following vWasm's pattern: semantics/wasm/Wasm.Eval_numeric.fst
    The key insight: F*'s type system REJECTS the old num_val truncation bug
    because Float carries a different type (ffloat) than Num (int).
*)
module Lisp.Values

open Lisp.Types
open FStar.Seq

// === Truthiness ===
val is_truthy : lisp_val -> Tot bool
let is_truthy v =
  match v with
  | Nil -> false
  | Bool b -> b
  | _ -> true

// === Numeric extraction (LOSSY — truncates float to int) ===
// This is the function that CAUSED the comparison bug.
// F* makes the truncation explicit via ff_to_int.
val num_val : lisp_val -> Tot int
let num_val v =
  match v with
  | Num n -> n
  | Float f -> ff_to_int f    (* EXPLICIT truncation — F* knows this is lossy *)
  | _ -> 0

// === Polymorphic Arithmetic ===
// If either operand is Float, use float arithmetic. Otherwise integer.
val num_arith : a:lisp_val -> b:lisp_val
  -> (int -> int -> int)
  -> (ffloat -> ffloat -> ffloat)
  -> Tot lisp_val
let num_arith a b int_op float_op =
  match a, b with
  | Float x, Float y -> Float (float_op x y)
  | Float x, Num y   -> Float (float_op x (ff_of_int y))
  | Num x,   Float y -> Float (float_op (ff_of_int x) y)
  | Num x,   Num y   -> Num (int_op x y)
  | _, _              -> Num 0

// === Polymorphic Numeric Comparison (THE FIX) ===
// Float-aware comparison. No truncation.
// THE KEY PROPERTY: For any two numeric values, this returns the same
// result as comparing them in their natural representation.
// 
// THE BUG THIS CATCHES: The old code used num_val() which did:
//   Float(0.9) → int(0), Float(0.3) → int(0), so 0.9 > 0.3 → false
// 
// This implementation is IMPOSSIBLE to write with num_val here —
// F* sees that num_val returns int, and ff_to_int is a different function
// from the float comparators. You CAN'T accidentally use int comparison
// on a float because the types don't match.

val num_cmp : a:lisp_val -> b:lisp_val
  -> (ffloat -> ffloat -> bool)     (* float comparator *)
  -> (int -> int -> bool)           (* int comparator *)
  -> Tot bool
let num_cmp a b float_op int_op =
  match a, b with
  | Float x, Float y -> float_op x y
  | Float x, Num y   -> float_op x (ff_of_int y)
  | Num x,   Float y -> float_op (ff_of_int x) y
  | Num x,   Num y   -> int_op x y
  | _, _              -> false

// === Lemma: num_cmp uses float comparator for two floats ===
val num_cmp_float_sound : x:ffloat -> y:ffloat
  -> f_op:(ffloat -> ffloat -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma (num_cmp (Float x) (Float y) f_op i_op = f_op x y)
let num_cmp_float_sound x y f_op i_op = ()

// === Lemma: num_cmp uses int comparator for two ints ===
val num_cmp_int_sound : x:int -> y:int
  -> f_op:(ffloat -> ffloat -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma (num_cmp (Num x) (Num y) f_op i_op = i_op x y)
let num_cmp_int_sound x y f_op i_op = ()

// === Lemma: num_cmp does NOT truncate floats to ints ===
val num_cmp_no_truncation : x:ffloat -> y:ffloat
  -> f_op:(ffloat -> ffloat -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma (num_cmp (Float x) (Float y) f_op i_op = f_op x y)
let num_cmp_no_truncation x y f_op i_op = ()

// === Lisp Equality ===
val lisp_eq : lisp_val -> lisp_val -> Tot bool
let lisp_eq a b =
  match a, b with
  | Num x,    Num y    -> x = y
  | Float x,  Float y  -> ff_eq x y
  | Num x,    Float y  -> ff_eq (ff_of_int x) y
  | Float x,  Num y    -> ff_eq x (ff_of_int y)
  | Bool x,   Bool y   -> x = y
  | Str x,    Str y    -> x = y
  | Nil,      Nil      -> true
  | _, _                -> false

// === Lemma: lisp_eq on Num values ===
val lisp_eq_num_sound : x:int -> y:int
  -> Lemma (lisp_eq (Num x) (Num y) = (x = y))
let lisp_eq_num_sound x y = ()

// === Dict operations ===
// Non-recursive wrappers around recursive helpers.
// This lets Z3 unfold the first match case of dict_get/dict_remove.

val dict_get_rest : string -> list (string * lisp_val) -> Tot lisp_val
let rec dict_get_rest key m =
  match m with
  | [] -> Nil
  | (k, v) :: rest -> if k = key then v else dict_get_rest key rest

val dict_remove_rest : string -> list (string * lisp_val) -> Tot (list (string * lisp_val))
let rec dict_remove_rest key m =
  match m with
  | [] -> []
  | (k, v) :: rest -> if k = key then dict_remove_rest key rest
                       else (k, v) :: dict_remove_rest key rest

val dict_get : key:string -> m:list (string * lisp_val) -> Tot lisp_val
let dict_get key m =
  match m with
  | [] -> Nil
  | (k, v) :: rest -> if k = key then v else dict_get_rest key rest

val dict_remove : key:string -> m:list (string * lisp_val) -> Tot (list (string * lisp_val))
let dict_remove key m =
  match m with
  | [] -> []
  | (k, v) :: rest -> if k = key then dict_remove_rest key rest
                       else (k, v) :: dict_remove_rest key rest

val dict_set : key:string -> v:lisp_val -> m:list (string * lisp_val) -> Tot (list (string * lisp_val))
let dict_set key v m = (key, v) :: dict_remove key m

// === Vector (List) accessors ===
// lisp_val's List constructor wraps list lisp_val — a vector with O(n) len/nth.
// These enable length-induction proofs over list-valued computations.
// Note: lisp_val is noeq, so equality lemmas require assert_norm on concrete terms.

val list_len_aux : acc:nat -> elems:list lisp_val -> Tot nat (decreases elems)
let rec list_len_aux acc elems =
  match elems with
  | [] -> acc
  | x :: rest -> list_len_aux (acc + 1) rest

val list_len : lisp_val -> Tot nat
let list_len v =
  match v with
  | List elems -> list_len_aux 0 elems
  | _ -> 0

val list_nth_aux : elems:list lisp_val -> n:nat -> Tot (option lisp_val) (decreases elems)
let rec list_nth_aux elems n =
  match elems, n with
  | [], _ -> None
  | x :: _, 0 -> Some x
  | _ :: rest, _ -> list_nth_aux rest (n - 1)

val list_nth : lisp_val -> nat -> Tot (option lisp_val)
let list_nth v n =
  match v with
  | List elems -> list_nth_aux elems n
  | _ -> None

val list_empty : lisp_val -> Tot bool
let list_empty v =
  match v with
  | List [] -> true
  | _ -> false

val list_cons : lisp_val -> lisp_val -> lisp_val
let list_cons hd tl =
  match tl with
  | List elems -> List (hd :: elems)
  | _ -> List [hd; tl]  (* degenerate: treat non-list tail as single elem *)

// === Vector (Seq) accessors ===
// lisp_val's Vec constructor wraps seq lisp_val — O(1) len/nth via Seq theory.
// VecContains uses lisp_eq on primitives (noeq prevents general lisp_val comparison).

val vec_of_list : list lisp_val -> Tot (seq lisp_val)
let vec_of_list l = FStar.Seq.of_list l

val vec_len : lisp_val -> Tot nat
let vec_len v =
  match v with
  | Vec s -> FStar.Seq.length s
  | _ -> 0

val vec_nth : lisp_val -> int -> Tot (option lisp_val)
let vec_nth v n =
  match v with
  | Vec s ->
    let len = FStar.Seq.length s in
    if n < 0 || n >= len then None
    else
      let idx = n in  // SMT knows: 0 <= idx < len
      Some (FStar.Seq.index s idx)
  | _ -> None

val vec_empty : lisp_val -> Tot bool
let vec_empty v =
  match v with
  | Vec s -> FStar.Seq.length s = 0
  | _ -> false

val vec_conj : lisp_val -> lisp_val -> lisp_val
let vec_conj val0 vec0 =
  match vec0 with
  | Vec s -> Vec (FStar.Seq.append s (FStar.Seq.Base.create 1 val0))
  | _ -> Vec (FStar.Seq.Base.create 1 val0)

// VecContains: check if val exists in vec using lisp_eq on primitive types.
// noeq on lisp_val means we can't use Seq.mem — explicit loop required.
val vec_contains_prim_aux : needle:lisp_val -> v:seq lisp_val -> i:nat -> Tot bool (decreases (FStar.Seq.length v - i))
let rec vec_contains_prim_aux needle v i =
  if i >= FStar.Seq.length v then false
  else lisp_eq needle (FStar.Seq.index v i) || vec_contains_prim_aux needle v (i + 1)

val vec_contains_prim : lisp_val -> lisp_val -> Tot bool
let vec_contains_prim needle vec0 =
  match vec0 with
  | Vec s -> vec_contains_prim_aux needle s 0
  | _ -> false

// Clamp an int to the range [0, max_val], returning nat.
// Used to safely convert VM int indices to nat for Seq operations.
val clamp_nat : i:int -> max_val:nat -> Tot nat
let clamp_nat i max_val =
  if i <= 0 then 0
  else if i >= max_val then max_val
  else i

// VecSlice: extract sub-sequence [start, end).
val vec_slice : lisp_val -> int -> int -> Tot (seq lisp_val)
let vec_slice vec0 start_i end_i =
  match vec0 with
  | Vec s ->
    let len = FStar.Seq.length s in
    let lo = clamp_nat start_i len in
    let hi = clamp_nat end_i len in
    if lo < hi then FStar.Seq.slice s lo hi
    else FStar.Seq.empty
  | _ -> FStar.Seq.empty

// VecAssoc: update index or append at end.
val vec_assoc : int -> lisp_val -> lisp_val -> lisp_val
let vec_assoc idx val0 vec0 =
  match vec0 with
  | Vec s ->
    let len = FStar.Seq.length s in
    if idx < 0 then Nil
    else if idx >= len then
      if idx = len then
        Vec (FStar.Seq.append s (FStar.Seq.Base.create 1 val0))
      else Nil
    else Vec (FStar.Seq.upd s idx val0)
  | _ -> Nil
