module TestFalse
open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
open LispIR.Tokenizer
open LispIR.Parser
module U32 = FStar.UInt32

val eval_expr : fuel:int -> expr -> Tot int (decreases fuel)
let rec eval_expr fuel e =
  if fuel <= 0 then 0
  else match e with
  | Num n -> n
  | Add (a, b) -> eval_expr (fuel - 1) a + eval_expr (fuel - 1) b
  | _ -> 0

val eval_string : string -> Tot int
let eval_string s =
  let toks = tokenize_string s in
  match parse toks with
  | Some e -> eval_expr 20 e
  | None -> 0

val test_wrong : unit -> Lemma (eval_string "42" = 999)
let test_wrong () = assert_norm (eval_string "42" = 999)
