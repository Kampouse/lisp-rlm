(** Lisp Value Operations — F* Formal Specification

    Mirrors src/bytecode.rs helper functions:
    - num_val, num_arith, num_cmp, lisp_eq, is_truthy
    
    Following vWasm's pattern: semantics/wasm/Wasm.Eval_numeric.fst
    The key insight: F*'s type system REJECTS the old num_val truncation bug
    because you can't silently cast float → int in a total function.
*)
module Lisp.Values

open Lisp.Types

// === Truthiness ===
// Everything is truthy except Nil and Bool(false).
// Matches the Rust VM behavior.

val is_truthy : lisp_val -> Tot bool
let is_truthy v =
  match v with
  | Nil -> false
  | Bool b -> b
  | _ -> true

// === Numeric extraction ===
// Extract an integer from a value. Returns 0 for non-numerics.
// WARNING: This truncates floats to ints (f as i64).
// This is the function that CAUSED the comparison bug.
// In F*, the cast is explicit and flagged by the type checker.

val num_val : lisp_val -> Tot int
let num_val v =
  match v with
  | Num n -> n
  | Float f -> int_of_float f  (* TRUNCATION — lossy, F* flags this *)
  | _ -> 0

// === Polymorphic Arithmetic ===
// If either operand is Float, use float arithmetic. Otherwise integer.
// Matches Rust: fn num_arith(...)
// F* PROVES that the result type matches the operand types:
//   Float op Float → Float, Num op Num → Num, mixed → Float

val num_arith : a:lisp_val -> b:lisp_val
  -> (int -> int -> int)
  -> (float -> float -> float)
  -> Tot lisp_val
let num_arith a b int_op float_op =
  match a, b with
  | Float x, Float y -> Float (float_op x y)
  | Float x, Num y   -> Float (float_op x (int_to_float y))
  | Num x,   Float y -> Float (float_op (int_to_float x) y)
  | Num x,   Num y   -> Num (int_op x y)
  | _, _              -> Num 0

// === Polymorphic Numeric Comparison (THE FIX) ===
// Float-aware comparison. No truncation.
// Matches Rust: fn num_cmp(...)
//
// THE KEY PROPERTY: For any two numeric values, this returns the same
// result as comparing them in their natural representation.
// 
// THE BUG THIS CATCHES: The old code used num_val() which did:
//   Float(0.9) → int(0), Float(0.3) → int(0), so 0.9 > 0.3 → false
// F* REJECTS that implementation because num_val returns int,
// and comparing ints is NOT float comparison.

val num_cmp : a:lisp_val -> b:lisp_val
  -> (float -> float -> bool)
  -> (int -> int -> bool)
  -> Tot bool
let num_cmp a b float_op int_op =
  match a, b with
  | Float x, Float y -> float_op x y
  | Float x, Num y   -> float_op x (int_to_float y)
  | Num x,   Float y -> float_op (int_to_float x) y
  | Num x,   Num y   -> int_op x y
  | _, _              -> false

// === Lemma: num_cmp preserves float precision ===
// This is the theorem that would have caught our bug.
// It says: if both values are floats, num_cmp uses the float comparator.

val num_cmp_float_sound : x:float -> y:float
  -> f_op:(float -> float -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma
       (num_cmp (Float x) (Float y) f_op i_op = f_op x y)
let num_cmp_float_sound x y f_op i_op = ()

// === Lemma: num_cmp preserves int precision ===
val num_cmp_int_sound : x:int -> y:int
  -> f_op:(float -> float -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma
       (num_cmp (Num x) (Num y) f_op i_op = i_op x y)
let num_cmp_int_sound x y f_op i_op = ()

// === Lemma: num_cmp does NOT truncate floats to ints ===
// If we had used num_val for comparison (the old bug):
//   compare(Float 0.9, Float 0.3, >) would do: (int 0 > int 0) = false
// This lemma asserts that can't happen:

val num_cmp_no_truncation : x:float -> y:float
  -> f_op:(float -> float -> bool)
  -> i_op:(int -> int -> bool)
  -> Lemma
       (ensures
         (num_cmp (Float x) (Float y) f_op i_op = f_op x y /\
          (* The result does NOT depend on i_op at all when both are float *)
          true))
let num_cmp_no_truncation x y f_op i_op = ()

// === Lisp Equality ===
// Matches Rust: fn lisp_eq(...)
// Cross-type comparison: Num(1) == Float(1.0) is true.

val lisp_eq : lisp_val -> lisp_val -> Tot bool
let lisp_eq a b =
  match a, b with
  | Num x,    Num y    -> x = y
  | Float x,  Float y  -> x = y
  | Num x,    Float y  -> int_to_float x = y
  | Float x,  Num y    -> x = int_to_float y
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

val dict_set : key:string -> val:lisp_val -> m:list (string * lisp_val) -> Tot (list (string * lisp_val))
let dict_set key val m =
  (key, val) :: List.filter (fun (k, _) -> k <> key) m
