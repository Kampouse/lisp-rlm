use crate::types::LispVal;

pub fn is_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "mod"
            | "="
            | "=="
            | "!="
            | "/="
            | "<"
            | ">"
            | "<="
            | ">="
            | "list"
            | "car"
            | "cdr"
            | "cons"
            | "len"
            | "append"
            | "nth"
            | "str-concat"
            | "str-contains"
            | "to-string"
            | "str-length"
            | "str-substring"
            | "str-split"
            | "str-split-exact"
            | "str-trim"
            | "str-index-of"
            | "str-upcase"
            | "str-downcase"
            | "str-starts-with"
            | "str-ends-with"
            | "str="
            | "str!="
            | "nil?"
            | "list?"
            | "number?"
            | "string?"
            | "map?"
            | "bool?"
            | "to-float"
            | "to-int"
            | "to-num"
            | "type?"
            | "dict"
            | "dict/get"
            | "dict/set"
            | "dict/has?"
            | "dict/keys"
            | "dict/vals"
            | "dict/remove"
            | "dict/merge"
            | "error"
            | "empty?"
            | "range"
            | "reverse"
            | "sort"
            | "zip"
            | "map"
            | "filter"
            | "reduce"
            | "find"
            | "some"
            | "every"
            | "print"
            | "println"
            | "file/read"
            | "file/write"
            | "file/exists?"
            | "file/list"
            | "env/get"
            | "rlm/signature"
            | "rlm/format-prompt"
            | "rlm/trace"
            | "rlm/config"
            | "write-file"
            | "read-file"
            | "append-file"
            | "file-exists?"
            | "shell"
            | "shell-bg"
            | "shell-kill"
            | "http-get"
            | "http-post"
            | "http-get-json"
            | "llm"
            | "llm-code"
            | "check"
            | "check!"
            | "matches?"
            | "valid-type?"
            | "type-of"
            | "defschema"
            | "validate"
            | "schema"
            | "snapshot"
            | "rollback"
            | "rollback-to"
            | "rlm"
            | "read-all"
            | "load-file"
            | "sub-rlm"
            | "rlm-tokens"
            | "rlm-calls"
            | "show-vars"
            | "str-chunk"
            | "str-join"
            | "llm-batch"
            | "show-context"
            | "final"
            | "final-var"
    )
}

pub fn is_truthy(v: &LispVal) -> bool {
    !matches!(v, LispVal::Nil | LispVal::Bool(false))
}

pub fn as_num(v: &LispVal) -> Result<i64, String> {
    match v {
        LispVal::Num(n) => Ok(*n),
        _ => Err(format!("expected number, got {}", v)),
    }
}

pub fn as_float(v: &LispVal) -> Result<f64, String> {
    match v {
        LispVal::Float(f) => Ok(*f),
        LispVal::Num(n) => Ok(*n as f64),
        _ => Err(format!("expected number, got {}", v)),
    }
}

pub fn any_float(args: &[LispVal]) -> bool {
    args.iter().any(|a| matches!(a, LispVal::Float(_)))
}

pub fn as_str(v: &LispVal) -> Result<String, String> {
    match v {
        LispVal::Str(s) => Ok(s.clone()),
        LispVal::Sym(s) => Ok(s.clone()),
        LispVal::Num(n) => Ok(n.to_string()),
        LispVal::Float(f) => Ok(f.to_string()),
        _ => Err(format!("expected string, got {}", v)),
    }
}

pub fn do_arith(
    args: &[LispVal],
    op_int: fn(i64, i64) -> i64,
    op_float: fn(f64, f64) -> f64,
) -> Result<LispVal, String> {
    if args.len() < 2 {
        return Err("arith needs 2+ args".into());
    }
    if any_float(args) {
        let init = as_float(&args[0])?;
        let res: Result<f64, String> = args[1..]
            .iter()
            .try_fold(init, |a, b| Ok(op_float(a, as_float(b)?)));
        Ok(LispVal::Float(res?))
    } else {
        let init = as_num(&args[0])?;
        let res: Result<i64, String> = args[1..]
            .iter()
            .try_fold(init, |a, b| Ok(op_int(a, as_num(b)?)));
        Ok(LispVal::Num(res?))
    }
}

pub fn parse_params(val: &LispVal) -> Result<(Vec<String>, Option<String>), String> {
    match val {
        LispVal::List(p) => {
            let mut params = Vec::new();
            let mut rest_param = None;
            let mut seen_rest = false;
            for v in p {
                match v {
                    LispVal::Sym(s) if s == "&rest" => {
                        seen_rest = true;
                    }
                    LispVal::Sym(s) if seen_rest => {
                        rest_param = Some(s.clone());
                        seen_rest = false;
                    }
                    LispVal::Sym(s) => {
                        params.push(s.clone());
                    }
                    _ => return Err("param must be sym".into()),
                }
            }
            Ok((params, rest_param))
        }
        _ => Err("params must be list".into()),
    }
}

pub fn match_pattern(pattern: &LispVal, value: &LispVal) -> Option<Vec<(String, LispVal)>> {
    match pattern {
        LispVal::Sym(s) if s == "_" => Some(vec![]),
        LispVal::Sym(s) if s == "else" => Some(vec![]),
        LispVal::Sym(s) if s.starts_with('?') => Some(vec![(s[1..].to_string(), value.clone())]),
        LispVal::Sym(s) => Some(vec![(s.clone(), value.clone())]),
        LispVal::Num(n) => {
            if value == &LispVal::Num(*n) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::Float(f) => {
            if let LispVal::Float(vf) = value {
                if (*f - *vf).abs() < f64::EPSILON {
                    Some(vec![])
                } else {
                    None
                }
            } else {
                None
            }
        }
        LispVal::Str(s) => {
            if value == &LispVal::Str(s.clone()) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::Bool(b) => {
            if value == &LispVal::Bool(*b) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::List(pats) if !pats.is_empty() => {
            if let LispVal::Sym(s) = &pats[0] {
                if s == "list" {
                    if let LispVal::List(vals) = value {
                        if vals.len() == pats.len() - 1 {
                            let mut all = vec![];
                            for (p, v) in pats[1..].iter().zip(vals.iter()) {
                                if let Some(b) = match_pattern(p, v) {
                                    all.extend(b);
                                } else {
                                    return None;
                                }
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if s == "cons" && pats.len() == 3 {
                    if let LispVal::List(vals) = value {
                        if !vals.is_empty() {
                            let mut all = vec![];
                            if let Some(b1) = match_pattern(&pats[1], &vals[0]) {
                                all.extend(b1);
                            } else {
                                return None;
                            }
                            if let Some(b2) =
                                match_pattern(&pats[2], &LispVal::List(vals[1..].to_vec()))
                            {
                                all.extend(b2);
                            } else {
                                return None;
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    if let LispVal::List(vals) = value {
                        if vals.len() == pats.len() {
                            let mut all = vec![];
                            for (p, v) in pats.iter().zip(vals.iter()) {
                                if let Some(b) = match_pattern(p, v) {
                                    all.extend(b);
                                } else {
                                    return None;
                                }
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            } else {
                if let LispVal::List(vals) = value {
                    if vals.len() == pats.len() {
                        let mut all = vec![];
                        for (p, v) in pats.iter().zip(vals.iter()) {
                            if let Some(b) = match_pattern(p, v) {
                                all.extend(b);
                            } else {
                                return None;
                            }
                        }
                        Some(all)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
        _ => None,
    }
}
