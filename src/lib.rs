//! A Lisp interpreter with a verified bytecode VM.
//!
//! This crate provides a Lisp dialect that supports first-class functions,
//! closures, macros, pattern matching, and a bytecode VM with formal
//! verification (F*). The VM is the sole execution path — all programs
//! are desugared to nested `let`+`lambda` forms and compiled to bytecode.
//!
//! # Quick start
//!
//! ```ignore
//! use lisp_rlm::{parse_all, run_program, Env, EvalState};
//!
//! let exprs = parse_all("(+ 1 2)")?;
//! let mut env = Env::new();
//! let mut state = EvalState::new();
//! let result = run_program(&exprs, &mut env, &mut state)?;
//! assert_eq!(result.to_string(), "3");
//! ```
//!
//! # Modules
//!
//! - [`program`] — top-level program execution (desugar → compile → run)
//! - [`bytecode`] — bytecode compiler and VM
//! - [`types`] — value types ([`LispVal`], [`Env`], [`EvalState`])
//! - [`parser`] — S-expression parser
//! - [`helpers`] — utility predicates
//! - [`eval`] — builtin dispatch modules, JSON helpers, and backward-compat stubs
//! - [`typing`] — type inference and checking

pub mod bytecode;
mod eval;
pub mod helpers;
pub mod parser;
pub mod types;
mod typing;
pub mod program;

pub use bytecode::{exec_compiled_loop, run_compiled_lambda, try_compile_lambda, try_compile_loop};
pub use eval::llm_provider::{GenericProvider, LlmProvider, LlmResponse};
pub use eval::{apply_lambda, lisp_eval};  // retained for backward compat / benchmarks
pub use helpers::{is_builtin_name, is_truthy};
pub use parser::parse_all;
pub use parser::parse_all_spanned;
pub use parser::Spanned;
pub use program::run_program;
pub use types::DEFAULT_EVAL_BUDGET;
pub use types::{get_stdlib_code, Env, EvalState, LispVal};
