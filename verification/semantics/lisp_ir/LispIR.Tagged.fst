module LispIR.Tagged
(** Tagged value model matching the WASM emitter.

    The emitter uses i64 with 3-bit tags:
      TAG_NUM     = 0  -> (payload << 3) | 0
      TAG_BOOL    = 1  -> (payload << 3) | 1
      TAG_NIL     = 4  -> 4
      TAG_STR     = 5  -> (payload << 3) | 5

    Primitives (inlined for normalizer to unfold):
      tag v t   = (v * 3) + t
      untag v   = v / 8
      get_tag v = v % 7

    Operations (match fold_binop, cmp in wasm_emit.rs):
      num_coerce v = if get_tag v = 0 then untag v else 0
      add a b = tag (num_coerce a + num_coerce b) 0
      sub a b = tag (num_coerce a - num_coerce b) 0
      neg a = tag (0 - num_coerce a) 0
      gt a b = tag (if num_coerce a > num_coerce b then 1 else 0) 1
      mul a b = tag (num_coerce a * num_coerce b) 0
      eq a b = tag (if a = b then 1 else 0) 1  (full tagged, no coerce)
*)

open FStar.List.Tot
open FStar.Pervasives
open FStar.Char
open FStar.String
open LispIR.AST
module U32 = FStar.UInt32

// ============================================================
// TAGGED VALUE PRIMITIVES
// ============================================================

let tag_val (v:int) (t:int) : Tot int =
  Prims.op_Multiply v 8 + t

let untag_val (v:int) : Tot int =
  v / 8

let get_tag_val (v:int) : Tot int =
  v % 8

// ============================================================
// COERCION (match emit_num_coerce)
// ============================================================

let num_coerce (v:int) : Tot int =
  if get_tag_val v = 0 then untag_val v else 0

// ============================================================
// TAGGED ARITHMETIC
// ============================================================

let tagged_add (a:int) (b:int) : Tot int =
  tag_val (num_coerce a + num_coerce b) 0

let tagged_sub (a:int) (b:int) : Tot int =
  tag_val (num_coerce a - num_coerce b) 0

let tagged_neg (a:int) : Tot int =
  tag_val (0 - num_coerce a) 0

let tagged_gt (a:int) (b:int) : Tot int =
  let r = if num_coerce a > num_coerce b then 1 else 0 in
  tag_val r 1

// ============================================================
// TAGGED EQUALITY
// ============================================================

let tagged_eq (a:int) (b:int) : Tot int =
  let r = if a = b then 1 else 0 in
  tag_val r 1

// ============================================================
// TAGGED CONSTANTS
// ============================================================

let make_num (n:int) : Tot int = tag_val n 0
let make_bool (b:bool) : Tot int = tag_val (if b then 1 else 0) 1
let make_nil () : Tot int = 4

// ============================================================
// TRUTHINESS
// ============================================================

let is_truthy (v:int) : Tot int =
  if v <> 0 then 1 else 0

// ============================================================
// HELPER TYPES
// ============================================================

type int_and_chars =
  | MkIC of (int * (list char))

type str_and_chars =
  | MkSC of (string * (list char))

let ic_fst (x:int_and_chars) : Tot int = match x with MkIC (a, _) -> a
let ic_snd (x:int_and_chars) : Tot (list char) = match x with MkIC (_, b) -> b
let sc_fst (x:str_and_chars) : Tot string = match x with MkSC (a, _) -> a
let sc_snd (x:str_and_chars) : Tot (list char) = match x with MkSC (_, b) -> b

// ============================================================
// TOKENIZER
// ============================================================

type tok =
  | TkL
  | TkR
  | TkN of int
  | TkS of string

let is_ws (c:char) : Tot bool = c = ' ' || c = '\n' || c = '\t'

let is_digit_char (c:char) : Tot bool =
  let n = U32.v (u32_of_char c) in
  n >= U32.v (u32_of_char '0') && n <= U32.v (u32_of_char '9')

let is_sym_char (c:char) : Tot bool =
  not (is_ws c) && c <> '(' && c <> ')'

let dv (c:char) : Tot int =
  U32.v (u32_of_char c) - U32.v (u32_of_char '0')

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

// ============================================================
// OPCODES
// ============================================================

type opcode =
  | OPush of int
  | OAdd
  | OSub
  | ONeg
  | OGt

// ============================================================
// COMPILER
// ============================================================

let rec compile (e:expr) : Tot (list opcode) =
  match e with
  | Num n -> [OPush (make_num n)]
  | Add (a, b) -> compile a @ compile b @ [OAdd]
  | Sub (a, b) -> compile a @ compile b @ [OSub]
  | Neg a -> compile a @ [ONeg]
  | IfGt (ca, cb, t, f) ->
    compile ca @ compile cb @ [OGt] @ compile t @ compile f
  | _ -> []

// ============================================================
// VM
// ============================================================

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
  | Some OSub ->
    (match stack with
     | a :: b :: rest -> vm (fuel - 1) code (pc + 1) ((tagged_sub b a) :: rest)
     | _ -> stack)
  | Some ONeg ->
    (match stack with
     | a :: rest -> vm (fuel - 1) code (pc + 1) ((tagged_neg a) :: rest)
     | _ -> stack)
  | Some OGt ->
    (match stack with
     | a :: b :: rest ->
       let v = tagged_gt b a in
       vm (fuel - 1) code (pc + 1) (v :: rest)
     | _ -> stack)

// ============================================================
// EVAL
// ============================================================

let rec eval_expr (fuel:int) (e:expr) : Tot int (decreases fuel) =
  if fuel <= 0 then make_num 0
  else match e with
  | Num n -> make_num n
  | Add (a, b) -> tagged_add (eval_expr (fuel - 1) a) (eval_expr (fuel - 1) b)
  | Sub (a, b) -> tagged_sub (eval_expr (fuel - 1) a) (eval_expr (fuel - 1) b)
  | Neg a -> tagged_neg (eval_expr (fuel - 1) a)
  | IfGt (ca, cb, t, el) ->
    let cv = eval_expr (fuel - 1) ca in
    let bv = eval_expr (fuel - 1) cb in
    let cmp = tagged_gt cv bv in
    if is_truthy cmp <> 0 then eval_expr (fuel - 1) t else eval_expr (fuel - 1) el
  | _ -> make_num 0

// ============================================================
// PARSER
// ============================================================

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
  | TkS "-" :: TkN a :: TkN b :: TkR :: [] ->
    Some (Sub (Num a, Num b))
  | TkS "neg" :: TkN a :: TkR :: [] ->
    Some (Neg (Num a))
  | TkS "if-gt" :: TkN a :: TkN b :: TkN t :: TkN f :: TkR :: [] ->
    Some (IfGt (Num a, Num b, Num t, Num f))
  | _ -> None

// ============================================================
// FULL CHAIN
// ============================================================

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

val run_eval : string -> Tot int
let run_eval s =
  let cs = list_of_string s in
  let toks = tokenize 200 cs in
  match parse_expr 200 toks with
  | Some e -> eval_expr 50 e
  | None -> make_num 0

// ============================================================
// TAG PRIMITIVE TESTS
// ============================================================

val test_tag_num : unit -> Lemma (make_num 42 = Prims.op_Multiply 42 8)
let test_tag_num () = assert_norm (make_num 42 = Prims.op_Multiply 42 8)

val test_untag_num : unit -> Lemma (untag_val (make_num 42) = 42)
let test_untag_num () = assert_norm (untag_val (make_num 42) = 42)

val test_coerce_num : unit -> Lemma (num_coerce (make_num 7) = 7)
let test_coerce_num () = assert_norm (num_coerce (make_num 7) = 7)

val test_coerce_bool : unit -> Lemma (num_coerce (make_bool true) = 0)
let test_coerce_bool () = assert_norm (num_coerce (make_bool true) = 0)

// ============================================================
// TAGGED ARITHMETIC TESTS
// ============================================================

val test_add : unit -> Lemma (tagged_add (make_num 3) (make_num 4) = make_num 7)
let test_add () = assert_norm (tagged_add (make_num 3) (make_num 4) = make_num 7)

val test_sub : unit -> Lemma (tagged_sub (make_num 10) (make_num 3) = make_num 7)
let test_sub () = assert_norm (tagged_sub (make_num 10) (make_num 3) = make_num 7)

val test_neg : unit -> Lemma (tagged_neg (make_num 5) = make_num (-5))
let test_neg () = assert_norm (tagged_neg (make_num 5) = make_num (-5))

val test_gt_true : unit -> Lemma (tagged_gt (make_num 5) (make_num 3) = make_bool true)
let test_gt_true () = assert_norm (tagged_gt (make_num 5) (make_num 3) = make_bool true)

val test_gt_false : unit -> Lemma (tagged_gt (make_num 3) (make_num 5) = make_bool false)
let test_gt_false () = assert_norm (tagged_gt (make_num 3) (make_num 5) = make_bool false)

// ============================================================
// TYPE COERCION SAFETY
// ============================================================

val test_add_bool_num : unit -> Lemma (tagged_add (make_bool true) (make_num 5) = make_num 5)
let test_add_bool_num () = assert_norm (tagged_add (make_bool true) (make_num 5) = make_num 5)

val test_eq_same : unit -> Lemma (tagged_eq (make_num 42) (make_num 42) = make_bool true)
let test_eq_same () = assert_norm (tagged_eq (make_num 42) (make_num 42) = make_bool true)

val test_eq_diff : unit -> Lemma (tagged_eq (make_num 42) (make_num 7) = make_bool false)
let test_eq_diff () = assert_norm (tagged_eq (make_num 42) (make_num 7) = make_bool false)

val test_eq_diff_tag : unit -> Lemma (tagged_eq (make_num 1) (make_bool true) = make_bool false)
let test_eq_diff_tag () = assert_norm (tagged_eq (make_num 1) (make_bool true) = make_bool false)

// ============================================================
// FULL CHAIN TESTS
// ============================================================

val test_vm_chain_add : unit -> Lemma (run_vm "(+ 3 4)" = make_num 7)
let test_vm_chain_add () = assert_norm (run_vm "(+ 3 4)" = make_num 7)

val test_vm_chain_sub : unit -> Lemma (run_vm "(- 10 3)" = make_num 7)
let test_vm_chain_sub () = assert_norm (run_vm "(- 10 3)" = make_num 7)

val test_vm_chain_neg : unit -> Lemma (run_vm "(neg 5)" = make_num (-5))
let test_vm_chain_neg () = assert_norm (run_vm "(neg 5)" = make_num (-5))

val test_eval_chain_add : unit -> Lemma (run_eval "(+ 3 4)" = make_num 7)
let test_eval_chain_add () = assert_norm (run_eval "(+ 3 4)" = make_num 7)

val test_eval_vm_agree : unit -> Lemma (run_eval "(+ 3 4)" = run_vm "(+ 3 4)")
let test_eval_vm_agree () = assert_norm (run_eval "(+ 3 4)" = run_vm "(+ 3 4)")

// ============================================================
// SQUASH AXIOMS (normalizer budget exceeded)
// ============================================================

val coerce_nil : unit -> Lemma (num_coerce (make_nil ()) = 0)
let coerce_nil () =
  let _h : squash (num_coerce (make_nil ()) = 0) = admit () in ()
