//! Arithmetic and comparison builtins.

use crate::helpers::*;

pub fn handle(name: &str, args: &[crate::types::LispVal]) -> Result<Option<crate::types::LispVal>, String> {
    use crate::types::LispVal;
    match name {
        "+" => Ok(Some(do_arith(args, |a, b| a + b, |a, b| a + b)?)),
        "-" => Ok(Some(do_arith(args, |a, b| a - b, |a, b| a - b)?)),
        "*" => Ok(Some(do_arith(args, |a, b| a * b, |a, b| a * b)?)),
        "/" => {
            if any_float(args) {
                let b = as_float(args.get(1).ok_or("/ needs 2 args")?)?;
                if b == 0.0 { return Err("div by zero".into()); }
                Ok(Some(LispVal::Float(as_float(&args[0])? / b)))
            } else {
                let b = as_num(args.get(1).ok_or("/ needs 2 args")?)?;
                if b == 0 { return Err("div by zero".into()); }
                Ok(Some(LispVal::Num(as_num(&args[0])? / b)))
            }
        }
        "mod" => Ok(Some(do_arith(args, |a, b| i64::rem_euclid(a, b), |a, b| a % b)?)),
        "=" | "==" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(as_float(&args[0])? == as_float(&args[1])?)))
            } else {
                Ok(Some(LispVal::Bool(args.get(0) == args.get(1))))
            }
        }
        "!=" | "/=" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(as_float(&args[0])? != as_float(&args[1])?)))
            } else {
                Ok(Some(LispVal::Bool(args.get(0) != args.get(1))))
            }
        }
        "<" => {
            if any_float(args) { Ok(Some(LispVal::Bool(as_float(&args[0])? < as_float(&args[1])?))) }
            else { Ok(Some(LispVal::Bool(as_num(&args[0])? < as_num(&args[1])?))) }
        }
        ">" => {
            if any_float(args) { Ok(Some(LispVal::Bool(as_float(&args[0])? > as_float(&args[1])?))) }
            else { Ok(Some(LispVal::Bool(as_num(&args[0])? > as_num(&args[1])?))) }
        }
        "<=" => {
            if any_float(args) { Ok(Some(LispVal::Bool(as_float(&args[0])? <= as_float(&args[1])?))) }
            else { Ok(Some(LispVal::Bool(as_num(&args[0])? <= as_num(&args[1])?))) }
        }
        ">=" => {
            if any_float(args) { Ok(Some(LispVal::Bool(as_float(&args[0])? >= as_float(&args[1])?))) }
            else { Ok(Some(LispVal::Bool(as_num(&args[0])? >= as_num(&args[1])?))) }
        }
        _ => Ok(None),
    }
}
