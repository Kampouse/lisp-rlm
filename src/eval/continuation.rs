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
    pub fn unwrap_value(self) -> LispVal {
        match self {
            EvalResult::Value(v) => v,
            EvalResult::TailCall { .. } => panic!("EvalResult::unwrap_value on TailCall"),
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
pub enum Cont {
    IfBranch { then_branch: LispVal, else_branch: LispVal },
    CondTest { result_expr: Option<LispVal>, remaining: Vec<LispVal> },
    DefineSet { name: String },
    SetVal { name: String },
    BeginSeq { remaining: Vec<LispVal> },
    AndNext { remaining: Vec<LispVal> },
    OrNext { remaining: Vec<LispVal> },
    NotArg,
    LetBind { name: String, remaining_pairs: Vec<(String, LispVal)>, body_exprs: Vec<LispVal> },
    LetRestore { snapshot: im::HashMap<String, LispVal> },
    MatchScrutinee { val: LispVal, arms: Vec<LispVal> },
    MatchRestore { snapshot: im::HashMap<String, LispVal> },
    TryCatch { var: String, catch_body_exprs: Vec<LispVal> },
    LoopBind { names: Vec<String>, vals: Vec<LispVal>, remaining: Vec<(String, LispVal)>, body: LispVal },
    LoopIter { binding_names: Vec<String>, binding_vals: Vec<LispVal>, body: LispVal, snapshot: im::HashMap<String, LispVal> },
    RecurArg { done: Vec<LispVal>, remaining: Vec<LispVal> },
    FinalVal,
    AssertCheck { message: Option<String> },
    RlmSetVal { name: String },
}
