//! TS frontend (M2): TypeScript-syntax surface → lisp s-expression source.
//!
//! Lowering pipeline: TS source --oxc_parser--> TS AST --this module--> lisp
//! source text --existing parser/checker/emitters--> all backends (near wasm,
//! bytecode, wasi) unchanged.
//!
//! M2 subset:
//!   ✓ function declarations (exported or not) → define (+ export form)
//!   ✓ const/let locals (single declarator, initializer required)
//!   ✓ if / else (full expression in any position)
//!   ✓ return (any position — early returns via flag guard)
//!   ✓ assignment/mutation: x = e, x += e, x -= e, i++, i--
//!   ✓ for / while loops (with break/return support)
//!   ✓ numeric/string/boolean/null/bigint literals, template literals → (str ...)
//!   ✓ binary ops: + - * / % < > <= >= == === != !== && || ^ | & << >>
//!   ✓ ! - unary, ++/-- (expression value)
//!   ✓ calls: bare identifiers + member calls via builtin mapping
//!   ✓ string methods: .length, .slice(), .startsWith(), .indexOf(), .includes()
//!   ✓ arrays: [a, b, c] → (list ...), arr[i] → (nth arr i)
//!   ✓ arrow functions: (a, b) => expr, (a) => { stmts }
//!   ✓ object literals: { key: val } → (json-obj (pair "key" val))
//!   ✗ classes, async, destructuring, optional chaining,
//!     for-in/for-of, switch, try/catch, imports
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
    let mut out = Vec::new();
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
                let (name, define, wrapper) = lower_function(f, true)?;
                let view = name.starts_with("get_");
                out.push(define);
                if let Some(wrapper_def) = wrapper {
                    out.push(wrapper_def);
                    out.push(list(vec![
                        sym("export"),
                        str(name.clone()),
                        sym(format!("_{}", name)),
                        if view { sym("#t") } else { sym("#f") },
                    ]));
                } else {
                    out.push(list(vec![
                        sym("export"),
                        str(name.clone()),
                        sym(name),
                        if view { sym("#t") } else { sym("#f") },
                    ]));
                }
            }
            Statement::FunctionDeclaration(f) => {
                out.push(lower_function(f, false)?.1);
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let init = d
                        .init
                        .as_ref()
                        .ok_or("ts_frontend: top-level declarations need initializers")?;
                    out.push(list(vec![sym("define"), sym(name), lower_expr(init)?]));
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
    Ok(out)
}

/// Lower a function declaration → (define (name params...) body)
/// When `exported`, the real function keeps its params and a separate
/// `_name` wrapper is returned that reads args from JSON input.
fn lower_function(
    f: &TsFunction<'_>,
    exported: bool,
) -> Result<(String, LispVal, Option<LispVal>), String> {
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
    for (n, _) in &param_names {
        params.push(sym(n.clone()));
    }

    let body = f
        .body
        .as_ref()
        .ok_or("ts_frontend: function overloads/declarations unsupported")?;

    // view convention: get_* functions' returns become json_return_str
    let view = name.starts_with("get_");

    // Always emit the real function with proper params and body
    let inner_body = lower_block_tail(&body.statements, view)?;
    let mut define_items = Vec::new();
    let mut sig = vec![sym(name.clone())];
    sig.extend(params);
    define_items.push(sym("define"));
    define_items.push(list(sig));
    define_items.push(inner_body);
    let define = list(define_items);

    // For exported functions with params, generate a thin wrapper that reads
    // args from JSON input and calls the real function.
    let export_wrapper = if exported && !param_names.is_empty() {
        let wrapper_name = format!("_{}", name);
        let bindings: Vec<LispVal> = param_names
            .iter()
            .map(|(n, is_num)| {
                let get = list(vec![sym("near/json_get_str"), str(n.clone())]);
                let v = if *is_num {
                    list(vec![sym("str->num"), get])
                } else {
                    get
                };
                list(vec![sym(n.clone()), v])
            })
            .collect();
        let mut call_items = vec![sym(name.clone())];
        for (n, _) in &param_names {
            call_items.push(sym(n.clone()));
        }
        let wrapper_def = list(vec![
            sym("define"),
            list(vec![sym(wrapper_name.clone())]),
            list(vec![sym("let"), list(bindings), list(call_items)]),
        ]);
        Some(wrapper_def)
    } else {
        None
    };

    Ok((name, define, export_wrapper))
}

/// Lower a statement list whose value is the tail expression.
/// When any non-final statement contains `return`, the whole block is
/// wrapped in a `(__fn_done __res)` flag guard.
/// Check if a statement is `return expr` (for tail-if-else detection).
fn is_return_stmt(s: &Statement<'_>) -> bool {
    matches!(s, Statement::ReturnStatement(_))
}

fn lower_block_tail(stmts: &[Statement<'_>], view: bool) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(num(0));
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    let has_early_return = init.iter().any(stmt_has_return);

    // Optimize: if init is [if-with-return, ...] and tail is return,
    // this is an if-else pattern — no flag guard needed. Each
    // if-with-return gets absorbed as an else branch.
    let is_tail_return = is_return_stmt(&last[0]);
    if has_early_return && is_tail_return {
        // Check if ALL init returns are inside if statements (not arbitrary mid-function)
        let all_init_returns_in_if = init.iter().all(|s| {
            matches!(s, Statement::IfStatement(_))
        });
        if all_init_returns_in_if {
            // Lower as if-else chain: each if-return becomes a branch,
            // the tail return becomes the final else.
            let tail_expr = match &last[0] {
                Statement::ReturnStatement(r) => match &r.argument {
                    Some(e) => {
                        let v = lower_expr(e)?;
                        if view { list(vec![sym("near/json_return_str"), v]) } else { v }
                    }
                    None => sym("nil"),
                },
                _ => unreachable!(),
            };
            return lower_if_else_chain(init, tail_expr, view);
        }
    }

    if has_early_return {
        let tail = lower_tail_stmt(&last[0], view)?;
        // Always assign tail to __fn_res so the return type is consistent.
        // This avoids if-else branch type mismatch (str vs nil).
        let assign_tail = list(vec![sym("set!"), sym("__fn_res"), tail]);
        let body = lower_prefix_around_with_return(init, assign_tail, view)?;
        Ok(list(vec![
            sym("let"),
            list(vec![
                list(vec![sym("__fn_done"), num(0)]),
                list(vec![sym("__fn_res"), sym("nil")]),
            ]),
            body,
        ]))
    } else {
        let tail = lower_tail_stmt(&last[0], view)?;
        lower_prefix_around(init, tail, view)
    }
}

/// Lower a chain of if-return statements followed by a final return as
/// a nested if-else, avoiding the flag-guard overhead.
///   if (c1) return e1;
///   if (c2) return e2;
///   return e3;
/// → (if c1 e1 (if c2 e2 e3))
fn lower_if_else_chain(
    if_stmts: &[Statement<'_>],
    else_val: LispVal,
    view: bool,
) -> Result<LispVal, String> {
    if if_stmts.is_empty() {
        return Ok(else_val);
    }
    let (rest, last_if) = if_stmts.split_at(if_stmts.len() - 1);
    let i = match &last_if[0] {
        Statement::IfStatement(i) => i,
        _ => return Err("ts_frontend: internal: expected if in if-else chain".into()),
    };
    // Get the return value from inside the if's consequent
    let then_val = extract_return_value(stmts_of(&i.consequent), view)?;
    let else_branch = match &i.alternate {
        Some(alt) => {
            // if-else itself: lower the else branch, which may also contain returns
            if stmt_has_return(&i.consequent) || i.alternate.as_ref().is_some_and(|a| stmt_has_return(a)) {
                // Has return in alternate too — recurse into it
                lower_block_tail(stmts_of(alt), view)?
            } else {
                lower_block_tail(stmts_of(alt), view)?
            }
        }
        None => lower_if_else_chain(rest, else_val, view)?,
    };
    Ok(list(vec![sym("if"), truthy(&i.test)?, then_val, else_branch]))
}

/// Extract the expression from a `return expr;` statement (must be single return).
fn extract_return_value(stmts: &[Statement<'_>], view: bool) -> Result<LispVal, String> {
    if stmts.len() != 1 {
        return Err("ts_frontend: expected single return in if-else branch".into());
    }
    match &stmts[0] {
        Statement::ReturnStatement(r) => match &r.argument {
            Some(e) => {
                let v = lower_expr(e)?;
                if view { Ok(list(vec![sym("near/json_return_str"), v])) } else { Ok(v) }
            }
            None => Ok(num(0)),
        },
        _ => Err("ts_frontend: expected return in if-else branch".into()),
    }
}

/// Like `lower_prefix_around` but mid-function `return` writes the
/// `__fn_done/__fn_res` flag pair instead of being an error.
fn lower_prefix_around_with_return(stmts: &[Statement<'_>], tail: LispVal, view: bool) -> Result<LispVal, String> {
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
                bindings.push(list(vec![sym(name), lower_expr(init_e)?]));
            }
            list(vec![sym("let"), list(bindings), tail])
        }
        Statement::ExpressionStatement(e) => {
            let e2 = match &e.expression {
                Expression::AssignmentExpression(asg) => {
                    let (v, expr) = lower_assignment(asg)?;
                    list(vec![sym("set!"), sym(v), expr])
                }
                other => lower_expr(other)?,
            };
            list(vec![sym("begin"), e2, tail])
        }
        Statement::IfStatement(i) => {
            let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                None => sym("nil"),
            };
            let guarded = list(vec![
                sym("if"),
                list(vec![sym("="), sym("__fn_done"), num(0)]),
                list(vec![sym("if"), truthy(&i.test)?, then_e, else_e]),
                sym("nil"),
            ]);
            list(vec![sym("begin"), guarded, tail])
        }
        Statement::ReturnStatement(r) => {
            let val = match &r.argument {
                Some(e) => {
                    let v = lower_expr(e)?;
                    if view { list(vec![sym("near/json_return_str"), v]) } else { v }
                }
                None => sym("nil"),
            };
            list(vec![
                sym("begin"),
                list(vec![sym("set!"), sym("__fn_res"), val]),
                list(vec![sym("set!"), sym("__fn_done"), num(1)]),
                tail,
            ])
        }
        Statement::WhileStatement(_) => {
            let while_e = lower_while_value(&last[0])?;
            list(vec![
                sym("begin"),
                while_e,
                list(vec![sym("if"), sym("__wl_done"),
                    list(vec![sym("begin"),
                        list(vec![sym("set!"), sym("__fn_res"), sym("__wl_res")]),
                        list(vec![sym("set!"), sym("__fn_done"), num(1)]),
                    ]),
                ]),
                tail,
            ])
        }
        Statement::ForStatement(fr) => {
            let for_e = lower_for(fr)?;
            list(vec![
                sym("begin"),
                for_e,
                list(vec![sym("if"), sym("__wl_done"),
                    list(vec![sym("begin"),
                        list(vec![sym("set!"), sym("__fn_res"), sym("__wl_res")]),
                        list(vec![sym("set!"), sym("__fn_done"), num(1)]),
                    ]),
                ]),
                tail,
            ])
        }
        Statement::BlockStatement(b) => {
            let inner = lower_block_tail(&b.body, view)?;
            list(vec![sym("begin"), inner, tail])
        }
        Statement::EmptyStatement(_) => tail,
        s => {
            return Err(format!(
                "ts_frontend: statement `{}` not allowed mid-function",
                stmt_kind(s)
            ))
        }
    };
    lower_prefix_around_with_return(init, inner, view)
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
                bindings.push(list(vec![sym(name), lower_expr(init_e)?]));
            }
            list(vec![sym("let"), list(bindings), tail])
        }
        Statement::ExpressionStatement(e) => {
            // side-effect expression, discard value
            let e2 = match &e.expression {
                Expression::AssignmentExpression(asg) => {
                    let (v, expr) = lower_assignment(asg)?;
                    list(vec![sym("set!"), sym(v), expr])
                }
                other => lower_expr(other)?,
            };
            list(vec![sym("begin"), e2, tail])
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
                            list(vec![sym("if"), truthy(&i.test)?, then_e, else_cont]),
                            view,
                        )
                    }
                    None => {
                        let cont = tail;
                        lower_prefix_around(
                            init,
                            list(vec![sym("if"), truthy(&i.test)?, then_e, cont]),
                            view,
                        )
                    }
                };
            }
            // non-tail if: side-effect only; branches are void-ish blocks.
            let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                None => sym("nil"),
            };
            list(vec![
                sym("begin"),
                list(vec![sym("if"), truthy(&i.test)?, then_e, else_e]),
                tail,
            ])
        }
        Statement::ReturnStatement(_) => {
            return Err("ts_frontend: `return` only allowed as the last statement".into())
        }
        Statement::WhileStatement(_) => {
            list(vec![sym("begin"), lower_while_value(&last[0])?, tail])
        }
        Statement::ForStatement(fr) => {
            list(vec![sym("begin"), lower_for(fr)?, tail])
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
        list(vec![sym("begin"), v, num(0)])
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
                    Ok(list(vec![sym("near/json_return_str"), v]))
                } else {
                    Ok(v)
                }
            }
            None => Ok(num(0)),
        },
        Statement::IfStatement(i) => {
            let then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                None => sym("nil"),
            };
            Ok(list(vec![sym("if"), truthy(&i.test)?, then_e, else_e]))
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
                bindings.push(list(vec![sym(name), lower_expr(init_e)?]));
            }
            Ok(list(vec![sym("let"), list(bindings), num(0)]))
        }
        Statement::EmptyStatement(_) => Ok(num(0)),
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
        let mut body_items = vec![sym("begin")];
        for s in body_stmts {
            if let Statement::VariableDeclaration(_) = s {
                continue; // already hoisted below via hoisted list
            }
            body_items.push(tail_stmt_as_expr(s)?);
        }
        for (name, init) in &hoisted {
            body_items.insert(1, list(vec![sym("set!"), sym(name.clone()), init.clone()]));
        }
        let body_e = if body_items.len() == 1 { num(0) } else { list(body_items) };
        let while_e = list(vec![sym("while"), truthy(&w.test)?, body_e]);
        if hoisted.is_empty() {
            return Ok(while_e);
        }
        let binds: Vec<LispVal> = hoisted
            .iter()
            .map(|(n, _)| list(vec![sym(n.clone()), num(0)]))
            .collect();
        return Ok(list(vec![sym("let"), list(binds), while_e]));
    }
    // break/return rewrite
    let mut body_items = vec![sym("begin")];
    for (name, init) in &hoisted {
        body_items.push(list(vec![sym("set!"), sym(name.clone()), init.clone()]));
    }
    let mut seen_exit = false;
    for s in body_stmts {
        if let Statement::VariableDeclaration(_) = s {
            continue; // hoisted above
        }
        let piece = match s {
            Statement::BreakStatement(_) => list(vec![
                sym("begin"),
                list(vec![sym("set!"), sym("__wl_done"), num(1)]),
                num(0),
            ]),
            Statement::ReturnStatement(r) => {
                let val = match &r.argument {
                    Some(e) => lower_expr(e)?,
                    None => sym("nil"),
                };
                list(vec![
                    sym("begin"),
                    list(vec![sym("set!"), sym("__wl_res"), val]),
                    list(vec![sym("set!"), sym("__wl_done"), num(1)]),
                    num(0), // set! types nil — keep the begin int-typed
                ])
            }
            other => {
                let e = tail_stmt_as_expr(other)?;
                if seen_exit {
                    // dead code after break/return in the same iteration — guard
                    list(vec![
                        sym("if"),
                        list(vec![sym("="), sym("__wl_done"), num(0)]),
                        e,
                        num(0),
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
    let body_e = if body_items.len() == 1 { num(0) } else { list(body_items) };
    let cond_e = list(vec![
        sym("if"),
        list(vec![sym("="), sym("__wl_done"), num(0)]),
        truthy(&w.test)?,
        list(vec![sym("="), num(1), num(0)]), // bool false — keep branch types aligned
    ]);
    let mut binds = vec![
        list(vec![sym("__wl_done"), num(0)]),
        list(vec![sym("__wl_res"), num(0)]),
    ];
    for (n, _) in &hoisted {
        binds.push(list(vec![sym(n.clone()), num(0)]));
    }
    Ok(list(vec![
        sym("let"),
        list(binds),
        list(vec![
            sym("begin"),
            list(vec![sym("while"), cond_e, body_e]),
            sym("__wl_res"),
        ]),
    ]))
}

/// Body of a while/for: statements → single begin-expression (side effects).
fn loop_body_expr(stmts: &[Statement<'_>]) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(num(0));
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
        exprs.push(num(0));
    }
    if exprs.len() == 1 {
        Ok(exprs.into_iter().next().unwrap())
    } else {
        let mut items = vec![sym("begin")];
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
                Ok(list(vec![sym("set!"), sym(v), expr]))
            }
            other => Ok(ensure_int_value(lower_expr(other)?)),
        },
        Statement::ReturnStatement(r) => {
            let val = match &r.argument {
                Some(e) => lower_expr(e)?,
                None => sym("nil"),
            };
            Ok(list(vec![
                sym("begin"),
                list(vec![sym("set!"), sym("__wl_res"), val]),
                list(vec![sym("set!"), sym("__wl_done"), num(1)]),
                num(0),
            ]))
        }
        Statement::BreakStatement(_) => Ok(list(vec![
            sym("begin"),
            list(vec![sym("set!"), sym("__wl_done"), num(1)]),
            num(0),
        ])),
        Statement::ContinueStatement(_) => {
            Err("ts_frontend: continue not supported (use the loop condition)".into())
        }
        Statement::IfStatement(i) => {
            let then_e = loop_body_expr(stmts_of(&i.consequent))?;
            let else_e = match &i.alternate {
                Some(alt) => loop_body_expr(stmts_of(alt))?,
                None => sym("nil"),
            };
            Ok(list(vec![sym("if"), truthy(&i.test)?, then_e, else_e]))
        }
        Statement::VariableDeclaration(v) => {
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                bindings.push(list(vec![sym(name), lower_expr(init_e)?]));
            }
            Ok(list(vec![sym("let"), list(bindings), num(0)]))
        }
        Statement::WhileStatement(_) => lower_while_value(s),
        Statement::ForStatement(fr) => lower_for(fr),
        Statement::BlockStatement(b) => loop_body_expr(&b.body),
        Statement::EmptyStatement(_) => Ok(num(0)),
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
        bindings.push(list(vec![sym(n.clone()), lower_expr(init_e)?]));
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
                list(vec![sym("set!"), sym(v.clone()), list(vec![sym("+"), sym(v), num(one)])])
            }
            Expression::AssignmentExpression(asg) => {
                let (v, expr) = lower_assignment(asg)?;
                list(vec![sym("set!"), sym(v), expr])
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
    let mut begin_items = vec![sym("begin")];
    begin_items.extend(body_items);
    let begin_e = if begin_items.len() == 1 {
        num(0)
    } else {
        list(begin_items)
    };

    // (let ((v init)...) (begin (while cond body) 0))
    Ok(list(vec![
        sym("let"),
        list(bindings),
        list(vec![
            sym("begin"),
            list(vec![sym("while"), truthy(test)?, begin_e]),
            num(0),
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
        AssignmentOperator::Addition => list(vec![sym("+"), sym(v.clone()), rhs]),
        AssignmentOperator::Subtraction => list(vec![sym("-"), sym(v.clone()), rhs]),
        _ => return Err("ts_frontend: only = / += / -= assignments supported".into()),
    };
    Ok((v, out))
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
        Expression::NumericLiteral(n) => Ok(num(n.value as i64)),
        Expression::StringLiteral(s) => Ok(str(s.value.as_str().to_string())),
        Expression::BooleanLiteral(b) => Ok(num(if b.value { 1 } else { 0 })),
        Expression::NullLiteral(_) => Ok(LispVal::Nil),
        Expression::BigIntLiteral(b) => Ok(str(b.raw.as_ref().map(|s| s.as_str().to_string()).unwrap_or_default())), // u128-style digits-as-string
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
                    parts.push(str(cooked));
                }
                if i < t.expressions.len() {
                    // Auto to-string: shields TS authors from the (str)
                    // int-arg renders-empty quirk.
                    parts.push(list(vec![sym("to-string"), lower_expr(&t.expressions[i])?]));
                }
            }
            if parts.is_empty() {
                return Ok(str(String::new()));
            }
            let mut items = vec![sym("str")];
            items.extend(parts);
            Ok(list(items))
        }
        Expression::Identifier(id) => Ok(sym(id.name.as_str().to_string())),
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
                sym(op),
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
                LogicalOperator::And => list(vec![sym("if"), a, b, list(vec![sym("="), num(1), num(0)])]),
                LogicalOperator::Or => list(vec![sym("if"), a, list(vec![sym("="), num(1), num(1)]), b]),
                LogicalOperator::Coalesce => return Err("ts_frontend: ?? not in M1".into()),
            })
        }
        Expression::UnaryExpression(u) => match u.operator {
            UnaryOperator::LogicalNot => {
                if statically_bool(&u.argument) {
                    // bool negation: (= x 0) would be bool≠int — flip instead
                    Ok(list(vec![
                        sym("if"),
                        lower_expr(&u.argument)?,
                        list(vec![sym("="), num(1), num(0)]),
                        list(vec![sym("="), num(1), num(1)]),
                    ]))
                } else {
                    Ok(list(vec![sym("="), lower_expr(&u.argument)?, num(0)]))
                }
            }
            UnaryOperator::UnaryNegation => Ok(list(vec![
                sym("-"),
                num(0),
                lower_expr(&u.argument)?,
            ])),
            UnaryOperator::UnaryPlus => lower_expr(&u.argument),
            _ => Err("ts_frontend: unary operator not in M1".into()),
        },
        Expression::CallExpression(c) => {
            // Detect string/list method calls: receiver.method(args)
            // → (lisp-method receiver args...)
            let (head, receiver) = match &c.callee {
                Expression::StaticMemberExpression(s) => {
                    let obj_name = match &s.object {
                        Expression::Identifier(id) => Some(id.name.as_str().to_string()),
                        Expression::StringLiteral(sl) => Some(sl.value.as_str().to_string()),
                        _ => None,
                    };
                    let prop = s.property.name.as_str();
                    let mapped = map_member_fn(obj_name.as_deref().unwrap_or(""), prop);
                    // Check if it's a string/list method (not a module path)
                    let is_instance_method = matches!(
                        prop,
                        "length" | "slice" | "startsWith" | "endsWith" | "indexOf"
                        | "includes" | "charAt" | "concat" | "toString" | "valueOf"
                        | "push" | "pop" | "join" | "split"
                    ) && obj_name.is_some();
                    if is_instance_method {
                        (mapped, Some((&s.object, obj_name.unwrap())))
                    } else {
                        (map_builtin_call(&mapped), None)
                    }
                }
                other => {
                    let h = map_builtin_call(&callee_name(other)?);
                    (h, None)
                }
            };
            let mut items = vec![sym(head.clone())];
            // Prepend receiver as first arg for instance methods
            if let Some((obj_expr, _name)) = &receiver {
                items.push(lower_expr(obj_expr)?);
            }
            for a in &c.arguments {
                if let Argument::SpreadElement(_) = a {
                    return Err("ts_frontend: spread not supported".into());
                }
                let e2 = a
                    .as_expression()
                    .ok_or("ts_frontend: unsupported call argument")?;
                items.push(lower_expr(e2)?);
            }
            if head == "json-get" {
                return Ok(list(vec![sym("to-string"), list(items)]));
            }
            // str-char-at(s, i) -> (str-slice s i (+ i 1))
            if head == "str-char-at" && items.len() == 3 {
                let s = items[1].clone();
                let idx = items[2].clone();
                return Ok(list(vec![sym("str-slice"), s, idx.clone(), list(vec![sym("+"), idx, num(1)])]));
            }
            Ok(list(items))
        }
        Expression::ConditionalExpression(c) => Ok(list(vec![
            sym("if"),
            truthy(&c.test)?,
            lower_expr(&c.consequent)?,
            lower_expr(&c.alternate)?,
        ])),
        Expression::ParenthesizedExpression(p) => lower_expr(&p.expression),
        // Static member (property access, not call): str.length -> (str-length str)
        Expression::StaticMemberExpression(s) => {
            let obj_e = lower_expr(&s.object)?;
            match s.property.name.as_str() {
                "length" => Ok(list(vec![sym("str-length"), obj_e])),
                prop => Err(format!(
                    "ts_frontend: property access `.{}(syscall_status)` not supported (use method call instead)",
                    prop
                )),
            }
        }
        // Array literal: [a, b, c] -> (list a b c)
        // NOTE: list/nth only available in interpreter, not compiled WASM backends.
        // Compiling with --target near will error if arrays are used.
        Expression::ArrayExpression(arr) => {
            let mut items = vec![sym("list")];
            for el in &arr.elements {
                match el {
                    oxc_ast::ast::ArrayExpressionElement::SpreadElement(_) => {
                        return Err("ts_frontend: spread not supported".into());
                    }
                    oxc_ast::ast::ArrayExpressionElement::Elision(_) => {
                        items.push(num(0));
                    }
                    _ => {
                        if let Some(expr) = el.as_expression() {
                            items.push(lower_expr(expr)?);
                        } else {
                            return Err("ts_frontend: unsupported array element".into());
                        }
                    }
                }
            }
            Ok(list(items))
        }
        // Computed member: arr[i] -> (nth arr i)
        // NOTE: nth only available in interpreter, not compiled WASM.
        Expression::ComputedMemberExpression(cm) => {
            let obj_e = lower_expr(&cm.object)?;
            let idx = lower_expr(&cm.expression)?;
            Ok(list(vec![sym("nth"), obj_e, idx]))
        }
        // Arrow function: (a, b) => expr  or  (a) => { stmts }
        Expression::ArrowFunctionExpression(arrow) => {
            let mut params = Vec::new();
            for p in &arrow.params.items {
                let n = binding_name(&p.pattern)?;
                params.push(sym(n));
            }
            let body = if arrow.body.is_expression() {
                lower_expr(arrow.body.as_expression().unwrap())?
            } else {
                use oxc_ast::ast::ArrowFunctionBody;
                match &arrow.body {
                    ArrowFunctionBody::FunctionBody(b) => {
                        lower_block_tail(&b.statements, false)?
                    }
                    _ => return Err("ts_frontend: unexpected arrow body".into()),
                }
            };
            Ok(list(vec![sym("lambda"), list(params), body]))
        }
        // Object literal: { key: val } -> (json-obj (pair "key" val))
        Expression::ObjectExpression(obj) => {
            let mut items = vec![sym("json-obj")];
            for prop in &obj.properties {
                match prop {
                    oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                        let key_str = match &p.key {
                            oxc_ast::ast::PropertyKey::StaticIdentifier(id) => {
                                id.name.as_str().to_string()
                            }
                            other => {
                                // String literals etc. come via INHERIT
                                if let Some(expr) = other.as_expression() {
                                    if let Expression::StringLiteral(s) = expr {
                                        s.value.as_str().to_string()
                                    } else {
                                        return Err("ts_frontend: object key must be identifier or string".into());
                                    }
                                } else {
                                    return Err("ts_frontend: object key must be identifier or string".into());
                                }
                            }
                        };
                        let val = lower_expr(&p.value)?;
                        items.push(list(vec![sym("pair"), str(key_str), val]));
                    }
                    oxc_ast::ast::ObjectPropertyKind::SpreadProperty(_) => {
                        return Err("ts_frontend: object spread not supported".into());
                    }
                }
            }
            Ok(list(items))
        }
        // Update expressions: i++ -> (+ i 1), i-- -> (- i 1)
        Expression::UpdateExpression(u) => {
            let v = update_target_simple(&u.argument)?;
            let one = if matches!(u.operator, oxc_syntax::operator::UpdateOperator::Increment) { 1 } else { -1 };
            Ok(list(vec![sym("+"), sym(v), num(one)]))
        }
        _ => Err(format!(
            "ts_frontend: expression `{}` not supported",
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
                    "u128/gt" | "u128/lt" | "u128/gte" | "u128/lte" | "u128/eq"
                    | "near/deposit-gte"
                    | "str-starts-with" | "str-ends-with" | "str-contains"
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
        Ok(list(vec![sym("!="), lower_expr(e)?, num(0)]))
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
                Expression::StringLiteral(sl) => sl.value.as_str().to_string(),
                _ => return Err("ts_frontend: nested member chains not in M1".into()),
            };
            Ok(map_member_fn(&obj, s.property.name.as_str()))
        }
        Expression::ComputedMemberExpression(_) => {
            Err("ts_frontend: computed member calls not supported".into())
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
    // near_storage_get → near/storage_get (namespace prefix convention)
    if let Some(rest) = name.strip_prefix("near_") {
        return format!("near/{}", snake(rest));
    }
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
        return "near/deposit-gte".into();
    }
    // String instance methods -> lisp string builtins
    match prop {
        "length" => return "str-length".into(),
        "slice" => return "str-slice".into(),
        "startsWith" => return "str-starts-with".into(),
        "endsWith" => return "str-ends-with".into(),
        "indexOf" => return "str-index-of".into(),
        "includes" => return "str-contains".into(),
        "charAt" => return "str-char-at".into(),
        "concat" => return "str-cat".into(),
        "toString" | "valueOf" => return "to-string".into(),
        _ => {}
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
fn sym(s: impl Into<String>) -> LispVal {
    LispVal::Sym(s.into())
}
fn num(n: i64) -> LispVal {
    LispVal::Num(n)
}
fn str(s: impl Into<String>) -> LispVal {
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
        AssignmentExpression(_) => "assignment",
        AwaitExpression(_) => "await",
        ChainExpression(_) => "optional-chain",
        ClassExpression(_) => "class",
        ConditionalExpression(_) => "ternary",
        NewExpression(_) => "new",
        SequenceExpression(_) => "sequence",
        TaggedTemplateExpression(_) => "tagged-template",
        ThisExpression(_) => "this",
        YieldExpression(_) => "yield",
        _ => "other",
    }
}
