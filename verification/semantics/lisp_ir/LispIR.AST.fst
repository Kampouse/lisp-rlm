module LispIR.AST
(** Shared types for the mini language — used by Parser, Compiler, and Proofs.

    Core expr: Num, Add, Sub, Neg, IfGt, Let
    Extended expr: Bool, Str, Sym (for parser; not compiled)
    Token type: shared between Tokenizer and Parser
*)

type token =
  | TkLParen
  | TkRParen
  | TkNum of int
  | TkSym of string
  | TkBool of bool
  | TkStr of string

type expr =
  | Num of int
  | Add of (expr * expr)
  | Sub of (expr * expr)
  | Neg of expr
  | IfGt of (expr * expr * expr * expr)
  | Let of (string * expr * expr)
  | Bool of bool
  | Str of string
  | Sym of string
