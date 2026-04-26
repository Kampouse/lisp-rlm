//! Runtime type probing — infer behavioral types by running pure functions with sample inputs.
//!
//! Given a pure lambda, probe each parameter with representative values of each type,
//! observe which inputs succeed and what type the output is. Build an inferred signature.

use crate::types::{Env, EvalState, LispVal};
use crate::eval::lisp_eval;
use crate::parser::parse_all;

/// Representative sample values for each type.
fn sample_values() -> Vec<(&'static str, LispVal)> {
    vec![
        ("int", LispVal::Num(42)),
        ("float", LispVal::Float(3.14)),
        ("str", LispVal::Str("hello".into())),
        ("bool", LispVal::Bool(true)),
        ("nil", LispVal::Nil),
        ("list", LispVal::List(vec![LispVal::Num(1), LispVal::Num(2)])),
    ]
}

/// Map a LispVal to its runtime type keyword.
fn type_keyword(val: &LispVal) -> &'static str {
    match val {
        LispVal::Num(_) => "int",
        LispVal::Float(_) => "float",
        LispVal::Str(_) => "str",
        LispVal::Bool(_) => "bool",
        LispVal::Nil => "nil",
        LispVal::Sym(s) if s.starts_with(':') => "keyword",
        LispVal::Sym(_) => "sym",
        LispVal::List(_) => "list",
        LispVal::Map(_) => "map",
        LispVal::Lambda { .. } => "fn",
        LispVal::Memoized { .. } => "fn",
        LispVal::CaseLambda { .. } => "fn",
        _ => "any",
    }
}

/// Result of type probing a single parameter.
#[derive(Debug)]
struct ParamProbe {
    /// Type names that the parameter accepted (didn't error).
    accepted: Vec<String>,
    /// (input_type -> output_type) for accepted inputs.
    io_types: Vec<(String, String)>,
}

/// Probe a lambda by testing each parameter independently with sample values.
/// Returns (param_types, return_type) as Lisp-friendly type descriptors.
pub fn probe_function(
    func: &LispVal,
    env: &mut Env,
    state: &mut EvalState,
) -> Result<(Vec<String>, String), String> {
    let (params, _rest, body, closed_env) = match func {
        LispVal::Lambda { params, rest_param, body, closed_env, .. } => {
            (params.clone(), rest_param.clone(), body.clone(), closed_env.clone())
        }
        _ => return Err("infer-type: expected lambda".into()),
    };

    if params.is_empty() {
        // No params — just run it and check return type
        let result = call_with_args(func, &[], env, state)?;
        return Ok((vec![], type_keyword(&result).to_string()));
    }

    let samples = sample_values();
    let mut param_probes: Vec<ParamProbe> = Vec::new();

    for (idx, _param_name) in params.iter().enumerate() {
        let mut probe = ParamProbe {
            accepted: Vec::new(),
            io_types: Vec::new(),
        };

        for (type_name, sample_val) in &samples {
            // Build args: all params get a default except the one being probed
            let mut args: Vec<LispVal> = Vec::new();
            for (i, _) in params.iter().enumerate() {
                if i == idx {
                    args.push(sample_val.clone());
                } else {
                    // Default: use int as filler for other params
                    args.push(LispVal::Num(0));
                }
            }

            match call_with_args(func, &args, env, state) {
                Ok(result) => {
                    probe.accepted.push(type_name.to_string());
                    probe.io_types.push((type_name.to_string(), type_keyword(&result).to_string()));
                }
                Err(_) => {
                    // This type was rejected — parameter doesn't accept it
                }
            }
        }

        param_probes.push(probe);
    }

    // Build type descriptors
    let mut param_types: Vec<String> = Vec::new();
    let mut return_types: Vec<String> = Vec::new();

    for probe in &param_probes {
        if probe.accepted.len() == samples.len() {
            param_types.push(":any".to_string());
        } else if probe.accepted.len() == 1 {
            param_types.push(format!(":{}", probe.accepted[0]));
        } else {
            // Union of accepted types
            let arms: Vec<String> = probe.accepted.iter().map(|t| format!(":{}", t)).collect();
            param_types.push(format!("(:or {})", arms.join(" ")));
        }

        // Collect return types
        for (_, ret) in &probe.io_types {
            if !return_types.contains(&ret.to_string()) {
                return_types.push(ret.clone());
            }
        }
    }

    let return_type = if return_types.len() == 1 {
        format!(":{}", return_types[0])
    } else if return_types.is_empty() {
        ":any".to_string()
    } else {
        let arms: Vec<String> = return_types.iter().map(|t| format!(":{}", t)).collect();
        format!("(:or {})", arms.join(" "))
    };

    Ok((param_types, return_type))
}

/// Call a lambda with given arguments using a forked env.
fn call_with_args(
    func: &LispVal,
    args: &[LispVal],
    _env: &mut Env,
    _state: &mut EvalState,
) -> Result<LispVal, String> {
    // Create a fresh env and eval a function application
    let args_str: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let call_expr = if args.is_empty() {
        format!("(__probe_fn)")
    } else {
        format!("(__probe_fn {})", args_str.join(" "))
    };

    let mut env = Env::new();
    env.push("__probe_fn".to_string(), func.clone());

    let mut state = EvalState::new();
    let exprs = parse_all(&call_expr).map_err(|e| e.to_string())?;
    let mut result = LispVal::Nil;
    for expr in &exprs {
        result = lisp_eval(expr, &mut env, &mut state)?;
    }
    Ok(result)
}

/// Format a probe result as a Lisp-readable type signature string.
pub fn format_signature(param_types: &[String], return_type: &str) -> String {
    if param_types.is_empty() {
        format!("(→ {})", return_type)
    } else {
        let params = param_types.join(" ");
        format!("({} → {})", params, return_type)
    }
}
