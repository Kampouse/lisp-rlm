//! Solidity → LispVal translator.
//!
//! Uses `solang-parser` to parse Solidity source into its AST,
//! then walks the tree and emits our `LispVal` IR that feeds
//! into the existing type checker and WASM emitter.

use std::collections::HashSet;

use crate::types::LispVal;
use solang_parser::pt;

pub fn translate_solidity(src: &str) -> Result<Vec<LispVal>, String> {
    let (unit, _comments) = solang_parser::parse(src, 0).map_err(|errs| {
        errs.iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("\n")
    })?;
    let mut output = Vec::new();
    for part in &unit.0 {
        if let pt::SourceUnitPart::ContractDefinition(c) = part {
            output.extend(translate_contract(c)?);
        }
    }
    Ok(output)
}

pub fn translate_solidity_to_lisp(src: &str) -> Result<String, String> {
    let vals = translate_solidity(src)?;
    Ok(vals.iter().map(|v| format!("{}\n", v)).collect())
}

fn lisp_list(items: Vec<LispVal>) -> LispVal {
    LispVal::List(items)
}

struct Ctx<'a> {
    storage: &'a HashSet<String>,
    mappings: &'a HashSet<String>,
}
impl<'a> Ctx<'a> {
    fn is_storage(&self, n: &str) -> bool {
        self.storage.contains(n)
    }
    fn is_mapping(&self, n: &str) -> bool {
        self.mappings.contains(n)
    }
}

fn translate_contract(c: &pt::ContractDefinition) -> Result<Vec<LispVal>, String> {
    let _name = c.name.as_ref().ok_or("contract has no name")?.name.clone();
    let mut storage: HashSet<String> = HashSet::new();
    let mut mappings: HashSet<String> = HashSet::new();
    let mut functions: Vec<&pt::FunctionDefinition> = Vec::new();

    for part in &c.parts {
        if let pt::ContractPart::VariableDefinition(v) = part {
            if let Some(ident) = &v.name {
                storage.insert(ident.name.clone());
                // Detect mapping types: Type(_, Type::Mapping { ... })
                if let pt::Expression::Type(_, pt::Type::Mapping { .. }) = &v.ty {
                    mappings.insert(ident.name.clone());
                }
            }
        }
    }
    for part in &c.parts {
        if let pt::ContractPart::FunctionDefinition(f) = part {
            functions.push(f);
        }
    }

    let mut output = vec![LispVal::List(vec![
        LispVal::Sym("memory".into()),
        LispVal::Num(4),
    ])];
    for part in &c.parts {
        if let pt::ContractPart::VariableDefinition(v) = part {
            output.extend(translate_storage_var(v)?);
        }
    }
    let ctx = Ctx {
        storage: &storage,
        mappings: &mappings,
    };
    for func in &functions {
        output.extend(translate_function(func, &ctx)?);
    }
    Ok(output)
}

fn translate_storage_var(sv: &pt::VariableDefinition) -> Result<Vec<LispVal>, String> {
    let vn = sv
        .name
        .as_ref()
        .ok_or("state var has no name")?
        .name
        .clone();
    if let pt::Expression::Type(_, pt::Type::Mapping { .. }) = &sv.ty {
        return translate_mapping(&vn, &None, &None);
    }
    let gn = format!("get_{}", vn);
    let sn = format!("set_{}", vn);
    Ok(vec![
        lisp_list(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![LispVal::Sym(gn)]),
            lisp_list(vec![
                LispVal::Sym("near/load".into()),
                LispVal::Str(vn.clone()),
            ]),
        ]),
        lisp_list(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![LispVal::Sym(sn), LispVal::Sym("val".into())]),
            lisp_list(vec![
                LispVal::Sym("near/store".into()),
                LispVal::Str(vn),
                LispVal::Sym("val".into()),
            ]),
        ]),
    ])
}

fn translate_mapping(
    name: &str,
    _key: &Option<Box<pt::Expression>>,
    _val: &Option<Box<pt::Expression>>,
) -> Result<Vec<LispVal>, String> {
    let gn = format!("get_{}", name);
    let sn = format!("set_{}", name);
    let prefix = format!("{}:", name);
    Ok(vec![
        lisp_list(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![LispVal::Sym(gn), LispVal::Sym("key".into())]),
            lisp_list(vec![
                LispVal::Sym("near/load".into()),
                lisp_list(vec![
                    LispVal::Sym("str".into()),
                    LispVal::Str(prefix.clone()),
                    lisp_list(vec![
                        LispVal::Sym("to-string".into()),
                        LispVal::Sym("key".into()),
                    ]),
                ]),
            ]),
        ]),
        lisp_list(vec![
            LispVal::Sym("define".into()),
            LispVal::List(vec![
                LispVal::Sym(sn),
                LispVal::Sym("key".into()),
                LispVal::Sym("val".into()),
            ]),
            lisp_list(vec![
                LispVal::Sym("near/store".into()),
                lisp_list(vec![
                    LispVal::Sym("str".into()),
                    LispVal::Str(prefix),
                    lisp_list(vec![
                        LispVal::Sym("to-string".into()),
                        LispVal::Sym("key".into()),
                    ]),
                ]),
                LispVal::Sym("val".into()),
            ]),
        ]),
    ])
}

fn translate_function(func: &pt::FunctionDefinition, ctx: &Ctx) -> Result<Vec<LispVal>, String> {
    let fn_name = match &func.name {
        None => "init".to_string(),
        Some(n) => n.name.clone(),
    };
    let is_pub = func.attributes.iter().any(|a| {
        matches!(
            a,
            pt::FunctionAttribute::Visibility(pt::Visibility::Public(_))
                | pt::FunctionAttribute::Visibility(pt::Visibility::External(_))
        )
    });
    let mut params = vec![LispVal::Sym(fn_name.clone())];
    for (_, p) in &func.params {
        if let Some(p) = p {
            if let Some(n) = &p.name {
                params.push(LispVal::Sym(n.name.clone()));
            }
        }
    }
    let body = match &func.body {
        Some(s) => translate_statement(s, ctx)?,
        None => LispVal::List(vec![LispVal::Sym("nil".into())]),
    };
    let mut out = vec![lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(params),
        body,
    ])];
    if is_pub {
        let is_view = func.attributes.iter().any(|a| {
            matches!(
                a,
                pt::FunctionAttribute::Mutability(pt::Mutability::View(_))
            )
        });
        out.push(lisp_list(vec![
            LispVal::Sym("export".into()),
            LispVal::Str(fn_name.clone()),
            LispVal::Sym(fn_name.clone()),
            LispVal::Bool(is_view),
        ]));
    }
    Ok(out)
}

fn translate_statement(s: &pt::Statement, ctx: &Ctx) -> Result<LispVal, String> {
    match s {
        pt::Statement::Block { statements, .. } => {
            if statements.is_empty() {
                return Ok(LispVal::List(vec![LispVal::Sym("nil".into())]));
            }
            if statements.len() == 1 {
                return translate_statement(&statements[0], ctx);
            }
            let mut items = vec![LispVal::Sym("begin".into())];
            for s in statements {
                items.push(translate_statement(s, ctx)?);
            }
            Ok(lisp_list(items))
        }
        pt::Statement::If(_, cond, then_br, else_br) => {
            let mut items = vec![
                LispVal::Sym("if".into()),
                translate_expr(cond, ctx)?,
                translate_statement(then_br, ctx)?,
            ];
            if let Some(e) = else_br {
                items.push(translate_statement(e, ctx)?);
            }
            Ok(lisp_list(items))
        }
        pt::Statement::For(_, init, cond, update, body) => {
            let mut items = vec![LispVal::Sym("begin".into())];
            if let Some(i) = init {
                items.push(translate_statement(i, ctx)?);
            }
            let cond_expr = match cond {
                Some(c) => translate_expr(c, ctx)?,
                None => LispVal::Bool(true),
            };
            let body_stmt = match body {
                Some(b) => translate_statement(b, ctx)?,
                None => LispVal::List(vec![LispVal::Sym("nil".into())]),
            };
            items.push(lisp_list(vec![
                LispVal::Sym("while".into()),
                cond_expr,
                body_stmt,
            ]));
            if let Some(u) = update {
                items.push(translate_expr(u, ctx)?);
            }
            Ok(lisp_list(items))
        }
        pt::Statement::While(_, cond, body) => Ok(lisp_list(vec![
            LispVal::Sym("while".into()),
            translate_expr(cond, ctx)?,
            translate_statement(body, ctx)?,
        ])),
        pt::Statement::DoWhile(_, body, cond) => Ok(lisp_list(vec![
            LispVal::Sym("begin".into()),
            translate_statement(body, ctx)?,
            lisp_list(vec![
                LispVal::Sym("while".into()),
                translate_expr(cond, ctx)?,
                translate_statement(body, ctx)?,
            ]),
        ])),
        pt::Statement::Return(_, e) => match e {
            Some(e) => translate_expr(e, ctx),
            None => Ok(LispVal::List(vec![LispVal::Sym("nil".into())])),
        },
        pt::Statement::Emit(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("near/log".into()),
            translate_expr(e, ctx)?,
        ])),
        pt::Statement::Revert(_, _, _args) => Ok(lisp_list(vec![
            LispVal::Sym("near/panic".into()),
            LispVal::Str("revert".into()),
        ])),
        pt::Statement::Expression(_, e) => translate_expr(e, ctx),
        pt::Statement::VariableDefinition(_, def, init) => {
            let n = def
                .name
                .as_ref()
                .ok_or("local var has no name")?
                .name
                .clone();
            let v = match init {
                Some(e) => translate_expr(e, ctx)?,
                None => LispVal::Num(0),
            };
            Ok(lisp_list(vec![
                LispVal::Sym("set!".into()),
                LispVal::Sym(n),
                v,
            ]))
        }
        _ => Err(format!("unsupported statement: {:?}", s)),
    }
}

fn translate_expr(expr: &pt::Expression, ctx: &Ctx) -> Result<LispVal, String> {
    match expr {
        // Literals handled directly
        pt::Expression::BoolLiteral(_, b) => Ok(LispVal::Bool(*b)),
        pt::Expression::NumberLiteral(_, n, _, _) => {
            let s = n.to_string().replace('_', "");
            Ok(LispVal::Num(
                s.parse().map_err(|_| format!("bad num: {}", n))?,
            ))
        }
        pt::Expression::HexNumberLiteral(_, s, _) => {
            let c = s.trim_start_matches("0x").replace('_', "");
            Ok(LispVal::Num(
                i64::from_str_radix(&c, 16).map_err(|_| format!("bad hex: {}", s))?,
            ))
        }
        pt::Expression::StringLiteral(ss) => {
            Ok(LispVal::Str(ss.iter().map(|s| s.string.clone()).collect()))
        }
        pt::Expression::HexLiteral(ss) => {
            Ok(LispVal::Str(ss.iter().map(|s| s.hex.clone()).collect()))
        }
        pt::Expression::AddressLiteral(_, s) => Ok(LispVal::Str(s.clone())),

        pt::Expression::Variable(ident) => {
            let n = &ident.name;
            if ctx.is_storage(n) && !ctx.is_mapping(n) {
                return Ok(lisp_list(vec![LispVal::Sym(format!("get_{}", n))]));
            }
            if ctx.is_mapping(n) {
                return Err(format!("mapping '{}' needs subscript", n));
            }
            Ok(LispVal::Sym(n.clone()))
        }

        // Binary ops
        pt::Expression::Add(_, l, r)
        | pt::Expression::Subtract(_, l, r)
        | pt::Expression::Multiply(_, l, r)
        | pt::Expression::Divide(_, l, r)
        | pt::Expression::Modulo(_, l, r)
        | pt::Expression::Power(_, l, r) => {
            let op = match expr {
                pt::Expression::Add(..) => "+",
                pt::Expression::Subtract(..) => "-",
                pt::Expression::Multiply(..) => "*",
                pt::Expression::Divide(..) => "/",
                pt::Expression::Modulo(..) => "mod",
                pt::Expression::Power(..) => "pow",
                _ => unreachable!(),
            };
            Ok(lisp_list(vec![
                LispVal::Sym(op.into()),
                translate_expr(l, ctx)?,
                translate_expr(r, ctx)?,
            ]))
        }
        pt::Expression::Equal(_, l, r)
        | pt::Expression::NotEqual(_, l, r)
        | pt::Expression::Less(_, l, r)
        | pt::Expression::LessEqual(_, l, r)
        | pt::Expression::More(_, l, r)
        | pt::Expression::MoreEqual(_, l, r) => {
            let op = match expr {
                pt::Expression::Equal(..) => "=",
                pt::Expression::NotEqual(..) => "!=",
                pt::Expression::Less(..) => "<",
                pt::Expression::LessEqual(..) => "<=",
                pt::Expression::More(..) => ">",
                pt::Expression::MoreEqual(..) => ">=",
                _ => unreachable!(),
            };
            Ok(lisp_list(vec![
                LispVal::Sym(op.into()),
                translate_expr(l, ctx)?,
                translate_expr(r, ctx)?,
            ]))
        }
        pt::Expression::And(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("and".into()),
            translate_expr(l, ctx)?,
            translate_expr(r, ctx)?,
        ])),
        pt::Expression::Or(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("or".into()),
            translate_expr(l, ctx)?,
            translate_expr(r, ctx)?,
        ])),
        pt::Expression::Not(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("not".into()),
            translate_expr(e, ctx)?,
        ])),
        pt::Expression::BitwiseAnd(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("band".into()),
            translate_expr(l, ctx)?,
            translate_expr(r, ctx)?,
        ])),
        pt::Expression::BitwiseOr(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("bor".into()),
            translate_expr(l, ctx)?,
            translate_expr(r, ctx)?,
        ])),
        pt::Expression::BitwiseXor(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("bxor".into()),
            translate_expr(l, ctx)?,
            translate_expr(r, ctx)?,
        ])),
        pt::Expression::BitwiseNot(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("bnot".into()),
            translate_expr(e, ctx)?,
        ])),
        pt::Expression::Negate(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("-".into()),
            LispVal::Num(0),
            translate_expr(e, ctx)?,
        ])),
        pt::Expression::Parenthesis(_, e) => translate_expr(e, ctx),

        pt::Expression::PostIncrement(_, e) | pt::Expression::PreIncrement(_, e) => {
            translate_increment(e, ctx, 1)
        }
        pt::Expression::PostDecrement(_, e) | pt::Expression::PreDecrement(_, e) => {
            translate_increment(e, ctx, -1)
        }

        pt::Expression::ConditionalOperator(_, c, t, e) => Ok(lisp_list(vec![
            LispVal::Sym("if".into()),
            translate_expr(c, ctx)?,
            translate_expr(t, ctx)?,
            translate_expr(e, ctx)?,
        ])),

        pt::Expression::Assign(_, t, v) => translate_assignment(t, translate_expr(v, ctx)?, ctx),
        pt::Expression::AssignAdd(_, t, v)
        | pt::Expression::AssignSubtract(_, t, v)
        | pt::Expression::AssignMultiply(_, t, v)
        | pt::Expression::AssignDivide(_, t, v)
        | pt::Expression::AssignModulo(_, t, v) => {
            let op = match expr {
                pt::Expression::AssignAdd(..) => "+",
                pt::Expression::AssignSubtract(..) => "-",
                pt::Expression::AssignMultiply(..) => "*",
                pt::Expression::AssignDivide(..) => "/",
                pt::Expression::AssignModulo(..) => "mod",
                _ => unreachable!(),
            };
            let rhs = translate_expr(v, ctx)?;
            let cur = translate_expr(t, ctx)?;
            translate_assignment(t, lisp_list(vec![LispVal::Sym(op.into()), cur, rhs]), ctx)
        }

        pt::Expression::ArraySubscript(_, arr, idx) => {
            let index = idx.as_ref().ok_or("subscript missing index")?;
            if let pt::Expression::Variable(ident) = arr.as_ref() {
                if ctx.is_mapping(&ident.name) {
                    return Ok(lisp_list(vec![
                        LispVal::Sym(format!("get_{}", ident.name)),
                        translate_expr(index, ctx)?,
                    ]));
                }
            }
            Ok(lisp_list(vec![
                LispVal::Sym("nth".into()),
                translate_expr(arr, ctx)?,
                translate_expr(index, ctx)?,
            ]))
        }

        pt::Expression::MemberAccess(_, obj, field) => {
            let f = field.name.clone();
            if let pt::Expression::Variable(ident) = obj.as_ref() {
                match ident.name.as_str() {
                    "msg" => match f.as_str() {
                        "sender" => {
                            return Ok(lisp_list(vec![LispVal::Sym(
                                "near/signer_account_id".into(),
                            )]))
                        }
                        "value" => {
                            return Ok(lisp_list(vec![LispVal::Sym(
                                "near/attached_deposit".into(),
                            )]))
                        }
                        "data" => return Ok(lisp_list(vec![LispVal::Sym("near/input".into())])),
                        _ => {}
                    },
                    "block" => match f.as_str() {
                        "timestamp" => {
                            return Ok(lisp_list(vec![LispVal::Sym("near/block_timestamp".into())]))
                        }
                        "number" => {
                            return Ok(lisp_list(vec![LispVal::Sym("near/block_height".into())]))
                        }
                        _ => {}
                    },
                    "tx" => match f.as_str() {
                        "origin" => {
                            return Ok(lisp_list(vec![LispVal::Sym("near/predecessor".into())]))
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            Ok(lisp_list(vec![translate_expr(obj, ctx)?, LispVal::Sym(f)]))
        }

        pt::Expression::FunctionCall(_, func, args) => {
            if let pt::Expression::Variable(ident) = func.as_ref() {
                match ident.name.as_str() {
                    "require" => {
                        let cond = args.first().ok_or("require needs condition")?;
                        let msg = if args.len() > 1 {
                            translate_expr(&args[1], ctx)?
                        } else {
                            LispVal::Str("assertion failed".into())
                        };
                        return Ok(lisp_list(vec![
                            LispVal::Sym("assert".into()),
                            translate_expr(cond, ctx)?,
                            msg,
                        ]));
                    }
                    "assert" => {
                        let cond = args.first().ok_or("assert needs condition")?;
                        return Ok(lisp_list(vec![
                            LispVal::Sym("assert".into()),
                            translate_expr(cond, ctx)?,
                            LispVal::Str("assertion failed".into()),
                        ]));
                    }
                    "revert" => {
                        return Ok(lisp_list(vec![
                            LispVal::Sym("near/panic".into()),
                            LispVal::Str("revert".into()),
                        ]))
                    }
                    "keccak256" | "sha256" => {
                        let arg = args.first().ok_or("hash needs arg")?;
                        return Ok(lisp_list(vec![
                            LispVal::Sym(format!("near/{}", ident.name)),
                            translate_expr(arg, ctx)?,
                        ]));
                    }
                    _ => {}
                }
            }
            let mut items = vec![translate_expr(func, ctx)?];
            for a in args {
                items.push(translate_expr(a, ctx)?);
            }
            Ok(lisp_list(items))
        }

        pt::Expression::New(_, e) => translate_expr(e, ctx),

        _ => Err(format!("unsupported expr: {:?}", expr)),
    }
}

fn translate_increment(target: &pt::Expression, ctx: &Ctx, delta: i64) -> Result<LispVal, String> {
    let cur = translate_expr(target, ctx)?;
    translate_assignment(
        target,
        lisp_list(vec![LispVal::Sym("+".into()), cur, LispVal::Num(delta)]),
        ctx,
    )
}

fn translate_assignment(
    target: &pt::Expression,
    rhs: LispVal,
    ctx: &Ctx,
) -> Result<LispVal, String> {
    if let pt::Expression::ArraySubscript(_, arr, idx) = target {
        if let pt::Expression::Variable(ident) = arr.as_ref() {
            if ctx.is_mapping(&ident.name) {
                let index = idx.as_ref().ok_or("subscript needs index")?;
                return Ok(lisp_list(vec![
                    LispVal::Sym(format!("set_{}", ident.name)),
                    translate_expr(index, ctx)?,
                    rhs,
                ]));
            }
        }
    }
    if let pt::Expression::Variable(ident) = target {
        if ctx.is_storage(&ident.name) {
            return Ok(lisp_list(vec![
                LispVal::Sym(format!("set_{}", ident.name)),
                rhs,
            ]));
        }
        return Ok(lisp_list(vec![
            LispVal::Sym("set!".into()),
            LispVal::Sym(ident.name.clone()),
            rhs,
        ]));
    }
    Err(format!("unsupported assignment target: {:?}", target))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tr(src: &str) -> String {
        translate_solidity(src)
            .unwrap()
            .iter()
            .map(|v| format!("{}\n", v))
            .collect()
    }

    #[test]
    fn test_simple() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract C { uint256 count; function inc() public { count = count + 1; } }"#,
        );
        assert!(s.contains("(set_count (+ (get_count) 1))"), "{}", s);
    }
    #[test]
    fn test_arith() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract M { function f(uint a, uint b) public pure returns (uint) { return a + b; } }"#,
        );
        assert!(s.contains("(+ a b)"), "{}", s);
    }
    #[test]
    fn test_storage() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract S { uint256 val; function set(uint v) public { val = v; } }"#,
        );
        assert!(s.contains("(near/load"), "missing load: {}", s);
        assert!(s.contains("(near/store"), "missing store: {}", s);
    }
    #[test]
    fn test_require() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract G { function f(uint x) public pure { require(x > 0); } }"#,
        );
        assert!(s.contains("(assert"), "{}", s);
    }
    #[test]
    fn test_msg() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract A { function f() public view returns (address) { return msg.sender; } }"#,
        );
        assert!(s.contains("near/signer_account_id"), "{}", s);
    }
    #[test]
    fn test_memory() {
        let s = tr(r#"pragma solidity ^0.8.0; contract E {}"#);
        assert!(s.contains("(memory 4)"), "{}", s);
    }
    #[test]
    fn test_mapping() {
        let s = tr(
            r#"pragma solidity ^0.8.0; contract L { mapping(address => uint) bal; function set(address a, uint v) public { bal[a] = v; } function get(address a) public view returns (uint) { return bal[a]; } }"#,
        );
        assert!(s.contains("(get_bal a)"), "read: {}", s);
        assert!(s.contains("(set_bal a v)"), "write: {}", s);
    }
}
