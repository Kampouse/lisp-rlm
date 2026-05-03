(** Inductive Compilation Relation

    Defines compiles_to as an inductive type (one constructor per source form).
    Each constructor witnesses that a source expression compiles to specific bytecode.
    
    This is the compositional alternative to the monolithic compile() function
    in Lisp.Compiler.fst. Instead of unfolding compile_chain recursively,
    proofs do structural induction on compiles_to evidence — each case
    is self-contained and Z3 can discharge it independently.
    
    Following vWasm's pattern: compilation as a relation, not a function.
*)
module LispIR.CompRel

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open FStar.List.Tot

(** The compilation relation.
    
    Each constructor carries:
    - The source expression being compiled
    - The slot environment (list of variable names in scope)
    - The compiled bytecode (list of opcodes)
    - Recursive compiles_to evidence for sub-expressions
    
    For simple forms (literals, symbol lookup), no recursive evidence needed.
    For compound forms (binop, if, let, lambda), recursive evidence proves
    sub-expressions compile correctly.
*)
noeq type compiles_to =
  // === LITERALS ===
  | CNum of int
  | CFloat of ffloat
  | CBool of bool
  | CStr of string
  | CNil

  // === SYMBOL LOOKUP ===
  // Sym x → LoadSlot i when slot_of x slots = Some i
  | CSymLoad of string & int & list string

  // === BINARY OPERATIONS ===
  // (op a b) → bc_a @ bc_b @ [opcode]
  | CBinop of lisp_val & lisp_val & lisp_val & list opcode & list opcode & compiles_to & compiles_to & opcode

  // === CHAINED ARITHMETIC ===
  // (+ a b c) → bc_a @ bc_b @ bc_c @ [OpAdd; OpAdd]
  | CChain of list lisp_val & list (list opcode) & list compiles_to & opcode

  // === IF ===
  // (if test then else) → bc_test @ [JF else_start] @ bc_then @ [J end] @ bc_else
  | CIf of lisp_val & lisp_val & lisp_val
         & list opcode & list opcode & list opcode
         & compiles_to & compiles_to & compiles_to

  // === IF (no else) ===
  | CIfOne of lisp_val & lisp_val
            & list opcode & list opcode
            & compiles_to & compiles_to

  // === LET ===
  // (let ((x v)) body) → bc_v @ [StoreSlot i] @ bc_body
  | CLet of string & lisp_val & lisp_val
          & list opcode & list opcode
          & compiles_to & compiles_to

  // === BODY (begin/progn) ===
  // (begin e1 e2 ... en) → bc_1 @ bc_2 @ ... @ bc_n
  | CBody of list lisp_val & list (list opcode) & list compiles_to

  // === LAMBDA ===
  // (lambda (params) body) → [PushClosure idx]
  | CLambda of list string & list lisp_val
             & list (list opcode) & list compiles_to

  // === LIST ===
  // (list a b c) → bc_a @ bc_b @ bc_c @ [MakeList 3]
  | CList of list lisp_val & list (list opcode) & list compiles_to

  // === NOT ===
  // (not a) → bc_a @ [Dup; JF skip; PushBool false; J end; skip: PushBool true]
  | CNot of lisp_val & list opcode & compiles_to

  // === NIL? ===
  // (nil? a) → bc_a @ [PushNil; OpEq]
  | CNilQ of lisp_val & list opcode & compiles_to

  // === GET ===
  // (get map key) → bc_map @ bc_key @ [DictGet]
  | CGet of lisp_val & lisp_val & list opcode & list opcode & compiles_to & compiles_to

  // === SET ===
  // (set map key val) → bc_map @ bc_key @ bc_val @ [DictSet]
  | CSet of lisp_val & lisp_val & lisp_val
          & list opcode & list opcode & list opcode
          & compiles_to & compiles_to & compiles_to
