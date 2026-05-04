module TestFalseE2E
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

type opcode =
  | OPush of int
  | OAdd

let is_ws (c:char) : Tot bool = c = ' '
let is_digit (c:char) : Tot bool =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')
let dv (c:char) : Tot int = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_ws c then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else
      let p = pn (fuel - 1) cs 0 in
      TkN (ic_fst p) :: tokenize (fuel - 1) (ic_snd p)

and pn (fuel:int) (cs:list char) (acc:int) : Tot int_and_chars (decreases fuel) =
  if fuel <= 0 then MkIC (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c then pn (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else MkIC (acc, cs)
  | [] -> MkIC (acc, [])

let rec parse_expr (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | TkN n :: [] -> Some (Num n)
  | TkL :: rest -> parse_compound (fuel - 1) rest
  | _ -> None

and parse_compound (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | TkS "+" :: TkN a :: TkN b :: TkR :: [] -> Some (Add (Num a, Num b))
  | _ -> None

let rec list_nth (fuel:int) (n:int) (l:list opcode) : Tot (option opcode) (decreases fuel) =
  if fuel <= 0 then None
  else match n, l with
  | 0, x :: _ -> Some x
  | _, [] -> None
  | _, _ :: rest -> list_nth (fuel - 1) (n - 1) rest

let rec compile (e:expr) : Tot (list opcode) =
  match e with
  | Num n -> [OPush n]
  | Add (a, b) -> compile a @ compile b @ [OAdd]
  | _ -> []

let rec vm (fuel:int) (code:list opcode) (pc:int) (stack:list int) : Tot (list int) (decreases fuel) =
  if fuel <= 0 then stack
  else match list_nth fuel pc code with
  | None -> stack
  | Some (OPush n) -> vm (fuel - 1) code (pc + 1) (n :: stack)
  | Some OAdd ->
    (match stack with
     | a :: b :: rest -> vm (fuel - 1) code (pc + 1) ((a + b) :: rest)
     | _ -> stack)

val run_vm : string -> Tot int
let run_vm s =
  let cs = list_of_string s in
  let toks = tokenize 100 cs in
  match parse_expr 100 toks with
  | Some e ->
    let code = compile e in
    let result = vm 200 code 0 [] in
    (match result with x :: _ -> x | _ -> 0)
  | None -> 0

// FALSE: 3 + 4 != 99
val t_bad : unit -> Lemma (run_vm "(+ 3 4)" = 99)
let t_bad () = assert_norm (run_vm "(+ 3 4)" = 99)
