//! Arithmetic and comparison builtins.

use crate::helpers::*;

pub fn handle(
    name: &str,
    args: &[crate::types::LispVal],
) -> Result<Option<crate::types::LispVal>, String> {
    use crate::types::LispVal;
    match name {
        "+" => Ok(Some(do_arith(args, |a, b| a + b, |a, b| a + b)?)),
        "-" => Ok(Some(do_arith(args, |a, b| a - b, |a, b| a - b)?)),
        "*" => Ok(Some(do_arith(args, |a, b| a * b, |a, b| a * b)?)),
        "/" => {
            if any_float(args) {
                let b = as_float(args.get(1).ok_or("/ needs 2 args")?)?;
                if b == 0.0 {
                    return Err("div by zero".into());
                }
                Ok(Some(LispVal::Float(as_float(&args[0])? / b)))
            } else {
                let b = as_num(args.get(1).ok_or("/ needs 2 args")?)?;
                if b == 0 {
                    return Err("div by zero".into());
                }
                Ok(Some(LispVal::Num(as_num(&args[0])? / b)))
            }
        }
        "mod" => Ok(Some(do_arith(
            args,
            |a, b| i64::rem_euclid(a, b),
            |a, b| a % b,
        )?)),
        "=" | "==" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? == as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(args.get(0) == args.get(1))))
            }
        }
        "!=" | "/=" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? != as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(args.get(0) != args.get(1))))
            }
        }
        "<" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? < as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(as_num(&args[0])? < as_num(&args[1])?)))
            }
        }
        ">" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? > as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(as_num(&args[0])? > as_num(&args[1])?)))
            }
        }
        "<=" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? <= as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(as_num(&args[0])? <= as_num(&args[1])?)))
            }
        }
        ">=" => {
            if any_float(args) {
                Ok(Some(LispVal::Bool(
                    as_float(&args[0])? >= as_float(&args[1])?,
                )))
            } else {
                Ok(Some(LispVal::Bool(as_num(&args[0])? >= as_num(&args[1])?)))
            }
        }
        // ── Tier 1: Numeric ──
        "abs" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Num(n.abs()))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Float(f.abs()))),
            _ => Err("abs: need number".into()),
        },
        "min" => {
            if args.is_empty() {
                return Err("min: need at least 1 arg".into());
            }
            if any_float(args) {
                let mut best = as_float(&args[0])?;
                for a in &args[1..] {
                    let v = as_float(a)?;
                    if v < best {
                        best = v;
                    }
                }
                Ok(Some(LispVal::Float(best)))
            } else {
                let mut best = as_num(&args[0])?;
                for a in &args[1..] {
                    let v = as_num(a)?;
                    if v < best {
                        best = v;
                    }
                }
                Ok(Some(LispVal::Num(best)))
            }
        }
        "max" => {
            if args.is_empty() {
                return Err("max: need at least 1 arg".into());
            }
            if any_float(args) {
                let mut best = as_float(&args[0])?;
                for a in &args[1..] {
                    let v = as_float(a)?;
                    if v > best {
                        best = v;
                    }
                }
                Ok(Some(LispVal::Float(best)))
            } else {
                let mut best = as_num(&args[0])?;
                for a in &args[1..] {
                    let v = as_num(a)?;
                    if v > best {
                        best = v;
                    }
                }
                Ok(Some(LispVal::Num(best)))
            }
        }
        "floor" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Num(*n))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Num(f.floor() as i64))),
            _ => Err("floor: need number".into()),
        },
        "ceiling" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Num(*n))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Num(f.ceil() as i64))),
            _ => Err("ceiling: need number".into()),
        },
        "round" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Num(*n))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Num(f.round() as i64))),
            _ => Err("round: need number".into()),
        },
        "sqrt" => match args.first() {
            Some(LispVal::Num(n)) => {
                let n = *n as f64;
                let r = n.sqrt();
                if r == r.floor() {
                    Ok(Some(LispVal::Num(r as i64)))
                } else {
                    Ok(Some(LispVal::Float(r)))
                }
            }
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Float(f.sqrt()))),
            _ => Err("sqrt: need number".into()),
        },
        "number->string" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Str(n.to_string()))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Str(f.to_string()))),
            _ => Err("number->string: need number".into()),
        },
        "expt" => {
            let base = as_num(args.first().ok_or("expt: need base")?)?;
            let exp = as_num(args.get(1).ok_or("expt: need exponent")?)?;
            let result = (base as f64).powf(exp as f64);
            if result == result.floor() && result.abs() < 1e18 {
                Ok(Some(LispVal::Num(result as i64)))
            } else {
                Ok(Some(LispVal::Float(result)))
            }
        }
        "atan" => {
            let y = as_num(args.first().ok_or("atan: need number")?)?;
            if args.len() >= 2 {
                let x = as_num(args.get(1).ok_or("atan: need x")?)?;
                Ok(Some(LispVal::Float((y as f64).atan2(x as f64))))
            } else {
                Ok(Some(LispVal::Float((y as f64).atan())))
            }
        }
        "exact-integer-sqrt" => {
            let n = as_num(args.first().ok_or("exact-integer-sqrt: need number")?)?;
            if n < 0 { return Err("exact-integer-sqrt: need non-negative".into()); }
            let s = (n as f64).sqrt().floor() as i64;
            let r = n - s * s;
            Ok(Some(LispVal::List(vec![LispVal::Num(s), LispVal::Num(r)])))
        }
        "exp" => {
            let n = as_num(args.first().ok_or("exp: need number")?)?;
            Ok(Some(LispVal::Float((n as f64).exp())))
        }
        "rational?" => {
            match args.first() {
                Some(LispVal::Num(_)) | Some(LispVal::Float(_)) => Ok(Some(LispVal::Bool(true))),
                _ => Ok(Some(LispVal::Bool(false))),
            }
        }
        // R7RS arithmetic aliases
        "zero?" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Bool(*n == 0))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Bool(*f == 0.0))),
            _ => Err("zero?: need number".into()),
        },
        "positive?" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Bool(*n > 0))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Bool(*f > 0.0))),
            _ => Err("positive?: need number".into()),
        },
        "negative?" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Bool(*n < 0))),
            Some(LispVal::Float(f)) => Ok(Some(LispVal::Bool(*f < 0.0))),
            _ => Err("negative?: need number".into()),
        },
        "even?" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Bool(n % 2 == 0))),
            _ => Err("even?: need integer".into()),
        },
        "odd?" => match args.first() {
            Some(LispVal::Num(n)) => Ok(Some(LispVal::Bool(n % 2 != 0))),
            _ => Err("odd?: need integer".into()),
        },
        // R7RS arithmetic aliases
        "modulo" => handle("mod", args),
        "remainder" => handle("mod", args),
        "quotient" => match (&args[0], &args[1]) {
            (LispVal::Num(a), LispVal::Num(b)) if *b != 0 => Ok(Some(LispVal::Num(a / b))),
            _ => Err("quotient: need two numbers".into()),
        },
        _ => Ok(None),
    }
}
