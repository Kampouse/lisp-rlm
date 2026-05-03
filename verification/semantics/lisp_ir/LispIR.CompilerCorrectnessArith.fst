module LispIR.CompilerCorrectnessArith
open FStar.Pervasives
open FStar.List.Tot

type arith_expr =
  | ANum of int
  | AAdd of arith_expr * arith_expr
  | ASub of arith_expr * arith_expr
  | ANeg of arith_expr
  | AMul of arith_expr * arith_expr

type arith_op =
  | Push of int
  | OpAdd
  | OpSub
  | OpNeg
  | OpMul

val arith_compile : arith_expr -> list arith_op
let rec arith_compile = function
  | ANum n -> [Push n]
  | AAdd (a, b) -> arith_compile a @ arith_compile b @ [OpAdd]
  | ASub (a, b) -> arith_compile a @ arith_compile b @ [OpSub]
  | ANeg a -> arith_compile a @ [OpNeg]
  | AMul (a, b) -> arith_compile a @ arith_compile b @ [OpMul]

val arith_eval : arith_expr -> int
let rec arith_eval = function
  | ANum n -> n
  | AAdd (a, b) -> arith_eval a + arith_eval b
  | ASub (a, b) -> arith_eval a - arith_eval b
  | ANeg a -> 0 - arith_eval a
  | AMul (a, b) -> Prims.op_Multiply (arith_eval a) (arith_eval b)

val arith_vm : fuel:int -> code:list arith_op -> stack:list int ->
  Tot (list int * int) (decreases fuel)
let rec arith_vm fuel code stack =
  if fuel <= 0 then (stack, fuel)
  else match code with
  | [] -> (stack, fuel)
  | Push n :: rest -> arith_vm (fuel - 1) rest (n :: stack)
  | OpAdd :: rest ->
    (match stack with
     | a :: b :: stk' -> arith_vm (fuel - 1) rest ((b + a) :: stk')
     | _ -> (stack, 0))
  | OpSub :: rest ->
    (match stack with
     | a :: b :: stk' -> arith_vm (fuel - 1) rest ((b - a) :: stk')
     | _ -> (stack, 0))
  | OpNeg :: rest ->
    (match stack with
     | a :: stk' -> arith_vm (fuel - 1) rest ((0 - a) :: stk')
     | _ -> (stack, 0))
  | OpMul :: rest ->
    (match stack with
     | a :: b :: stk' -> arith_vm (fuel - 1) rest (Prims.op_Multiply b a :: stk')
     | _ -> (stack, 0))

val run_then : fuel:int -> c1:list arith_op -> s:list int ->
  c2:list arith_op -> Tot (list int * int)
let run_then fuel c1 s c2 =
  let (s1, f1) = arith_vm fuel c1 s in
  arith_vm f1 c2 s1

val arith_vm_sequential : c1:list arith_op -> fuel:int -> c2:list arith_op -> s:list int ->
  Lemma (ensures arith_vm fuel (c1 @ c2) s = run_then fuel c1 s c2)
let rec arith_vm_sequential c1 fuel c2 s =
  match c1 with
  | [] -> ()
  | Push n :: rest -> arith_vm_sequential rest (fuel - 1) c2 (n :: s)
  | OpAdd :: rest ->
    (match s with
     | a :: b :: stk' -> arith_vm_sequential rest (fuel - 1) c2 ((b + a) :: stk')
     | _ -> ())
  | OpSub :: rest ->
    (match s with
     | a :: b :: stk' -> arith_vm_sequential rest (fuel - 1) c2 ((b - a) :: stk')
     | _ -> ())
  | OpNeg :: rest ->
    (match s with
     | a :: stk' -> arith_vm_sequential rest (fuel - 1) c2 ((0 - a) :: stk')
     | _ -> ())
  | OpMul :: rest ->
    (match s with
     | a :: b :: stk' -> arith_vm_sequential rest (fuel - 1) c2 (Prims.op_Multiply b a :: stk')
     | _ -> ())

val get_stack : r:list int * int -> list int
let get_stack (s, _) = s

val arith_compiler_correctness : fuel:int -> e:arith_expr -> Lemma
  (get_stack (arith_vm fuel (arith_compile e) []) = [arith_eval e])
let rec arith_compiler_correctness fuel e =
  match e with
  | ANum _ -> ()
  | AAdd (a, b) ->
    arith_compiler_correctness fuel a;
    arith_compiler_correctness fuel b;
    arith_vm_sequential (arith_compile a) fuel (arith_compile b @ [OpAdd]) []
  | ASub (a, b) ->
    arith_compiler_correctness fuel a;
    arith_compiler_correctness fuel b;
    arith_vm_sequential (arith_compile a) fuel (arith_compile b @ [OpSub]) []
  | ANeg a ->
    arith_compiler_correctness fuel a;
    arith_vm_sequential (arith_compile a) fuel [OpNeg] []
  | AMul (a, b) ->
    arith_compiler_correctness fuel a;
    arith_compiler_correctness fuel b;
    arith_vm_sequential (arith_compile a) fuel (arith_compile b @ [OpMul]) []

