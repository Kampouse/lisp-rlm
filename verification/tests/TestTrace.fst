module TestTrace
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
module U32 = FStar.UInt32

type tok =
  | TkL
  | TkR
  | TkN of int
  | TkS of string

type expr =
  | Num of int
  | Add of (expr * expr)

val is_ws : char -> bool
let is_ws c = c = ' ' || c = '
' || c = '	'

val is_digit : char -> bool
let is_digit c =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')

val is_sym : char -> bool
let is_sym c = not (is_ws c) && c <> '(' && c <> ')'

val dv : char -> int
let dv c = U32.v (u32_of_char c) - U32.v (u32_of_char '0')

let rec do_tok (fuel:int) (cs:list char) : Tot (list tok) (decreases fuel) =
  if fuel <= 0 then []
  else match cs with
  | [] -> []
  | c :: rest ->
    if is_ws c then do_tok (fuel - 1) rest
    else if c = '(' then TkL :: do_tok (fuel - 1) rest
    else if c = ')' then TkR :: do_tok (fuel - 1) rest
    else if is_digit c then
      let (n, r) = do_pn (fuel - 1) cs 0 in
      TkN n :: do_tok (fuel - 1) r
    else do_tok (fuel - 1) rest

and do_pn (fuel:int) (cs:list char) (acc:int) : Tot (int * (list char)) (decreases fuel) =
  if fuel <= 0 then (acc, cs)
  else match cs with
  | c :: rest ->
    if is_digit c then do_pn (fuel - 1) rest (Prims.op_Multiply acc 10 + dv c)
    else (acc, cs)
  | [] -> (acc, [])

let rec do_par (fuel:int) (toks:list tok) : Tot (option expr) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkN n :: [] -> Some (Num n)
  | TkL :: TkS "+" :: TkN a :: TkN b :: TkR :: [] -> Some (Add (Num a, Num b))
  | _ -> None

let rec do_ev (fuel:int) (e:expr) : Tot int (decreases fuel) =
  if fuel <= 0 then 0
  else match e with
  | Num n -> n
  | Add (a, b) -> do_ev (fuel - 1) a + do_ev (fuel - 1) b

val run : string -> Tot int
let run s =
  let cs = FStar.String.list_of_string s in
  let toks = do_tok 100 cs in
  match do_par 100 toks with
  | Some e -> do_ev 20 e
  | None -> 0

val t1 : unit -> Lemma (run "42" = 42)
let t1 () = assert_norm (run "42" = 42)

val t2 : unit -> Lemma (run "(+ 3 4)" = 7)
let t2 () = assert_norm (run "(+ 3 4)" = 7)

val t3 : unit -> Lemma (run "42" = 999)
let t3 () = assert_norm (run "42" = 999)
