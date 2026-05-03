(** Vector (List) Properties -- F* Formal Verification

    Proves structural properties of the List vector type using assert_norm.
    
    IMPORTANT: lisp_val is noeq, so we cannot write `Some (Num n) = Some (Num n)`
    directly. Instead we match on the option and extract the int field, then
    compare the int (which IS an eqtype).
*)
module VectorProperties

open Lisp.Types
open Lisp.Values

// ============================================================
// Section 1: Length
// ============================================================

val len_empty : unit -> Lemma
  (list_len (List []) = 0)
let len_empty () = assert_norm (list_len (List []) = 0)

val len_one : n:int -> Lemma
  (list_len (List [Num n]) = 1)
let len_one n = assert_norm (list_len (List [Num n]) = 1)

val len_two : a:int -> b:int -> Lemma
  (list_len (List [Num a; Num b]) = 2)
let len_two a b = assert_norm (list_len (List [Num a; Num b]) = 2)

val len_three : a:int -> b:int -> c:int -> Lemma
  (list_len (List [Num a; Num b; Num c]) = 3)
let len_three a b c = assert_norm (list_len (List [Num a; Num b; Num c]) = 3)

// ============================================================
// Section 2: Nth (avoid noeq by comparing extracted ints)
// ============================================================

// First element of singleton list contains the right int
val nth_one_zero : n:int -> Lemma
  (match list_nth (List [Num n]) 0 with
   | Some (Num m) -> m = n
   | _ -> false)
let nth_one_zero n =
  assert_norm (
    match list_nth (List [Num n]) 0 with
    | Some (Num m) -> m = n
    | _ -> false)

// First element of two-element list
val nth_two_first : a:int -> b:int -> Lemma
  (match list_nth (List [Num a; Num b]) 0 with
   | Some (Num m) -> m = a
   | _ -> false)
let nth_two_first a b =
  assert_norm (
    match list_nth (List [Num a; Num b]) 0 with
    | Some (Num m) -> m = a
    | _ -> false)

// Second element of two-element list
val nth_two_second : a:int -> b:int -> Lemma
  (match list_nth (List [Num a; Num b]) 1 with
   | Some (Num m) -> m = b
   | _ -> false)
let nth_two_second a b =
  assert_norm (
    match list_nth (List [Num a; Num b]) 1 with
    | Some (Num m) -> m = b
    | _ -> false)

// Out of bounds returns None
val nth_two_oob : a:int -> b:int -> Lemma
  (match list_nth (List [Num a; Num b]) 2 with None -> true | _ -> false)
let nth_two_oob a b =
  assert_norm (
    match list_nth (List [Num a; Num b]) 2 with
    | None -> true
    | _ -> false)

// Middle element of three-element list
val nth_three_mid : a:int -> b:int -> c:int -> Lemma
  (match list_nth (List [Num a; Num b; Num c]) 1 with
   | Some (Num m) -> m = b
   | _ -> false)
let nth_three_mid a b c =
  assert_norm (
    match list_nth (List [Num a; Num b; Num c]) 1 with
    | Some (Num m) -> m = b
    | _ -> false)

// ============================================================
// Section 3: Empty / truthiness
// ============================================================

val empty_true : unit -> Lemma
  (list_empty (List []) = true)
let empty_true () = assert_norm (list_empty (List []) = true)

val empty_false : n:int -> Lemma
  (list_empty (List [Num n]) = false)
let empty_false n = assert_norm (list_empty (List [Num n]) = false)

val nil_not_empty : unit -> Lemma
  (list_empty Nil = false)
let nil_not_empty () = assert_norm (list_empty Nil = false)

// ============================================================
// Section 4: Cons
// ============================================================

val cons_empty : n:int -> Lemma
  (list_len (list_cons (Num n) (List [])) = 1)
let cons_empty n = assert_norm (list_len (list_cons (Num n) (List [])) = 1)

val cons_increments : a:int -> b:int -> Lemma
  (list_len (list_cons (Num a) (List [Num b])) = 2)
let cons_increments a b = assert_norm (list_len (list_cons (Num a) (List [Num b])) = 2)

// Cons prepends: first element is the prepended value
val cons_prepends : a:int -> b:int -> Lemma
  (match list_nth (list_cons (Num a) (List [Num b])) 0 with
   | Some (Num m) -> m = a
   | _ -> false)
let cons_prepends a b =
  assert_norm (
    match list_nth (list_cons (Num a) (List [Num b])) 0 with
    | Some (Num m) -> m = a
    | _ -> false)

// Cons preserves tail
val cons_preserves_tail : a:int -> b:int -> c:int -> Lemma
  (match list_nth (list_cons (Num a) (List [Num b; Num c])) 1 with
   | Some (Num m) -> m = b
   | _ -> false)
let cons_preserves_tail a b c =
  assert_norm (
    match list_nth (list_cons (Num a) (List [Num b; Num c])) 1 with
    | Some (Num m) -> m = b
    | _ -> false)

// ============================================================
// Section 5: Non-list values return length 0
// ============================================================

val len_non_list_num : unit -> Lemma
  (list_len (Num 42) = 0)
let len_non_list_num () = assert_norm (list_len (Num 42) = 0)

val len_non_list_nil : unit -> Lemma
  (list_len Nil = 0)
let len_non_list_nil () = assert_norm (list_len Nil = 0)

val len_non_list_str : unit -> Lemma
  (list_len (Str "hi") = 0)
let len_non_list_str () = assert_norm (list_len (Str "hi") = 0)
