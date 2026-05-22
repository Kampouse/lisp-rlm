//! Solidity → LispVal translator.
//!
//! Uses `solang-parser` to parse Solidity source into its AST,
//! then walks the tree and emits our `LispVal` IR that feeds
//! into the existing type checker → WASM emit → NEAR pipeline.
//!
//! Mapping:
//!   contract       → (memory 4) + (define ...) + (export ...)
//!   function       → (define (name params...) body)
//!   mapping(K=>V)  → near/storage_read/write with key encoding
//!   msg.sender     → (near/signer_account_id)
//!   msg.value      → (near/attached_deposit)
//!   require(cond)  → (assert cond "require failed")
//!   emit Event(..) → (near/log ...)
//!   address        → string (NEAR uses string accounts)
//!   uint256        → i64
//!   bool/string    → direct

use crate::types::LispVal;
use solang_parser::pt;
use std::collections::HashSet;

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse Solidity source and translate to a Vec<LispVal> (our IR).
pub fn translate_solidity(src: &str) -> Result<Vec<LispVal>, String> {
    let (unit, _comments) =
        solang_parser::parse(src, 0).map_err(|errs| {
            errs.iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n")
        })?;

    let mut output = Vec::new();
    for part in &unit.0 {
        match part {
            pt::SourceUnitPart::ContractDefinition(c) => {
                output.extend(translate_contract(c)?);
            }
            pt::SourceUnitPart::PragmaDirective(_) => { /* skip */ }
            pt::SourceUnitPart::ImportDirective(_) => { /* skip for now */ }
            _ => {}
        }
    }

    if output.is_empty() {
        return Err("no contract definitions found".into());
    }
    Ok(output)
}

/// Parse Solidity source and return the Lisp IR as a string (for debugging).
pub fn translate_solidity_to_lisp(src: &str) -> Result<String, String> {
    let vals = translate_solidity(src)?;
    Ok(vals
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

// ── Contract ────────────────────────────────────────────────────────────────

fn translate_contract(c: &pt::ContractDefinition) -> Result<Vec<LispVal>, String> {
    let _name = c.name
        .as_ref()
        .ok_or_else(|| "contract has no name".to_string())?
        .name
        .clone();

    let mut storage_vars: HashSet<String> = HashSet::new();
    let mut functions: Vec<&pt::FunctionDefinition> = Vec::new();
    let mut events: Vec<&pt::EventDefinition> = Vec::new();
    let mut structs: Vec<&pt::StructDefinition> = Vec::new();
    let mut enums: Vec<&pt::EnumDefinition> = Vec::new();

    // First pass: collect storage var names
    for part in &c.parts {
        if let pt::ContractPart::VariableDefinition(v) = part {
            if let Some(ident) = &v.name {
                storage_vars.insert(ident.name.clone());
            }
        }
    }

    // Second pass: collect everything else
    for part in &c.parts {
        match part {
            pt::ContractPart::FunctionDefinition(f) => functions.push(f),
            pt::ContractPart::EventDefinition(e) => events.push(e),
            pt::ContractPart::StructDefinition(s) => structs.push(s),
            pt::ContractPart::EnumDefinition(e) => enums.push(e),
            _ => {}
        }
    }

    let mut output = Vec::new();

    // NEAR needs (memory N) directive
    output.push(LispVal::List(vec![
        LispVal::Sym("memory".into()),
        LispVal::Num(4),
    ]));

    // Emit storage getters/setters for each state variable
    for part in &c.parts {
        if let pt::ContractPart::VariableDefinition(v) = part {
            output.extend(translate_storage_var(v)?);
        }
    }

    // Emit functions
    for func in &functions {
        output.extend(translate_function(func, &storage_vars)?);
    }

    Ok(output)
}

// ── Storage Variables ───────────────────────────────────────────────────────

fn translate_storage_var(sv: &pt::VariableDefinition) -> Result<Vec<LispVal>, String> {
    let var_name = sv.name
        .as_ref()
        .ok_or_else(|| "state variable has no name".to_string())?
        .name
        .clone();

    let ty = &sv.ty;
    let mut output = Vec::new();

    // Check if it's a mapping
    if let pt::Expression::Type(_, pt::Type::Mapping { key, value, .. }) = ty {
        return translate_mapping(&var_name, key, value);
    }

    // getter: (define (get_VARNAME) (near/load "VARNAME"))
    let getter_name = format!("get_{}", var_name);
    output.push(lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(vec![LispVal::Sym(getter_name.clone())]),
        lisp_list(vec![
            LispVal::Sym("near/load".into()),
            LispVal::Str(var_name.clone()),
        ]),
    ]));

    // setter: (define (set_VARNAME val) (near/store "VARNAME" val))
    let setter_name = format!("set_{}", var_name);
    output.push(lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(vec![
            LispVal::Sym(setter_name.clone()),
            LispVal::Sym("val".into()),
        ]),
        lisp_list(vec![
            LispVal::Sym("near/store".into()),
            LispVal::Str(var_name.clone()),
            LispVal::Sym("val".into()),
        ]),
    ]));

    Ok(output)
}

fn translate_mapping(
    name: &str,
    _key: &pt::Expression,
    _value: &pt::Expression,
) -> Result<Vec<LispVal>, String> {
    let mut output = Vec::new();

    // getter: (define (get_MAPNAME key) (near/load (str-concat "MAPNAME:" (to-string key))))
    let getter_name = format!("get_{}", name);
    output.push(lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(vec![
            LispVal::Sym(getter_name.clone()),
            LispVal::Sym("key".into()),
        ]),
        lisp_list(vec![
            LispVal::Sym("near/load".into()),
            lisp_list(vec![
                LispVal::Sym("str-concat".into()),
                LispVal::Str(format!("{}:", name)),
                lisp_list(vec![
                    LispVal::Sym("to-string".into()),
                    LispVal::Sym("key".into()),
                ]),
            ]),
        ]),
    ]));

    // setter: (define (set_MAPNAME key val) (near/store (str-concat ...) val))
    let setter_name = format!("set_{}", name);
    output.push(lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(vec![
            LispVal::Sym(setter_name.clone()),
            LispVal::Sym("key".into()),
            LispVal::Sym("val".into()),
        ]),
        lisp_list(vec![
            LispVal::Sym("near/store".into()),
            lisp_list(vec![
                LispVal::Sym("str-concat".into()),
                LispVal::Str(format!("{}:", name)),
                lisp_list(vec![
                    LispVal::Sym("to-string".into()),
                    LispVal::Sym("key".into()),
                ]),
            ]),
            LispVal::Sym("val".into()),
        ]),
    ]));

    Ok(output)
}

// ── Functions ───────────────────────────────────────────────────────────────

fn translate_function(
    func: &pt::FunctionDefinition,
    storage_vars: &HashSet<String>,
) -> Result<Vec<LispVal>, String> {
    let func_name = func.name
        .as_ref()
        .ok_or_else(|| "function has no name".to_string())?
        .name
        .clone();

    let is_public = func.attributes.iter().any(|a| {
        matches!(
            a,
            pt::FunctionAttribute::Visibility(pt::Visibility::Public(_))
        ) || matches!(
            a,
            pt::FunctionAttribute::Visibility(pt::Visibility::External(_))
        )
    });

    let mut output = Vec::new();

    // Build param list
    let mut params = vec![LispVal::Sym(func_name.clone())];
    for (_loc, param) in &func.params {
        if let Some(p) = param {
            if let Some(name) = &p.name {
                params.push(LispVal::Sym(name.name.clone()));
            }
        }
    }

    // Translate body
    let body = if let Some(stmt) = &func.body {
        translate_statement(stmt, storage_vars)?
    } else {
        LispVal::List(vec![LispVal::Sym("nil".into())])
    };

    output.push(lisp_list(vec![
        LispVal::Sym("define".into()),
        LispVal::List(params),
        body,
    ]));

    // Export public functions
    if is_public {
        output.push(lisp_list(vec![
            LispVal::Sym("export".into()),
            LispVal::Sym(func_name),
        ]));
    }

    Ok(output)
}

// ── Statements ──────────────────────────────────────────────────────────────

fn translate_statement(
    stmt: &pt::Statement,
    storage_vars: &HashSet<String>,
) -> Result<LispVal, String> {
    match stmt {
        pt::Statement::Block { statements, .. } => {
            if statements.is_empty() {
                return Ok(LispVal::List(vec![LispVal::Sym("nil".into())]));
            }
            if statements.len() == 1 {
                return translate_statement(&statements[0], storage_vars);
            }
            let mut parts = vec![LispVal::Sym("begin".into())];
            for s in statements {
                parts.push(translate_statement(s, storage_vars)?);
            }
            Ok(lisp_list(parts))
        }

        pt::Statement::Expression(_, expr) => translate_expr(expr, storage_vars),

        pt::Statement::VariableDefinition(_, var, init) => {
            let name = var
                .name
                .as_ref()
                .ok_or_else(|| "local var has no name".to_string())?
                .name
                .clone();
            let val = match init {
                Some(e) => translate_expr(e, storage_vars)?,
                None => LispVal::List(vec![LispVal::Sym("nil".into())]),
            };
            // Imperative: just use define for local scope
            Ok(lisp_list(vec![
                LispVal::Sym("define".into()),
                LispVal::List(vec![LispVal::Sym(name)]),
                val,
            ]))
        }

        pt::Statement::If(_, cond, then_br, else_br) => {
            let mut parts = vec![
                LispVal::Sym("if".into()),
                translate_expr(cond, storage_vars)?,
                translate_statement(then_br, storage_vars)?,
            ];
            if let Some(else_s) = else_br {
                parts.push(translate_statement(else_s, storage_vars)?);
            }
            Ok(lisp_list(parts))
        }

        pt::Statement::For(_, init, cond, update, body) => {
            let mut parts = vec![LispVal::Sym("begin".into())];
            if let Some(i) = init {
                parts.push(translate_statement(i, storage_vars)?);
            }

            let mut loop_body = vec![LispVal::Sym("begin".into())];
            if let Some(b) = body {
                loop_body.push(translate_statement(b, storage_vars)?);
            }
            if let Some(u) = update {
                loop_body.push(translate_expr(u, storage_vars)?);
            }

            let cond_expr = match cond {
                Some(c) => translate_expr(c, storage_vars)?,
                None => LispVal::Bool(true),
            };

            parts.push(lisp_list(vec![
                LispVal::Sym("while".into()),
                cond_expr,
                lisp_list(loop_body),
            ]));

            Ok(lisp_list(parts))
        }

        pt::Statement::While(_, cond, body) => Ok(lisp_list(vec![
            LispVal::Sym("while".into()),
            translate_expr(cond, storage_vars)?,
            translate_statement(body, storage_vars)?,
        ])),

        pt::Statement::DoWhile(_, body, cond) => {
            Ok(lisp_list(vec![
                LispVal::Sym("begin".into()),
                translate_statement(body, storage_vars)?,
                lisp_list(vec![
                    LispVal::Sym("while".into()),
                    translate_expr(cond, storage_vars)?,
                    translate_statement(body, storage_vars)?,
                ]),
            ]))
        }

        pt::Statement::Return(_, expr) => match expr {
            Some(e) => translate_expr(e, storage_vars),
            None => Ok(LispVal::List(vec![LispVal::Sym("nil".into())])),
        },

        pt::Statement::Emit(_, event_expr) => {
            Ok(lisp_list(vec![
                LispVal::Sym("near/log".into()),
                translate_expr(event_expr, storage_vars)?,
            ]))
        }

        pt::Statement::Revert(_, _path, args) => {
            let msg = if args.is_empty() {
                "revert".to_string()
            } else {
                args.iter()
                    .map(|a| format!("{:?}", a))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            Ok(lisp_list(vec![
                LispVal::Sym("near/panic".into()),
                LispVal::Str(format!("revert: {}", msg)),
            ]))
        }

        pt::Statement::Continue(_) => Ok(LispVal::Sym("continue".into())),
        pt::Statement::Break(_) => Ok(LispVal::Sym("break".into())),
        pt::Statement::Error(_) => Err("parse error in statement".into()),

        _ => Err(format!("unsupported statement type: {:?}", stmt)),
    }
}

// ── Expressions ─────────────────────────────────────────────────────────────

fn translate_expr(
    expr: &pt::Expression,
    storage_vars: &HashSet<String>,
) -> Result<LispVal, String> {
    match expr {
        // Literals
        pt::Expression::BoolLiteral(_, b) => Ok(LispVal::Bool(*b)),

        pt::Expression::NumberLiteral(_, n, unit, _) => {
            let num_str = n.replace('_', "");
            let val: i64 = num_str
                .parse()
                .map_err(|_| format!("invalid number: {}", n))?;
            let _ = unit;
            Ok(LispVal::Num(val))
        }

        pt::Expression::HexNumberLiteral(_, h, _) => {
            let val = i64::from_str_radix(h.trim_start_matches("0x"), 16)
                .map_err(|_| format!("invalid hex: {}", h))?;
            Ok(LispVal::Num(val))
        }

        pt::Expression::StringLiteral(strs) => {
            let s: String = strs.iter().map(|sl| sl.string.clone()).collect();
            Ok(LispVal::Str(s))
        }

        // Variable reference — redirect storage vars to getters
        pt::Expression::Variable(ident) => {
            let name = ident.name.clone();
            if storage_vars.contains(&name) {
                // Storage var → call getter
                Ok(lisp_list(vec![LispVal::Sym(format!("get_{}", name))]))
            } else {
                // Magic Solidity identifiers
                Ok(match name.as_str() {
                    "this" => lisp_list(vec![LispVal::Sym("near/account_id".into())]),
                    _ => LispVal::Sym(name),
                })
            }
        }

        // Binary ops
        pt::Expression::Add(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("+".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Subtract(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("-".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Multiply(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("*".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Divide(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("/".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Modulo(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("mod".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Power(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("pow".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),

        // Comparison
        pt::Expression::Equal(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("=".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::NotEqual(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("!=".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Less(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("<".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::More(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym(">".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::LessEqual(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("<=".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::MoreEqual(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym(">=".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),

        // Logical
        pt::Expression::And(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("and".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Or(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("or".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::Not(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("not".into()),
            translate_expr(e, storage_vars)?,
        ])),

        // Bitwise
        pt::Expression::BitwiseAnd(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("band".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::BitwiseOr(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("bor".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::BitwiseXor(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("bxor".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::BitwiseNot(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("bnot".into()),
            translate_expr(e, storage_vars)?,
        ])),
        pt::Expression::ShiftLeft(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("shl".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),
        pt::Expression::ShiftRight(_, l, r) => Ok(lisp_list(vec![
            LispVal::Sym("shr".into()),
            translate_expr(l, storage_vars)?,
            translate_expr(r, storage_vars)?,
        ])),

        // Unary
        pt::Expression::Negate(_, e) => Ok(lisp_list(vec![
            LispVal::Sym("-".into()),
            translate_expr(e, storage_vars)?,
        ])),
        pt::Expression::PreIncrement(_, e) => {
            // x++ → set_x (+ (get_x) 1)
            translate_increment(e, storage_vars, 1)
        }
        pt::Expression::PreDecrement(_, e) => {
            translate_increment(e, storage_vars, -1)
        }

        // Ternary: cond ? a : b → (if cond a b)
        pt::Expression::ConditionalOperator(_, cond, then_v, else_v) => {
            Ok(lisp_list(vec![
                LispVal::Sym("if".into()),
                translate_expr(cond, storage_vars)?,
                translate_expr(then_v, storage_vars)?,
                translate_expr(else_v, storage_vars)?,
            ]))
        }

        // Assignment: x = val
        // For storage vars: (set_x val)
        // For local vars: (set! x val)
        pt::Expression::Assign(_, target, val) => {
            let rhs = translate_expr(val, storage_vars)?;
            translate_assignment(target, rhs, storage_vars)
        }

        // Compound assignment: x += val → (set_x (+ (get_x) val))
        pt::Expression::AssignAdd(_, target, val) => {
            let rhs = lisp_list(vec![
                LispVal::Sym("+".into()),
                translate_storage_read_or_var(target, storage_vars)?,
                translate_expr(val, storage_vars)?,
            ]);
            translate_assignment(target, rhs, storage_vars)
        }
        pt::Expression::AssignSubtract(_, target, val) => {
            let rhs = lisp_list(vec![
                LispVal::Sym("-".into()),
                translate_storage_read_or_var(target, storage_vars)?,
                translate_expr(val, storage_vars)?,
            ]);
            translate_assignment(target, rhs, storage_vars)
        }
        pt::Expression::AssignMultiply(_, target, val) => {
            let rhs = lisp_list(vec![
                LispVal::Sym("*".into()),
                translate_storage_read_or_var(target, storage_vars)?,
                translate_expr(val, storage_vars)?,
            ]);
            translate_assignment(target, rhs, storage_vars)
        }
        pt::Expression::AssignDivide(_, target, val) => {
            let rhs = lisp_list(vec![
                LispVal::Sym("/".into()),
                translate_storage_read_or_var(target, storage_vars)?,
                translate_expr(val, storage_vars)?,
            ]);
            translate_assignment(target, rhs, storage_vars)
        }

        // Array literal: [a, b, c] → (list a b c)
        pt::Expression::ArrayLiteral(_, elems) => {
            let mut parts = vec![LispVal::Sym("list".into())];
            for e in elems {
                parts.push(translate_expr(e, storage_vars)?);
            }
            Ok(lisp_list(parts))
        }

        // Array subscript: arr[i] → (nth arr i)
        pt::Expression::ArraySubscript(_, arr, idx) => {
            let index = idx
                .as_ref()
                .ok_or_else(|| "array subscript missing index".to_string())?;
            Ok(lisp_list(vec![
                LispVal::Sym("nth".into()),
                translate_expr(arr, storage_vars)?,
                translate_expr(index, storage_vars)?,
            ]))
        }

        // Member access: obj.field
        pt::Expression::MemberAccess(_, obj, field) => {
            let field_name = field.name.clone();
            if let pt::Expression::Variable(ident) = obj.as_ref() {
                match ident.name.as_str() {
                    "msg" => match field_name.as_str() {
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
                        "data" => {
                            return Ok(lisp_list(vec![LispVal::Sym("near/input".into())]))
                        }
                        "sig" => return Err("msg.sig not supported on NEAR".into()),
                        _ => {}
                    },
                    "block" => match field_name.as_str() {
                        "timestamp" => {
                            return Ok(lisp_list(vec![LispVal::Sym(
                                "near/block_timestamp".into(),
                            )]))
                        }
                        "number" => {
                            return Ok(lisp_list(vec![LispVal::Sym(
                                "near/block_height".into(),
                            )]))
                        }
                        _ => {}
                    },
                    "tx" => match field_name.as_str() {
                        "origin" => {
                            return Ok(lisp_list(vec![LispVal::Sym(
                                "near/predecessor".into(),
                            )]))
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            Ok(lisp_list(vec![
                LispVal::Sym(".".into()),
                translate_expr(obj, storage_vars)?,
                LispVal::Str(field_name),
            ]))
        }

        // Function call
        pt::Expression::FunctionCall(_, callee, args) => {
            // Special builtins
            if let pt::Expression::Variable(ident) = callee.as_ref() {
                match ident.name.as_str() {
                    "require" => {
                        let cond = args
                            .first()
                            .ok_or_else(|| "require needs condition".to_string())?;
                        let msg = args
                            .get(1)
                            .map(|e| translate_expr(e, storage_vars))
                            .transpose()?
                            .unwrap_or(LispVal::Str("require failed".into()));
                        return Ok(lisp_list(vec![
                            LispVal::Sym("assert".into()),
                            translate_expr(cond, storage_vars)?,
                            msg,
                        ]));
                    }
                    "assert" => {
                        let cond = args
                            .first()
                            .ok_or_else(|| "assert needs condition".to_string())?;
                        return Ok(lisp_list(vec![
                            LispVal::Sym("assert".into()),
                            translate_expr(cond, storage_vars)?,
                            LispVal::Str("assertion failed".into()),
                        ]));
                    }
                    "revert" => {
                        let msg = args
                            .first()
                            .map(|e| translate_expr(e, storage_vars))
                            .transpose()?
                            .unwrap_or(LispVal::Str("revert".into()));
                        return Ok(lisp_list(vec![LispVal::Sym("near/panic".into()), msg]));
                    }
                    "keccak256" | "sha256" => {
                        let mut parts = vec![LispVal::Sym("near/sha256".into())];
                        for a in args {
                            parts.push(translate_expr(a, storage_vars)?);
                        }
                        return Ok(lisp_list(parts));
                    }
                    "ecrecover" => {
                        return Err(
                            "ecrecover not supported — use near/ed25519_verify or near/p256_verify"
                                .into(),
                        );
                    }
                    "abi" => {
                        return Err("abi.* not yet supported".into());
                    }
                    "type" => {
                        return Err("type() not yet supported".into());
                    }
                    _ => {}
                }
            }

            // General function call: f(a, b) → (f a b)
            let mut parts = vec![translate_expr(callee, storage_vars)?];
            for a in args {
                parts.push(translate_expr(a, storage_vars)?);
            }
            Ok(lisp_list(parts))
        }

        pt::Expression::Parenthesis(_, inner) => translate_expr(inner, storage_vars),
        pt::Expression::Type(_, ty) => Ok(LispVal::Sym(format!("{:?}", ty).to_lowercase())),
        pt::Expression::AddressLiteral(_, addr) => Ok(LispVal::Str(addr.clone())),

        pt::Expression::Delete(_, target) => Ok(lisp_list(vec![
            LispVal::Sym("set!".into()),
            translate_expr(target, storage_vars)?,
            LispVal::List(vec![LispVal::Sym("nil".into())]),
        ])),

        _ => Err(format!("unsupported expression: {:?}", expr)),
    }
}

// ── Assignment Helpers ──────────────────────────────────────────────────────

/// Translate an assignment target.
/// For storage vars: (set_NAME rhs)
/// For local vars: (set! target rhs)
fn translate_assignment(
    target: &pt::Expression,
    rhs: LispVal,
    storage_vars: &HashSet<String>,
) -> Result<LispVal, String> {
    if let pt::Expression::Variable(ident) = target {
        let name = &ident.name;
        if storage_vars.contains(name) {
            return Ok(lisp_list(vec![
                LispVal::Sym(format!("set_{}", name)),
                rhs,
            ]));
        } else {
            return Ok(lisp_list(vec![
                LispVal::Sym("set!".into()),
                LispVal::Sym(name.clone()),
                rhs,
            ]));
        }
    }
    // For member access or subscript targets, use set!
    Ok(lisp_list(vec![
        LispVal::Sym("set!".into()),
        translate_expr(target, storage_vars)?,
        rhs,
    ]))
}

/// Read a storage var or just return the expression as-is.
fn translate_storage_read_or_var(
    expr: &pt::Expression,
    storage_vars: &HashSet<String>,
) -> Result<LispVal, String> {
    if let pt::Expression::Variable(ident) = expr {
        if storage_vars.contains(&ident.name) {
            return Ok(lisp_list(vec![LispVal::Sym(format!("get_{}", ident.name))]));
        }
    }
    translate_expr(expr, storage_vars)
}

/// Translate increment/decrement of a storage var or local var.
fn translate_increment(
    e: &pt::Expression,
    storage_vars: &HashSet<String>,
    delta: i64,
) -> Result<LispVal, String> {
    let op = if delta > 0 { "+" } else { "-" };
    let rhs = lisp_list(vec![
        LispVal::Sym(op.into()),
        translate_storage_read_or_var(e, storage_vars)?,
        LispVal::Num(delta.abs()),
    ]);
    translate_assignment(e, rhs, storage_vars)
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn lisp_list(items: Vec<LispVal>) -> LispVal {
    LispVal::List(items)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_contract() {
        let src = r#"
            pragma solidity ^0.8.0;

            contract Counter {
                uint256 public count;

                function increment() public {
                    count = count + 1;
                }

                function getCount() public view returns (uint256) {
                    return count;
                }
            }
        "#;

        let result = translate_solidity(src);
        assert!(result.is_ok(), "parse failed: {:?}", result.err());
        let vals = result.unwrap();
        assert!(!vals.is_empty());

        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        println!("Translated:\n{}", lisp_str);

        // Storage vars should use getter/setter
        assert!(lisp_str.contains("(set_count (+ (get_count) 1))"),
            "increment should use getter/setter, got: {}", lisp_str);
        assert!(lisp_str.contains("(get_count)"),
            "getCount should call getter, got: {}", lisp_str);
    }

    #[test]
    fn test_storage_getter_setter() {
        let src = r#"
            contract Simple {
                uint256 public balance;
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("get_balance"));
        assert!(lisp_str.contains("set_balance"));
    }

    #[test]
    fn test_mapping() {
        let src = r#"
            contract Token {
                mapping(address => uint256) public balances;
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("get_balances"));
        assert!(lisp_str.contains("set_balances"));
    }

    #[test]
    fn test_msg_sender() {
        let src = r#"
            contract Auth {
                function owner() public view returns (address) {
                    return msg.sender;
                }
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("signer_account_id"));
    }

    #[test]
    fn test_require() {
        let src = r#"
            contract Guard {
                function deposit() public {
                    require(msg.value > 0, "must send NEAR");
                }
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("assert"));
    }

    #[test]
    fn test_arithmetic() {
        let src = r#"
            contract Math {
                function add(uint256 a, uint256 b) public pure returns (uint256) {
                    return a + b;
                }
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("(+ a b)"));
    }

    #[test]
    fn test_memory_directive() {
        let src = r#"
            contract Foo {
                uint256 public x;
            }
        "#;

        let vals = translate_solidity(src).unwrap();
        let lisp_str = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(lisp_str.contains("(memory 4)"), "should emit (memory 4)");
    }
}
