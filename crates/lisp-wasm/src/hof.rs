use crate::emit::WasmEmitter;
use lisp_core::types::LispVal;

impl WasmEmitter {
    pub(crate) fn extract_lambda(form: &LispVal) -> Result<(String, LispVal), String> {
        match form {
            LispVal::List(items) if items.len() >= 3 => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "lambda" || s == "fn" {
                        if let LispVal::List(params) = &items[1] {
                            if let Some(LispVal::Sym(p)) = params.first() {
                                let body = if items.len() > 3 {
                                    LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(items[2..].iter().cloned()).collect())
                                } else { items[2].clone() };
                                return Ok((p.clone(), body));
                            }
                        }
                        if let LispVal::Sym(p) = &items[1] {
                            let body = if items.len() > 3 {
                                LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                    .chain(items[2..].iter().cloned()).collect())
                            } else { items[2].clone() };
                            return Ok((p.clone(), body));
                        }
                    }
                }
                Err(format!("hof: expected (lambda (param) body), got {:?}", form))
            }
            _ => Err(format!("hof: expected lambda form, got {:?}", form)),
        }
    }

    /// Extract (lambda (p1 p2) body) → (vec![p1, p2], body)
    pub(crate) fn extract_lambda_2param(form: &LispVal) -> Result<(Vec<String>, LispVal), String> {
        match form {
            LispVal::List(items) if items.len() >= 3 => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "lambda" || s == "fn" {
                        if let LispVal::List(params) = &items[1] {
                            let names: Vec<String> = params.iter()
                                .filter_map(|p| if let LispVal::Sym(s) = p { Some(s.clone()) } else { None })
                                .collect();
                            if names.len() == 2 {
                                let body = if items.len() > 3 {
                                    LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(items[2..].iter().cloned()).collect())
                                } else { items[2].clone() };
                                return Ok((names, body));
                            }
                        }
                    }
                }
                Err(format!("hof/reduce: expected (lambda (acc x) body), got {:?}", form))
            }
            _ => Err(format!("hof/reduce: expected lambda form, got {:?}", form)),
        }
    }

}
