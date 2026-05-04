module TestEvalDef
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
  | Sub (a, b) -> eval_expr (fuel - 1) a - eval_expr (fuel - 1) b
  | Neg a -> 0 - eval_expr (fuel - 1) a
  | IfGt (ca, cb, t, el) ->
    let cv = eval_expr (fuel - 1) ca in
    let bv = eval_expr (fuel - 1) cb in
    if cv > bv then eval_expr (fuel - 1) t else eval_expr (fuel - 1) el
  | Let (_name, _val_e, body) -> eval_expr (fuel - 1) body
  | Bool _ -> 0
  | Str _ -> 0
  | Sym _ -> 0

val eval_string : string -> Tot int
let eval_string s =
  let toks = tokenize_string s in
  match parse toks with
  | Some e -> eval_expr 20 e
  | None -> 0
