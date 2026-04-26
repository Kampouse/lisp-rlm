use crate::types::{Env, LispVal};

// ---------------------------------------------------------------------------
// EvalResult — returned by apply_lambda / call_val / dispatch_call
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum EvalResult {
    Value(LispVal),
    TailCall { expr: LispVal, env: Env },
}

impl EvalResult {
    pub fn unwrap_value(self) -> Result<LispVal, String> {
        match self {
            EvalResult::Value(v) => Ok(v),
            EvalResult::TailCall { .. } => {
                Err("EvalResult::unwrap_value called on TailCall".into())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Step — one evaluation step
// ---------------------------------------------------------------------------

pub enum Step {
    Done(LispVal),
    EvalNext {
        expr: LispVal,
        conts: Vec<Cont>,
        new_env: Option<Env>,
    },
}

// ---------------------------------------------------------------------------
// Cont — a continuation
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
pub enum Cont {
    IfBranch {
        then_branch: LispVal,
        else_branch: LispVal,
    },
    CondTest {
        result_expr: Option<LispVal>,
        remaining: Vec<LispVal>,
        is_arrow: bool,
    },
    DefineSet {
        name: String,
    },
    SetVal {
        name: String,
    },
    BeginSeq {
        remaining: Vec<LispVal>,
    },
    AndNext {
        remaining: Vec<LispVal>,
    },
    OrNext {
        remaining: Vec<LispVal>,
    },
    NotArg,
    LetBind {
        name: String,
        remaining_pairs: Vec<(String, LispVal)>,
        body_exprs: Vec<LispVal>,
        bound_keys: Vec<String>,
    },
    LetRestore {
        bound_keys: Vec<String>,
    },
    MatchScrutinee {
        val: LispVal,
        arms: Vec<LispVal>,
    },
    MatchRestore {
        snapshot: im::HashMap<String, LispVal>,
    },
    TryCatch {
        var: String,
        catch_body_exprs: Vec<LispVal>,
    },
    LoopBind {
        names: Vec<String>,
        vals: Vec<LispVal>,
        remaining: Vec<(String, LispVal)>,
        body: LispVal,
    },
    LoopIter {
        binding_names: Vec<String>,
        binding_vals: Vec<LispVal>,
        body: LispVal,
        snapshot: im::HashMap<String, LispVal>,
    },
    RecurArg {
        done: Vec<LispVal>,
        remaining: Vec<LispVal>,
    },
    CaseMatch {
        clauses: Vec<LispVal>,
    },
    DefineValues {
        names: Vec<String>,
    },
    LetValuesBind {
        names: Vec<Vec<String>>,
        remaining_exprs: Vec<LispVal>,
        body_exprs: Vec<LispVal>,
        current_idx: usize,
    },
    /// Collect function arguments one at a time, then dispatch the call
    ArgCollect {
        head: LispVal,                              // function to call (evaluated or symbol)
        done: Vec<LispVal>,                         // already-evaluated args
        remaining: Vec<LispVal>,                    // args still to evaluate
        env_snapshot: im::HashMap<String, LispVal>, // saved env for restore
    },
    FinalVal,
    AssertCheck {
        message: Option<String>,
    },
    RlmSetVal {
        name: String,
    },
}
