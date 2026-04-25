use crate::types::{Env, LispVal};

/// Result from the call chain (apply_lambda → call_val → dispatch_call).
///
/// `TailCall` breaks the recursion: instead of calling `lisp_eval` to evaluate
/// a lambda body, `apply_lambda` returns the body and its local env. The caller
/// (the trampoline in `lisp_eval_inner`) loops to evaluate it — zero stack growth.
#[derive(Debug)]
pub enum EvalResult {
    /// A concrete value — evaluation is complete.
    Value(LispVal),
    /// A tail call — evaluate `expr` in `env` iteratively.
    TailCall { expr: LispVal, env: Env },
}

impl EvalResult {
    /// Unwrap a concrete value. Panics on TailCall.
    pub fn unwrap_value(self) -> LispVal {
        match self {
            EvalResult::Value(v) => v,
            EvalResult::TailCall { .. } => panic!("EvalResult::unwrap_value on TailCall"),
        }
    }

    /// Unwrap the tail call components. Panics on Value.
    pub fn unwrap_tailcall(self) -> (LispVal, Env) {
        match self {
            EvalResult::TailCall { expr, env } => (expr, env),
            EvalResult::Value(_) => panic!("EvalResult::unwrap_tailcall on Value"),
        }
    }
}
