//! TS frontend (M1): TypeScript-syntax surface → lisp s-expression source.
//!
//! Lowering pipeline: TS source --oxc_parser--> TS AST --this module--> lisp
//! source text --existing parser/checker/emitters--> all backends (near wasm,
//! bytecode, wasi) unchanged.
//!
//! M1 subset (deliberately small, differential-provable):
//!   ✓ function declarations (exported or not) → define (+ export form)
//!   ✓ const/let locals (single declarator, initializer required)
//!   ✓ if / else (tail position: full expression; non-tail: side-effect begin)
//!   ✓ return (tail position only)
//!   ✓ numeric/string/boolean/null literals, template literals → (str ...)
//!   ✓ binary ops: + - * / % < > <= >= == === != !== (numbers only)
//!   ✓ && || (short-circuit, boolean-valued 0/1 — NOT JS value semantics)
//!   ✓ ! - unary
//!   ✓ calls: bare identifiers + member calls via builtin mapping
//!   ✗ classes, async, closures/arrow fns (T4 landmine), destructuring,
//!     optional chaining, assignment/mutation, early returns, imports
//!
//! Truthiness: JS `if (x)` → `(if (!= x 0) ...)` — numeric truthiness by
//! decree (the lisp's 0-truthy landsmine sidestepped explicitly). String
//! truthiness is M2.
//!
//! Every template-literal interpolation is auto-wrapped (to-string e) —
//! the (str) int-arg renders-empty quirk cannot bite TS authors.

use crate::types::LispVal;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, Declaration, Expression, FormalParameter, Function as TsFunction, Program, Statement,
    TSType,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_syntax::operator::{BinaryOperator, LogicalOperator, UnaryOperator};

// ── Public entry ──────────────────────────────────────────────────────────

/// Parse TypeScript source and lower it to lisp source text.
pub fn ts_to_lisp_source(src: &str) -> Result<String, String> {
    let exprs = parse_ts(src)?;
    let mut out = String::new();
    for e in &exprs {
        out.push_str(&sexp(e));
        out.push('\n');
    }
    Ok(out)
}

/// Parse TypeScript source and lower it to top-level lisp forms.
pub fn parse_ts(src: &str) -> Result<Vec<LispVal>, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::default().with_typescript(true).with_module(true);
    let ret = Parser::new(&allocator, src, source_type).parse();
    if ret.panicked || !ret.diagnostics.is_empty() {
        let msg = ret
            .diagnostics
            .first()
            .map(|d| d.message.to_string())
            .unwrap_or_else(|| "unknown parse error".into());
        return Err(format!("TS parse error: {}", msg));
    }
    lower_program(&ret.program)
}

// ── Program / statements ──────────────────────────────────────────────────

fn lower_program(p: &Program<'_>) -> Result<Vec<LispVal>, String> {
    // TypeScript hoists function declarations: a call may textually precede
    // the helper's definition. Lisp requires define-before-use, so we reorder:
    //   1. top-level consts (module-load-time, source order)
    //   2. non-exported functions (hoisted, source order)
    //   3. everything else (exported defines, exports, top-level exprs) in order
    let mut consts: Vec<LispVal> = Vec::new();
    let mut hoisted: Vec<LispVal> = Vec::new();
    let mut out: Vec<LispVal> = Vec::new();
    for stmt in &p.body {
        match stmt {
            Statement::ExportDeclaration(decl) => {
                let f = match &decl.declaration {
                    Declaration::FunctionDeclaration(f) => f,
                    d => {
                        return Err(format!(
                            "ts_frontend: only `export function` is supported, got {}",
                            decl_kind(d)
                        ))
                    }
                };
                let (name, define) = lower_function(f, true)?;
                let view = name.starts_with("get_");
                out.push(define);
                // `new` is a reserved word in TypeScript — `new_` is the
                // dialect's spelling for NEAR's `new` constructor export.
                let export_name = if name == "new_" { "new".to_string() } else { name.clone() };
                out.push(list(vec![
                    Sym("export"),
                    Str(export_name),
                    Sym(name),
                    if view { Sym("#t") } else { Sym("#f") },
                ]));
            }
            Statement::FunctionDeclaration(f) => {
                hoisted.push(lower_function(f, false)?.1);
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let init = d
                        .init
                        .as_ref()
                        .ok_or("ts_frontend: top-level declarations need initializers")?;
                    consts.push(list(vec![Sym("define"), Sym(name), lower_expr(init)?]));
                }
            }
            Statement::ExpressionStatement(e) => {
                out.push(lower_expr(&e.expression)?);
            }
            Statement::EmptyStatement(_) => {}
            s => {
                return Err(format!(
                    "ts_frontend: statement `{}` not in M1 subset",
                    stmt_kind(s)
                ))
            }
        }
    }
    let mut result = consts;
    result.extend(hoisted);
    result.extend(out);
    Ok(result)
}

/// Lower a function declaration → (define (name params...) body)
fn lower_function(f: &TsFunction<'_>, exported: bool) -> Result<(String, LispVal), String> {
    let name = f
        .id
        .as_ref()
        .map(|i| i.name.as_str().to_string())
        .ok_or("ts_frontend: anonymous functions unsupported (M1)")?;

    let mut params = Vec::new();
    let mut param_names: Vec<(String, bool)> = Vec::new(); // (name, is_number)
    for p in &f.params.items {
        let n = binding_name(&p.pattern)?;
        param_names.push((n.clone(), param_is_number(p)));
    }

    let body = f
        .body
        .as_ref()
        .ok_or("ts_frontend: function overloads/declarations unsupported")?;

    // view convention: get_* functions' returns become json_return_str
    // (the define tail value alone does not call value_return)
    let view = name.starts_with("get_");

    // Exported contracts read args from the transaction input JSON
    // (json_get_str pattern); `: number` annotations wrap str->num.
    let expr = if exported {
        if !param_names.is_empty() {
            let bindings = param_names
                .iter()
                .map(|(n, num)| {
                    let get = list(vec![Sym("near/json_get_str"), Str(n.clone())]);
                    let v = if *num {
                        list(vec![Sym("str->num"), get])
                    } else {
                        get
                    };
                    list(vec![Sym(n.clone()), v])
                })
                .collect();
            let inner = lower_block_tail(&body.statements, view)?;
            list(vec![Sym("let"), list(bindings), inner])
        } else {
            lower_block_tail(&body.statements, view)?
        }
    } else {
        // helper fns keep real lisp params
        for (n, _) in &param_names {
            params.push(Sym(n.clone()));
        }
        lower_block_tail(&body.statements, false)?
    };

    let mut define_items = Vec::new();
    let mut sig = vec![Sym(name.clone())];
    let lisp_param_count = params.len();
    sig.extend(params);
    define_items.push(Sym("define"));
    define_items.push(list(sig));

    // Emit a `::` annotation when every TS param maps 1:1 onto the lowered
    // lisp params (helpers) or the function takes no params (exported fns
    // read args from JSON, so their lisp arity is 0), and the return is
    // annotated with a supported type. `void` returns skip the annotation.
    let param_anns: Vec<Option<&str>> = f
        .params
        .items
        .iter()
        .map(|p| ts_ann_to_lisp(p.type_annotation.as_ref().map(|v| &**v)))
        .collect();
    let ret_ann = ts_ann_to_lisp(f.return_type.as_ref().map(|v| &**v));
    let complete = param_anns.len() == lisp_param_count // lisp params == TS params
        && param_anns.iter().all(|a| a.is_some())
        && ret_ann.is_some();
    if complete {
        define_items.push(Sym("::".to_string()));
        for a in param_anns.iter().map(|a| a.unwrap()) {
            define_items.push(Sym(a.to_string()));
        }
        define_items.push(Sym("->".to_string()));
        define_items.push(Sym(ret_ann.unwrap().to_string()));
    }

    define_items.push(expr);
    Ok((name, list(define_items)))
}

/// Lower a statement list whose value is the tail expression.
fn lower_block_tail(stmts: &[Statement<'_>], view: bool) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(Num(0));
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    let tail = lower_tail_stmt(&last[0], view)?;
    lower_prefix_around(init, tail, view)
}

/// Prefix statements wrap the tail expression like let-nesting.
fn lower_prefix_around(stmts: &[Statement<'_>], tail: LispVal, view: bool) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(tail);
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    let inner = match &last[0] {
        Statement::VariableDeclaration(v) => {
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
            }
            list(vec![Sym("let"), list(bindings), tail])
        }
        Statement::ExpressionStatement(e) => {
            // side-effect expression, discard value
            let e2 = match &e.expression {
                Expression::AssignmentExpression(asg) => {
                    let (v, expr) = lower_assignment(asg)?;
                    list(vec![Sym("set!"), Sym(v), expr])
                }
                other => lower_expr(other)?,
            };
            list(vec![Sym("begin"), e2, tail])
        }
        Statement::IfStatement(i) => {
            // Branches containing `return` take over the continuation:
            //   if (c) return e; REST  →  (if c (branch-value) REST-value)
            // (otherwise an early return would fall through to REST).
            let then_returns = stmt_has_return(&i.consequent)
                || i.alternate.as_ref().is_some_and(|a| stmt_has_return(a));
            if then_returns {
                let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
                return match &i.alternate {
                    Some(alt) => {
                        // else branch runs, then the continuation
                        let else_cont = lower_prefix_around(stmts_of(alt), tail, view)?;
                        lower_prefix_around(
                            init,
                            list(vec![Sym("if"), truthy(&i.test)?, then_e, else_cont]),
                            view,
                        )
                    }
                    None => {
                        let cont = tail;
                        lower_prefix_around(
                            init,
                            list(vec![Sym("if"), truthy(&i.test)?, then_e, cont]),
                            view,
                        )
                    }
                };
            }
            // non-tail if: side-effect only; branches are void-ish blocks.
            let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                None => Num(0),
            };
            list(vec![
                Sym("begin"),
                list(vec![Sym("if"), truthy(&i.test)?, then_e, else_e]),
                tail,
            ])
        }
        Statement::ReturnStatement(_) => {
            return Err("ts_frontend: `return` only allowed as the last statement".into())
        }
        Statement::WhileStatement(_) => {
            list(vec![Sym("begin"), lower_while_value(&last[0])?, tail])
        }
        Statement::ForStatement(fr) => {
            list(vec![Sym("begin"), lower_for(fr)?, tail])
        }
        s => {
            return Err(format!(
                "ts_frontend: statement `{}` not allowed mid-function",
                stmt_kind(s)
            ))
        }
    };
    lower_prefix_around(init, inner, view)
}

/// Statements of a branch: block → its body, single stmt → slice of itself.
fn stmts_of<'a>(s: &'a Statement<'a>) -> &'a [Statement<'a>] {
    match s {
        Statement::BlockStatement(b) => &b.body,
        other => std::slice::from_ref(other),
    }
}


/// Nil-returning builtins — their call forms can't sit in value position
/// (if-branch / function tail) without an int tail.
fn is_nil_call(v: &LispVal) -> bool {
    let LispVal::List(items) = v else { return false };
    let Some(LispVal::Sym(head)) = items.first() else { return false };
    matches!(
        head.as_str(),
        "near/storage_set" | "near/storage_remove" | "near/abort" | "near/value_return"
    )
}

/// Wrap a nil-typed call so it is int-typed in value position.
fn ensure_int_value(v: LispVal) -> LispVal {
    if is_nil_call(&v) {
        list(vec![Sym("begin"), v, Num(0)])
    } else {
        v
    }
}

/// Last statement of a block — may `return` / full-expression `if`.
fn lower_tail_stmt(s: &Statement<'_>, view: bool) -> Result<LispVal, String> {
    match s {
        Statement::ReturnStatement(r) => match &r.argument {
            Some(e) => {
                let v = lower_expr(e)?;
                if view {
                    // view fns: value_return via json_return_str
                    Ok(list(vec![Sym("near/json_return_str"), v]))
                } else {
                    Ok(v)
                }
            }
            None => Ok(Num(0)),
        },
        Statement::IfStatement(i) => {
            let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                None => Num(0),
            };
            Ok(list(vec![Sym("if"), truthy(&i.test)?, then_e, else_e]))
        }
        Statement::BlockStatement(b) => lower_block_tail(&b.body, view),
        Statement::ExpressionStatement(e) => Ok(ensure_int_value(lower_expr(&e.expression)?)),
        Statement::VariableDeclaration(v) => {
            // trailing let: bind, value 0
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
            }
            Ok(list(vec![Sym("let"), list(bindings), Num(0)]))
        }
        Statement::EmptyStatement(_) => Ok(Num(0)),
        Statement::WhileStatement(_) => lower_while_value(s),
        Statement::ForStatement(fr) => lower_for(fr),
        s2 => Err(format!(
            "ts_frontend: statement `{}` not in tail subset",
            stmt_kind(s2)
        )),
    }
}


/// Does this statement contain a `return` (anywhere, incl. nested ifs)?
/// Does not descend into loops — a return inside a loop body belongs to the
/// loop-exit rewrite, not to this function's tail.
fn stmt_has_return(s: &Statement<'_>) -> bool {
    match s {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(b) => b.body.iter().any(stmt_has_return),
        Statement::IfStatement(i) => {
            stmt_has_return(&i.consequent)
                || i.alternate.as_ref().is_some_and(|a| stmt_has_return(a))
        }
        _ => false,
    }
}

/// Does this statement list contain a `break` or `return` (for loop-exit
/// rewriting)? Does not descend into nested loops — their exits are their own.
fn stmts_have_exit(stmts: &[Statement<'_>]) -> bool {
    stmts.iter().any(stmt_has_exit)
}

fn stmt_has_exit(s: &Statement<'_>) -> bool {
    match s {
        Statement::BreakStatement(_) | Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(b) => stmts_have_exit(&b.body),
        Statement::IfStatement(i) => {
            stmt_has_exit(&i.consequent)
                || i.alternate.as_ref().is_some_and(|a| stmt_has_exit(a))
        }
        _ => false,
    }
}

/// Lower a while statement to a value-producing form.
/// Without break/return in the body: `(while cond body)`.
/// With them: flag-guarded rewrite —
///   (let ((done 0) (res 0))
///     (begin (while (if (= done 0) cond 0)
///              body' ;; return e -> (set! res e)(set! done 1); rest guarded
///            res))
fn lower_while_value(w: &Statement<'_>) -> Result<LispVal, String> {
    let Statement::WhileStatement(w) = w else {
        return Err("ts_frontend: internal: not a while".into());
    };
    let body_stmts = stmts_of(&w.body);

    // Hoist loop-body `let/const` declarations: TS consts are per-iteration
    // but write-before-read (TDZ), so rewrite `const x = e;` in place as
    // (set! x e) with the binding (x 0) added to the wrapper let.
    let mut hoisted: Vec<(String, LispVal)> = Vec::new();
    for s in body_stmts {
        if let Statement::VariableDeclaration(v) = s {
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                hoisted.push((name, lower_expr(init_e)?));
            }
        }
    }

    if !stmts_have_exit(body_stmts) {
        let mut body_items = vec![Sym("begin")];
        for s in body_stmts {
            if let Statement::VariableDeclaration(_) = s {
                continue; // already hoisted below via hoisted list
            }
            body_items.push(tail_stmt_as_expr(s)?);
        }
        for (name, init) in &hoisted {
            body_items.insert(1, list(vec![Sym("set!"), Sym(name.clone()), init.clone()]));
        }
        let body_e = if body_items.len() == 1 { Num(0) } else { list(body_items) };
        let while_e = list(vec![Sym("while"), truthy(&w.test)?, body_e]);
        if hoisted.is_empty() {
            return Ok(while_e);
        }
        let binds: Vec<LispVal> = hoisted
            .iter()
            .map(|(n, _)| list(vec![Sym(n.clone()), Num(0)]))
            .collect();
        return Ok(list(vec![Sym("let"), list(binds), while_e]));
    }
    // break/return rewrite
    let mut body_items = vec![Sym("begin")];
    for (name, init) in &hoisted {
        body_items.push(list(vec![Sym("set!"), Sym(name.clone()), init.clone()]));
    }
    let mut seen_exit = false;
    for s in body_stmts {
        if let Statement::VariableDeclaration(_) = s {
            continue; // hoisted above
        }
        let piece = match s {
            Statement::BreakStatement(_) => list(vec![
                Sym("begin"),
                list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                Num(0),
            ]),
            Statement::ReturnStatement(r) => {
                let val = match &r.argument {
                    Some(e) => lower_expr(e)?,
                    None => Num(0),
                };
                list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__wl_res"), val]),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    Num(0), // set! types nil — keep the begin int-typed
                ])
            }
            other => {
                let e = tail_stmt_as_expr(other)?;
                if seen_exit {
                    // dead code after break/return in the same iteration — guard
                    list(vec![
                        Sym("if"),
                        list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                        e,
                        Num(0),
                    ])
                } else {
                    e
                }
            }
        };
        if matches!(s, Statement::BreakStatement(_) | Statement::ReturnStatement(_)) {
            seen_exit = true;
        }
        body_items.push(piece);
    }
    let body_e = if body_items.len() == 1 { Num(0) } else { list(body_items) };
    let cond_e = list(vec![
        Sym("if"),
        list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
        truthy(&w.test)?,
        list(vec![Sym("="), Num(1), Num(0)]), // bool false — keep branch types aligned
    ]);
    let mut binds = vec![
        list(vec![Sym("__wl_done"), Num(0)]),
        list(vec![Sym("__wl_res"), Num(0)]),
    ];
    for (n, _) in &hoisted {
        binds.push(list(vec![Sym(n.clone()), Num(0)]));
    }
    Ok(list(vec![
        Sym("let"),
        list(binds),
        list(vec![
            Sym("begin"),
            list(vec![Sym("while"), cond_e, body_e]),
            Sym("__wl_res"),
        ]),
    ]))
}

/// Body of a while/for: statements → single begin-expression (side effects).
fn loop_body_expr(stmts: &[Statement<'_>]) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(Num(0));
    }
    let mut exprs = Vec::new();
    for s in stmts {
        exprs.push(tail_stmt_as_expr(s)?);
    }
    // set! (and break/return rewrites ending in set!) type nil — if the last
    // item is one, append 0 so branch contexts stay type-consistent.
    let last_is_setbang = matches!(exprs.last(), Some(LispVal::List(items))
        if matches!(items.first(), Some(LispVal::Sym(s)) if s == "set!"))
        || exprs.last().is_some_and(is_nil_call);
    if last_is_setbang {
        exprs.push(Num(0));
    }
    if exprs.len() == 1 {
        Ok(exprs.into_iter().next().unwrap())
    } else {
        let mut items = vec![Sym("begin")];
        items.extend(exprs);
        Ok(list(items))
    }
}

/// A statement inside a loop body, as a pure expression.
/// Loop context: `break` / `return` rewrite to __wl_done/__wl_res flag writes
/// (provided by lower_while_value's exit-rewrite). Recurses through if
/// branches so mid-branch exits lower correctly.
fn tail_stmt_as_expr(s: &Statement<'_>) -> Result<LispVal, String> {
    match s {
        Statement::ExpressionStatement(e) => match &e.expression {
            Expression::AssignmentExpression(asg) => {
                let (v, expr) = lower_assignment(asg)?;
                Ok(list(vec![Sym("set!"), Sym(v), expr]))
            }
            other => Ok(ensure_int_value(lower_expr(other)?)),
        },
        Statement::ReturnStatement(r) => {
            let val = match &r.argument {
                Some(e) => lower_expr(e)?,
                None => Num(0),
            };
            Ok(list(vec![
                Sym("begin"),
                list(vec![Sym("set!"), Sym("__wl_res"), val]),
                list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                Num(0),
            ]))
        }
        Statement::BreakStatement(_) => Ok(list(vec![
            Sym("begin"),
            list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
            Num(0),
        ])),
        Statement::ContinueStatement(_) => {
            Err("ts_frontend: continue not supported (use the loop condition)".into())
        }
        Statement::IfStatement(i) => {
            let then_e = loop_body_expr(stmts_of(&i.consequent))?;
            let else_e = match &i.alternate {
                Some(alt) => loop_body_expr(stmts_of(alt))?,
                None => Num(0),
            };
            Ok(list(vec![Sym("if"), truthy(&i.test)?, then_e, else_e]))
        }
        Statement::VariableDeclaration(v) => {
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
            }
            Ok(list(vec![Sym("let"), list(bindings), Num(0)]))
        }
        Statement::WhileStatement(_) => lower_while_value(s),
        Statement::ForStatement(fr) => lower_for(fr),
        Statement::BlockStatement(b) => loop_body_expr(&b.body),
        Statement::EmptyStatement(_) => Ok(Num(0)),
        s2 => Err(format!(
            "ts_frontend: statement `{}` not allowed inside loops",
            stmt_kind(s2)
        )),
    }
}

/// Desugar `for (let i = 0; i < n; i++) { ... }` into the lisp's TCO loop:
///   (loop ((i init)...) (if (!= test 0) (begin body... (recur i'...)) 0))
/// Body assignments to loop vars (x = e / x += e / x -= e) are threaded
/// through recur. DEVIATION: assignments take effect at iteration end —
/// a later read in the SAME iteration sees the old value. Restructure with
/// fresh consts if read-after-write is needed.
fn lower_for(fr: &oxc_ast::ast::ForStatement<'_>) -> Result<LispVal, String> {
    use oxc_ast::ast::{ForStatementInit, AssignmentOperator};

    // init: must be a let/const declaration
    let decl = match &fr.init {
        Some(ForStatementInit::VariableDeclaration(v)) => v,
        _ => return Err("ts_frontend: for-loop init must be `let` declarations (e.g. `for (let i = 0; ...)`)".to_string()),
    };
    let mut loop_vars: Vec<String> = Vec::new();
    let mut bindings = Vec::new();
    for d in &decl.declarations {
        let n = binding_name(&d.id)?;
        let init_e = d
            .init
            .as_ref()
            .ok_or("ts_frontend: for-loop vars need initializers")?;
        bindings.push(list(vec![Sym(n.clone()), lower_expr(init_e)?]));
        loop_vars.push(n);
    }

    let test = fr
        .test
        .as_ref()
        .ok_or("ts_frontend: for-loop needs a condition")?;

    // update clause → (set! v expr); runs after the body each iteration
    let mut update_form: Option<LispVal> = None;
    if let Some(u) = &fr.update {
        let e = match u {
            Expression::UpdateExpression(upd) => {
                let v = update_target_simple(&upd.argument)?;
                let one = if matches!(upd.operator, oxc_syntax::operator::UpdateOperator::Increment) { 1 } else { -1 };
                list(vec![Sym("set!"), Sym(v.clone()), list(vec![Sym("+"), Sym(v), Num(one)])])
            }
            Expression::AssignmentExpression(asg) => {
                let (v, expr) = lower_assignment(asg)?;
                list(vec![Sym("set!"), Sym(v), expr])
            }
            _ => return Err("ts_frontend: for-loop update must be `i++`/`i--`/`i = e`/`i += e`".into()),
        };
        update_form = Some(e);
    }

    // body statements as effects; assignments become set! (while compiles
    // INLINE in wasm, so set! writes the actual local — exact JS semantics,
    // including read-after-write within an iteration)
    let body_stmts = stmts_of(&fr.body);
    let mut body_items = Vec::new();
    for s in body_stmts {
        body_items.push(tail_stmt_as_expr(s)?);
    }
    if let Some(u) = update_form {
        body_items.push(u);
    }
    let mut begin_items = vec![Sym("begin")];
    begin_items.extend(body_items);
    let begin_e = if begin_items.len() == 1 {
        Num(0)
    } else {
        list(begin_items)
    };

    // (let ((v init)...) (begin (while cond body) 0))
    Ok(list(vec![
        Sym("let"),
        list(bindings),
        list(vec![
            Sym("begin"),
            list(vec![Sym("while"), truthy(test)?, begin_e]),
            Num(0),
        ]),
    ]))
}

/// `x = e` / `x += e` / `x -= e` → (var, expr). Only plain identifiers.
fn lower_assignment(
    asg: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Result<(String, LispVal), String> {
    use oxc_syntax::operator::AssignmentOperator;
    let v = match &asg.left {
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) => {
            id.name.as_str().to_string()
        }
        _ => return Err("ts_frontend: assignment target must be a plain variable".into()),
    };
    let rhs = lower_expr(&asg.right)?;
    let out = match asg.operator {
        AssignmentOperator::Assign => rhs,
        AssignmentOperator::Addition => list(vec![Sym("+"), Sym(v.clone()), rhs]),
        AssignmentOperator::Subtraction => list(vec![Sym("-"), Sym(v.clone()), rhs]),
        _ => return Err("ts_frontend: only = / += / -= assignments supported".into()),
    };
    Ok((v, out))
}

fn update_target_name(e: &Expression<'_>) -> Result<String, String> {
    match e {
        Expression::Identifier(id) => Ok(id.name.as_str().to_string()),
        _ => Err("ts_frontend: loop update target must be a plain variable".into()),
    }
}

fn update_target_simple(t: &oxc_ast::ast::SimpleAssignmentTarget<'_>) -> Result<String, String> {
    match t {
        oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
            Ok(id.name.as_str().to_string())
        }
        _ => Err("ts_frontend: loop update target must be a plain variable".into()),
    }
}

// ── Expressions ───────────────────────────────────────────────────────────

fn lower_expr(e: &Expression<'_>) -> Result<LispVal, String> {
    match e {
        Expression::NumericLiteral(n) => Ok(Num(n.value as i64)),
        Expression::StringLiteral(s) => Ok(Str(s.value.as_str().to_string())),
        Expression::BooleanLiteral(b) => Ok(Num(if b.value { 1 } else { 0 })),
        Expression::NullLiteral(_) => Ok(LispVal::Nil),
        Expression::BigIntLiteral(b) => Ok(Str(b.raw.as_ref().map(|s| s.as_str().to_string()).unwrap_or_default())), // u128-style digits-as-string
        Expression::TemplateLiteral(t) => {
            let mut parts = Vec::new();
            for i in 0..t.quasis.len() {
                let cooked = t.quasis[i]
                    .value
                    .cooked
                    .as_ref()
                    .map(|s| s.as_str().to_string())
                    .unwrap_or_default();
                if !cooked.is_empty() {
                    parts.push(Str(cooked));
                }
                if i < t.expressions.len() {
                    // Auto to-string: shields TS authors from the (str)
                    // int-arg renders-empty quirk.
                    parts.push(list(vec![Sym("to-string"), lower_expr(&t.expressions[i])?]));
                }
            }
            if parts.is_empty() {
                return Ok(Str(String::new()));
            }
            let mut items = vec![Sym("str")];
            items.extend(parts);
            Ok(list(items))
        }
        Expression::Identifier(id) => Ok(Sym(id.name.as_str().to_string())),
        Expression::BinaryExpression(b) => {
            let op: &str = match b.operator {
                BinaryOperator::Addition => "+",
                BinaryOperator::Subtraction => "-",
                BinaryOperator::Multiplication => "*",
                BinaryOperator::Division => "/",
                BinaryOperator::Remainder => "%",
                BinaryOperator::LessThan => "<",
                BinaryOperator::GreaterThan => ">",
                BinaryOperator::LessEqualThan => "<=",
                BinaryOperator::GreaterEqualThan => ">=",
                BinaryOperator::Equality | BinaryOperator::StrictEquality => "=",
                BinaryOperator::Inequality | BinaryOperator::StrictInequality => "!=",
                BinaryOperator::BitwiseAnd => "band",
                BinaryOperator::BitwiseOR => "bor",
                BinaryOperator::BitwiseXOR => "bxor",
                BinaryOperator::ShiftLeft => "shl",
                BinaryOperator::ShiftRight => "shr",
                BinaryOperator::ShiftRightZeroFill => "shr",
                _ => return Err("ts_frontend: exponent/assign-ops in expressions not supported".into()),
            };
            Ok(list(vec![
                Sym(op),
                lower_expr(&b.left)?,
                lower_expr(&b.right)?,
            ]))
        }
        Expression::LogicalExpression(l) => {
            // Short-circuit, boolean-valued — NOT JS value semantics.
            // Both arms lowered to bool type so the if-form is type-consistent.
            let a = to_bool(&l.left)?;
            let b = to_bool(&l.right)?;
            Ok(match l.operator {
                LogicalOperator::And => list(vec![Sym("if"), a, b, list(vec![Sym("="), Num(1), Num(0)])]),
                LogicalOperator::Or => list(vec![Sym("if"), a, list(vec![Sym("="), Num(1), Num(1)]), b]),
                // `a ?? b` — value-level nil-handling: (default a b)
                LogicalOperator::Coalesce => {
                    list(vec![Sym("default"), lower_expr(&l.left)?, lower_expr(&l.right)?])
                }
            })
        }
        Expression::UnaryExpression(u) => match u.operator {
            UnaryOperator::LogicalNot => {
                if statically_bool(&u.argument) {
                    // bool negation: (= x 0) would be bool≠int — flip instead
                    Ok(list(vec![
                        Sym("if"),
                        lower_expr(&u.argument)?,
                        list(vec![Sym("="), Num(1), Num(0)]),
                        list(vec![Sym("="), Num(1), Num(1)]),
                    ]))
                } else {
                    Ok(list(vec![Sym("="), lower_expr(&u.argument)?, Num(0)]))
                }
            }
            UnaryOperator::UnaryNegation => Ok(list(vec![
                Sym("-"),
                Num(0),
                lower_expr(&u.argument)?,
            ])),
            UnaryOperator::UnaryPlus => lower_expr(&u.argument),
            _ => Err("ts_frontend: unary operator not in M1".into()),
        },
        Expression::CallExpression(c) => {
            let head = callee_name(&c.callee)?;
            let head = map_builtin_call(&head);
            let mut items = vec![Sym(head.clone())];
            for a in &c.arguments {
                if let Argument::SpreadElement(_) = a {
                    return Err("ts_frontend: spread not in M1".into());
                }
                let e2 = a
                    .as_expression()
                    .ok_or("ts_frontend: unsupported call argument (M1)")?;
                items.push(lower_expr(e2)?);
            }
            // json-get is dynamically str-or-num; the checker types it Int,
            // which breaks string comparisons. to-string is tag-aware
            // (identity on str, decimal on num) — safe cast for the dialect.
            if head == "json-get" {
                return Ok(list(vec![Sym("to-string"), list(items)]));
            }
            Ok(list(items))
        }
        Expression::ConditionalExpression(c) => Ok(list(vec![
            Sym("if"),
            truthy(&c.test)?,
            lower_expr(&c.consequent)?,
            lower_expr(&c.alternate)?,
        ])),
        Expression::ParenthesizedExpression(p) => lower_expr(&p.expression),
        _ => Err(format!(
            "ts_frontend: expression `{}` not in M1 subset",
            expr_kind(e)
        )),
    }
}

/// Bool-typed lowering of an expression (shared by truthy/&&/||/!).
/// Statically-boolean exprs pass through; numerics get (!= x 0).
fn statically_bool(e: &Expression<'_>) -> bool {
    let bool_call = match e {
        Expression::CallExpression(c) => callee_name(&c.callee)
            .ok()
            .map(|h| {
                matches!(
                    h.as_str(),
                    "u128/gt" | "u128/lt" | "u128/gte" | "u128/lte" | "u128/eq" | "near/deposit-gte"
                )
            })
            .unwrap_or(false),
        _ => false,
    };
    let is_not = matches!(
        e,
        Expression::UnaryExpression(u) if matches!(u.operator, UnaryOperator::LogicalNot)
    );
    matches!(e, Expression::LogicalExpression(_))
        || is_not
        || bool_call
        || matches!(
            e,
            Expression::BinaryExpression(b) if matches!(
                b.operator,
                BinaryOperator::Equality
                    | BinaryOperator::Inequality
                    | BinaryOperator::StrictEquality
                    | BinaryOperator::StrictInequality
                    | BinaryOperator::LessThan
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::LessEqualThan
                    | BinaryOperator::GreaterEqualThan
            )
        )
}

fn to_bool(e: &Expression<'_>) -> Result<LispVal, String> {
    let is_not = matches!(
        e,
        Expression::UnaryExpression(u) if matches!(u.operator, UnaryOperator::LogicalNot)
    );
    let _ = is_not;
    let already_bool = statically_bool(e);
    if already_bool {
        lower_expr(e)
    } else {
        Ok(list(vec![Sym("!="), lower_expr(e)?, Num(0)]))
    }
}

/// Numeric truthiness by decree: `if (x)` → `(if (!= x 0) ...)`.
/// Statically-boolean exprs (comparisons, && || !) pass through unwrapped —
/// the checker types them bool and rejects (!= bool 0).
fn truthy(e: &Expression<'_>) -> Result<LispVal, String> {
    to_bool(e)
}

/// Resolve a callee to a lisp symbol:
///   near.storageSet(...) → near/storage_set
///   strToNum(...)        → str->num
///   foo(...)             → foo
fn callee_name(e: &Expression<'_>) -> Result<String, String> {
    match e {
        Expression::Identifier(id) => Ok(map_global_fn(id.name.as_str())),
        Expression::StaticMemberExpression(s) => {
            let obj = match &s.object {
                Expression::Identifier(id) => id.name.as_str().to_string(),
                _ => return Err("ts_frontend: nested member chains not in M1".into()),
            };
            Ok(map_member_fn(&obj, s.property.name.as_str()))
        }
        _ => Err("ts_frontend: callee must be an identifier or member (M1)".into()),
    }
}

/// camelCase → snake_case
fn snake(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Bare global functions with special lisp names.
fn map_global_fn(name: &str) -> String {
    match name {
        "strToNum" => "str->num".into(),
        "toStr" | "toString" => "to-string".into(),
        "strLen" => "str-length".into(),
        other => other.to_string(),
    }
}

/// Object.method(...) → object/method_snake (near.* passthrough + snake).
fn map_member_fn(obj: &str, prop: &str) -> String {
    if obj == "near" && prop == "depositGte" {
        // lisp lib predates the snake convention here
        return "near/deposit-gte".into();
    }
    format!("{}/{}", obj, snake(prop))
}

/// camelCase free function → lisp builtin. Unknown names pass through
/// (user-defined TS helpers keep their own names).
fn map_builtin_call(name: &str) -> String {
    match name {
        "strLength" => "str-length",
        "strSlice" => "str-slice",
        "strCat" => "str-cat",
        "strIndexOf" => "str-index-of",
        "strToNum" => "str->num",
        "toStr" | "toString" => "to-string",
        "jsonGet" => "json-get",
        "hexDecode" => "hex-decode",
        "sha256Hash" => "sha256-hash",
        "schnorrVerify" => "schnorr-verify",
        _ => return name.to_string(),
    }
    .to_string()
}

fn param_is_number(p: &FormalParameter<'_>) -> bool {
    match &p.type_annotation {
        Some(a) => matches!(&a.type_annotation, TSType::TSNumberKeyword(_)),
        None => false,
    }
}

/// Map a TS type annotation to the lisp IR's annotation vocabulary.
/// Returns None for `void` / missing / unsupported annotations.
fn ts_ann_to_lisp(t: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>) -> Option<&'static str> {
    let a = t?;
    match &a.type_annotation {
        TSType::TSNumberKeyword(_) => Some("int"),
        TSType::TSStringKeyword(_) => Some("str"),
        // TS booleans lower as 0/1 ints (JS numeric semantics — see
        // BooleanLiteral → Num(1|0) in lower_expr), so the annotation
        // must agree: `:: ... int`. `:: bool` would make every annotated
        // boolean function a type error (found on the first annotated
        // `: boolean` return, Counter TS demo 2026-08-30).
        TSType::TSBooleanKeyword(_) => Some("int"),
        _ => None,
    }
}

fn binding_name(p: &oxc_ast::ast::BindingPattern<'_>) -> Result<String, String> {
    use oxc_ast::ast::BindingPattern::*;
    match p {
        BindingIdentifier(b) => Ok(b.name.as_str().to_string()),
        _ => Err("ts_frontend: destructuring patterns not in M1".into()),
    }
}

// ── LispVal helpers + s-expression printer ───────────────────────────────

fn list(items: Vec<LispVal>) -> LispVal {
    LispVal::List(items)
}
fn Sym(s: impl Into<String>) -> LispVal {
    LispVal::Sym(s.into())
}
fn Num(n: i64) -> LispVal {
    LispVal::Num(n)
}
fn Str(s: impl Into<String>) -> LispVal {
    LispVal::Str(s.into())
}

fn sexp(v: &LispVal) -> String {
    match v {
        LispVal::Nil => "nil".into(),
        LispVal::Bool(b) => if *b { "1" } else { "0" }.into(),
        LispVal::Num(n) => n.to_string(),
        LispVal::U64(n) => n.to_string(),
        LispVal::Float(f) => format!("{}", f),
        LispVal::Str(s) => format!("\"{}\"", escape_str(s)),
        LispVal::Sym(s) => s.clone(),
        LispVal::List(items) => {
            let inner: Vec<String> = items.iter().map(sexp).collect();
            format!("({})", inner.join(" "))
        }
        _ => format!("{:?}", v), // fallback: debug (shouldn't hit in M1)
    }
}

fn escape_str(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out
}

// ── kind names for error messages ────────────────────────────────────────

fn stmt_kind(s: &Statement<'_>) -> &'static str {
    match s {
        Statement::BlockStatement(_) => "block",
        Statement::BreakStatement(_) => "break",
        Statement::ClassDeclaration(_) => "class",
        Statement::ContinueStatement(_) => "continue",
        Statement::DebuggerStatement(_) => "debugger",
        Statement::DoWhileStatement(_) => "do-while",
        Statement::EmptyStatement(_) => "empty",
        Statement::ExpressionStatement(_) => "expression",
        Statement::ForInStatement(_) => "for-in",
        Statement::ForOfStatement(_) => "for-of",
        Statement::ForStatement(_) => "for",
        Statement::FunctionDeclaration(_) => "function",
        Statement::IfStatement(_) => "if",
        Statement::LabeledStatement(_) => "label",
        Statement::ReturnStatement(_) => "return",
        Statement::SwitchStatement(_) => "switch",
        Statement::ThrowStatement(_) => "throw",
        Statement::TryStatement(_) => "try",
        Statement::VariableDeclaration(_) => "variable",
        Statement::WhileStatement(_) => "while",
        Statement::WithStatement(_) => "with",
        _ => "other",
    }
}

fn decl_kind(d: &Declaration<'_>) -> &'static str {
    match d {
        Declaration::VariableDeclaration(_) => "variable",
        Declaration::ClassDeclaration(_) => "class",
        Declaration::FunctionDeclaration(_) => "function",
        Declaration::TSTypeAliasDeclaration(_) => "type-alias",
        _ => "other",
    }
}

fn expr_kind(e: &Expression<'_>) -> &'static str {
    use Expression::*;
    match e {
        ArrayExpression(_) => "array",
        ArrowFunctionExpression(_) => "arrow-function",
        AssignmentExpression(_) => "assignment",
        AwaitExpression(_) => "await",
        ChainExpression(_) => "optional-chain",
        ClassExpression(_) => "class",
        ConditionalExpression(_) => "ternary",
        NewExpression(_) => "new",
        ObjectExpression(_) => "object-literal",
        SequenceExpression(_) => "sequence",
        TaggedTemplateExpression(_) => "tagged-template",
        ThisExpression(_) => "this",
        UpdateExpression(_) => "++/--",
        YieldExpression(_) => "yield",
        _ => "other",
    }
}
