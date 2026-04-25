//! Predicate and type conversion builtins.

use crate::types::LispVal;

pub fn handle(name: &str, args: &[LispVal]) -> Result<Option<LispVal>, String> {
    match name {
        "nil?" => Ok(Some(LispVal::Bool(
            matches!(&args[0], LispVal::Nil)
                || matches!(&args[0], LispVal::List(ref v) if v.is_empty()),
        ))),
        "list?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::List(_))))),
        "number?" => Ok(Some(LispVal::Bool(matches!(
            &args[0],
            LispVal::Num(_) | LispVal::Float(_)
        )))),
        "bool?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::Bool(_))))),
        "string?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::Str(_))))),
        "map?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::Map(_))))),
        "macro?" => Ok(Some(LispVal::Bool(matches!(
            &args[0],
            LispVal::Macro { .. }
        )))),
        "type?" => Ok(Some(LispVal::Str(
            match &args[0] {
                LispVal::Nil => "nil",
                LispVal::Bool(_) => "boolean",
                LispVal::Num(_) => "number",
                LispVal::Float(_) => "number",
                LispVal::Str(_) => "string",
                LispVal::List(_) => "list",
                LispVal::Map(_) => "map",
                LispVal::Lambda { .. } => "lambda",
                LispVal::Macro { .. } => "macro",
                LispVal::Sym(_) => "symbol",
                _ => "unknown",
            }
            .to_string(),
        ))),
        "to-float" => match &args[0] {
            LispVal::Float(f) => Ok(Some(LispVal::Float(*f))),
            LispVal::Num(n) => Ok(Some(LispVal::Float(*n as f64))),
            LispVal::Str(s) => s
                .parse::<f64>()
                .map(LispVal::Float)
                .map(Some)
                .map_err(|_| format!("to-float: cannot parse '{}'", s)),
            other => Err(format!("to-float: expected number, got {}", other)),
        },
        "to-int" => match &args[0] {
            LispVal::Num(n) => Ok(Some(LispVal::Num(*n))),
            LispVal::Float(f) => Ok(Some(LispVal::Num(*f as i64))),
            LispVal::Str(s) => s
                .parse::<i64>()
                .map(LispVal::Num)
                .map(Some)
                .map_err(|_| format!("to-int: cannot parse '{}'", s)),
            other => Err(format!("to-int: expected number, got {}", other)),
        },
        "to-num" => match &args[0] {
            LispVal::Num(n) => Ok(Some(LispVal::Num(*n))),
            LispVal::Float(f) => Ok(Some(LispVal::Num(*f as i64))),
            LispVal::Str(s) => s
                .parse::<i64>()
                .map(LispVal::Num)
                .map(Some)
                .map_err(|_| format!("to-num: cannot parse '{}'", s)),
            other => Err(format!("to-num: expected number, got {}", other)),
        },
        // R7RS aliases
        "null?" => handle("nil?", args),
        "boolean?" => handle("bool?", args),
        "pair?" => handle("list?", args),
        "symbol?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::Sym(_))))),
        "procedure?" => Ok(Some(LispVal::Bool(matches!(&args[0], LispVal::Lambda { .. })))),
        "exact" => Ok(Some(args[0].clone())), // identity for integers
        "inexact" => match &args[0] {
            LispVal::Num(n) => Ok(Some(LispVal::Float(*n as f64))),
            other => Ok(Some(other.clone())),
        },
        _ => Ok(None),
    }
}
