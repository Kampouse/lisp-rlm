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
//!   ✓ arrow fns (2026-08-30): expression/single-return bodies, as
//!     callbacks; (2026-08-31) full block bodies via lower_block_tail
//!     (begin/let/if sequencing, early returns)
//!     .map/.filter/.reduce callbacks — inlined by resolve_lambda_1/2,
//!     so the T4 closure-aliasing landmine never triggers
//!   ✓ array pipeline chaining (2026-08-30): join/map/filter/reduce take
//!     any receiver — xs.filter(f).map(g).join(s) stacks
//!   ✗ classes, async, general closures (non-callback position),
//!     destructuring, optional chaining, early returns, imports
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
    TSType, VariableDeclarator,
};
use oxc_parser::Parser;
use oxc_span::SourceType;
use oxc_syntax::operator::{BinaryOperator, LogicalOperator, UnaryOperator};

// ── Public entry ──────────────────────────────────────────────────────────

/// Parse TypeScript source and lower it to lisp source text.
pub fn ts_to_lisp_source(src: &str) -> Result<String, String> {
    // compilation is stateless from the caller's view — reset all
    // cross-compilation side maps (tests compile many programs on one
    // thread; stale consts/aliases would shadow)
    IDENT_OFFSETS.with(|m| m.borrow_mut().clear());
    NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
    OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
    BIGINT_NAMES.with(|s| s.borrow_mut().clear());
    BIGINT_LOCALS.with(|s| s.borrow_mut().clear());
    STRING_LOCALS.with(|s| s.borrow_mut().clear());
    SHAPE_BIGINT_FIELDS.with(|s| s.borrow_mut().clear());
    TYPE_ALIASES.with(|s| s.borrow_mut().clear());
    CONST_FOLDS.with(|s| s.borrow_mut().clear());
    BIGINT_CONSTS.with(|s| s.borrow_mut().clear());
    let exprs = parse_ts(src)?;
    let mut out = String::new();
    for e in &exprs {
        out.push_str(&sexp(e));
        out.push('\n');
    }
    Ok(out)
}

// ── TS source positions for error reporting ─────────────────────────────
// The lowering walk drops oxc spans, so we thread a side map (thread_local
// to avoid churning every lower_* signature): every identifier reference and
// declaration records (name, byte-offset). Downstream errors mention names;
// the CLI boundary resolves name → TS line.

thread_local! {
    static IDENT_OFFSETS: std::cell::RefCell<Vec<(String, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Numeric-typed parameter names in scope during body lowering — used
    /// by object-literal value encoding (`{votes: votes}` encodes bare
    /// number when `votes: number` was annotated).
    static NUM_PARAM_NAMES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// `bigint`-annotated param names in scope — u128-precision amounts.
    /// Drives operator selection (`a + b` lowers to u128/add).
    static BIGINT_NAMES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// `let x = <bigint expr>;` locals in the function being lowered —
    /// bigint-shaped for later operator selection in the same body.
    static BIGINT_LOCALS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// String-typed locals in the function being lowered: seeded by
    /// `let s = <stringy>;` (literal/template/method-call) and GROWN by
    /// `s = <stringy>` / `s += x` / `s = s + x` assignments. Drives `+`
    /// dispatch on var+var operands — neither side is a literal, so the
    /// static stringy checks can't see it (surface tour 2 for-of
    /// accumulator, 2026-09-01: `out = out + x` emitted numeric + on
    /// strings → interp type-error / wasm tagged-garbage).
    static STRING_LOCALS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Record-typed locals with bigint fields, inferred from the shape
    /// literal in `let rec = storageGet(...) ?? '{"amt":"0",...}'`:
    /// keys whose default is a QUOTED NUMERIC string are bigint fields
    /// (the storageGet ?? record pattern; found via the HTLC contract
    /// 2026-09-01 — `rec.amt + x` lowered to plain numeric + because
    /// dot-access never carried the shape's bigint typing).
    static SHAPE_BIGINT_FIELDS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Object-typed params in scope: (param, props) where props carry
    /// is_number per key. Drives (1) read-time auto str->num on
    /// `param.numericProp`, (2) encode-time raw embedding of the param
    /// into object literals (its value already IS JSON text).
    static OBJ_PARAM_PROPS: std::cell::RefCell<Vec<(String, Vec<(String, bool)>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// `type X = { ... }` aliases collected at statement level; resolved
    /// when a param is annotated with a named type. Compile-time only.
    static TYPE_ALIASES: std::cell::RefCell<Vec<(String, Vec<(String, bool)>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Top-level `const K = <literal>;` — folded into every use site.
    /// (2026-08-31) a value-define at top level emits a stub (known emitter
    /// limitation), so numeric/string consts INSTEAD substitute inline and
    /// emit nothing. Non-literal top-level consts keep the old path.
    static CONST_FOLDS: std::cell::RefCell<Vec<(String, LispVal)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Top-level `const K = <n-literal>;` names — bigint-shaped identifiers
    /// for operator selection (fold value lands in CONST_FOLDS as Str).
    static BIGINT_CONSTS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn note_ident(name: &str, offset: u32) {
    IDENT_OFFSETS.with(|m| {
        let mut m = m.borrow_mut();
        // keep first occurrence per name — declarations usually precede refs,
        // and refs-before-def (hoisting) are exactly the undefined ones
        if !m.iter().any(|(n, _)| n == name) {
            m.push((name.to_string(), offset));
        }
    });
}

/// byte offset → (line, col), both 1-based
fn line_col(src: &str, offset: u32) -> (u32, u32) {
    let off = (offset as usize).min(src.len());
    let (mut line, mut col) = (1u32, 1u32);
    for &b in &src.as_bytes()[..off] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// one-line source excerpt with the offending line, caret under col
fn src_excerpt(src: &str, line: u32) -> String {
    src.lines()
        .nth((line as usize).saturating_sub(1))
        .map(|l| format!("\n  {:>4} | {}\n       | {}^", line, l.trim_end(), " ".repeat(0)))
        .unwrap_or_default()
}

/// Parse TypeScript source and lower it to top-level lisp forms.
pub fn parse_ts(src: &str) -> Result<Vec<LispVal>, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::default().with_typescript(true).with_module(true);
    let ret = Parser::new(&allocator, src, source_type).parse();
    if ret.panicked || !ret.diagnostics.is_empty() {
        let d = ret.diagnostics.first();
        let msg = d
            .map(|d| d.message.to_string())
            .unwrap_or_else(|| "unknown parse error".into());
        if let Some(d) = d {
            // span lives in labels[0] (LabeledSpan::offset)
            let off = d
                .labels
                .iter()
                .next()
                .map(|l| l.offset() as u32)
                .unwrap_or(0);
            let (line, col) = line_col(src, off);
            return Err(format!(
                "TS parse error at line {}, col {}: {}{}",
                line,
                col,
                msg,
                src_excerpt(src, line)
            ));
        }
        return Err(format!("TS parse error: {}", msg));
    }
    IDENT_OFFSETS.with(|m| m.borrow_mut().clear());
    // on success (or error) the map holds first-occurrence offsets for every
    // identifier seen during the walk — drained by take_ident_offsets()
    lower_program(&ret.program)
}

/// Parse + retain the ident→offset map (for augmenting downstream errors).
/// Consumes the map the walk just produced — call immediately after a
/// successful `parse_ts` on the SAME thread.
pub fn take_ident_offsets() -> Vec<(String, u32)> {
    IDENT_OFFSETS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Best-effort: name → "line N" hint for error augmentation.
pub fn ts_line_hint(map: &[(String, u32)], src: &str, name: &str) -> Option<String> {
    map.iter()
        .find(|(n, _)| n == name)
        .map(|(_, off)| {
            let (line, _col) = line_col(src, *off);
            format!("{}", line)
        })
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
                match &decl.declaration {
                    Declaration::FunctionDeclaration(f) => {
                        if f.r#async {
                            for form in lower_async_function(f)? {
                                out.push(form);
                            }
                            continue;
                        }
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
                    // export const f = (params) => body — arrow exported as a
                    // named entry. Function-shaped define required (a value
                    // define `(define f (lambda...))` compiles to a stub).
                    // Non-arrow exported consts stay a hard error.
                    Declaration::VariableDeclaration(v) => {
                        if v.declarations.len() != 1 {
                            return Err("ts_frontend: `export const` supports exactly one declarator".into());
                        }
                        let d = &v.declarations[0];
                        let name = binding_name(&d.id)?;
                        match d.init.as_ref() {
                            Some(Expression::ArrowFunctionExpression(a)) => {
                                let (define, export_form) = lower_exported_arrow(&name, a)?;
                                out.push(define);
                                out.push(export_form);
                            }
                            _ => {
                                return Err(format!(
                                    "ts_frontend: `export const {}` needs an arrow initializer (other exports must use `export function`)",
                                    name
                                ))
                            }
                        }
                    }
                    d => {
                        return Err(format!(
                            "ts_frontend: only `export function` or `export const f = arrow` are supported, got {}",
                            decl_kind(d)
                        ))
                    }
                }
            }
            Statement::FunctionDeclaration(f) => {
                if f.r#async {
                    return Err(
                        "ts_frontend: V1 async functions must be exported (continuation is an on-chain entry)"
                            .into(),
                    );
                }
                hoisted.push(lower_function(f, false)?.1);
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let init = d
                        .init
                        .as_ref()
                        .ok_or("ts_frontend: top-level declarations need initializers")?;
                    // Literal initializer → fold at use sites (top-level
                    // value-defines emit stubs — see CONST_FOLDS note).
                    let mut is_bigint = false;
                    let literal = match init {
                        Expression::NumericLiteral(n) => Some(Num(n.value as i64)),
                        Expression::StringLiteral(s) => {
                            Some(Str(s.value.as_str().to_string()))
                        }
                        Expression::BooleanLiteral(b) => {
                            Some(Num(if b.value { 1 } else { 0 }))
                        }
                        // `const FEE_BP = 500n;` — u128 const: folds as a
                        // decimal string AND marks the name bigint-shaped
                        Expression::BigIntLiteral(b) => {
                            is_bigint = true;
                            Some(Str(
                                b.raw
                                    .as_ref()
                                    .map(|s| s.as_str().trim_end_matches('n').to_string())
                                    .unwrap_or_default(),
                            ))
                        }
                        _ => None,
                    };
                    if let Some(v) = literal {
                        if is_bigint {
                            BIGINT_CONSTS.with(|m| m.borrow_mut().push(name.clone()));
                        }
                        CONST_FOLDS.with(|m| m.borrow_mut().push((name, v)));
                    } else {
                        consts.push(list(vec![Sym("define"), Sym(name), lower_expr(init)?]));
                    }
                }
            }
            Statement::ExpressionStatement(e) => {
                out.push(lower_expr(&e.expression)?);
            }
            Statement::EmptyStatement(_) => {}
            // `type X = { ... }` — data-shape declaration, compile-time
            // only: record the shape for object-param annotations, emit
            // nothing. (Aliases must appear before use — single pass.)
            s if matches!(
                s.as_declaration(),
                Some(Declaration::TSTypeAliasDeclaration(_))
            ) =>
            {
                let a = match s.as_declaration() {
                    Some(Declaration::TSTypeAliasDeclaration(a)) => a,
                    _ => unreachable!(),
                };
                let props = alias_props(a);
                TYPE_ALIASES.with(|m| {
                    m.borrow_mut()
                        .push((a.id.name.as_str().to_string(), props));
                });
            }
            // Types-only imports from the near module family are ELIDED.
            // The ambient d.ts (ts/lisp-rlm.d.ts → Monaco addExtraLib)
            // provides editor completions without any import; near-sdk-js
            // muscle memory pastes an import line, so accept it. Anything
            // else is a hard error (no module system at runtime).
            Statement::ImportDeclaration(imp) => {
                let src = imp.source.value.as_str();
                if src == "near" || src.starts_with("near-") || src.starts_with("./near") {
                    // `import near from "near"` would SHADOW the ambient
                    // global — hint the importless spelling.
                    let has_default = imp
                        .specifiers
                        .iter()
                        .flatten()
                        .any(|s| matches!(s, oxc_ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(_)));
                    if has_default {
                        return Err(
                            "ts_frontend: `import near from \"near\"` shadows the built-in `near` global — delete the import line; `near.*` works without it".into(),
                        );
                    }
                    // named/type imports: types-only, elide
                } else {
                    return Err(format!(
                        "ts_frontend: imports are not supported (module `{}`) — only types-only `import {{...}} from \"near*\"` is elided",
                        src
                    ));
                }
            }
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

/// Lower an async function into entry + continuation via near/call-await.
/// V1 (fail-loud): exactly one await, `const x = await near.call(...)`,
/// and it must be the FIRST statement (their original V1 silently dropped
/// pre-await statements — we hard-error instead).
/// Returns forms: entry define, entry export, continuation define, cont. export.
fn lower_async_function(f: &TsFunction<'_>) -> Result<Vec<LispVal>, String> {
    let name = f
        .id
        .as_ref()
        .map(|i| i.name.as_str().to_string())
        .ok_or("ts_frontend: anonymous async functions unsupported")?;

    // (name, kind): 0 = string, 1 = number, 2 = string[], 3 = object
    // (object = JSON-text binding; numeric props auto-decode on read)
    NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
    OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
    BIGINT_NAMES.with(|s| s.borrow_mut().clear());
    BIGINT_LOCALS.with(|s| s.borrow_mut().clear());
    STRING_LOCALS.with(|s| s.borrow_mut().clear());
    SHAPE_BIGINT_FIELDS.with(|s| s.borrow_mut().clear());
    let mut param_names: Vec<(String, u8)> = Vec::new();
    for p in &f.params.items {
        let n = binding_name(&p.pattern)?;
        let kind = if param_is_bigint(p) {
            4
        } else if param_is_number(p) {
            1
        } else if param_is_str_array(p) {
            2
        } else if let Some(props) = param_object_props(p) {
            register_obj_param(&n, props);
            3
        } else if param_is_type_ref(p) {
            return Err(
                "ts_frontend: named type params unsupported — use an inline object literal type"
                    .into(),
            );
        } else {
            0
        };
        if kind == 4 {
            BIGINT_NAMES.with(|s| s.borrow_mut().push(n.clone()));
        }
        if kind == 0 {
            // See lower_function: string params skip the to-string
            // interpolation wrap (2026-09-02).
            mark_string_local(&n);
        }
        param_names.push((n.clone(), kind));
    }

    let body = f
        .body
        .as_ref()
        .ok_or("ts_frontend: async function missing body")?;
    let stmts = &body.statements;

    // Find `const x = await expr;`
    let mut await_idx = None;
    let mut await_var = None;
    let mut await_expr = None;
    for (i, s) in stmts.iter().enumerate() {
        if let oxc_ast::ast::Statement::VariableDeclaration(vd) = s {
            if vd.declarations.len() == 1 {
                let decl = &vd.declarations[0];
                if let Some(init) = &decl.init {
                    if let oxc_ast::ast::Expression::AwaitExpression(ae) = init {
                        await_idx = Some(i);
                        await_var = Some(binding_name(&decl.id)?);
                        await_expr = Some(&ae.argument);
                        break;
                    }
                }
            }
        }
    }
    let await_idx =
        await_idx.ok_or("ts_frontend: async function must contain `const x = await expr;`")?;
    if await_idx != 0 {
        return Err(
            "ts_frontend: V1 async — await must be the first statement (pre-await code unsupported)"
                .into(),
        );
    }
    let await_var = await_var.unwrap();
    let await_expr = await_expr.unwrap();
    let after_stmts = &stmts[await_idx + 1..];

    let state_key = format!("__await:{}", name);
    let cb_name = format!("{}__resume", name);

    // ── entry: read params from tx json, save state, fire call-await ──
    let mut entry_inner: Vec<LispVal> = Vec::new(); // begin-items after bindings
    for (n, kind) in &param_names {
        let v = if *kind == 1 {
            list(vec![Sym("to-string"), Sym(n.clone())])
        } else {
            Sym(n.clone())
        };
        entry_inner.push(list(vec![
            Sym("near/storage_set"),
            Str(format!("{}:{}", state_key, n)),
            v,
        ]));
    }
    // await near.call(target, method, args, gas, deposit)
    //   → near/call-await(target, method, args, gas, cb, 50Tgas, "{}")
    let await_lisp = lower_expr(await_expr)?;
    let call_await = match &await_lisp {
        LispVal::List(items) if items.len() >= 6 && items[0] == Sym("near/call") => {
            // fail-loud: dropped deposit must be zero
            let dep_ok = matches!(&items[5], LispVal::Num(0))
                || matches!(&items[5], LispVal::Str(x) if x == "0");
            if !dep_ok {
                return Err(
                    "ts_frontend: await near.call(...) — deposit must be 0 (call-await is zero-deposit; use the raw near/call-await form for payable)"
                        .into(),
                );
            }
            let mut new_items = vec![Sym("near/call-await")];
            new_items.extend(items[1..5].iter().cloned());
            new_items.push(Str(cb_name.clone()));
            new_items.push(Num(50_000_000_000_000));
            new_items.push(Str("{}".to_string()));
            list(new_items)
        }
        LispVal::List(items) if !items.is_empty() && items[0] == Sym("near/call") => {
            return Err("ts_frontend: await must wrap near.call() with 5 args (target, method, args, gas, 0)".into());
        }
        _ => {
            return Err(
                "ts_frontend: V1 async — await expression must be near.call(...)".into(),
            )
        }
    };
    entry_inner.push(call_await);

    let entry_body = if param_names.is_empty() {
        let mut b = vec![Sym("begin")];
        b.extend(entry_inner);
        list(b)
    } else {
        let bindings = param_names
            .iter()
            .map(|(n, kind)| {
                let get = list(vec![Sym("near/json_get_str"), Str(n.clone())]);
                let v = match kind {
                    1 => list(vec![Sym("str->num"), get]),
                    2 => list(vec![Sym("near/json_get_arr"), Str(n.clone())]),
                    _ => get,
                };
                list(vec![Sym(n.clone()), v])
            })
            .collect();
        list(vec![
            Sym("let"),
            list(bindings),
            {
                let mut b = vec![Sym("begin")];
                b.extend(entry_inner);
                list(b)
            },
        ])
    };
    let entry_define = list(vec![Sym("define"), list(vec![Sym(name.clone())]), entry_body]);
    let view = name.starts_with("get_");
    let entry_export = list(vec![
        Sym("export"),
        Str(name.clone()),
        Sym(name.clone()),
        if view { Sym("#t") } else { Sym("#f") },
    ]);

    // ── continuation: restore state, bind promise result, run the rest ──
    let mut let_bindings: Vec<LispVal> = Vec::new();
    for (n, kind) in &param_names {
        // storage_get returns (opt str) — unwrap with default before use
        let getter = list(vec![
            Sym("default"),
            list(vec![Sym("near/storage_get"), Str(format!("{}:{}", state_key, n))]),
            Str(String::new()),
        ]);
        let val = if *kind == 1 {
            list(vec![Sym("str->num"), getter])
        } else {
            getter
        };
        let_bindings.push(list(vec![Sym(n.clone()), val]));
    }
    let_bindings.push(list(vec![
        Sym(await_var.clone()),
        list(vec![Sym("near/promise_result"), Num(0)]),
    ]));
    let after_body = lower_block_tail(after_stmts, false)?;
    let cb_define = list(vec![
        Sym("define"),
        list(vec![Sym(cb_name.clone())]),
        list(vec![Sym("let"), list(let_bindings), after_body]),
    ]);
    let cb_export = list(vec![
        Sym("export"),
        Str(cb_name.clone()),
        Sym(cb_name.clone()),
        Sym("#f"),
    ]);

    Ok(vec![entry_define, entry_export, cb_define, cb_export])
}

/// Forward scan: register every `let x = <bigint-init>;` in a function
/// body BEFORE lowering. The statement lowering is CPS-style (continuations
/// lower before the statement itself), so registering at the let-site was
/// too late for later statements that reference the binding.
/// STRING_LOCALS uses the same forward scan: `let out = "";` must be
/// marked before the `out + x` binary-+ site lowers.
fn scan_bigint_lets(stmts: &[Statement<'_>]) {
    for s in stmts {
        scan_one_bigint_let(s);
    }
}

fn scan_one_bigint_let(s: &Statement<'_>) {
    if let Statement::VariableDeclaration(v) = s {
        for d in &v.declarations {
            let Some(init) = &d.init else { continue };
            if expr_is_bigint(init) {
                if let Ok(name) = binding_name(&d.id) {
                    BIGINT_LOCALS.with(|m| m.borrow_mut().push(name));
                }
            }
            if let Ok(name) = binding_name(&d.id) {
                if expr_is_stringy(init)
                    || expr_is_str_method_call(init)
                    || matches!(init, Expression::Identifier(id) if is_string_local(id.name.as_str()))
                {
                    mark_string_local(&name);
                }
            }
            register_shape_fields(d, init);
        }
    }
    match s {
        Statement::BlockStatement(b) => scan_bigint_lets(&b.body),
        Statement::IfStatement(i) => {
            scan_one_bigint_let(&i.consequent);
            if let Some(alt) = &i.alternate {
                scan_one_bigint_let(alt); // covers `else if` chains
            }
        }
        Statement::WhileStatement(w) => scan_one_bigint_let(&w.body),
        _ => {}
    }
}

/// Lower a function declaration → (define (name params...) body)
fn lower_function(f: &TsFunction<'_>, exported: bool) -> Result<(String, LispVal), String> {
    let name = f
        .id
        .as_ref()
        .map(|i| i.name.as_str().to_string())
        .ok_or("ts_frontend: anonymous functions unsupported (M1)")?;

    let mut params = Vec::new();
        // (name, kind): 0 = string, 1 = number, 2 = string[], 3 = object
    // (object = JSON-text binding; numeric props auto-decode on read)
    NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
    OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
    BIGINT_NAMES.with(|s| s.borrow_mut().clear());
    BIGINT_LOCALS.with(|s| s.borrow_mut().clear());
    STRING_LOCALS.with(|s| s.borrow_mut().clear());
    SHAPE_BIGINT_FIELDS.with(|s| s.borrow_mut().clear());
    let mut param_names: Vec<(String, u8)> = Vec::new();
    for p in &f.params.items {
        let n = binding_name(&p.pattern)?;
        let kind = if param_is_bigint(p) {
            4
        } else if param_is_number(p) {
            1
        } else if param_is_str_array(p) {
            2
        } else if let Some(props) = param_object_props(p) {
            register_obj_param(&n, props);
            3
        } else if param_is_type_ref(p) {
            return Err(
                "ts_frontend: named type params unsupported — use an inline object literal type"
                    .into(),
            );
        } else {
            0
        };
        if kind == 4 {
            BIGINT_NAMES.with(|s| s.borrow_mut().push(n.clone()));
        }
        if kind == 0 {
            // String-typed params (unannotated defaults to str in this
            // dialect) register as string locals: template interpolation
            // skips the defensive to-string wrap, and `+` dispatch sees
            // stringness the same way the checker does (2026-09-02).
            mark_string_local(&n);
        }
        param_names.push((n.clone(), kind));
    }

    let body = f
        .body
        .as_ref()
        .ok_or("ts_frontend: function overloads/declarations unsupported")?;

    // forward-scan bigint lets (CPS lowering means let-site registration
    // runs after statements that reference the binding — see scan_bigint_lets)
    scan_bigint_lets(&body.statements);

    // view convention: get_* functions' returns become json_return_str
    // (the define tail value alone does not call value_return)
    let view = name.starts_with("get_");

    // Exported contracts read args from the transaction input JSON
    // (json_get_str pattern); `: number` annotations wrap str->num.
    let expr = if exported {
        if !param_names.is_empty() {
            let bindings = param_names
                .iter()
                .map(|(n, kind)| {
                    let get = list(vec![Sym("near/json_get_str"), Str(n.clone())]);
                    let v = match kind {
                        1 => {
                            NUM_PARAM_NAMES.with(|s| s.borrow_mut().push(n.clone()));
                            list(vec![Sym("str->num"), get])
                        }
                        2 => list(vec![Sym("near/json_get_arr"), Str(n.clone())]),
                        _ => get,
                    };
                    list(vec![Sym(n.clone()), v])
                })
                .collect();
            let inner = lower_block_tail(&body.statements, view)?;
            NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
            OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
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

/// A bare mid-function return: `return e;` as a statement at this level
/// (nested ifs/loops handle their own exits via takeover / __wl guards).
fn stmts_have_bare_return(stmts: &[Statement<'_>]) -> bool {
    stmts.iter().any(|s| match s {
        Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(b) => stmts_have_bare_return(&b.body),
        _ => false,
    })
}

/// Lower a statement list whose value is the tail expression.
fn lower_block_tail(stmts: &[Statement<'_>], view: bool) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(Num(0));
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    if stmts_have_bare_return(init) {
        // early-return function: flag-guard lowering (M2).
        // __fn_res starts as nil (bottom type — accepts str/num set!s).
        let tail = lower_tail_stmt(&last[0], view)?;
        let guarded_tail = list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
            tail,
            Sym("__fn_res"),
        ]);
        let body = lower_prefix_around_with_return(init, guarded_tail, view)?;
        return Ok(list(vec![
            Sym("let"),
            list(vec![
                list(vec![Sym("__fn_done"), Num(0)]),
                list(vec![Sym("__fn_res"), list(vec![Sym("quote"), LispVal::Nil])]),
            ]),
            body,
        ]));
    }
    let tail = lower_tail_stmt(&last[0], view)?;
    lower_prefix_around(init, tail, view)
}

/// Like lower_prefix_around, but a bare `return e;` mid-function stores
/// into __fn_res/__fn_done (bound by lower_block_tail's flag-guard path).
/// If-branches containing returns capture their value into __fn_res —
/// the uniform guard makes any post-return statement a no-op.
fn lower_prefix_around_with_return(
    stmts: &[Statement<'_>],
    tail: LispVal,
    view: bool,
) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(tail);
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    let inner = match &last[0] {
        Statement::VariableDeclaration(v) => {
            // pure binding — no guard needed (no side effects to skip)
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                if expr_is_bigint(init_e) {
                    BIGINT_LOCALS.with(|s| s.borrow_mut().push(name.clone()));
                }
                if expr_is_stringy(init_e) || expr_is_str_method_call(init_e) {
                    mark_string_local(&name);
                }
                bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
            }
            list(vec![Sym("let"), list(bindings), tail])
        }
        Statement::ExpressionStatement(e) => {
            // side-effect statement: skip entirely once the function has
            // returned (TS semantics — statements after return don't run)
            let e2 = effect_expr(&e.expression)?;
            list(vec![
                Sym("begin"),
                list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    e2,
                    Num(0),
                ]),
                tail,
            ])
        }
        Statement::IfStatement(i) => {
            let mut then_e = lower_block_tail(stmts_of(&i.consequent), view)?;
            if stmt_has_return(&i.consequent) {
                // branch value becomes the function result
                then_e = list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__fn_res"), then_e]),
                    list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                ]);
            }
            let else_e = match &i.alternate {
                Some(alt) => {
                    let mut e = lower_block_tail(stmts_of(alt), view)?;
                    if stmt_has_return(alt) {
                        e = list(vec![
                            Sym("begin"),
                            list(vec![Sym("set!"), Sym("__fn_res"), e]),
                            list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                        ]);
                    }
                    e
                }
                None => Num(0),
            };
            list(vec![
                Sym("begin"),
                list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    list(vec![Sym("if"), truthy(&i.test)?, then_e, else_e]),
                    Num(0),
                ]),
                tail,
            ])
        }
        Statement::ReturnStatement(r) => {
            let val = match &r.argument {
                Some(e) => {
                    let v = lower_expr(e)?;
                    if view {
                        list(vec![Sym("near/json_return_str"), v])
                    } else {
                        v
                    }
                }
                None => Num(0),
            };
            list(vec![
                Sym("begin"),
                list(vec![Sym("set!"), Sym("__fn_res"), val]),
                list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                tail,
            ])
        }
        Statement::WhileStatement(_) => {
            let (has_exits, core) = lower_while_parts(&last[0])?;
            let mut v = vec![Sym("begin"), core];
            if has_exits {
                // loop exit feeds the function-level flag too
                v.push(list(vec![
                    Sym("if"),
                    Sym("__wl_done"),
                    list(vec![
                        Sym("begin"),
                        list(vec![Sym("set!"), Sym("__fn_res"), Sym("__wl_res")]),
                        list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                    ]),
                ]));
            }
            v.push(tail);
            list(v)
        }
        Statement::ForOfStatement(fo) => {
            let (has_exits, core) = lower_for_of_parts(fo)?;
            let mut v = vec![Sym("begin"), core];
            if has_exits {
                v.push(list(vec![
                    Sym("if"),
                    Sym("__wl_done"),
                    list(vec![
                        Sym("begin"),
                        list(vec![Sym("set!"), Sym("__fn_res"), Sym("__wl_res")]),
                        list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                    ]),
                ]));
            }
            v.push(tail);
            list(v)
        }
        Statement::BlockStatement(b) => {
            let inner_blk = lower_block_tail(&b.body, view)?;
            list(vec![Sym("begin"), inner_blk, tail])
        }
        Statement::EmptyStatement(_) => tail,
        s => {
            return Err(format!(
                "ts_frontend: statement `{}` not allowed mid-function (M2 early-return)",
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
                if expr_is_bigint(init_e) {
                    BIGINT_LOCALS.with(|s| s.borrow_mut().push(name.clone()));
                }
                if expr_is_stringy(init_e) || expr_is_str_method_call(init_e) {
                    mark_string_local(&name);
                }
                register_shape_fields(d, init_e);
                bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
            }
            list(vec![Sym("let"), list(bindings), tail])
        }
        Statement::ExpressionStatement(e) => {
            // side-effect expression, discard value
            let e2 = effect_expr(&e.expression)?;
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
            // A loop whose body can return/break owns two extra locals.
            // Mid-function, the CONTINUATION must be guarded on the flags —
            // otherwise a `return` inside the loop would set __wl_res and
            // then fall through to `tail` anyway (the for+return bug of
            // 2026-08-30: loops ran past the return and the function kept
            // its trailing value).
            let (has_exits, core) = lower_while_parts(&last[0])?;
            if has_exits {
                let res_e = exit_result_form(view);
                list(vec![
                    Sym("let"),
                    list(vec![
                        list(vec![Sym("__wl_done"), Num(0)]),
                        list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            tail,
                            res_e,
                        ]),
                    ]),
                ])
            } else {
                list(vec![Sym("begin"), core, tail])
            }
        }
        Statement::ForOfStatement(fo) => {
            let (has_exits, core) = lower_for_of_parts(fo)?;
            if has_exits {
                // view exports must json-wrap the mid-loop return value too
                let res_e = exit_result_form(view);
                list(vec![
                    Sym("let"),
                    list(vec![
                        list(vec![Sym("__wl_done"), Num(0)]),
                        list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            tail,
                            res_e,
                        ]),
                    ]),
                ])
            } else {
                list(vec![Sym("begin"), core, tail])
            }
        }
        Statement::ForStatement(fr) => {
            let (has_exits, core) = lower_for_parts(fr)?;
            if has_exits {
                let res_e = exit_result_form(view);
                list(vec![
                    Sym("let"),
                    list(vec![
                        list(vec![Sym("__wl_done"), Num(0)]),
                        list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            tail,
                            res_e,
                        ]),
                    ]),
                ])
            } else {
                list(vec![Sym("begin"), core, tail])
            }
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
        // Tail assignment (`u.k = v;` as last statement, void fn): route
        // through the assignment form so member targets get the helpful
        // jsonSet message instead of "expression assignment not in M1".
        Statement::ExpressionStatement(e) => {
            if matches!(e.expression, Expression::AssignmentExpression(_)) {
                let v = lower_assign_form(match &e.expression {
                    Expression::AssignmentExpression(asg) => asg,
                    _ => unreachable!(),
                })?;
                return Ok(ensure_int_value(v));
            }
            Ok(ensure_int_value(lower_expr(&e.expression)?))
        }
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
        Statement::ForOfStatement(fo) => {
            let (has_exits, core) = lower_for_of_parts(fo)?;
            if !has_exits {
                return Ok(core);
            }
            Ok(list(vec![
                Sym("let"),
                list(vec![
                    list(vec![Sym("__wl_done"), Num(0)]),
                    list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                ]),
                list(vec![Sym("begin"), core, exit_result_form(view)]),
            ]))
        }
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
/// for (const x of xs) { ... } → (has_exits, core):
///   (let ((__of_a XS) (__of_i 0) (__of_n (vec-length __of_a)))
///     (while flag-cond (< __of_i __of_n)
///       (begin (let ((x (vec-nth __of_a __of_i))) BODY...)
///              (set! __of_i (+ __of_i 1)))))
/// Iterable must be an array value (M1: no string iteration — use strSplit
/// first). Body exits use the same flag protocol as while/for cores.
fn lower_for_of_parts(fo: &oxc_ast::ast::ForOfStatement<'_>) -> Result<(bool, LispVal), String> {
    let decl = match &fo.left {
        oxc_ast::ast::ForStatementLeft::VariableDeclaration(v) => v,
        _ => return Err("ts_frontend: for-of binding must be `const`/`let` declarations".into()),
    };
    if decl.declarations.len() != 1 {
        return Err("ts_frontend: for-of takes exactly one binding".into());
    }
    let name = binding_name(&decl.declarations[0].id)?;
    let arr_e = lower_expr(&fo.right)?;
    let body_stmts = stmts_of(&fo.body);
    let has_exits = stmts_have_exit(body_stmts);

    // body pieces (exit-aware, same shape as lower_for_parts)
    let mut body_items: Vec<LispVal> = vec![Sym("begin")];
    let mut seen_exit = false;
    for st in body_stmts {
        let piece = if has_exits {
            match st {
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
                        Num(0),
                    ])
                }
                other => {
                    let e = tail_stmt_as_expr(other)?;
                    if seen_exit {
                        // dead code after an exit — int-pad the branch
                        // (e may be set!/while-typed nil; nil ≠ int breaks
                        // the checker's branch unification)
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            list(vec![Sym("begin"), e, Num(0)]),
                            Num(0),
                        ])
                    } else {
                        e
                    }
                }
            }
        } else {
            tail_stmt_as_expr(st)?
        };
        if matches!(st, Statement::BreakStatement(_) | Statement::ReturnStatement(_)) {
            seen_exit = true;
        }
        body_items.push(piece);
    }
    body_items.push(list(vec![
        Sym("set!"),
        Sym("__of_i"),
        list(vec![Sym("+"), Sym("__of_i"), Num(1)]),
    ]));
    let body_e = if body_items.len() == 1 { Num(0) } else { list(body_items) };

    // per-iteration element binding wraps the body
    let body_bound = list(vec![
        Sym("let"),
        list(vec![list(vec![
            Sym(name),
            list(vec![Sym("vec-nth"), Sym("__of_a"), Sym("__of_i")]),
        ])]),
        body_e,
    ]);

    let test = list(vec![Sym("<"), Sym("__of_i"), Sym("__of_n")]);
    let cond_e = if has_exits {
        list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
            test,
            list(vec![Sym("="), Num(1), Num(0)]),
        ])
    } else {
        test
    };
    Ok((
        has_exits,
        list(vec![
            Sym("let*"),
            list(vec![
                list(vec![Sym("__of_a"), arr_e]),
                list(vec![Sym("__of_i"), Num(0)]),
                list(vec![Sym("__of_n"), list(vec![Sym("vec-length"), Sym("__of_a")])]),
            ]),
            list(vec![Sym("while"), cond_e, body_bound]),
        ]),
    ))
}

/// While statement → (has_exits, core form). Core assumes exit flags
/// are bound by the surrounding context when has_exits.
fn lower_while_parts(s: &Statement<'_>) -> Result<(bool, LispVal), String> {
    let Statement::WhileStatement(w) = s else {
        return Err("ts_frontend: internal: not a while".into());
    };
    lower_while_parts_core(w)
}

fn lower_while_parts_core(w: &oxc_ast::ast::WhileStatement<'_>) -> Result<(bool, LispVal), String> {
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
            return Ok((false, while_e));
        }
        let binds: Vec<LispVal> = hoisted
            .iter()
            .map(|(n, _)| list(vec![Sym(n.clone()), list(vec![Sym("quote"), LispVal::Nil])]))
            .collect();
        return Ok((false, list(vec![Sym("let"), list(binds), while_e])));
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
    // CORE: hoisted bindings + flag-guarded while. Flags themselves are
    // bound by the SURROUNDING context (mid-function continuation guard
    // or the value wrapper) — a return inside the loop must be visible
    // AFTER the loop, so the flags must outlive this let.
    let mut binds = Vec::new();
    for (n, _) in &hoisted {
        binds.push(list(vec![Sym(n.clone()), list(vec![Sym("quote"), LispVal::Nil])]));
    }
    let while_e = list(vec![Sym("while"), cond_e, body_e]);
    if binds.is_empty() {
        Ok((true, while_e))
    } else {
        Ok((true, list(vec![Sym("let"), list(binds), while_e])))
    }
}

/// While as a VALUE: binds the exit flags itself and yields __wl_res.
/// (Value position = nothing follows the loop, so local flags are fine.)
fn lower_while_value(w: &Statement<'_>) -> Result<LispVal, String> {
    let Statement::WhileStatement(w) = w else {
        return Err("ts_frontend: internal: not a while".into());
    };
    let (has_exits, core) = lower_while_parts_core(w)?;
    if !has_exits {
        return Ok(core);
    }
    Ok(list(vec![
        Sym("let"),
        list(vec![
            list(vec![Sym("__wl_done"), Num(0)]),
            list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
        ]),
        list(vec![Sym("begin"), core, Sym("__wl_res")]),
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
        Statement::ExpressionStatement(e) => Ok(ensure_int_value(effect_expr(&e.expression)?)),
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
        Statement::ForOfStatement(fo) => {
            let (has_exits, core) = lower_for_of_parts(fo)?;
            if !has_exits {
                return Ok(core);
            }
            Ok(list(vec![
                Sym("let"),
                list(vec![
                    list(vec![Sym("__wl_done"), Num(0)]),
                    list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                ]),
                list(vec![Sym("begin"), core, Sym("__wl_res")]),
            ]))
        }
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
/// For statement as a VALUE (tail position): binds exit flags, yields
/// __wl_res when the body can return/break.
fn lower_for(fr: &oxc_ast::ast::ForStatement<'_>) -> Result<LispVal, String> {
    let (has_exits, core) = lower_for_parts(fr)?;
    if !has_exits {
        return Ok(core);
    }
    Ok(list(vec![
        Sym("let"),
        list(vec![
            list(vec![Sym("__wl_done"), Num(0)]),
            list(vec![Sym("__wl_res"), list(vec![Sym("quote"), LispVal::Nil])]),
        ]),
        list(vec![Sym("begin"), core, Sym("__wl_res")]),
    ]))
}

/// For → (has_exits, core). Core = (let ((v init)...) (while cond body))
/// with flag-guarded cond when the body can exit; flags bound by caller.
fn lower_for_parts(fr: &oxc_ast::ast::ForStatement<'_>) -> Result<(bool, LispVal), String> {
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
    //
    // EXITS: `for` bodies with return/break write the __wl_done/__wl_res
    // flags — the while MUST then run flag-guarded and yield __wl_res,
    // exactly like while-loops (lower_while_value). Before 2026-08-30 the
    // raw while ignored the flags: the loop kept iterating past a `return`
    // and the function fell through to its trailing value (for+return
    // returned "fell-through" instead of "102" — found while building
    // for-of arrays).
    let body_stmts = stmts_of(&fr.body);
    let has_exits = stmts_have_exit(body_stmts);

    let mut body_items: Vec<LispVal> = vec![Sym("begin")];
    let mut seen_exit = false;
    for s in body_stmts {
        let piece = if has_exits {
            match s {
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
                        Num(0),
                    ])
                }
                other => {
                    let e = tail_stmt_as_expr(other)?;
                    if seen_exit {
                        // dead code after break/return in the same iteration
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
            }
        } else {
            tail_stmt_as_expr(s)?
        };
        if matches!(s, Statement::BreakStatement(_) | Statement::ReturnStatement(_)) {
            seen_exit = true;
        }
        body_items.push(piece);
    }
    // update clause runs after the body; guard it in exit mode so a
    // returned iteration doesn't keep mutating loop vars
    if let Some(u) = update_form {
        if has_exits {
            body_items.push(list(vec![
                Sym("if"),
                list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                list(vec![Sym("begin"), u, Num(0)]),
                Num(0),
            ]));
        } else {
            body_items.push(u);
        }
    }
    let begin_e = if body_items.len() == 1 {
        Num(0)
    } else {
        list(body_items)
    };

    if !has_exits {
        // (let ((v init)...) (begin (while cond body) 0))
        return Ok((
            false,
            list(vec![
                Sym("let"),
                list(bindings),
                list(vec![
                    Sym("begin"),
                    list(vec![Sym("while"), truthy(test)?, begin_e]),
                    Num(0),
                ]),
            ]),
        ));
    }
    // exit mode: flag-guarded condition; flags + __wl_res extraction are
    // the CALLER's job (continuation guard or value wrapper)
    let cond_e = list(vec![
        Sym("if"),
        list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
        truthy(test)?,
        list(vec![Sym("="), Num(1), Num(0)]),
    ]);
    Ok((
        true,
        list(vec![
            Sym("let"),
            list(bindings),
            list(vec![Sym("while"), cond_e, begin_e]),
        ]),
    ))
}

/// Mid-loop return value, surfaced after the loop. View exports must
/// json-wrap it like every other return path does.
fn exit_result_form(view: bool) -> LispVal {
    if view {
        list(vec![Sym("near/json_return_str"), Sym("__wl_res")])
    } else {
        Sym("__wl_res")
    }
}

/// Statement-position expression as a pure effect form.
/// Assignments (incl. element writes) and `i++`/`i--` become set!/vec-set!;
/// everything else lowers as a value expression.
fn effect_expr(e: &Expression<'_>) -> Result<LispVal, String> {
    match e {
        Expression::AssignmentExpression(asg) => lower_assign_form(asg),
        Expression::UpdateExpression(upd) => {
            let v = update_target_simple(&upd.argument)?;
            let one = if matches!(upd.operator, oxc_syntax::operator::UpdateOperator::Increment) { 1 } else { -1 };
            Ok(list(vec![
                Sym("set!"),
                Sym(v.clone()),
                list(vec![Sym("+"), Sym(v), Num(one)]),
            ]))
        }
        other => lower_expr(other),
    }
}

/// Assignment as an effect form. Plain vars → (set! v e); element writes
/// `xs[i] = e` → (vec-set! xs i e); compounds read via vec-nth.
fn lower_assign_form(
    asg: &oxc_ast::ast::AssignmentExpression<'_>,
) -> Result<LispVal, String> {
    use oxc_syntax::operator::AssignmentOperator;
    match &asg.left {
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) => {
            let (v, expr) = lower_assignment(asg)?;
            Ok(list(vec![Sym("set!"), Sym(v), expr]))
        }
        oxc_ast::ast::AssignmentTarget::StaticMemberExpression(_) => Err(
            "ts_frontend: property assignment not supported — objects are immutable JSON values; \
             rebuild with `o = jsonSet(o, \"key\", jsonQuote(v))` (numbers: toStr(v))"
                .into(),
        ),
        oxc_ast::ast::AssignmentTarget::ComputedMemberExpression(cm) => {
            let obj = lower_expr(&cm.object)?;
            let idx = lower_expr(&cm.expression)?;
            let rhs = lower_expr(&asg.right)?;
            let val = match asg.operator {
                AssignmentOperator::Assign => rhs,
                AssignmentOperator::Addition => list(vec![
                    Sym("+"),
                    list(vec![Sym("vec-nth"), obj.clone(), idx.clone()]),
                    rhs,
                ]),
                AssignmentOperator::Subtraction => list(vec![
                    Sym("-"),
                    list(vec![Sym("vec-nth"), obj.clone(), idx.clone()]),
                    rhs,
                ]),
                _ => return Err("ts_frontend: element writes support only = / += / -=".into()),
            };
            Ok(list(vec![Sym("vec-set!"), obj, idx, val]))
        }
        _ => return Err("ts_frontend: assignment target must be a variable or element access".into()),
    }
}

/// M2 objects: `{ k: v, ... }` → nested `(json-set "{}" "k" <encoded v>)`
/// folds. Objects are JSON-string values: storage/return/interop need no
/// conversion, reads go through near/json_get_str.
fn lower_object_literal(
    obj: &oxc_ast::ast::ObjectExpression<'_>,
) -> Result<LispVal, String> {
    let mut acc = Str("{}".to_string());
    for prop in &obj.properties {
        match prop {
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                let key = match &p.key {
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => {
                        id.name.as_str().to_string()
                    }
                    oxc_ast::ast::PropertyKey::StringLiteral(s) => s.value.as_str().to_string(),
                    _ => {
                        return Err(
                            "ts_frontend: object key must be an identifier or string literal"
                                .into(),
                        )
                    }
                };
                let val = encode_json_value(&p.value)?;
                acc = list(vec![Sym("json-set"), acc, Str(key), val]);
            }
            oxc_ast::ast::ObjectPropertyKind::SpreadProperty(_) => {
                return Err("ts_frontend: object spread not supported".into());
            }
        }
    }
    Ok(acc)
}

/// Encode a TS expression as a JSON VALUE expression for json-set's 3rd
/// arg (json-set takes already-encoded value text — string values keep
/// their quotes, numbers/bools are bare).
///
/// Statically-known shapes encode exactly: string/template → json-quote,
/// numeric literal → to-string, boolean literal → true/false, nested
/// object literal → recursion (its result IS encoded text). Everything
/// else: numberish-by-construction (arithmetic, Math.*, .length,
/// strToNum/strLength/jsonGetInt calls) → to-string; otherwise assume
/// string → json-quote.
fn encode_json_value(e: &Expression<'_>) -> Result<LispVal, String> {
    match e {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => {
            Ok(list(vec![Sym("json-quote"), lower_expr(e)?]))
        }
        Expression::NumericLiteral(_) => Ok(list(vec![Sym("to-string"), lower_expr(e)?])),
        Expression::BooleanLiteral(b) => {
            Ok(Str(if b.value { "true" } else { "false" }.to_string()))
        }
        Expression::ObjectExpression(_) => lower_expr(e), // already encoded
        _ => {
            // Object-typed param embedded as a value: its binding already
            // IS JSON text — embed raw (no quote, no to-string)
            if let Expression::Identifier(id) = e {
                if OBJ_PARAM_PROPS.with(|s| {
                    s.borrow().iter().any(|(n, _)| n == id.name.as_str())
                }) {
                    return lower_expr(e);
                }
            }
            // json-quote dynamic values. It's tag-aware at runtime
            // (interp + wasm): Str → escaped+quoted, Num → bare decimal
            // (valid JSON number), Bool → true/false. `: number` params
            // still encode bare via the numberish path (their tests pin
            // that shape); everything else quotes safely.
            if expr_is_numberish(e) {
                Ok(list(vec![Sym("to-string"), lower_expr(e)?]))
            } else {
                Ok(list(vec![Sym("json-quote"), lower_expr(e)?]))
            }
        }
    }
}

/// Numeric-by-construction expressions (no annotations in parse-only mode,
/// so classify by shape).
fn expr_is_numberish(e: &Expression<'_>) -> bool {
    match e {
        Expression::NumericLiteral(_) | Expression::BooleanLiteral(_) => true,
        // `: number`-annotated params (threaded through NUM_PARAM_NAMES
        // during body lowering) encode as bare numbers in object literals
        Expression::Identifier(id) => NUM_PARAM_NAMES
            .with(|s| s.borrow().iter().any(|n| n == id.name.as_str())),
        // A binary op is numberish only if BOTH sides are — `a + b` with
        // number params is arithmetic (bare), but `roster + "," + who` on
        // strings is concat and MUST json-quote (the multisig record
        // corruption, 2026-09-01: blanket `=> true` stored concat values
        // unquoted; the commas desynced the scanner, next set nuked keys).
        Expression::BinaryExpression(b) => {
            expr_is_numberish(&b.left) && expr_is_numberish(&b.right)
        }
        Expression::CallExpression(c) => match &c.callee {
            Expression::Identifier(id) => matches!(
                id.name.as_str(),
                "strToNum" | "strLength" | "strLen" | "jsonGetInt"
            ),
            Expression::StaticMemberExpression(sm) => {
                if let Expression::Identifier(id) = &sm.object {
                    matches!(id.name.as_str(), "Math" | "u128")
                        || sm.property.name.as_str() == "length"
                } else {
                    sm.property.name.as_str() == "length"
                }
            }
            _ => false,
        },
        Expression::StaticMemberExpression(sm) => sm.property.name.as_str() == "length",
        _ => false,
    }
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
        AssignmentOperator::Assign => {
            // Re-type the local when a stringy/numeric rhs overwrites it —
            // `let out = ""; out = 5;` makes `out + x` arithmetic again.
            let rhs_stringy = expr_is_stringy(&asg.right) || expr_is_str_method_call(&asg.right);
            if rhs_stringy {
                mark_string_local(&v);
            } else if matches!(&asg.right, Expression::NumericLiteral(_))
                && is_string_local(&v)
            {
                STRING_LOCALS.with(|s| s.borrow_mut().retain(|n| n != &v));
            }
            rhs
        }
        // `s += x`: string-VALUED rhs ⇒ str-cat, same rule as binary + (the
        // plain `+` path would emit num-only (+) — interp/wasm hard-error or
        // corrupt on str operands — surface tour 2, 2026-09-01). The lhs `v`
        // is by construction already a string here (it accumulated one), but
        // the DECIDER is the rhs shape, mirroring the binary `+` path below.
        // `+=` itself proves `v` stringy when `v` was already marked; when it
        // wasn't (first `s += "x"` after a stringy let), keep the rhs rule.
        AssignmentOperator::Addition => {
            let rhs_stringy = expr_is_stringy(&asg.right) || expr_is_str_method_call(&asg.right);
            if rhs_stringy || is_string_local(&v) {
                mark_string_local(&v);
                list(vec![Sym("str-cat"), Sym(v.clone()), rhs])
            } else {
                list(vec![Sym("+"), Sym(v.clone()), rhs])
            }
        }
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

/// Statically "stringy": string literal, template, to-string/toStr/strCat
/// calls, or another stringy concat. Conservative — misses non-literal strs
/// (typed only by annotation), which still hard-error at the checker.
fn expr_is_stringy(e: &Expression) -> bool {
    match e {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => true,
        Expression::BinaryExpression(b) => {
            b.operator == BinaryOperator::Addition
                && (expr_is_stringy(&b.left) || expr_is_stringy(&b.right))
        }
        Expression::CallExpression(c) => match &c.callee {
            Expression::Identifier(id) => {
                matches!(id.name.as_str(), "toStr" | "toString" | "strCat" | "to_string")
            }
            _ => false,
        },
        // strLength(s) / strLen(s) return int — numeric in + context. NOT
        // stringy: `strLength(a) + strLength(b)` is numeric addition
        // (hashTour, surface tour 2 exotic 2026-09-01). Adding these here
        // forced str-cat on int args → checker "num ≠ str".
        _ => false,
    }
}

/// String-method calls return str at runtime: S.slice/S.charAt/S.concat/
/// S.toUpperCase/… — any static member call is treated as string-valued for
/// `+` dispatch (strMethods surface tour 2, 2026-09-01). Conservative: only
/// method calls, not identifiers (those may be numbers).
fn expr_is_str_method_call(e: &Expression) -> bool {
    match e {
        Expression::CallExpression(c) => {
            matches!(&c.callee, Expression::StaticMemberExpression(_))
        }
        _ => false,
    }
}

/// Shared arrow lowering: params list + body value. Used by inline arrows
/// and by `export const f = (x) => ...` (which needs the body spliced into
/// a function-shaped define — `(define f (lambda ...))` exports compile to
/// a stub, only `(define (f x) body)` produces a real entry).
fn arrow_parts(a: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> Result<(Vec<LispVal>, LispVal), String> {
    let mut params: Vec<LispVal> = Vec::new();
    for p in &a.params.items {
        params.push(Sym(binding_name(&p.pattern)?));
    }
    let body_val: LispVal = if let Some(e) = a.get_expression() {
        lower_expr(e)?
    } else if let Some(fb) = a.get_function_body() {
        match fb.statements.as_slice() {
            [] => return Err("ts_frontend: empty arrow body not in M1".into()),
            [Statement::ExpressionStatement(es)] => lower_expr(&es.expression)?,
            [Statement::ReturnStatement(r)] => match r.argument.as_ref() {
                Some(e) => lower_expr(e)?,
                None => return Err("ts_frontend: bare return in arrow not in M1".into()),
            },
            stmts => lower_block_tail(stmts, false)?,
        }
    } else {
        return Err("ts_frontend: empty arrow body not in M1".into());
    };
    Ok((params, body_val))
}

/// `export const f = (params) => body` → function-shaped define + export.
fn lower_exported_arrow(
    name: &str,
    a: &oxc_ast::ast::ArrowFunctionExpression<'_>,
) -> Result<(LispVal, LispVal), String> {
    let (params, body) = arrow_parts(a)?;
    let define = list(vec![
        Sym("define"),
        list({
            let mut d = vec![Sym(name.to_string())];
            d.extend(params);
            d
        }),
        body,
    ]);
    // mirror lower_function's view convention (get_* → view)
    let view = name.starts_with("get_");
    let export_name = if name == "new_" { "new".to_string() } else { name.to_string() };
    let export = list(vec![
        Sym("export"),
        Str(export_name),
        Sym(name.to_string()),
        if view { Sym("#t") } else { Sym("#f") },
    ]);
    Ok((define, export))
}

/// true when the expression is bigint-shaped: `10n` literal, a bigint-typed
/// param reference, a u128Xxx(...) call result, or nested bigint arithmetic.
/// Register `rec.field` pairs as bigint-typed when the let's default is a
/// JSON shape literal with quoted-numeric field defaults: `?? '{"amt":"0"}'`
/// ⇒ rec.amt is bigint-shaped. Called from every bigint-let scan site.
fn register_shape_fields(d: &VariableDeclarator<'_>, init: &Expression<'_>) {
    // unwrap parens / ?? chains to the rightmost default
    let mut e = init;
    loop {
        match e {
            Expression::ParenthesizedExpression(pe) => e = &pe.expression,
            Expression::LogicalExpression(l)
                if l.operator == LogicalOperator::Coalesce =>
            {
                e = &l.right
            }
            _ => break,
        }
    }
    let Expression::StringLiteral(sl) = e else { return };
    let Ok(name) = binding_name(&d.id) else { return };
    for field in shape_bigint_fields(&sl.value) {
        SHAPE_BIGINT_FIELDS.with(|m| m.borrow_mut().push((name.clone(), field)));
    }
}

/// Extract keys whose values are quoted numeric strings: `"amt":"0"` → amt.
/// Empty strings and non-numeric values are excluded (those fields are
/// genuinely string-typed: state, owner, hash…).
fn shape_bigint_fields(lit: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = lit.trim().trim_start_matches('{').trim_end_matches('}');
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        let Some(k_end) = rest.find('"') else { break };
        let key = &rest[..k_end];
        rest = &rest[k_end + 1..];
        let Some(c_end) = rest.find(':') else { break };
        rest = &rest[c_end + 1..];
        let v = rest.trim_start();
        if let Some(stripped) = v.strip_prefix('"') {
            if let Some(v_end) = stripped.find('"') {
                let val = &stripped[..v_end];
                if !val.is_empty() && val.bytes().all(|b| b.is_ascii_digit()) {
                    out.push(key.to_string());
                }
                rest = &stripped[v_end + 1..];
                continue;
            }
        }
        // unquoted or missing value: skip to next comma
        match rest.find(',') {
            Some(c) => rest = &rest[c + 1..],
            None => break,
        }
    }
    out
}

/// String-typed local in the CURRENT function body (STRING_LOCALS).
fn is_string_local(n: &str) -> bool {
    STRING_LOCALS.with(|s| s.borrow().iter().any(|x| x == n))
}

/// CERTAINTY check for template interpolation: does this expression lower to
/// a string without needing the defensive (to-string …) wrap? Whitelist-only
/// — every false answer keeps today's correct-but-costly wrap, so unknown
/// shapes can never regress the int-arg-renders-empty protection.
/// (2026-09-02: the wrap costs a constant ~622 emitted instructions per
/// interpolation — probes hand_str h2−h1 and hand_let h4−h5; nostr-gov
/// carried 131 wraps. Trust basis = the same predicates `+` dispatch
/// already uses for str-cat vs num-add, plus STRING_LOCALS/CONST_FOLDS.)
fn interp_arg_is_string(e: &Expression<'_>) -> bool {
    match e {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => true,
        Expression::Identifier(id) => {
            is_string_local(id.name.as_str())
                || BIGINT_LOCALS.with(|m| m.borrow().iter().any(|x| *x == id.name.as_str()))
                || CONST_FOLDS.with(|m| {
                    m.borrow()
                        .iter()
                        .any(|(k, v)| k == id.name.as_str() && matches!(v, LispVal::Str(_)))
                })
        }
        Expression::BinaryExpression(b) => {
            if b.operator == BinaryOperator::Addition
                && (interp_arg_is_string(&b.left) || interp_arg_is_string(&b.right))
            {
                return true;
            }
            // u128 arithmetic family: results are decimal STRINGS in this
            // ABI — the checker types u128/* as str (str-cat accepts a raw
            // (u128/add …) operand; the ft suite asserted that exact IR).
            // Certainty needs BOTH sides bigint-shaped; comparisons
            // (lt/gt/…) emit ints and stay excluded.
            matches!(
                b.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
            ) && expr_is_bigint(&b.left)
                && expr_is_bigint(&b.right)
        }
        // parens hide the inner expression — look through them
        // (expr_is_bigint does the same; `"x" + (a + b)` lands here)
        Expression::ParenthesizedExpression(pe) => interp_arg_is_string(&pe.expression),
        Expression::CallExpression(c) => {
            if let Expression::Identifier(id) = &c.callee {
                // u128 ops return DECIMAL STRINGS at runtime (bigint surface
                // convention) — str-cat takes them raw; wrapping in to-string
                // would corrupt the exact-IR contract (FT supply test).
                if id.name.as_str().starts_with("u128/") {
                    return true;
                }
                match id.name.as_str() {
                    // explicit converters + the string-returning json getter
                    "toStr" | "toString" | "strCat" | "jsonGetStr" => return true,
                    _ => {}
                }
            }
            // S.slice/S.charAt/S.concat/… return str — but ONLY the
            // string-returning ones: indexOf/charCodeAt/codePointAt/
            // lastIndexOf/search return NUMBERS, and a bare num inside
            // (str …) renders empty (the quirk the to-string wrap exists
            // to shield). Whitelist strictly (tour2 indexOf, 2026-09-02).
            expr_returns_str_method(e)
        }
        _ => false,
    }
}

/// str-cat operand lowering: operands not PROVABLY strings get wrapped in
/// (to-string …) — the checker rejects raw nums in str-cat even though the
/// variadic emitter coerces at runtime (2026-09-02: `s + x` with a string
/// param s reached str-cat with a bare number and failed type_check).
fn lower_strcat_operand(e: &Expression<'_>) -> Result<LispVal, String> {
    if interp_arg_is_string(e) {
        lower_expr(e)
    } else {
        Ok(list(vec![Sym("to-string"), lower_expr(e)?]))
    }
}

/// Strictly string-RETURNING string methods (safe to skip the to-string
/// wrap). Complement of expr_is_str_method_call, which is dispatch-trust
/// only (any method call makes `+` concat) and includes number-returning
/// members like indexOf.
fn expr_returns_str_method(e: &Expression<'_>) -> bool {
    match e {
        Expression::CallExpression(c) => {
            if let Expression::StaticMemberExpression(m) = &c.callee {
                let prop = m.property.name.as_str();
                return matches!(
                    prop,
                    "slice" | "substring" | "substr" | "charAt" | "concat"
                        | "toUpperCase" | "toLowerCase" | "trim"
                        | "trimStart" | "trimEnd" | "repeat"
                        | "padStart" | "padEnd" | "at" | "toString"
                );
            }
            false
        }
        _ => false,
    }
}

fn mark_string_local(n: &str) {
    STRING_LOCALS.with(|s| {
        let mut b = s.borrow_mut();
        if !b.iter().any(|x| x == n) {
            b.push(n.to_string());
        }
    });
}

fn expr_is_bigint(e: &Expression<'_>) -> bool {
    match e {
        Expression::BigIntLiteral(_) => true,
        // `x ?? 0n` — the DEFAULT defines the miss-type: storageGet
        // results are decimal strings in this ABI, so a bigint default
        // makes the whole local bigint-shaped. (HTLC 2026-09-01:
        // `bal + rec.amt` needed this; bal was `storageGet(...) ?? 0n`.)
        Expression::LogicalExpression(l)
            if l.operator == LogicalOperator::Coalesce =>
        {
            expr_is_bigint(&l.right)
        }
        // `rec.amt` where rec's shape literal has a quoted-numeric default
        Expression::StaticMemberExpression(sm) => {
            let Expression::Identifier(base) = &sm.object else { return false };
            let field = sm.property.name.as_str();
            SHAPE_BIGINT_FIELDS
                .with(|s| s.borrow().iter().any(|(b, f)| b == base.name.as_str() && f == field))
        }
        Expression::Identifier(id) => {
            let n = id.name.as_str();
            BIGINT_NAMES.with(|s| s.borrow().iter().any(|x| x == n))
                || BIGINT_LOCALS.with(|s| s.borrow().iter().any(|x| x == n))
                || BIGINT_CONSTS.with(|s| s.borrow().iter().any(|x| x == n))
        }
        // `(a * b) / c` — parens hide the inner binary from detection
        Expression::ParenthesizedExpression(pe) => expr_is_bigint(&pe.expression),
        Expression::CallExpression(c) => {
            if let Expression::Identifier(id) = &c.callee {
                matches!(
                    id.name.as_str(),
                    "u128Add"
                        | "u128Sub"
                        | "u128Mul"
                        | "u128Div"
                        | "u128Mod"
                        | "u128FromNum"
                )
            } else {
                false
            }
        }
        Expression::BinaryExpression(b) => {
            matches!(
                b.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
            ) && (expr_is_bigint(&b.left) || expr_is_bigint(&b.right))
        }
        _ => false,
    }
}

fn lower_expr(e: &Expression<'_>) -> Result<LispVal, String> {
    match e {
        Expression::NumericLiteral(n) => Ok(Num(n.value as i64)),
        Expression::StringLiteral(s) => Ok(Str(s.value.as_str().to_string())),
        Expression::BooleanLiteral(b) => Ok(Num(if b.value { 1 } else { 0 })),
        Expression::NullLiteral(_) => Ok(LispVal::Nil),
                // u128-style digits-as-string. oxc raw for `1000n` is "1000n" —
        // strip the suffix: a stray 'n' would trap the u128/* parsers.
        Expression::BigIntLiteral(b) => Ok(Str(
            b.raw
                .as_ref()
                .map(|s| s.as_str().trim_end_matches('n').to_string())
                .unwrap_or_default(),
        )),
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
                    // int-arg renders-empty quirk. SKIP the wrap when the
                    // expression is CERTAIN to lower to a string (same
                    // trust basis `+` uses for str-cat dispatch — see
                    // interp_arg_is_string): the wrap costs ~622 emitted
                    // instructions per interpolation (2026-09-02 probes).
                    if interp_arg_is_string(&t.expressions[i]) {
                        parts.push(lower_expr(&t.expressions[i])?);
                    } else {
                        parts.push(list(vec![Sym("to-string"), lower_expr(&t.expressions[i])?]));
                    }
                }
            }
            if parts.is_empty() {
                return Ok(Str(String::new()));
            }
            let mut items = vec![Sym("str")];
            items.extend(parts);
            Ok(list(items))
        }
        Expression::Identifier(id) => {
            note_ident(&id.name, id.span.start);
            // top-level const substitution (literals only)
            if let Some(v) = CONST_FOLDS.with(|m| {
                m.borrow()
                    .iter()
                    .find(|(k, _)| k == id.name.as_str())
                    .map(|(_, v)| v.clone())
            }) {
                return Ok(v);
            }
            Ok(Sym(id.name.as_str().to_string()))
        }
        Expression::ArrayExpression(a) => {
            // [e0, e1, ...] → (array e0 e1 ...) — TAG_ARRAY heap block
            let mut items = vec![Sym("array")];
            for el in &a.elements {
                match el {
                    oxc_ast::ast::ArrayExpressionElement::SpreadElement(_) => {
                        return Err("ts_frontend: spread in array literal not in M1".into())
                    }
                    oxc_ast::ast::ArrayExpressionElement::Elision(_) => {
                        return Err("ts_frontend: holes in array literal not in M1".into())
                    }
                    other => items.push(lower_expr(other.as_expression().ok_or(
                        "ts_frontend: unsupported array element (M1)",
                    )?)?),
                }
            }
            Ok(list(items))
        }
        Expression::ComputedMemberExpression(m) => {
            // xs[i] → (vec-nth xs i) — nil on out-of-bounds (same as wasm)
            Ok(list(vec![
                Sym("vec-nth"),
                lower_expr(&m.object)?,
                lower_expr(&m.expression)?,
            ]))
        }
        Expression::StaticMemberExpression(sm) => {
            // Value-position member reads.
            // `.length` on arrays → vec-length (string length stays
            // strLength(s) — .length is ARRAY-typed).
            // M2 objects: any other `.key` is a property read on a
            // JSON-string object → (json-get-str "key" obj). The scanner
            // takes DOT PATHS natively, so o.a.b folds into one call
            // (json-get-str "a.b" o) — no per-hop temp binding. Absent
            // keys read as "" (json-get-str's missing-key contract).
            match sm.property.name.as_str() {
                "length" => Ok(list(vec![Sym("vec-length"), lower_expr(&sm.object)?])),
                prop => {
                    // fold the static-member chain into a dot path
                    let mut path = vec![prop.to_string()];
                    let mut base = &sm.object;
                    loop {
                        match base {
                            Expression::StaticMemberExpression(inner) => {
                                path.push(inner.property.name.as_str().to_string());
                                base = &inner.object;
                            }
                            _ => break,
                        }
                    }
                    if let Expression::Identifier(id) = base {
                        if matches!(
                            id.name.as_str(),
                            "near" | "storage" | "u128" | "console" | "Math" | "JSON"
                        ) {
                            return Err(format!(
                                "ts_frontend: `{}.{}` — namespaces are not values; call it",
                                id.name,
                                path.last().unwrap()
                            ));
                        }
                    }
                    let recv = lower_expr(base)?;
                    // Object-param numeric prop: `user.votes` where the
                    // annotation says number → auto str->num decode
                    if let (Expression::Identifier(id), 1) = (base, path.len()) {
                        if let Some(is_num) = OBJ_PARAM_PROPS.with(|s| {
                            s.borrow()
                                .iter()
                                .find(|(n, _)| n == id.name.as_str())
                                .and_then(|(_, props)| {
                                    props
                                        .iter()
                                        .find(|(k, _)| k.as_str() == path[0].as_str())
                                        .map(|(_, num)| *num)
                                })
                        }) {
                            if is_num {
                                return Ok(list(vec![
                                    Sym("str->num"),
                                    list(vec![
                                        Sym("json-get-str"),
                                        Str(path[0].clone()),
                                        recv,
                                    ]),
                                ]));
                            }
                        }
                    }
                    let dotted = path.iter().rev().cloned().collect::<Vec<_>>().join(".");
                    Ok(list(vec![Sym("json-get-str"), Str(dotted), recv]))
                }
            }
        }
        Expression::ObjectExpression(obj) => lower_object_literal(obj),
        Expression::BinaryExpression(b) => {
            // bigint operators (2026-08-31): either side bigint-shaped
            // (`10n` literal, bigint param, u128Xxx(...) result, nested
            // bigint arithmetic) ⇒ lower to the u128/* string family —
            // i64 math silently truncates yocto-scale amounts.
            if expr_is_bigint(&b.left) || expr_is_bigint(&b.right) {
                // Mixed `+` with a NON-NUMERIC string literal is
                // concatenation, not arithmetic (2026-09-01, found via the
                // FT contract): `"supply:" + (supply + amount)` lowered to
                // (u128/add "supply:" …) which traps parsing "supply:".
                // u128 values are decimal strings at runtime — str-cat is
                // the correct join. Numeric-only literals keep u128/add
                // (they mean real arithmetic).
                if b.operator == BinaryOperator::Addition {
                    // String-VALUED operands concat — not just direct
                    // literals. `("a" + user) + "b" + 5n` lowers left-assoc:
                    // the outer + sees a BinaryExpression on the left, which
                    // the old literal-only check missed → u128/add on
                    // "a…b" → parse trap (found via cross-contract vault,
                    // 2026-09-01). Only a PURELY numeric string literal
                    // still means arithmetic.
                    fn stringy_nonnumeric(e: &Expression) -> bool {
                        match e {
                            Expression::StringLiteral(sl) => {
                                // "" means CONCAT (found via portfolio's
                                // `(x ?? 0n) + ""` — empty string passed the
                                // all-digits test → u128/add(x, "") → parse
                                // trap. An empty string is never arithmetic.)
                                sl.value.is_empty()
                                    || sl.value.bytes().any(|b| !b.is_ascii_digit())
                            }
                            Expression::TemplateLiteral(_) => true,
                            Expression::BinaryExpression(be) => {
                                be.operator == BinaryOperator::Addition
                                    && (stringy_nonnumeric(&be.left)
                                        || stringy_nonnumeric(&be.right)
                                        || expr_is_stringy(&be.left)
                                        || expr_is_stringy(&be.right))
                            }
                            Expression::CallExpression(c) => match &c.callee {
                                Expression::Identifier(id) => matches!(
                                    id.name.as_str(),
                                    "toStr" | "toString" | "strCat" | "to_string"
                                ),
                                _ => false,
                            },
                            _ => false,
                        }
                    }
                    if stringy_nonnumeric(&b.left) || stringy_nonnumeric(&b.right) {
                        // str-cat dispatch: wrap operands that are NOT
                        // provably strings in (to-string …) — the checker
                        // rejects raw nums in str-cat (2026-09-02: string
                        // PARAMS became stringy via mark_string_local, so
                        // `s + x` reached str-cat with a bare number and
                        // failed type_check "args must all be str"; the
                        // variadic emitter coerces at runtime but the
                        // checker is stricter — make both sides provably
                        // str).
                        let l = lower_strcat_operand(&b.left)?;
                        let r = lower_strcat_operand(&b.right)?;
                        return Ok(list(vec![Sym("str-cat"), l, r]));
                    }
                }
                let uop: Option<&str> = match b.operator {
                    BinaryOperator::Addition => Some("u128/add"),
                    BinaryOperator::Subtraction => Some("u128/sub"),
                    BinaryOperator::Multiplication => Some("u128/mul"),
                    BinaryOperator::Division => Some("u128/div"),
                    BinaryOperator::Remainder => Some("u128/mod"),
                    BinaryOperator::LessThan => Some("u128/lt"),
                    BinaryOperator::GreaterThan => Some("u128/gt"),
                    _ => None,
                };
                let l = lower_expr(&b.left)?;
                let r = lower_expr(&b.right)?;
                if let Some(uop) = uop {
                    return Ok(list(vec![Sym(uop), l, r]));
                }
                match b.operator {
                    // NOTE (2026-09-01): a <= b is NOT (b > a) in u128 land
                    // when a == b — strict ops lose the boundary. Lower to
                    // negated strict: l <= r ≡ NOT(l > r), l >= r ≡ NOT(l < r),
                    // l != r ≡ NOT(l = r). `not` — NOT `(= 0 …)`: u128
                    // comparisons are bool-typed in the checker, and a
                    // num-typed 0 against bool is a type error (the old `!=`
                    // lowering had this latent bug, never exercised).
                    // Caught by lending v4's liquidation guard firing at
                    // exactly health == LIQ_LINE.
                    BinaryOperator::LessEqualThan => {
                        return Ok(list(vec![Sym("not"), list(vec![Sym("u128/gt"), l, r])]))
                    }
                    BinaryOperator::GreaterEqualThan => {
                        return Ok(list(vec![Sym("not"), list(vec![Sym("u128/lt"), l, r])]))
                    }
                    BinaryOperator::Equality | BinaryOperator::StrictEquality => {
                        return Ok(list(vec![Sym("u128/eq"), l, r]))
                    }
                    BinaryOperator::Inequality | BinaryOperator::StrictInequality => {
                        return Ok(list(vec![
                            Sym("not"),
                            list(vec![Sym("u128/eq"), l, r]),
                        ]))
                    }
                    _ => {
                        return Err(format!(
                            "ts_frontend: operator {:?} not supported on bigint operands",
                            b.operator
                        ))
                    }
                }
            }
            // stringy +: fold into nested binary str-cat (checker's + is num-only;
            // any string literal / template operand ⇒ concat semantics)
            if b.operator == BinaryOperator::Addition
                && (expr_is_stringy(&b.left) || expr_is_stringy(&b.right))
            {
                let l = lower_strcat_operand(&b.left)?;
                let r = lower_strcat_operand(&b.right)?;
                return Ok(list(vec![Sym("str-cat"), l, r]));
            }
            // String-METHOD receivers concat too: `acc + S.slice(...)`,
            // `acc + S.charAt(1)` — the callee side is a str-returning method
            // call even though expr_is_stringy can't see it statically
            // (surface tour 2 strMethods, 2026-09-01). Without this the + went
            // to num-add → checker "num ≠ str".
            if b.operator == BinaryOperator::Addition
                && (expr_is_str_method_call(&b.left) || expr_is_str_method_call(&b.right))
            {
                let l = lower_strcat_operand(&b.left)?;
                let r = lower_strcat_operand(&b.right)?;
                return Ok(list(vec![Sym("str-cat"), l, r]));
            }
            // String-TYPED LOCAL operands concat: `out + x` where `out` was
            // seeded by `let out = ""` (or any stringy init) — neither operand
            // is a literal, so the checks above can't see it. The interp's `+`
            // hard-errors on str operands and the wasm emitter's tagged add
            // silently corrupts them, so this MUST lower to str-cat (surface
            // tour 2 for-of accumulator, 2026-09-01).
            if b.operator == BinaryOperator::Addition {
                let side_is_string_local = |e: &Expression| {
                    matches!(e, Expression::Identifier(id) if is_string_local(id.name.as_str()))
                };
                if side_is_string_local(&b.left) || side_is_string_local(&b.right) {
                    let l = lower_strcat_operand(&b.left)?;
                    let r = lower_strcat_operand(&b.right)?;
                    return Ok(list(vec![Sym("str-cat"), l, r]));
                }
            }
            // `%`: JS truncated remainder (sign follows dividend: -7%2=-1).
            // The lisp `mod` builtin is EUCLIDEAN (always >= 0) — mapping
            // % to it silently returned wrong signs for negative operands.
            // Exact JS semantics via existing truncated ops: a - b*(a/b).
            if b.operator == BinaryOperator::Remainder {
                let a = lower_expr(&b.left)?;
                let bsym = lower_expr(&b.right)?;
                return Ok(list(vec![
                    Sym("-"),
                    a.clone(),
                    list(vec![
                        Sym("*"),
                        bsym.clone(),
                        list(vec![Sym("/"), a, bsym]),
                    ]),
                ]));
            }
            let op: &str = match b.operator {
                BinaryOperator::Addition => "+",
                BinaryOperator::Subtraction => "-",
                BinaryOperator::Multiplication => "*",
                BinaryOperator::Division => "/",
                // (2026-08-31) `%` was emitted as a lisp `%` — undefined in
                // the checker/interp/emitter (the builtin is `mod`); every
                // TS modulo failed to compile.
                BinaryOperator::Remainder => "mod",
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
            // ── JS std shims (2026-08-30): console.log / Math / JSON ──
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                if let Expression::Identifier(oid) = &sm.object {
                    match (oid.name.as_str(), sm.property.name.as_str()) {
                        ("console", "log") => {
                            if c.arguments.is_empty() {
                                return Ok(list(vec![
                                    Sym("near/log"),
                                    LispVal::Str(String::new()),
                                ]));
                            }
                            // checker types str-cat as BINARY — fold args with
                            // space separators into nested (str-cat a b) forms
                            let mut acc: Option<LispVal> = None;
                            for (idx, a) in c.arguments.iter().enumerate() {
                                if let Argument::SpreadElement(_) = a {
                                    return Err("ts_frontend: spread not in M1".into());
                                }
                                let e2 = a.as_expression()
                                    .ok_or("ts_frontend: unsupported console.log argument (M1)")?;
                                let piece = list(vec![Sym("to-string"), lower_expr(e2)?]);
                                acc = Some(match acc {
                                    None => piece,
                                    Some(prev) => list(vec![
                                        Sym("str-cat"),
                                        list(vec![Sym("str-cat"), prev, LispVal::Str(" ".into())]),
                                        piece,
                                    ]),
                                });
                                let _ = idx;
                            }
                            let joined = acc.unwrap_or(LispVal::Str(String::new()));
                            return Ok(list(vec![Sym("near/log"), joined]));
                        }
                        ("Math", "abs") | ("Math", "max") | ("Math", "min") => {
                            let op = sm.property.name.as_str();
                            if c.arguments.is_empty() {
                                return Err(format!(
                                    "ts_frontend: Math.{} needs at least one argument",
                                    op
                                ));
                            }
                            let mut items = vec![Sym(op)];
                            for a in &c.arguments {
                                let e2 = a.as_expression()
                                    .ok_or("ts_frontend: unsupported Math argument (M1)")?;
                                items.push(lower_expr(e2)?);
                            }
                            return Ok(list(items));
                        }
                        ("JSON", "stringify") => {
                            if c.arguments.len() != 1 {
                                return Err("ts_frontend: JSON.stringify takes exactly one value (M1)".into());
                            }
                            let e2 = c.arguments[0].as_expression()
                                .ok_or("ts_frontend: unsupported JSON.stringify argument (M1)")?;
                            return Ok(list(vec![Sym("json-quote"), lower_expr(e2)?]));
                        }
                        ("JSON", "stringifyArr") => {
                            if c.arguments.len() != 1 {
                                return Err("ts_frontend: JSON.stringifyArr takes exactly one array (M1)".into());
                            }
                            let e2 = c.arguments[0].as_expression()
                                .ok_or("ts_frontend: unsupported JSON.stringifyArr argument (M1)")?;
                            // "[" + join(",", map(json-quote, arr)) + "]" —
                            // nested binary str-cat (checker constraint)
                            return Ok(list(vec![
                                Sym("str-cat"),
                                list(vec![
                                    Sym("str-cat"),
                                    LispVal::Str("[".into()),
                                    list(vec![
                                        Sym("str-join"),
                                        LispVal::Str(",".into()),
                                        list(vec![
                                            Sym("map"),
                                            list(vec![
                                                Sym("lambda"),
                                                list(vec![Sym("__jv")]),
                                                list(vec![Sym("json-quote"), Sym("__jv")]),
                                            ]),
                                            lower_expr(e2)?,
                                        ]),
                                    ]),
                                ]),
                                LispVal::Str("]".into()),
                            ]));
                        }
                        ("JSON", "parse") => {
                            return Err(
                                "ts_frontend: JSON.parse not needed — tx args arrive parsed (use near.jsonGet(key) / typed params)"
                                    .into(),
                            );
                        }
                        _ => {} // fall through
                    }
                }
            }
            // ── string instance methods (M2): receiver prepended ──
            // s.startsWith(x) → (str-starts-with s x), etc.
            // Note: string-typed only — the checker rejects wrong arg types
            // (str-index-of on an array errors loudly rather than guessing).
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                let prop = sm.property.name.as_str();
                let is_str_method = matches!(
                    prop,
                    "slice"
                        | "startsWith"
                        | "endsWith"
                        | "indexOf"
                        | "includes"
                        | "charAt"
                        | "trim"
                        | "toUpperCase"
                        | "toLowerCase"
                        | "concat"
                        | "split"
                );
                if is_str_method {
                    let recv = lower_expr(&sm.object)?;
                    let arg = |i: usize| -> Result<LispVal, String> {
                        c.arguments
                            .get(i)
                            .and_then(|a| a.as_expression())
                            .map(lower_expr)
                            .transpose()?
                            .ok_or_else(|| {
                                format!("ts_frontend: .{} needs argument {}", prop, i + 1)
                            })
                    };
                    let argc = c.arguments.len();
                    return match prop {
                        "slice" => {
                            let start = arg(0)?;
                            let end = if argc >= 2 {
                                arg(1)?
                            } else {
                                // JS s.slice(i) = to end
                                list(vec![Sym("str-length"), recv.clone()])
                            };
                            Ok(list(vec![Sym("str-slice"), recv, start, end]))
                        }
                        "startsWith" => Ok(list(vec![Sym("str-starts-with"), recv, arg(0)?])),
                        "endsWith" => Ok(list(vec![Sym("str-ends-with"), recv, arg(0)?])),
                        "indexOf" => Ok(list(vec![Sym("str-index-of"), recv, arg(0)?])),
                        "includes" => Ok(list(vec![Sym("str-contains"), recv, arg(0)?])),
                        "charAt" => {
                            let i = arg(0)?;
                            Ok(list(vec![
                                Sym("str-slice"),
                                recv,
                                i.clone(),
                                list(vec![Sym("+"), i, Num(1)]),
                            ]))
                        }
                        "trim" => Ok(list(vec![Sym("str-trim"), recv])),
                        "toUpperCase" => Ok(list(vec![Sym("str-upcase"), recv])),
                        "toLowerCase" => Ok(list(vec![Sym("str-downcase"), recv])),
                        "concat" => {
                            if argc != 1 {
                                return Err("ts_frontend: .concat takes exactly one argument (binary str-cat)".into());
                            }
                            Ok(list(vec![Sym("str-cat"), recv, arg(0)?]))
                        }
                        "split" => Ok(list(vec![Sym("str-split"), recv, arg(0)?])),
                        _ => unreachable!(),
                    };
                }
            }
            // Array method calls first: xs.push(v) → (vec-push xs v),
            // xs.join(sep) → (str-join sep xs) — note the arg reordering.
            // Pipeline members (join/map/filter/reduce, 2026-08-30) accept
            // ARBITRARY receivers — xs.filter(f).map(g).join(",") stacks —
            // push stays identifier-only (it rebinds via set!).
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                match sm.property.name.as_str() {
                    "join" => {
                        if c.arguments.len() != 1 {
                            return Err("ts_frontend: join takes exactly one separator".into());
                        }
                        let e2 = c.arguments[0]
                            .as_expression()
                            .ok_or("ts_frontend: unsupported join argument (M1)")?;
                        return Ok(list(vec![
                            Sym("str-join"),
                            lower_expr(e2)?,
                            lower_expr(&sm.object)?,
                        ]));
                    }
                    "map" | "filter" => {
                        let op = sm.property.name.as_str();
                        if c.arguments.len() != 1 {
                            return Err(format!(
                                "ts_frontend: {} takes exactly one callback",
                                op
                            ));
                        }
                        let cb = c.arguments[0]
                            .as_expression()
                            .ok_or("ts_frontend: unsupported callback (M1)")?;
                        if !matches!(cb, Expression::ArrowFunctionExpression(_)) {
                            return Err(format!(
                                "ts_frontend: .{} callback must be an arrow function (M1)",
                                op
                            ));
                        }
                        return Ok(list(vec![
                            Sym(op),
                            lower_expr(cb)?,
                            lower_expr(&sm.object)?,
                        ]));
                    }
                    "reduce" => {
                        if c.arguments.len() != 2 {
                            return Err(
                                "ts_frontend: reduce takes a callback and an initial value"
                                    .into(),
                            );
                        }
                        let cb = c.arguments[0]
                            .as_expression()
                            .ok_or("ts_frontend: unsupported reduce callback (M1)")?;
                        if !matches!(cb, Expression::ArrowFunctionExpression(_)) {
                            return Err(
                                "ts_frontend: .reduce callback must be an arrow function (M1)"
                                    .into(),
                            );
                        }
                        let init = c.arguments[1]
                            .as_expression()
                            .ok_or("ts_frontend: unsupported reduce initial value (M1)")?;
                        return Ok(list(vec![
                            Sym("reduce"),
                            lower_expr(cb)?,
                            lower_expr(init)?,
                            lower_expr(&sm.object)?,
                        ]));
                    }
                    _ => {} // fall through
                }
                if matches!(&sm.object, Expression::Identifier(_)) {
                    match sm.property.name.as_str() {
                        "push" => {
                            if c.arguments.len() != 1 {
                                return Err("ts_frontend: push takes exactly one value".into());
                            }
                            let e2 = c.arguments[0]
                                .as_expression()
                                .ok_or("ts_frontend: unsupported push argument (M1)")?;
                            // vec-push is FUNCTIONAL (allocates + returns a
                            // new array) — JS-style mutation needs a rebind:
                            // xs.push(v) → (set! xs (vec-push xs v)).
                            // Statement position discards the set! value.
                            if let Expression::Identifier(id) = &sm.object {
                                let name = id.name.as_str();
                                return Ok(list(vec![
                                    Sym("set!"),
                                    Sym(name),
                                    list(vec![
                                        Sym("vec-push"),
                                        Sym(name),
                                        lower_expr(e2)?,
                                    ]),
                                ]));
                            }
                            return Err(
                                "ts_frontend: push target must be a plain variable".into(),
                            );
                        }
                        _ => {} // push handled; pipeline members were matched above
                        _ => {} // fall through to the generic path
                    }
                }
            }
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
            // json-set's 3rd arg is JSON-ENCODED value text — but TS users
            // pass raw values (escrow stored UNQUOTED strings: invalid JSON
            // that only wasm's tolerant scanner could read back, and any
            // embedded quote/brace corrupted the record — the multisig
            // protocol lost a field entirely. 2026-09-01). SELF-ENCODE the
            // value argument: strings → json-quote, numbers → to-string,
            // pre-encoded (jsonQuote(...) / object-literal) results pass
            // through unchanged.
            if head == "json-set" && items.len() == 4 {
                let key = items[2].clone();
                let raw = c.arguments.get(2)
                    .and_then(|a| a.as_expression())
                    .ok_or("ts_frontend: bad jsonSet 3rd arg")?;
                // jsonQuote(x) / {object-literal} / jsonSet(...) already
                // produce encoded text — splice raw.
                let already = match raw {
                    Expression::CallExpression(ic) => match &ic.callee {
                        Expression::Identifier(id) => matches!(id.name.as_str(), "jsonQuote" | "jsonSet"),
                        _ => false,
                    },
                    Expression::ObjectExpression(_) => true,
                    _ => false,
                };
                let encoded = if already {
                    lower_expr(raw)?
                } else {
                    encode_json_value(raw)?
                };
                return Ok(list(vec![Sym("json-set"), items[1].clone(), key, encoded]));
            }
            // str-cat is variadic in the EMITTER but 2-ary in the CHECKER —
            // fold n-ary strCat calls into nested 2-arg applications
            if head == "str-cat" && items.len() > 3 {
                let mut acc = items.pop().unwrap();
                while items.len() > 1 {
                    let rhs = items.pop().unwrap();
                    acc = list(vec![Sym("str-cat"), rhs, acc]);
                }
                return Ok(acc);
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
        // Arrow functions (2026-08-30): expression-bodied or single-return
        // block bodies. Used by .map/.filter/.reduce callbacks. The body is
        // inlined by the wasm emitters (resolve_lambda_1/2) with the param
        // bound, so outer consts stay visible.
        Expression::ArrowFunctionExpression(a) => {
            // body: expression form (x => e) or block body — see arrow_parts
            // (shared with `export const f = arrow`).
            let (params, body_val) = arrow_parts(a)?;
            Ok(list(vec![
                Sym("lambda"),
                list(params),
                body_val,
            ]))
        }
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
        Expression::CallExpression(c) => {
            // string instance predicates (M2): startsWith / endsWith / includes
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                if matches!(
                    sm.property.name.as_str(),
                    "startsWith" | "endsWith" | "includes"
                ) {
                    true
                } else {
                    callee_name(&c.callee)
                        .ok()
                        .map(|h| {
                            // camel names (u128Lt) map to the lisp builtin
                            // (u128/lt) — check the POST-mapping name
                            let mapped = map_builtin_call(&h);
                            matches!(
                                mapped.as_str(),
                                "u128/gt" | "u128/lt" | "u128/gte" | "u128/lte" | "u128/eq"
                                    | "near/deposit-gte"
                            )
                        })
                        .unwrap_or(false)
                }
            } else {
                callee_name(&c.callee)
                    .ok()
                    .map(|h| {
                        let mapped = map_builtin_call(&h);
                        matches!(
                            mapped.as_str(),
                            "u128/gt" | "u128/lt" | "u128/gte" | "u128/lte" | "u128/eq"
                                | "near/deposit-gte"
                        )
                    })
                    .unwrap_or(false)
            }
        }
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
        Expression::Identifier(id) => {
            note_ident(&id.name, id.span.start);
            Ok(map_global_fn(id.name.as_str()))
        }
        Expression::StaticMemberExpression(s) => {
            let obj = match &s.object {
                Expression::Identifier(id) => {
                    note_ident(&id.name, id.span.start);
                    id.name.as_str().to_string()
                }
                _ => return Err("ts_frontend: nested member chains not in M1".into()),
            };
            note_ident(s.property.name.as_str(), s.property.span.start);
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
    // near-sdk-js spelling: storage.set/get/has/del(...) — same builtins
    // as near.storageSet/Get/… so both dialect spellings coexist.
    if obj == "storage" {
        let mapped = match prop {
            "set" | "write" => "near/storage_set",
            "get" | "read" => "near/storage_get",
            "has" | "hasKey" => "near/storage_has",
            // BUG (latent, 2026-09-01): mapped to near/storage_del which
            // exists in NO engine — checker reject or silent nil. Real op
            // (interp + wasm emitter + checker) is near/storage_remove.
            "del" | "remove" => "near/storage_remove",
            _ => return format!("near/storage_{}", snake(prop)),
        };
        return mapped.into();
    }
    if obj == "near" && prop == "depositGte" {
        // lisp lib predates the snake convention here
        return "near/deposit-gte".into();
    }
    if obj == "near" && prop == "callAwait" {
        // lisp sugar form predates the snake convention (hyphen, not underscore)
        return "near/call-await".into();
    }
    if obj == "near" && (prop == "yieldCreate" || prop == "promiseYieldCreate") {
        return "near/promise_yield_create".into();
    }
    if obj == "near" && (prop == "yieldResume" || prop == "promiseYieldResume") {
        return "near/promise_yield_resume".into();
    }
    if obj == "near" {
        // kebab-canonical builtins: these map to unprefixed kebab ops, NOT
        // the default near/snake_name path. The default path produced
        // near/json_get / near/json_set — which don't exist (only the free
        // function spellings jsonGet()/jsonSet() reached the real ops).
        // Found via the HTLC contract (2026-09-01): every earlier contract
        // had accidentally used the free-function form.
        if let Some(kebab) = match prop {
            "jsonGet" => Some("json-get"),
            "jsonSet" => Some("json-set"),
            "jsonQuote" => Some("json-quote"),
            "sha256Hash" => Some("sha256-hash"),
            "hexDecode" => Some("hex-decode"),
            "schnorrVerify" => Some("schnorr-verify"),
            _ => None,
        } {
            return kebab.into();
        }
    }
    if obj == "near" && prop == "jsonArr" {
        // json array args: {"k": ["a","b"]} → TAG_ARRAY of strings
        return "near/json_get_arr".into();
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
        "strSplit" => "str-split",
        "hexDecode" => "hex-decode",
        "sha256Hash" => "sha256-hash",
        "schnorrVerify" => "schnorr-verify",
        "jsonSet" => "json-set",
        "jsonQuote" => "json-quote",
        // u128-precision arithmetic (decimal-string ABI, both runtimes)
        "u128Add" => "u128/add",
        "u128Sub" => "u128/sub",
        "u128Mul" => "u128/mul",
        "u128Div" => "u128/div",
        "u128Mod" => "u128/mod",
        "u128Lt" => "u128/lt",
        "u128Gt" => "u128/gt",
        "u128Eq" => "u128/eq",
        "u128IsZero" => "u128/is-zero",
        "u128FromNum" => "u128/from-i64",
        "u128ToNum" => "u128/to-i64",
        _ => return name.to_string(),
    }
    .to_string()
}

/// `amt: bigint` — u128-precision amount param (decimal-string ABI).
fn param_is_bigint(p: &FormalParameter<'_>) -> bool {
    match &p.type_annotation {
        Some(a) => matches!(&a.type_annotation, TSType::TSBigIntKeyword(_)),
        None => false,
    }
}

fn param_is_str_array(p: &FormalParameter<'_>) -> bool {
    match &p.type_annotation {
        Some(a) => matches!(
            &a.type_annotation,
            TSType::TSArrayType(arr)
                if matches!(&arr.element_type, TSType::TSStringKeyword(_))
        ),
        None => false,
    }
}

fn param_is_number(p: &FormalParameter<'_>) -> bool {
    match &p.type_annotation {
        Some(a) => matches!(&a.type_annotation, TSType::TSNumberKeyword(_)),
        None => false,
    }
}

/// Object-typed param: `p: { name: string; votes: number }` → the inline
/// literal type's properties (name, is_number). Returns None for every
/// other annotation shape. Named type references (TypeReference) are
/// deliberately rejected — see param_is_type_ref.
fn param_object_props(p: &FormalParameter<'_>) -> Option<Vec<(String, bool)>> {
    let a = p.type_annotation.as_ref()?;
    match &a.type_annotation {
        TSType::TSTypeLiteral(lit) => {
            let mut props = Vec::new();
            for m in &lit.members {
                let sig = match m {
                    oxc_ast::ast::TSSignature::TSPropertySignature(sig) => sig,
                    _ => return None, // call signatures etc. — not a data shape
                };
                let key = match &sig.key {
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => {
                        id.name.as_str().to_string()
                    }
                    _ => return None, // computed/string keys — not a data shape
                };
                let is_num = matches!(
                    sig.type_annotation.as_ref().map(|t| &t.type_annotation),
                    Some(TSType::TSNumberKeyword(_))
                );
                props.push((key, is_num));
            }
            if props.is_empty() {
                None
            } else {
                Some(props)
            }
        }
        // `type X = {...}` alias — resolved from the compile-time alias table
        TSType::TSTypeReference(r) => {
            let name = match &r.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(id) => {
                    id.name.as_str().to_string()
                }
                _ => return None, // qualified names — not a local alias
            };
            TYPE_ALIASES.with(|m| {
                m.borrow()
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, props)| props.clone())
            })
        }
        _ => None,
    }
}

fn param_is_type_ref(p: &FormalParameter<'_>) -> bool {
    // Unresolvable type refs only — known `type X = {...}` aliases are
    // fine (resolved in param_object_props).
    if let Some(a) = p.type_annotation.as_ref() {
        if let TSType::TSTypeReference(r) = &a.type_annotation {
            let name = match &r.type_name {
                oxc_ast::ast::TSTypeName::IdentifierReference(id) => {
                    Some(id.name.as_str())
                }
                _ => None,
            };
            let known = name.map(|n| {
                TYPE_ALIASES.with(|m| m.borrow().iter().any(|(k, _)| k == n))
            });
            return !known.unwrap_or(false);
        }
    }
    false
}

/// Shape of a `type X = { prop: string; num: number; ... }` alias.
fn alias_props(a: &oxc_ast::ast::TSTypeAliasDeclaration<'_>) -> Vec<(String, bool)> {
    let oxc_ast::ast::TSType::TSTypeLiteral(lit) = &a.type_annotation else {
        return Vec::new();
    };
    let mut props = Vec::new();
    for m in &lit.members {
        if let oxc_ast::ast::TSSignature::TSPropertySignature(sig) = m {
            if let oxc_ast::ast::PropertyKey::StaticIdentifier(id) = &sig.key {
                let is_num = matches!(
                    sig.type_annotation.as_ref().map(|t| &t.type_annotation),
                    Some(TSType::TSNumberKeyword(_))
                );
                props.push((id.name.as_str().to_string(), is_num));
            }
        }
    }
    props
}

/// Register an object param's shape for read-time numeric decoding and
/// encode-time raw embedding (side-channel, cleared with NUM_PARAM_NAMES
/// after body lowering).
fn register_obj_param(name: &str, props: Vec<(String, bool)>) {
    OBJ_PARAM_PROPS.with(|s| {
        s.borrow_mut().push((name.to_string(), props));
    });
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

#[cfg(test)]
mod ts_pos_tests {
    #[test]
    fn ts_ident_offsets_recorded_and_hints_resolve() {
        let src = "export function new_() {\n  let x = 1\n  let y = undefined_helper(x)\n  return y\n}\n";
        let r = super::parse_ts(src).expect("parses");
        assert!(!r.is_empty());
        let map = super::take_ident_offsets();
        eprintln!("MAP: {:?}", map);
        assert!(
            map.iter().any(|(n, _)| n == "undefined_helper"),
            "undefined_helper should be in the ident map"
        );
        let line = super::ts_line_hint(&map, src, "undefined_helper").expect("hint");
        assert_eq!(line, "3");
    }
}
