module TestFalseComp
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

type int_and_chars =
  | MkIC of (int * (list char))

type str_and_chars =
  | MkSC of (string * (list char))

let ic_fst (x:int_and_chars) : Tot int = match x with MkIC (a, _) -> a
let ic_snd (x:int_and_chars) : Tot (list char) = match x with MkIC (_, b) -> b
let sc_fst (x:str_and_chars) : Tot string = match x with MkSC (a, _) -> a
let sc_snd (x:str_and_chars) : Tot (list char) = match x with MkSC (_, b) -> b

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
    else if is_digit c then
      let p = parse_num (fuel - 1) cs 0 in
      TkN (ic_fst p) :: tokenize (fuel - 1) (ic_snd p)
    else tokenize (fuel - 1) rest

and parse_num (fuel:int) (cs:list char) (acc:int) : Tot int_and_chars (decreases fuel) =
  if fuel <= 0 then MkIC (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c
    then parse_num (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else MkIC (acc, cs)
  | [] -> MkIC (acc, [])

let rec parse_expr (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | TkN n :: [] -> Some (Num n)
  | _ -> None

let rec eval_expr (fuel:int) (e:expr) : Tot int (decreases fuel) =
  if fuel <= 0 then 0
  else match e with
  | Num n -> n
  | _ -> 0

val run : string -> Tot int
let run s =
  let cs = list_of_string s in
  let toks = tokenize 100 cs in
  match parse_expr 100 toks with
  | Some e -> eval_expr 20 e
  | None -> 0

// CORRECT
val t_ok : unit -> Lemma (run "42" = 42)
let t_ok () = assert_norm (run "42" = 42)

// FALSE — should fail
val t_bad : unit -> Lemma (run "42" = 999)
let t_bad () = assert_norm (run "42" = 999)
