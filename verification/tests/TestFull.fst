module TestFull
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

type string_pair = string * (list char)
type tok =
  | TkL
  | TkR
  | TkN of int
  | TkS of string
  | TkB of bool
  | TkSt of string

let is_ws (c:char) : Tot bool = c = ' ' || c = '\n' || c = '\t'
let is_digit (c:char) : Tot bool =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')
let is_paren (c:char) : Tot bool = c = '(' || c = ')'
let is_sym_char (c:char) : Tot bool = not (is_ws c) && not (is_paren c) && c <> '"'
let dv (c:char) : Tot int = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_ws c then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else if c = '"' then
      let (s, r) = parse_str (fuel - 1) rest [] in
      TkSt s :: tokenize (fuel - 1) r
    else if is_digit c then
      let (n, r) = parse_num (fuel - 1) cs 0 in
      TkN n :: tokenize (fuel - 1) r
    else
      let (sym, r) = parse_sym (fuel - 1) cs [] in
      TkS sym :: tokenize (fuel - 1) r

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot (int * (list char)) (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c then parse_num (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else (acc, cs)
  | [] -> (acc, cs)

and parse_sym (fuel:int) (cs:list char) (acc:list char) : Tot string_pair (decreases fuel) =
  if fuel <= 0 then ("", cs)
  else match cs with
  | [] -> ("", cs)
  | c :: rest ->
    if is_sym_char c then parse_sym (fuel - 1) rest (c :: acc)
    else ("", cs)

and parse_str (fuel:int) (cs:list char) (acc:list char) : Tot string_pair (decreases fuel) =
  if fuel <= 0 then ("", cs)
  else match cs with
  | [] -> ("", cs)
  | '"' :: rest -> ("", rest)
  | c :: rest -> parse_str (fuel - 1) rest (c :: acc)
