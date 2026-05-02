//! Builtin dispatch modules, JSON helpers, and backward-compat stubs.
//!
//! The tree-walking evaluator has been removed. All evaluation goes through
//! the bytecode compiler + VM (`crate::program::run_program`).
//! The thin wrapper functions below exist for backward compatibility with
//! dispatch modules that call `apply_lambda` / `lisp_eval`.

use crate::types::{Env, EvalState, LispVal};

// Re-export helpers so dispatch modules can use super::is_truthy etc.
pub use crate::helpers::is_truthy;

pub mod crypto;
pub mod helpers;
#[cfg(not(target_arch = "wasm32"))]
pub mod llm_provider;
pub mod quasiquote;

pub mod dispatch_arithmetic;
pub mod dispatch_collections;
#[cfg(not(target_arch = "wasm32"))]
pub mod dispatch_http;
#[cfg(not(target_arch = "wasm32"))]
pub mod dispatch_json;
pub mod dispatch_predicates;
#[cfg(not(target_arch = "wasm32"))]
pub mod dispatch_state;
pub mod dispatch_strings;
pub mod dispatch_types;

#[cfg(not(target_arch = "wasm32"))]
pub use llm_provider::*;

// ---------------------------------------------------------------------------
// JSON conversion
// ---------------------------------------------------------------------------

/// Convert a [`serde_json::Value`] into a [`LispVal`].
pub fn json_to_lisp(val: serde_json::Value) -> LispVal {
    match val {
        serde_json::Value::Null => LispVal::Nil,
        serde_json::Value::Bool(b) => LispVal::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                LispVal::Num(i)
            } else {
                LispVal::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => LispVal::Str(s),
        serde_json::Value::Array(a) => LispVal::List(a.into_iter().map(json_to_lisp).collect()),
        serde_json::Value::Object(m) => {
            let map: im::HashMap<String, LispVal> =
                m.into_iter().map(|(k, v)| (k, json_to_lisp(v))).collect();
            LispVal::Map(map)
        }
    }
}

/// Convert a [`LispVal`] reference into a [`serde_json::Value`].
pub fn lisp_to_json(val: &LispVal) -> serde_json::Value {
    match val {
        LispVal::Nil => serde_json::Value::Null,
        LispVal::Bool(b) => serde_json::Value::Bool(*b),
        LispVal::Num(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
        LispVal::Float(f) => {
            if let Some(n) = serde_json::Number::from_f64(*f) {
                serde_json::Value::Number(n)
            } else {
                serde_json::Value::Null
            }
        }
        LispVal::Str(s) => serde_json::Value::String(s.clone()),
        LispVal::List(items) => serde_json::Value::Array(items.iter().map(lisp_to_json).collect()),
        LispVal::Map(m) => {
            let obj: serde_json::Map<String, serde_json::Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), lisp_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        other => serde_json::Value::String(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Backward-compat stubs (delegate to bytecode VM)
// ---------------------------------------------------------------------------

/// Evaluate a single Lisp expression (delegates to VM via run_program).
pub fn lisp_eval(expr: &LispVal, env: &mut Env, state: &mut EvalState) -> Result<LispVal, String> {
    crate::program::run_program(&[expr.clone()], env, state)
}

/// Apply a function value to arguments (delegates to VM).
pub fn apply_lambda(
    func: &LispVal,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::vm_call_lambda(func, args, env, state)
}

/// Dispatch a builtin call by name with evaluated arguments.
pub fn dispatch_call_with_args(
    name: &str,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::eval_builtin(name, args, Some(env), Some(state))
}

/// Call a function value (delegates to VM).
pub fn call_val(
    func: &LispVal,
    args: &[LispVal],
    env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    crate::bytecode::vm_call_lambda(func, args, env, state)
}
