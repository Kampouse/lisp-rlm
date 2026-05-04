module TestAlias
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

type char_pair = string * (list char)
type int_pair = int * (list char)
type tok = TkL | TkR | TkN of int | TkS of string | TkB of bool | TkSt of string

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if c = ' ' then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else if c = '"' then
      let (s, r) = parse_str (fuel - 1) rest in
      TkSt s :: tokenize (fuel - 1) r
    else
      let (n, r) = parse_num (fuel - 1) cs 0 in
      TkN n :: tokenize (fuel - 1) r

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot int_pair (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | c :: rest -> parse_num (fuel - 1) rest acc
  | [] -> (acc, cs)

and parse_str (fuel:int) (cs:list char) (acc:list char) : Tot char_pair (decreases fuel) =
  if fuel <= 0 then ("", cs)
  else match cs with
  | [] -> ("", [])
  | '"' :: rest -> ("", rest)
  | c :: rest -> parse_str (fuel - 1) rest (c :: acc)
