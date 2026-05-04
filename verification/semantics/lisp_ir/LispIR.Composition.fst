module LispIR.Composition
(** End-to-end composition: tokenize → parse → eval

    Self-contained in one module — no cross-module calls,
    so SMT can see every body and normalizer can unfold everything.

    Uses the shared AST types from LispIR.AST.
*)

open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

// ============================================================
// HELPER TYPES — avoid * in Tot return annotations
// (F* parses * as intersection inside refinement contexts)
// ============================================================

type int_and_chars =
  | MkIC of (int * (list char))

type str_and_chars =
  | MkSC of (string * (list char))

let ic_fst (x:int_and_chars) : Tot int = match x with MkIC (a, _) -> a
let ic_snd (x:int_and_chars) : Tot (list char) = match x with MkIC (_, b) -> b
let sc_fst (x:str_and_chars) : Tot string = match x with MkSC (a, _) -> a
let sc_snd (x:str_and_chars) : Tot (list char) = match x with MkSC (_, b) -> b

// ============================================================
// TOKEN TYPE
// ============================================================

type tok =
  | TkL
  | TkR
  | TkN of int
  | TkS of string
  | TkB of bool
  | TkSt of string

// ============================================================
// CHARACTER HELPERS
// ============================================================

let is_ws (c:char) : Tot bool = c = ' ' || c = '\n' || c = '\t'

let is_digit (c:char) : Tot bool =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')

let is_paren (c:char) : Tot bool = c = '(' || c = ')'

let is_sym_char (c:char) : Tot bool =
  not (is_ws c) && not (is_paren c) && c <> '"'

let dv (c:char) : Tot int = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

// ============================================================
// TOKENIZER (fuel-based)
// ============================================================

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_ws c then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else if c = '#' then
      (match rest with
       | 't' :: r2 -> TkB true :: tokenize (fuel - 1) r2
       | 'f' :: r2 -> TkB false :: tokenize (fuel - 1) r2
       | _ -> tokenize (fuel - 1) rest)
    else if c = '"' then
      let p = parse_str (fuel - 1) rest [] in
      TkSt (sc_fst p) :: tokenize (fuel - 1) (sc_snd p)
    else if is_digit c then
      let p = parse_num (fuel - 1) cs 0 in
      TkN (ic_fst p) :: tokenize (fuel - 1) (ic_snd p)
    else
      let p = parse_sym (fuel - 1) cs [] in
      TkS (sc_fst p) :: tokenize (fuel - 1) (sc_snd p)

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot int_and_chars (decreases fuel) =
  if fuel <= 0 then MkIC (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c
    then parse_num (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else MkIC (acc, cs)
  | [] -> MkIC (acc, [])

and parse_sym (fuel:int) (cs:list char) (acc:list char) : Tot str_and_chars (decreases fuel) =
  if fuel <= 0 then MkSC ("", cs)
  else match cs with
  | [] -> MkSC ("", [])
  | c :: rest ->
    if is_sym_char c then parse_sym (fuel - 1) rest (c :: acc)
    else MkSC (string_of_list (List.rev acc), cs)

and parse_str (fuel:int) (cs:list char) (acc:list char) : Tot str_and_chars (decreases fuel) =
  if fuel <= 0 then MkSC ("", cs)
  else match cs with
  | [] -> MkSC ("", [])
  | '"' :: rest -> MkSC (string_of_list (List.rev acc), rest)
  | c :: rest -> parse_str (fuel - 1) rest (c :: acc)

// ============================================================
// PARSER (fuel-based, mutually recursive)
// ============================================================

let rec parse_expr (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkN n :: [] -> Some (Num n)
  | TkB b :: [] -> Some (Bool b)
  | TkSt s :: [] -> Some (Str s)
  | TkS name :: [] -> Some (Sym name)
  | TkL :: rest -> parse_compound (fuel - 1) rest
  | _ -> None

and parse_compound (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | TkS "+" :: TkN a :: TkN b :: TkR :: [] -> Some (Add (Num a, Num b))
  | TkS "-" :: TkN a :: TkN b :: TkR :: [] -> Some (Sub (Num a, Num b))
  | TkS "neg" :: TkN a :: TkR :: [] -> Some (Neg (Num a))
  | TkS "if-gt" :: TkN a :: TkN b :: TkN t :: TkN el :: TkR :: [] ->
    Some (IfGt (Num a, Num b, Num t, Num el))
  | _ -> None

// ============================================================
// EVAL (fuel-based)
// ============================================================

let rec eval_expr (fuel:int) (e:expr) : Tot int (decreases fuel) =
  if fuel <= 0 then 0
  else match e with
  | Num n -> n
  | Add (a, b) -> eval_expr (fuel - 1) a + eval_expr (fuel - 1) b
  | Sub (a, b) -> eval_expr (fuel - 1) a - eval_expr (fuel - 1) b
  | Neg a -> 0 - eval_expr (fuel - 1) a
  | IfGt (ca, cb, t, el) ->
    let cv = eval_expr (fuel - 1) ca in
    let bv = eval_expr (fuel - 1) cb in
    if cv > bv then eval_expr (fuel - 1) t else eval_expr (fuel - 1) el
  | Let (_name, _val_e, body) -> eval_expr (fuel - 1) body
  | Bool _ -> 0
  | Str _ -> 0
  | Sym _ -> 0

// ============================================================
// FULL CHAIN: string → tokenize → parse → eval
// ============================================================

val run : string -> Tot int
let run s =
  let cs = list_of_string s in
  let toks = tokenize 100 cs in
  match parse_expr 100 toks with
  | Some e -> eval_expr 20 e
  | None -> 0

// ============================================================
// TESTS — assert_norm through the full chain
// ============================================================

val test_num : unit -> Lemma (run "42" = 42)
let test_num () = assert_norm (run "42" = 42)

val test_add : unit -> Lemma (run "(+ 3 4)" = 7)
let test_add () = assert_norm (run "(+ 3 4)" = 7)

val test_sub : unit -> Lemma (run "(- 10 3)" = 7)
let test_sub () = assert_norm (run "(- 10 3)" = 7)

val test_neg : unit -> Lemma (run "(neg 5)" = -5)
let test_neg () = assert_norm (run "(neg 5)" = -5)

val test_if_gt_true : unit -> Lemma (run "(if-gt 5 3 10 20)" = 10)
let test_if_gt_true () = assert_norm (run "(if-gt 5 3 10 20)" = 10)

val test_if_gt_false : unit -> Lemma (run "(if-gt 3 5 10 20)" = 20)
let test_if_gt_false () = assert_norm (run "(if-gt 3 5 10 20)" = 20)
