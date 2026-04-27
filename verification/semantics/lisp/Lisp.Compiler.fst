(** Lisp Bytecode Compiler — F* Formal Specification

    Compiles a subset of Lisp expressions into VM opcodes.
    Uses fuel-based termination for the mutual recursion group.
*)
module Lisp.Compiler

open Lisp.Types
open Lisp.Values
open Lisp.Source

// === Compiler State ===
noeq type compiler = {
  code      : list opcode;
  slot_map  : list string;
}

// === List helpers ===
val list_append : list 'a -> list 'a -> Tot (list 'a)
let rec list_append a b =
  match a with
  | [] -> b
  | x :: rest -> x :: list_append rest b

val list_len_nat : list 'a -> Tot nat
let rec list_len_nat l =
  match l with
  | [] -> 0
  | _ :: rest -> list_len_nat rest + 1

val list_len_int : list 'a -> Tot int
let rec list_len_int l =
  match l with
  | [] -> 0
  | _ :: rest -> 1 + list_len_int rest

// === Compiler helpers ===
// slot_of: find the LAST occurrence of name in slot_map (right-to-left scan).
// Returns Some idx where idx is the index of the last match.
// This matches env_push semantics: newest binding shadows older ones.
val slot_of : string -> list string -> Tot (option int)
let rec slot_of name slots =
  match slots with
  | [] -> None
  | s :: rest ->
    (match slot_of name rest with
     | Some n -> Some (n + 1)      // found later — this is not the last
     | None -> if s = name then Some 0 else None)

val init_compiler : list string -> compiler
let init_compiler params = { code = []; slot_map = params }

val emit : opcode -> compiler -> compiler
let emit op c = { c with code = list_append c.code [op] }

// Patch jump target at given index
val patch_jump : list opcode -> nat -> nat -> list opcode
let rec patch_jump code idx target =
  match code with
  | [] -> []
  | op :: rest ->
    if idx = 0 then
      (match op with
       | JumpIfFalse _ -> JumpIfFalse target :: rest
       | Jump _ -> Jump target :: rest
       | _ -> op :: rest)
    else op :: patch_jump rest (idx - 1) target

// === Fuel-based mutual recursion group ===
// compile, compile_chain, compile_binop, compile_if, compile_let, compile_body

val compile : fuel:int -> lisp_val -> compiler -> Tot (option compiler)
  (decreases fuel)
let rec compile fuel expr c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match expr with
  | Num n -> Some (emit (PushI64 n) c)
  | Float fl -> Some (emit (PushFloat fl) c)
  | Bool b -> Some (emit (PushBool b) c)
  | Str s -> Some (emit (PushStr s) c)
  | Nil -> Some (emit PushNil c)
  | Sym name ->
    (match name with
    | "true"  -> Some (emit (PushBool true) c)
    | "false" -> Some (emit (PushBool false) c)
    | "nil"   -> Some (emit PushNil c)
    | _ -> (match slot_of name c.slot_map with
            | Some idx -> Some (emit (LoadSlot idx) c)
            | None -> None))
  | List [] -> Some (emit PushNil c)
  | List (Sym "+" :: a :: rest) -> compile_chain f OpAdd a rest c
  | List (Sym "-" :: a :: rest) -> compile_chain f OpSub a rest c
  | List (Sym "*" :: a :: rest) -> compile_chain f OpMul a rest c
  | List (Sym "/" :: a :: rest) -> compile_chain f OpDiv a rest c
  | List [Sym "="; a; b] -> compile_binop f OpEq a b c
  | List [Sym "<"; a; b] -> compile_binop f OpLt a b c
  | List [Sym ">"; a; b] -> compile_binop f OpGt a b c
  | List [Sym "<="; a; b] -> compile_binop f OpLe a b c
  | List [Sym ">="; a; b] -> compile_binop f OpGe a b c
  | List [Sym "if"; test; then_br] -> compile_if f test then_br Nil c
  | List [Sym "if"; test; then_br; else_br] -> compile_if f test then_br else_br c
  | List (Sym "let" :: List bindings :: body) -> compile_let f bindings body c
  | List [Sym "not"; a] ->
    compile_if f a (Bool false) (Bool true) c
  | List [Sym "nil?"; a] ->
    // nil?(a) = (a == nil)
    compile_binop f OpEq a Nil c
  | List [Sym "get"; map_expr; key_expr] ->
    // compile(map), compile(key), DictGet
    (match compile_binop f DictGet map_expr key_expr c with
     | r -> r)
  | List [Sym "set"; map_expr; key_expr; val_expr] ->
    // compile(map), compile(key), compile(val), DictSet
    (match compile f map_expr c with
     | None -> None
     | Some c1 ->
       (match compile f key_expr c1 with
        | None -> None
        | Some c2 ->
          (match compile f val_expr c2 with
           | None -> None
           | Some c3 -> Some (emit DictSet c3))))
  | _ -> None

// Compile chain: first operand + rest with binop
and compile_chain fuel binop first rest c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match compile f first c with
  | None -> None
  | Some c1 -> compile_chain_rest f binop rest c1

// Continue chain: emit binop for each remaining operand
and compile_chain_rest fuel binop args c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match args with
  | [] -> Some c
  | a :: rest ->
    (match compile f a c with
     | None -> None
     | Some c1 -> compile_chain_rest f binop rest (emit binop c1))

// Compile binary operator
and compile_binop fuel binop a b c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match compile f a c with
  | None -> None
  | Some c1 ->
    (match compile f b c1 with
     | None -> None
     | Some c2 -> Some (emit binop c2))

// Compile if-then-else
and compile_if fuel test then_br else_br c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match compile f test c with
  | None -> None
  | Some c1 ->
    let jf_idx = list_len_nat c1.code in
    let c2 = emit (JumpIfFalse 0) c1 in
    (match compile f then_br c2 with
     | None -> None
     | Some c3 ->
       let jmp_idx = list_len_nat c3.code in
       let c4 = emit (Jump 0) c3 in
       let else_start = list_len_nat c4.code in
       let c5 = { c4 with code = patch_jump c4.code jf_idx else_start } in
       (match compile f else_br c5 with
        | None -> None
        | Some c6 ->
          let end_addr = list_len_nat c6.code in
          Some { c6 with code = patch_jump c6.code jmp_idx end_addr }))

// Compile let bindings then body
and compile_let fuel bindings body c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match bindings with
  | [] -> compile_body f body c
  | List [Sym name; init] :: rest ->
    (match compile f init c with
     | None -> None
     | Some c1 ->
       let slot_idx = list_len_int c1.slot_map in
       let c2 = { c1 with slot_map = list_append c1.slot_map [name] } in
       let c3 = emit (StoreSlot slot_idx) c2 in
       compile_let f rest body c3)
  | _ -> None

// Compile body (sequence, return last)
and compile_body fuel body c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  match body with
  | [] -> Some (emit PushNil c)
  | [e] -> compile f e c
  | e :: rest ->
    (match compile f e c with
    | None -> None
    | Some c1 -> compile_body f rest c1)

// Top-level: compile a lambda with given params and body
val compile_lambda : fuel:int -> list string -> lisp_val -> Tot (option (list opcode))
let compile_lambda fuel params body =
  match compile_body fuel [body] (init_compiler params) with
  | None -> None
  | Some c -> Some (list_append c.code [Return])
