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
use oxc_ast::ast::{Argument, Declaration, Expression, Function as TsFunction, Program, Statement};
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
                let (name, define) = lower_function(f)?;
                let view = name.starts_with("get_");
                out.push(define);
                out.push(list(vec![
                    Sym("export"),
                    Str(name.clone()),
                    Sym(name),
                    if view { Sym("#t") } else { Sym("#f") },
                ]));
            }
            Statement::FunctionDeclaration(f) => {
                out.push(lower_function(f)?.1);
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let init = d
                        .init
                        .as_ref()
                        .ok_or("ts_frontend: top-level declarations need initializers")?;
                    out.push(list(vec![Sym("define"), Sym(name), lower_expr(init)?]));
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
fn lower_function(f: &TsFunction<'_>) -> Result<(String, LispVal), String> {
    let name = f
        .id
        .as_ref()
        .map(|i| i.name.as_str().to_string())
        .ok_or("ts_frontend: anonymous functions unsupported (M1)")?;

    let mut params = Vec::new();
    for p in &f.params.items {
        params.push(Sym(binding_name(&p.pattern)?));
    }

    let body = f
        .body
        .as_ref()
        .ok_or("ts_frontend: function overloads/declarations unsupported")?;

    // view convention: get_* functions' returns become json_return_str
    // (the define tail value alone does not call value_return)
    let view = name.starts_with("get_");
    let expr = lower_block_tail(&body.statements, view)?;

    let mut define_items = Vec::new();
    let mut sig = vec![Sym(name.clone())];
    sig.extend(params);
    define_items.push(Sym("define"));
    define_items.push(list(sig));
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
            list(vec![Sym("begin"), lower_expr(&e.expression)?, tail])
        }
        Statement::IfStatement(i) => {
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
            return Err("ts_frontend: `return` only allowed as the last statement (M1)".into())
        }
        s => {
            return Err(format!(
                "ts_frontend: statement `{}` not allowed mid-function (M1)",
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
        Statement::ExpressionStatement(e) => lower_expr(&e.expression),
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
        s2 => Err(format!(
            "ts_frontend: statement `{}` not in M1 tail subset",
            stmt_kind(s2)
        )),
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
                _ => return Err("ts_frontend: bitwise/shift/exponent ops not in M1".into()),
            };
            Ok(list(vec![
                Sym(op),
                lower_expr(&b.left)?,
                lower_expr(&b.right)?,
            ]))
        }
        Expression::LogicalExpression(l) => {
            // Short-circuit, boolean-valued (0/1) — NOT JS value semantics.
            let a = truthy(&l.left)?;
            let b = truthy(&l.right)?;
            Ok(match l.operator {
                LogicalOperator::And => list(vec![Sym("if"), a, b, Num(0)]),
                LogicalOperator::Or => list(vec![Sym("if"), a, Num(1), b]),
                LogicalOperator::Coalesce => return Err("ts_frontend: ?? not in M1".into()),
            })
        }
        Expression::UnaryExpression(u) => match u.operator {
            UnaryOperator::LogicalNot => Ok(list(vec![Sym("="), truthy(&u.argument)?, Num(0)])),
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
            let mut items = vec![Sym(head)];
            for a in &c.arguments {
                if let Argument::SpreadElement(_) = a {
                    return Err("ts_frontend: spread not in M1".into());
                }
                let e2 = a
                    .as_expression()
                    .ok_or("ts_frontend: unsupported call argument (M1)")?;
                items.push(lower_expr(e2)?);
            }
            Ok(list(items))
        }
        Expression::ParenthesizedExpression(p) => lower_expr(&p.expression),
        _ => Err(format!(
            "ts_frontend: expression `{}` not in M1 subset",
            expr_kind(e)
        )),
    }
}

/// `(if (!= test 0) ...)` — numeric truthiness by decree.
fn truthy(e: &Expression<'_>) -> Result<LispVal, String> {
    Ok(list(vec![Sym("!="), lower_expr(e)?, Num(0)]))
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
    format!("{}/{}", obj, snake(prop))
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
