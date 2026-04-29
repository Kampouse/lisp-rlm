module Lisp.Compiler

open Lisp.Types
open Lisp.Values
open Lisp.Source

noeq type compiler = {
  code         : list opcode;
  slot_map     : list string;
  code_table   : list (list opcode * nat * list (string * int));
  parent_slots : list string;
}

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

val slot_of : string -> list string -> Tot (option int)
let rec slot_of name slots =
  match slots with
  | [] -> None
  | s :: rest ->
    (match slot_of name rest with
     | Some n -> Some (n + 1)
     | None -> if s = name then Some 0 else None)

val init_compiler : list string -> compiler
let init_compiler params = { code = []; slot_map = params; code_table = []; parent_slots = [] }

val emit : opcode -> compiler -> compiler
let emit op c = { c with code = list_append c.code [op] }

val patch_jump : list opcode -> nat -> nat -> list opcode
let rec patch_jump code idx target =
  match code with
  | [] -> []
  | op :: rest ->
    if idx = 0 then
      (match op with
       | JumpIfFalse _ -> JumpIfFalse target :: rest
       | JumpIfTrue _ -> JumpIfTrue target :: rest
       | Jump _ -> Jump target :: rest
       | _ -> op :: rest)
    else op :: patch_jump rest (idx - 1) target

val extract_params : list lisp_val -> Tot (option (list string))
let rec extract_params params =
  match params with
  | [] -> Some []
  | Sym s :: rest ->
    (match extract_params rest with
     | None -> None
     | Some ps -> Some (s :: ps))
  | _ -> None

val compute_captures_from_map : list string -> list string -> int -> Tot (list (string * int))
let rec compute_captures_from_map full_map parent_slots start_idx =
  if start_idx > 0 then
    (match full_map with _ :: r -> compute_captures_from_map r parent_slots (start_idx - 1) | [] -> [])
  else
    (match full_map with
     | [] -> []
     | name :: rest ->
       (match slot_of name parent_slots with
        | Some idx -> (name, idx) :: compute_captures_from_map rest parent_slots 0
        | None -> compute_captures_from_map rest parent_slots 0))

val compute_runtime_captures : list string -> list string -> list string -> Tot (list (string * int))
let compute_runtime_captures inner_params parent_slots inner_slot_map =
  compute_captures_from_map inner_slot_map parent_slots (list_len_nat inner_params)

val compile : fuel:int -> lisp_val -> compiler -> Tot (option compiler)
  (decreases fuel)
let rec compile fuel expr c =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match expr with
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
      | _ ->
        (match slot_of name c.slot_map with
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
   | List [Sym "not"; a] -> compile_if f a (Bool false) (Bool true) c
   | List [Sym "nil?"; a] -> compile_binop f OpEq a Nil c
   | List [Sym "get"; map_expr; key_expr] -> compile_binop f DictGet map_expr key_expr c
   | List [Sym "set"; map_expr; key_expr; val_expr] ->
     (match compile f map_expr c with
      | None -> None
      | Some c1 ->
        (match compile f key_expr c1 with
         | None -> None
         | Some c2 ->
           (match compile f val_expr c2 with
            | None -> None
            | Some c3 -> Some (emit DictSet c3))))
   | List (Sym "list" :: args) -> compile_list f args c
   | List (Sym "progn" :: body) -> compile_body f body c
   | List (Sym "begin" :: body) -> compile_body f body c
   | List (Sym "and" :: args) -> compile_and f args c
   | List (Sym "or" :: args) -> compile_or f args c
   | List (Sym "cond" :: clauses) -> compile_cond f clauses c
   | List (Sym "lambda" :: List params :: body) -> compile_lambda_expr f params body c
   | _ -> None)

and compile_chain fuel binop first rest c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match compile f first c with
   | None -> None
   | Some c1 -> compile_chain_rest f binop rest c1)

and compile_chain_rest fuel binop args c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match args with
   | [] -> Some c
   | a :: rest ->
     (match compile f a c with
      | None -> None
      | Some c1 -> compile_chain_rest f binop rest (emit binop c1)))

and compile_binop fuel binop a b c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match compile f a c with
   | None -> None
   | Some c1 ->
     (match compile f b c1 with
      | None -> None
      | Some c2 -> Some (emit binop c2)))

and compile_if fuel test then_br else_br c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match compile f test c with
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
           Some { c6 with code = patch_jump c6.code jmp_idx end_addr })))

and compile_let fuel bindings body c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match bindings with
   | [] -> compile_body f body c
   | List [Sym name; init] :: rest ->
     (match compile f init c with
      | None -> None
      | Some c1 ->
        let slot_idx = list_len_int c1.slot_map in
        let c2 = { c1 with slot_map = list_append c1.slot_map [name] } in
        let c3 = emit (StoreSlot slot_idx) c2 in
        compile_let f rest body c3)
   | _ -> None)

and compile_body fuel body c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match body with
   | [] -> Some (emit PushNil c)
   | [e] -> compile f e c
   | e :: rest ->
     (match compile f e c with
      | None -> None
      | Some c1 -> compile_body f rest c1))

and compile_list fuel args c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match compile_list_body f args c with
   | None -> None
   | Some (c', n) -> Some (emit (MakeList n) c'))

and compile_list_body fuel args c : Tot (option (compiler * nat)) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match args with
   | [] -> Some (c, 0)
   | a :: rest ->
     (match compile f a c with
      | None -> None
      | Some c1 ->
        (match compile_list_body f rest c1 with
         | None -> None
         | Some (c2, n) -> Some (c2, n + 1))))

and compile_and fuel args c : Tot (option compiler) =
  if fuel <= 0 then None else
  (match args with
   | [] -> Some (emit (PushBool true) c)
   | [e] -> compile (fuel - 1) e c
   | e :: rest ->
     (match compile (fuel - 1) e c with
      | None -> None
      | Some c1 ->
        let c1_dup = emit Dup c1 in
        let jf_idx = list_len_nat c1_dup.code in
        let c2 = emit (JumpIfFalse 0) c1_dup in
        let c3 = emit Pop c2 in
        (match compile_and (fuel - 1) rest c3 with
         | None -> None
         | Some c4 ->
           let end_addr = list_len_nat c4.code in
           Some { c4 with code = patch_jump c4.code jf_idx end_addr })))

and compile_or fuel args c : Tot (option compiler) =
  if fuel <= 0 then None else
  (match args with
   | [] -> Some (emit (PushBool false) c)
   | [e] -> compile (fuel - 1) e c
   | e :: rest ->
     (match compile (fuel - 1) e c with
      | None -> None
      | Some c1 ->
        let c1_dup = emit Dup c1 in
        let jt_idx = list_len_nat c1_dup.code in
        let c2 = emit (JumpIfTrue 0) c1_dup in
        let c3 = emit Pop c2 in
        (match compile_or (fuel - 1) rest c3 with
         | None -> None
         | Some c4 ->
           let end_addr = list_len_nat c4.code in
           Some { c4 with code = patch_jump c4.code jt_idx end_addr })))

and compile_cond fuel clauses c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match clauses with
   | [] -> Some (emit PushNil c)
   | List (Sym "else" :: result :: _) :: _ -> compile f result c
   | List (test :: result :: _) :: rest ->
     (match compile f test c with
      | None -> None
      | Some c1 ->
        let jf_idx = list_len_nat c1.code in
        let c2 = emit (JumpIfFalse 0) c1 in
        (match compile f result c2 with
         | None -> None
         | Some c3 ->
           let jmp_idx = list_len_nat c3.code in
           let c4 = emit (Jump 0) c3 in
           let next_addr = list_len_nat c4.code in
           let c5 = { c4 with code = patch_jump c4.code jf_idx next_addr } in
           (match compile_cond f rest c5 with
            | None -> None
            | Some c6 ->
              let end_addr = list_len_nat c6.code in
              Some { c6 with code = patch_jump c6.code jmp_idx end_addr })))
   | _ -> None)

and compile_lambda_expr fuel params body c : Tot (option compiler) =
  if fuel <= 0 then None else
  let f = fuel - 1 in
  (match extract_params params with
   | None -> None
   | Some param_names ->
     let inner = { code = []; slot_map = param_names; code_table = []; parent_slots = c.slot_map } in
     (match compile_body f body inner with
      | None -> None
      | Some inner_done ->
        let inner_code = list_append inner_done.code [Return] in
        let chunk_idx = list_len_nat c.code_table in
        let rtcaps = compute_runtime_captures param_names inner_done.parent_slots inner_done.slot_map in
        let nslots = list_len_nat inner_done.slot_map in
        let new_table = list_append c.code_table [(inner_code, nslots, rtcaps)] in
        let c_with_closure = emit (PushClosure chunk_idx) c in
        Some { c_with_closure with code_table = new_table }))

val compile_lambda : fuel:int -> list string -> lisp_val -> Tot (option (list opcode))
let compile_lambda fuel params body =
  (match compile_body fuel [body] (init_compiler params) with
   | None -> None
   | Some c -> Some (list_append c.code [Return]))
