(** Closure Support -- F* Formal Specification

    Adds first-class functions (closures) to the verified Lisp subset.
    Two evaluation modes:
    - apply_lambda: non-recursive closures (no self-call)
    - apply_lambda_rec: recursive closures (supports (self args...))
    
    KEY DESIGN: eval_body_self is self-recursive (no mutual recursion).
    Self-call binds params to args inline, then recurses directly.
    This makes F* normalizer able to compute concrete recursive calls.
*)
module Lisp.Closure

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics

// === Helper: bind params to args ===

val bind_params : list string -> list lisp_val -> list (string * lisp_val)
  -> Tot (list (string * lisp_val))
let rec bind_params params args env =
  match params, args with
  | [], _ -> env
  | p :: prest, a :: arest -> bind_params prest arest ((p, a) :: env)
  | _ -> env

// === Non-recursive closure evaluation ===

val apply_lambda : fuel:int -> list string -> lisp_val -> list lisp_val
  -> list (string * lisp_val) -> Tot (option lisp_val)
  (decreases fuel)
let rec apply_lambda fuel params body args env =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match params, args with
  | [], [] ->
    Some (match eval_body_closure f body env with
          | Some v -> v
          | None -> Nil)
  | p :: prest, a :: arest ->
    apply_lambda f prest body arest ((p, a) :: env)
  | _ -> None

and eval_body_closure fuel expr env =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match expr with
  | Num n -> Some (Num n)
  | Bool b -> Some (Bool b)
  | Nil -> Some Nil
  | Sym name ->
    (match name with
    | "true" -> Some (Bool true)
    | "false" -> Some (Bool false)
    | "nil" -> Some Nil
    | _ -> (match env_get name env with
            | Some v -> Some v
            | None -> None))
  | List [Sym "+"; a; b] ->
    (match eval_body_closure f a env, eval_body_closure f b env with
     | Some va, Some vb -> Some (num_arith va vb op_int_add ff_add)
     | _ -> None)
  | List [Sym "-"; a; b] ->
    (match eval_body_closure f a env, eval_body_closure f b env with
     | Some va, Some vb -> Some (num_arith va vb op_int_sub ff_sub)
     | _ -> None)
  | List [Sym ">"; a; b] ->
    (match eval_body_closure f a env, eval_body_closure f b env with
     | Some va, Some vb -> Some (Bool (num_cmp va vb ff_gt op_int_gt))
     | _ -> None)
  | List [Sym "<"; a; b] ->
    (match eval_body_closure f a env, eval_body_closure f b env with
     | Some va, Some vb -> Some (Bool (num_cmp va vb ff_lt op_int_lt))
     | _ -> None)
  | List [Sym "="; a; b] ->
    (match eval_body_closure f a env, eval_body_closure f b env with
     | Some va, Some vb -> Some (Bool (lisp_eq va vb))
     | _ -> None)
  | List [Sym "if"; test; then_br; else_br] ->
    (match eval_body_closure f test env with
     | Some v -> if is_truthy v
                 then eval_body_closure f then_br env
                 else eval_body_closure f else_br env
     | None -> None)
  | List [Sym "if"; test; then_br] ->
    (match eval_body_closure f test env with
     | Some v -> if is_truthy v
                 then eval_body_closure f then_br env
                 else Some Nil
     | None -> None)
  | List (Sym "let" :: List bindings :: body_exprs) ->
    eval_let_closure f bindings env body_exprs
  | _ -> None

and eval_let_closure fuel bindings env body =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match bindings with
  | [] -> (match body with
           | [last] -> eval_body_closure f last env
           | _ -> None)
  | List [Sym name; init] :: rest ->
    (match eval_body_closure f init env with
     | Some v -> eval_let_closure f rest ((name, v) :: env) body
     | None -> None)
  | _ -> None

// === Recursive closure evaluation (self-call support) ===
// KEY: eval_body_self is self-recursive only, no mutual recursion.
// On (self args...), we bind params to the evaluated args and recurse.

val eval_body_self : fuel:int -> lisp_val -> list (string * lisp_val)
  -> list string -> lisp_val
  -> Tot (option lisp_val)
  (decreases fuel)
let rec eval_body_self fuel expr env params body =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match expr with
  | Num n -> Some (Num n)
  | Bool b -> Some (Bool b)
  | Nil -> Some Nil
  | Sym name ->
    (match name with
    | "true" -> Some (Bool true)
    | "false" -> Some (Bool false)
    | "nil" -> Some Nil
    | _ -> (match env_get name env with
            | Some v -> Some v
            | None -> None))
  | List [Sym "+"; a; b] ->
    (match eval_body_self f a env params body, eval_body_self f b env params body with
     | Some va, Some vb -> Some (num_arith va vb op_int_add ff_add)
     | _ -> None)
  | List [Sym "-"; a; b] ->
    (match eval_body_self f a env params body, eval_body_self f b env params body with
     | Some va, Some vb -> Some (num_arith va vb op_int_sub ff_sub)
     | _ -> None)
  | List [Sym ">"; a; b] ->
    (match eval_body_self f a env params body, eval_body_self f b env params body with
     | Some va, Some vb -> Some (Bool (num_cmp va vb ff_gt op_int_gt))
     | _ -> None)
  | List [Sym "<"; a; b] ->
    (match eval_body_self f a env params body, eval_body_self f b env params body with
     | Some va, Some vb -> Some (Bool (num_cmp va vb ff_lt op_int_lt))
     | _ -> None)
  | List [Sym "="; a; b] ->
    (match eval_body_self f a env params body, eval_body_self f b env params body with
     | Some va, Some vb -> Some (Bool (lisp_eq va vb))
     | _ -> None)
  | List [Sym "if"; test; then_br; else_br] ->
    (match eval_body_self f test env params body with
     | Some v -> if is_truthy v
                 then eval_body_self f then_br env params body
                 else eval_body_self f else_br env params body
     | None -> None)
  | List [Sym "if"; test; then_br] ->
    (match eval_body_self f test env params body with
     | Some v -> if is_truthy v
                 then eval_body_self f then_br env params body
                 else Some Nil
     | None -> None)
  // Self-call: bind params to args, then recurse on body
  | List [Sym "self"; arg_expr] ->
    (match eval_body_self f arg_expr env params body with
     | Some arg_val ->
       let new_env = bind_params params [arg_val] [] in
       eval_body_self f body new_env params body
     | None -> None)
  | List [Sym "self"; arg1_expr; arg2_expr] ->
    (match eval_body_self f arg1_expr env params body, eval_body_self f arg2_expr env params body with
     | Some a1, Some a2 ->
       let new_env = bind_params params [a1; a2] [] in
       eval_body_self f body new_env params body
     | _ -> None)
  | List (Sym "let" :: List bindings :: body_exprs) ->
    eval_let_self f bindings env params body body_exprs
  | _ -> None

and eval_let_self fuel bindings env params body_fn body =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match bindings with
  | [] -> (match body with
           | [last] -> eval_body_self f last env params body_fn
           | _ -> None)
  | List [Sym name; init] :: rest ->
    (match eval_body_self f init env params body_fn with
     | Some v -> eval_let_self f rest ((name, v) :: env) params body_fn body
     | None -> None)
  | _ -> None

// Wrapper: apply_lambda_rec just binds params and calls eval_body_self
val apply_lambda_rec : fuel:int -> list string -> lisp_val -> list lisp_val
  -> list (string * lisp_val) -> Tot (option lisp_val)
  (decreases fuel)
let apply_lambda_rec fuel params body args env =
  let bound_env = bind_params params args env in
  eval_body_self fuel body bound_env params body
