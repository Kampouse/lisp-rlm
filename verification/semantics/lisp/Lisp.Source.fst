(** Lisp Source Language — F* Formal Specification

    Source-level reference evaluator with fuel-based termination.
    All recursive functions are top-level with explicit fuel for termination.
*)
module Lisp.Source

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

// === Environment Model ===
type env = list (string * lisp_val)

val env_get : string -> env -> Tot (option lisp_val)
let rec env_get name e =
  match e with
  | [] -> None
  | (k, v) :: rest -> if k = name then Some v else env_get name rest

val env_push : string -> lisp_val -> env -> Tot env
let env_push name v e = (name, v) :: e

// === Result type ===
noeq type eval_result =
  | Ok of lisp_val
  | Err of string

// === Evaluate a body sequence (return last expression) ===
val eval_body : fuel:int -> list lisp_val -> env -> Tot eval_result
  (decreases fuel)
let rec eval_body fuel body env =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match body with
  | [] -> Ok Nil
  | [last] -> eval_expr f last env
  | one :: rest ->
    (match eval_expr f one env with
    | Err m -> Err m
    | Ok _ -> eval_body f rest env)

// === Main evaluator ===
and eval_expr fuel expr env =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match expr with
  | Num n -> Ok (Num n)
  | Float fl -> Ok (Float fl)
  | Bool b -> Ok (Bool b)
  | Str s -> Ok (Str s)
  | Nil -> Ok Nil
  
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
    | [Sym "if"; test; then_br] ->
      (match eval_expr f test env with
      | Ok v -> if is_truthy v then eval_expr f then_br env else Ok Nil
      | Err m -> Err m)
    | [Sym "if"; test; then_br; else_br] ->
      (match eval_expr f test env with
      | Ok v -> if is_truthy v then eval_expr f then_br env else eval_expr f else_br env
      | Err m -> Err m)
    | Sym "let" :: List bindings :: body_exprs ->
      (match eval_let_seq f bindings env body_exprs with
      | r -> r)
    | [Sym "+"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (num_arith va vb op_int_add ff_add)
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "-"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (num_arith va vb op_int_sub ff_sub)
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "*"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (num_arith va vb int_mul ff_mul)
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "/"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (num_arith va vb int_div ff_div)
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "="; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (Bool (lisp_eq va vb))
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "<"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_lt op_int_lt))
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym ">"; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_gt op_int_gt))
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "<="; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_le op_int_le))
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym ">="; a; b] ->
      (match eval_expr f a env, eval_expr f b env with
      | Ok va, Ok vb -> Ok (Bool (num_cmp va vb ff_ge op_int_ge))
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "not"; a] ->
      (match eval_expr f a env with
      | Ok v -> Ok (Bool (not (is_truthy v)))
      | Err m -> Err m)
    | [Sym "nil?"; a] ->
      (match eval_expr f a env with
      | Ok Nil -> Ok (Bool true)
      | Ok _ -> Ok (Bool false)
      | Err m -> Err m)
    | [Sym "get"; map_expr; key_expr] ->
      (match eval_expr f map_expr env, eval_expr f key_expr env with
      | Ok map_v, Ok key_v ->
        (match key_v with
        | Str k -> Ok (dict_get k (val_of_dict map_v))
        | _ -> Ok Nil)
      | Err m, _ -> Err m
      | _, Err m -> Err m)
    | [Sym "set"; map_expr; key_expr; val_expr] ->
      (match eval_expr f map_expr env, eval_expr f key_expr env, eval_expr f val_expr env with
      | Ok map_v, Ok key_v, Ok val_v ->
        (match key_v with
        | Str k -> Ok (Dict (dict_set k val_v (val_of_dict map_v)))
        | _ -> Err "set: key not a string")
      | Err m, _, _ -> Err m
      | _, Err m, _ -> Err m
      | _, _, Err m -> Err m)
    | _ -> Err "unsupported expression")
  
  | _ -> Err "unsupported expression"

// === Let binding evaluator ===
// Separate top-level function for termination proof
and eval_let_seq fuel bindings env body =
  if fuel <= 0 then Err "out of fuel" else
  let f = fuel - 1 in
  match bindings with
  | [] -> eval_body f body env
  | List [Sym name; init] :: rest ->
    (match eval_expr f init env with
    | Ok v -> eval_let_seq f rest (env_push name v env) body
    | Err m -> Err m)
  | _ -> Err "let: bad binding"
