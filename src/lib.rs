//! A minimal Lisp interpreter written in Rust.
//!
//! This crate provides a tree-walking evaluator for a Lisp dialect that supports
//! first-class functions, closures, macros, quasiquote/unquote, pattern matching,
//! a bytecode fast path for `map`/`filter`/`reduce`, and an execution budget to
//! prevent runaway infinite loops.
//!
//! # Quick start
//!
//! ```ignore
//! use lisp_rlm::{parse_all, lisp_eval, Env};
//!
//! let exprs = parse_all("(+ 1 2)")?;
//! let mut env = Env::new();
//! let result = lisp_eval(&exprs[0], &mut env)?;
//! assert_eq!(result.to_string(), "3");
//! ```
//!
//! # Modules
//!
//! - [`eval`] — core evaluator (`lisp_eval`, `apply_lambda`, JSON interop)
//! - [`types`] — value types ([`LispVal`], [`Env`]), standard-library source
//! - [`parser`] — S-expression parser (`parse_all`, `parse_all_spanned`)
//! - [`bytecode`] — compiled fast path for higher-order list operations
//! - [`helpers`] — utility predicates (`is_truthy`, `is_builtin_name`)

mod bytecode;
mod eval;
mod helpers;
mod parser;
mod types;

pub use bytecode::{try_compile_loop, exec_compiled_loop, try_compile_lambda, run_compiled_lambda};
pub use eval::{apply_lambda, lisp_eval};
pub use eval::llm_provider::{LlmProvider, GenericProvider, LlmResponse};
pub use helpers::{is_builtin_name, is_truthy};
pub use parser::parse_all;
pub use parser::parse_all_spanned;
pub use parser::Spanned;
pub use types::DEFAULT_EVAL_BUDGET;
pub use types::{get_stdlib_code, Env, LispVal};
