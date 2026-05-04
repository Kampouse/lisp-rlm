module LispIR.Parser
(** Fuel-based Parser — F* Formal Verification

    Tokens -> AST for the mini language.

    Grammar:
      expr    = atom | '(' compound ')'
      atom    = num | bool | str | sym
      compound = ('+' expr expr)
              | ('-' expr expr)
              | ('neg' expr)
              | ('if-gt' expr expr expr expr)
              | ('let' sym expr expr)

    Returns option (expr * remaining_tokens).
    Fuel = number of tokens remaining.
    Each recursive call consumes 1 fuel.

    NOTE: Higher-order arguments (mk) cause SMT termination issues.
    Constructors are inlined per operator.
*)

open FStar.List.Tot
open FStar.Pervasives

// ============================================================
// TYPES
// ============================================================

type expr =
  | ENum of int
  | EAdd of (expr * expr)
  | ESub of (expr * expr)
  | ENeg of expr
  | EIfGt of (expr * expr * expr * expr)
  | ELet of (string * expr * expr)
  | EBool of bool
  | EStr of string
  | ESym of string

type token =
  | TkLParen
  | TkRParen
  | TkNum of int
  | TkSym of string
  | TkBool of bool
  | TkStr of string

// ============================================================
// PARSER — mutually recursive with fuel
// ============================================================

let rec parse_expr (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkNum n :: rest -> Some (ENum n, rest)
  | TkBool b :: rest -> Some (EBool b, rest)
  | TkStr s :: rest -> Some (EStr s, rest)
  | TkSym name :: rest ->
    if name = "+" || name = "-" || name = "neg"
    || name = "if-gt" || name = "let"
    then None
    else Some (ESym name, rest)
  | TkLParen :: rest -> parse_compound (fuel - 1) rest
  | TkRParen :: _ -> None

and parse_compound (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkSym "+" :: args -> parse_add (fuel - 1) args
  | TkSym "-" :: args -> parse_sub (fuel - 1) args
  | TkSym "neg" :: args -> parse_neg (fuel - 1) args
  | TkSym "if-gt" :: args -> parse_if_gt (fuel - 1) args
  | TkSym "let" :: args -> parse_let (fuel - 1) args
  | _ -> None

and parse_add (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else
    match parse_expr (fuel - 1) toks with
    | None -> None
    | Some (a, r1) ->
      match parse_expr (fuel - 1) r1 with
      | None -> None
      | Some (b, r2) ->
        match r2 with
        | TkRParen :: r3 -> Some (EAdd (a, b), r3)
        | _ -> None

and parse_sub (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else
    match parse_expr (fuel - 1) toks with
    | None -> None
    | Some (a, r1) ->
      match parse_expr (fuel - 1) r1 with
      | None -> None
      | Some (b, r2) ->
        match r2 with
        | TkRParen :: r3 -> Some (ESub (a, b), r3)
        | _ -> None

and parse_neg (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else
    match parse_expr (fuel - 1) toks with
    | None -> None
    | Some (a, r1) ->
      match r1 with
      | TkRParen :: r2 -> Some (ENeg a, r2)
      | _ -> None

and parse_if_gt (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else
    match parse_expr (fuel - 1) toks with
    | None -> None
    | Some (ca, r1) ->
      match parse_expr (fuel - 1) r1 with
      | None -> None
      | Some (cb, r2) ->
        match parse_expr (fuel - 1) r2 with
        | None -> None
        | Some (t, r3) ->
          match parse_expr (fuel - 1) r3 with
          | None -> None
          | Some (el, r4) ->
            match r4 with
            | TkRParen :: r5 -> Some (EIfGt (ca, cb, t, el), r5)
            | _ -> None

and parse_let (fuel:int) (toks:list token) : Tot (option (expr * (list token))) (decreases fuel) =
  if fuel <= 0 then None
  else match toks with
  | [] -> None
  | TkSym name :: rest ->
    (match parse_expr (fuel - 1) rest with
     | None -> None
     | Some (val_e, r2) ->
       (match parse_expr (fuel - 1) r2 with
        | None -> None
        | Some (body, r3) ->
          (match r3 with
           | TkRParen :: r4 -> Some (ELet (name, val_e, body), r4)
           | _ -> None)))
  | _ -> None

// ============================================================
// TOP-LEVEL PARSE
// ============================================================

val parse : tokens:list token -> option expr
let parse toks =
  match parse_expr (FStar.List.Tot.length toks) toks with
  | None -> None
  | Some (e, rest) ->
    match rest with
    | [] -> Some e
    | _ -> None
