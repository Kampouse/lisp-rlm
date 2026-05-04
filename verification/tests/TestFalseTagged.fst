module TestFalseTagged
(** False lemma test — tagged oracle must REJECT incorrect claims.
    If this passes verification, the oracle is broken.
    Run: fstar.exe -c --fuel 64 --ifuel 64 TestFalseTagged.fst
    Expected: Error 19 (verification failure), exit 1
*)

open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
open LispIR.Tagged
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

let is_ws (c:char) : Tot bool = c = ' '
let is_digit_char (c:char) : Tot bool =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')
let is_sym_char (c:char) : Tot bool = not (is_ws c) && c <> '(' && c <> ')'
let dv (c:char) : Tot int = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

let rec tokenize (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_ws c then tokenize (fuel - 1) rest
    else if c = '(' then TkL :: tokenize (fuel - 1) rest
    else if c = ')' then TkR :: tokenize (fuel - 1) rest
    else if is_digit_char c then
      let p = pn (fuel - 1) cs 0 in
      TkN (ic_fst p) :: tokenize (fuel - 1) (ic_snd p)
    else
      let p = ps (fuel - 1) cs [] in
      TkS (sc_fst p) :: tokenize (fuel - 1) (sc_snd p)

and pn (fuel:int) (cs:list char) (acc:int) : Tot int_and_chars (decreases fuel) =
  if fuel <= 0 then MkIC (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit_char c then pn (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else MkIC (acc, cs)
  | [] -> MkIC (acc, [])

and ps (fuel:int) (cs:list char) (acc:list char) : Tot str_and_chars (decreases fuel) =
  if fuel <= 0 then MkSC ("", cs)
  else match cs with
  | [] -> MkSC ("", [])
  | c :: rest ->
    if is_sym_char c then ps (fuel - 1) rest (c :: acc)
    else MkSC (string_of_list (List.rev acc), cs)

type opcode =
  | OPush of int
  | OAdd

let rec compile (e:expr) : Tot (list opcode) =
  match e with
  | Num n -> [OPush (make_num n)]
  | Add (a, b) -> compile a @ compile b @ [OAdd]
  | _ -> []

let rec list_nth_op (fuel:int) (n:int) (l:list opcode) : Tot (option opcode) (decreases fuel) =
  if fuel <= 0 then None
  else match n, l with
  | 0, x :: _ -> Some x
  | _, [] -> None
  | _, _ :: rest -> list_nth_op (fuel - 1) (n - 1) rest

let rec vm (fuel:int) (code:list opcode) (pc:int) (stack:list int) : Tot (list int) (decreases fuel) =
  if fuel <= 0 then stack
  else match list_nth_op fuel pc code with
  | None -> stack
  | Some (OPush n) -> vm (fuel - 1) code (pc + 1) (n :: stack)
  | Some OAdd ->
    (match stack with
     | a :: b :: rest -> vm (fuel - 1) code (pc + 1) ((tagged_add b a) :: rest)
     | _ -> stack)

let rec parse_expr (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkN n :: [] -> Some (Num n)
  | TkL :: rest -> parse_compound (fuel - 1) rest
  | _ -> None

and parse_compound (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | TkS "+" :: TkN a :: TkN b :: TkR :: [] ->
    Some (Add (Num a, Num b))
  | _ -> None

val run_vm : string -> Tot int
let run_vm s =
  let cs = list_of_string s in
  let toks = tokenize 200 cs in
  match parse_expr 200 toks with
  | Some e ->
    let code = compile e in
    let result = vm 500 code 0 [] in
    (match result with
     | x :: _ -> x
     | _ -> make_num 0)
  | None -> make_num 0

// FALSE CLAIM: 3 + 4 = 99 in tagged arithmetic
// This MUST be rejected by F*
val test_false : unit -> Lemma (run_vm "(+ 3 4)" = make_num 99)
let test_false () = assert_norm (run_vm "(+ 3 4)" = make_num 99)
