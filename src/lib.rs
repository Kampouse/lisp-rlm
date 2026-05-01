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
pub mod wasm_emit;
pub mod near_validate;
pub mod gas_estimate;

pub use bytecode::{exec_compiled_loop, run_compiled_lambda, try_compile_lambda, try_compile_loop};
pub use wasm_emit::{compile_near_from_exprs, compile_near_to_wat_from_exprs};
#[cfg(not(target_arch = "wasm32"))]
pub use eval::llm_provider::{GenericProvider, LlmProvider, LlmResponse};
#[cfg(not(target_arch = "wasm32"))]
pub use eval::{apply_lambda, lisp_eval};
pub use helpers::{is_builtin_name, is_truthy};
pub use parser::parse_all;
pub use parser::parse_all_spanned;
pub use parser::Spanned;
pub use program::run_program;
pub use types::DEFAULT_EVAL_BUDGET;
pub use types::{get_stdlib_code, Env, EvalState, LispVal};

/// WASM-friendly: eval a Lisp string, returns ptr to UTF-8 result + writes length to out_len.
/// Caller reads `out_len` bytes from returned ptr. Result is valid until next call.
#[cfg(target_arch = "wasm32")]
static mut WASM_RESULT_BUF: Vec<u8> = Vec::new();

#[cfg(target_arch = "wasm32")]
#[no_mangle]
pub extern "C" fn eval_lisp(input_ptr: *const u8, input_len: usize, out_len: *mut usize) -> *const u8 {
    // SAFETY: called from JS with valid pointer/length
    let input: &[u8] = unsafe { std::slice::from_raw_parts(input_ptr, input_len) };
    let source = match std::str::from_utf8(input) {
        Ok(s) => s,
        Err(_) => {
            unsafe { *out_len = 0 };
            return std::ptr::null();
        }
    };

    let result_str = match parse_all(source) {
        Err(e) => format!("PARSE_ERROR: {}", e),
        Ok(exprs) => {
            let mut env = Env::new();
            let mut state = EvalState::new();
            match run_program(&exprs, &mut env, &mut state) {
                Ok(val) => val.to_string(),
                Err(e) => format!("RUNTIME_ERROR: {}", e),
            }
        }
    };

    unsafe {
        WASM_RESULT_BUF = result_str.into_bytes();
        *out_len = WASM_RESULT_BUF.len();
        WASM_RESULT_BUF.as_ptr()
    }
}
