module TestFull
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

type int_pair =
  | IP of int
  | IP of (list char)

type tok =
  | TkL
  | TkR
  | TkN of int
  | TkS of string
  | TkB of bool
  | TkSt of string

let is_ws (c:char) : Tot bool = c = ' ' || c = '
' || c = '	'
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

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot int_pair (decreases fuel) =
  if fuel <= 0 then IP acc
  else match cs with
  | c :: rest ->
    if is_digit c then parse_num (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else IP acc
  | [] -> IP acc

and parse_sym (fuel:int) (cs:list char) (acc:list char) : Tot int_pair (decreases fuel) =
  if fuel <= 0 then IP 0
  else match cs with
  | [] -> IP 0
  | c :: rest ->
    if is_sym_char c then parse_sym (fuel - 1) rest (c :: acc)
    else IP 0

and parse_str (fuel:int) (cs:list char) (acc:list char) : Tot int_pair (decreases fuel) =
  if fuel <= 0 then IP 0
  else match cs with
  | [] -> IP 0
  | '"' :: rest -> IP 0
  | c :: rest -> parse_str (fuel - 1) rest (c :: acc)
