module TestAlias

#set-options "--z3rlimit 100"
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

type tok = | TkL | TkR | TkN of int | TkS of string | TkSt of string

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if c = ' ' then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else if c = '"' then
      tokenize_after_str (fuel - 1) (parse_str (fuel - 1) rest [])
    else
      tokenize_after_num (fuel - 1) (parse_num (fuel - 1) cs 0)

and tokenize_after_str (fuel:int) (sr:string & list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else TkSt (fst sr) :: tokenize (fuel - 1) (snd sr)

and tokenize_after_num (fuel:int) (nr:int & list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else TkN (fst nr) :: tokenize (fuel - 1) (snd nr)

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot (int & list char) (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | _ :: rest -> parse_num (fuel - 1) rest acc
  | [] -> (acc, cs)

and parse_str (fuel:int) (cs:list char) (acc:list char) : Tot (string & list char) (decreases fuel) =
  if fuel <= 0 then ("", cs)
  else match cs with
  | [] -> ("", [])
  | '"' :: rest -> ("", rest)
  | c :: rest -> parse_str (fuel - 1) rest (c :: acc)
