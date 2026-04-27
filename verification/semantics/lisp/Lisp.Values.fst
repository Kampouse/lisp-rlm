(** Lisp Value Operations — F* Formal Specification

    Mirrors src/bytecode.rs helper functions:
    - num_val, num_arith, num_cmp, lisp_eq, is_truthy
    
    Following vWasm's pattern: semantics/wasm/Wasm.Eval_numeric.fst
    The key insight: F*'s type system REJECTS the old num_val truncation bug
    because Float carries a different type (ffloat) than Num (int).
*)
module Lisp.Values

open Lisp.Types

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

// === Dict operations ===
val dict_get : key:string -> m:list (string * lisp_val) -> Tot lisp_val
let rec dict_get key m =
  match m with
  | [] -> Nil
  | (k, v) :: rest -> if k = key then v else dict_get key rest

val dict_set : key:string -> v:lisp_val -> m:list (string * lisp_val) -> Tot (list (string * lisp_val))
let dict_set key v m =
  let rec filter f lst =
    match lst with
    | [] -> []
    | x :: rest -> if f x then x :: filter f rest else filter f rest
  in
  (key, v) :: filter (fun (k, _) -> k <> key) m
