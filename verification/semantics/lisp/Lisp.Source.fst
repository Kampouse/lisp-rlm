(** Lisp Source Language -- F* Formal Specification

    Source-level reference evaluator with fuel-based termination.
    Supports: atoms, arithmetic, if, let, fn (lambda), function call, map, quote.
*)
module Lisp.Source

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

type env = list (string * lisp_val)

val env_get : string -> env -> Tot (option lisp_val)
let rec env_get name e =
  match e with
  | [] -> None
  | (k, v) :: rest -> if k = name then Some v else env_get name rest

val env_push : string -> lisp_val -> env -> Tot env
let env_push name v e = (name, v) :: e

noeq type eval_result =
  | Ok of lisp_val
  | Err of string

let rec eval_body (fuel:int) (body:list lisp_val) (env:env) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match body with
  | [] -> Ok Nil
  | [last] -> eval_expr f last env
  | one :: rest ->
    (match eval_expr f one env with
    | Err m -> Err m
    | Ok _ -> eval_body f rest env)

and apply_fn (fuel:int) (params:list string) (body:lisp_val) (closure_env:list (string * lisp_val)) (args:list lisp_val) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match params, args with
  | [], [] -> eval_expr f body closure_env
  | p :: prest, a :: arest ->
    apply_fn f prest body ((p, a) :: closure_env) arest
  | _, _ -> Err "arity mismatch"

and eval_let_seq (fuel:int) (bindings:list lisp_val) (env:env) (body:list lisp_val) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match bindings with
  | [] -> eval_body f body env
  | List [Sym name; init] :: rest ->
    (match eval_expr f init env with
    | Ok v -> eval_let_seq f rest (env_push name v env) body
    | Err m -> Err m)
  | _ -> Err "let: bad binding"

and eval_expr (fuel:int) (expr:lisp_val) (env:env) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match expr with
  | Num n -> Ok (Num n)
  | Float fl -> Ok (Float fl)
  | Bool b -> Ok (Bool b)
  | Str s -> Ok (Str s)
  | Nil -> Ok Nil
  | Lambda _ -> Ok expr
  
  | Sym name ->
    (match name with
    | "true" -> Ok (Bool true)
    | "false" -> Ok (Bool false)
    | "nil" -> Ok Nil
    | _ -> (match env_get name env with
      | Some v -> Ok v
      | None -> Err ("undefined: " ^ name)))
  
  | List elems ->
    (match elems with
    | [] -> Ok Nil
    | [Sym "quote"; x] -> Ok x

    | Sym "fn" :: List [Sym p] :: [single_body] ->
      Ok (Lambda ([p], single_body, env))
    | Sym "fn" :: List [Sym p1; Sym p2] :: [single_body] ->
      Ok (Lambda ([p1; p2], single_body, env))
    | Sym "fn" :: List [] :: [single_body] ->
      Ok (Lambda ([], single_body, env))
    | Sym "fn" :: _ -> Err "fn: unsupported form"

    | [Sym "map"; func_expr; list_expr] ->
      (match eval_expr f func_expr env, eval_expr f list_expr env with
       | Ok (Lambda (params, body, closure_env)), Ok (List lst) ->
         eval_map_list f params body closure_env lst
       | Ok _, Ok Nil -> Ok (List [])
       | Ok _, Ok (List []) -> Ok (List [])
       | Ok _, Ok _ -> Err "map: second arg must be a list"
       | Err m, _ -> Err m
       | _, Err m -> Err m)

    | Sym fname :: _ ->
      (match fname with
       | "+" | "-" | "*" | "/" | "=" | "<" | ">" | "<=" | ">="
       | "if" | "let" | "fn" | "quote" | "map" | "not" | "nil?" | "get" | "set" ->
         eval_special f elems env
       | _ ->
         (match env_get fname env with
          | Some (Lambda (params, body, closure_env)) ->
            eval_apply_args f params body closure_env (match elems with _ :: rest -> rest | [] -> []) env
          | Some _ -> Err ("not a function: " ^ fname)
          | None -> Err ("undefined: " ^ fname)))

    | _ -> eval_special f elems env)

  | _ -> Err "unsupported expression"

and eval_special (fuel:int) (elems:list lisp_val) (env:env) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match elems with
  | [Sym "+"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (num_arith va vb op_int_add ff_add)
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "-"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (num_arith va vb op_int_sub ff_sub)
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "*"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (num_arith va vb int_mul ff_mul)
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "/"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (num_arith va vb int_div ff_div)
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "="; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (Bool (lisp_eq va vb))
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "<"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_lt op_int_lt))
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym ">"; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_gt op_int_gt))
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "<="; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_le op_int_le))
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym ">="; a; b] ->
    (match eval_expr f a env, eval_expr f b env with
    | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_ge op_int_ge))
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "if"; test; then_br] ->
    (match eval_expr f test env with
    | Ok v -> if is_truthy v then eval_expr f then_br env else Ok Nil
    | Err m -> Err m)
  | [Sym "if"; test; then_br; else_br] ->
    (match eval_expr f test env with
    | Ok v -> if is_truthy v then eval_expr f then_br env else eval_expr f else_br env
    | Err m -> Err m)
  | Sym "let" :: List bindings :: body_exprs ->
    eval_let_seq f bindings env body_exprs
  | [Sym "not"; a] ->
    (match eval_expr f a env with
    | Ok v -> Ok (Bool (not (is_truthy v))) | Err m -> Err m)
  | [Sym "nil?"; a] ->
    (match eval_expr f a env with
    | Ok Nil -> Ok (Bool true) | Ok _ -> Ok (Bool false) | Err m -> Err m)
  | [Sym "get"; map_expr; key_expr] ->
    (match eval_expr f map_expr env, eval_expr f key_expr env with
    | Ok map_v, Ok key_v ->
      (match key_v with
      | Str k -> Ok (dict_get k (val_of_dict map_v)) | _ -> Ok Nil)
    | Err m, _ -> Err m | _, Err m -> Err m)
  | [Sym "set"; map_expr; key_expr; val_expr] ->
    (match eval_expr f map_expr env, eval_expr f key_expr env, eval_expr f val_expr env with
    | Ok map_v, Ok key_v, Ok val_v ->
      (match key_v with
      | Str k -> Ok (Dict (dict_set k val_v (val_of_dict map_v)))
      | _ -> Err "set: key not a string")
    | Err m, _, _ -> Err m | _, Err m, _ -> Err m | _, _, Err m -> Err m)
  | _ -> Err "unsupported expression"

and eval_apply_args (fuel:int) (params:list string) (body:lisp_val) (closure_env:list (string * lisp_val)) (args:list lisp_val) (env:env) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match args with
  | [] -> apply_fn f params body closure_env []
  | a :: rest ->
    (match eval_expr f a env with
    | Ok _ ->
      (match eval_apply_args f params body closure_env rest env with
       | Ok result -> Ok result | Err m -> Err m)
    | Err m -> Err m)

and eval_map_list (fuel:int) (params:list string) (body:lisp_val) (closure_env:list (string * lisp_val)) (lst:list lisp_val) : eval_result =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match lst with
  | [] -> Ok (List [])
  | elem :: rest ->
    (match apply_fn f params body closure_env [elem] with
    | Ok result ->
      (match eval_map_list f params body closure_env rest with
       | Ok (List rest_results) -> Ok (List (result :: rest_results))
       | Ok _ -> Err "map: internal error" | Err m -> Err m)
    | Err m -> Err m)
