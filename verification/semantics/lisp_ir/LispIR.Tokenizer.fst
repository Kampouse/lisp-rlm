module LispIR.Tokenizer
(** Fuel-based Tokenizer — F* Formal Verification

    Token types: LParen, RParen, Num, Sym, Bool, Str
    Tokenizer: fuel:int -> list char -> list token

    Fuel = string length. Each recursive call consumes 1 fuel.
    SMT proves termination trivially.

    NOTE: F* has NO infix multiplication. Use Prims.op_Multiply.
    NOTE: No `when` clauses in --verify mode. Use `if` in body.
*)

open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
module U32 = FStar.UInt32

// ============================================================
// TOKEN TYPE
// ============================================================

type token =
  | TkLParen
  | TkRParen
  | TkNum of int
  | TkSym of string
  | TkBool of bool
  | TkStr of string

// ============================================================
// CHARACTER CLASSIFICATION
// ============================================================

val is_whitespace : char -> bool
let is_whitespace c = c = ' ' || c = '\n' || c = '\t'

val is_digit : char -> bool
let is_digit c =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')

val is_paren : char -> bool
let is_paren c = c = '(' || c = ')'

val is_sym_char : char -> bool
let is_sym_char c =
  not (is_whitespace c) && not (is_paren c) && c <> '"'

val digit_val : char -> int
let digit_val c = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

// ============================================================
// STRING/LIST CONVERSION
// ============================================================

val string_to_list : string -> list char
let string_to_list s = list_of_string s

val list_to_string : list char -> string
let list_to_string cs = string_of_list cs

// ============================================================
// TOKENIZER COMPONENTS (fuel-based)
// ============================================================

// Skip leading whitespace
val skip_ws : fuel:int -> list char -> Tot (list char) (decreases fuel)
let rec skip_ws fuel cs =
  if fuel <= 0 then cs
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_whitespace c then skip_ws (fuel - 1) rest
    else cs

// Parse a number: consume digits, return (value, remaining)
val parse_num : fuel:int -> list char -> int -> Tot (int * (list char)) (decreases fuel)
let rec parse_num fuel cs acc =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c
    then parse_num (fuel - 1) rest (Prims.op_Multiply acc 10 + digit_val c)
    else (acc, cs)
  | [] -> (acc, [])

// Parse a symbol: consume sym chars, return (string, remaining)
val parse_sym : fuel:int -> list char -> list char -> Tot (string * (list char)) (decreases fuel)
let rec parse_sym fuel cs acc =
  if fuel <= 0 then (list_to_string (List.rev acc), cs)
  else match cs with
  | [] -> (list_to_string (List.rev acc), [])
  | c :: rest ->
    if is_sym_char c
    then parse_sym (fuel - 1) rest (c :: acc)
    else (list_to_string (List.rev acc), cs)

// Parse a string literal: consume until closing quote
val parse_string_lit : fuel:int -> list char -> list char -> Tot (string * (list char)) (decreases fuel)
let rec parse_string_lit fuel cs acc =
  if fuel <= 0 then (list_to_string (List.rev acc), cs)
  else match cs with
  | [] -> (list_to_string (List.rev acc), [])
  | c :: rest ->
    if c = '"'
    then (list_to_string (List.rev acc), rest)
    else parse_string_lit (fuel - 1) rest (c :: acc)

// ============================================================
// MAIN TOKENIZER (fuel-based)
// ============================================================

val tokenize : fuel:int -> list char -> Tot (list token) (decreases fuel)
let rec tokenize fuel cs =
  if fuel <= 0 then []
  else
    let cs = skip_ws fuel cs in
    match cs with
    | [] -> []
    | c :: rest ->
      if c = '(' then TkLParen :: tokenize (fuel - 1) rest
      else if c = ')' then TkRParen :: tokenize (fuel - 1) rest
      else if c = '"' then
        let (s, rest') = parse_string_lit (fuel - 1) rest [] in
        TkStr s :: tokenize (fuel - 1) rest'
      else if is_digit c then
        let (n, rest') = parse_num (fuel - 1) cs 0 in
        TkNum n :: tokenize (fuel - 1) rest'
      else
        let (sym, rest') = parse_sym (fuel - 1) cs [] in
        if sym = "true" then TkBool true :: tokenize (fuel - 1) rest'
        else if sym = "false" then TkBool false :: tokenize (fuel - 1) rest'
        else TkSym sym :: tokenize (fuel - 1) rest'

val tokenize_string : string -> list token
let tokenize_string s =
  let cs = string_to_list s in
  tokenize (FStar.List.Tot.length cs) cs
