//! TS frontend (M1): TypeScript-syntax surface → lisp s-expression source.
//!
//! Lowering pipeline: TS source --oxc_parser--> TS AST --this module--> lisp
//! source text --existing parser/checker/emitters--> all backends (near wasm,
//! bytecode, wasi) unchanged.
//!
//! M1+ subset (differential-provable; truth-up 2026-10-09 after F1–F3):
//!   ✓ function declarations (exported or not) → define (+ export form)
//!   ✓ const/let locals (single declarator, initializer required)
//!   ✓ if / else (tail position: full expression; non-tail: side-effect begin)
//!   ✓ ternary `c ? a : b`
//!   ✓ early returns anywhere in a block (statement-level __fn_done
//!     flag-guard protocol; in-loop exits via __wl_* flags)
//!   ✓ numeric/string/boolean/null literals, template literals → (str ...)
//!     (every interpolation auto-wrapped (to-string e))
//!   ✓ binary ops: + - * / % < > <= >= == === != !== (numbers only)
//!   ✓ compound assigns += -= *= /= %= (d47156b8): *= /= single-op,
//!     %= JS truncated-mod with bind-once impure rhs; `**`/`**=` HARD ERROR
//!     (no expt builtin in the NEAR set)
//!   ✓ && || with JS VALUE semantics (431e1f3): a||b yields a-when-truthy
//!     else b (and inverse for &&); truthy = not(false|nil|0); string
//!     truthiness is M2. Left operand bound once (span-mangled temp),
//!     short-circuit proven by tests/test_ts_logical_values.rs
//!   ✓ ?? coalesce: handle-prop paths dispatch on the fallback's literal
//!     type (number fb → INT getter, string fb → STR getter)
//!   ✓ ! - unary (! is tag-aware — 920e79f6)
//!   ✓ calls: bare identifiers + member calls via builtin mapping
//!   ✓ arrow fns: expression bodies, single-return blocks, full blocks via
//!     lower_block_tail (begin/let/if sequencing, early returns).
//!     .map/.filter/.reduce callbacks — inlined by resolve_lambda_1/2.
//!     SCOPED CLOSURES (F3, d2fd048f): local `const f = (…) => …` bound to
//!     a real lambda, called directly by name, IMMUTABLE capture of
//!     enclosing locals — probed on wasm + interp. Whole-function static
//!     analysis (check_fn_closure_safety) HARD-ERRORS the probed wasm
//!     landmines: T4 mutable capture, dispatch freeze (lambda-local used
//!     in a pipeline callback), arrow-as-call-argument
//!   ✓ async/await (V2, 8e1b098b): awaits anywhere, near.all fanout,
//!     payable; continuation is an on-chain entry (exported async only)
//!   ✓ money taint (5665edc4): raw + - * / % on Yocto/Amount-domain values
//!     is a compile error — use the u128.* calls
//!   ✓ destructuring: `const {..} = near.args<{..}>()` param unpack only
//!   ✗ classes, general closures beyond the F3 shapes (passing/returning a
//!     lambda-valued var), optional chaining, imports
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

/// WASI/P2 lowering mode: exported `run` keeps REAL params (the P2 _start
/// wrapper passes the stdin input as an argument) instead of the NEAR
/// tx-input convention (params re-bound to near/json_get_str reads —
/// a NEAR host op that doesn't exist on the OutLayer runtime).
thread_local! {
    pub(crate) static TS_WASI_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Parse TypeScript source and lower it to lisp source text (NEAR
/// input convention — see TS_WASI_MODE for the P2 variant).
pub fn ts_to_lisp_source(src: &str) -> Result<String, String> {
    TS_WASI_MODE.with(|m| m.set(false));
    ts_to_lisp_source_inner(src)
}

/// P2/WASI variant: exported run(params) stay real function params.
pub fn ts_to_lisp_source_wasi(src: &str) -> Result<String, String> {
    TS_WASI_MODE.with(|m| m.set(true));
    ts_to_lisp_source_inner(src)
}

fn ts_to_lisp_source_inner(src: &str) -> Result<String, String> {
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
    OBJECT_PARAMS.with(|s| s.borrow_mut().clear());
    CONST_FOLDS.with(|s| s.borrow_mut().clear());
    BIGINT_CONSTS.with(|s| s.borrow_mut().clear());
    MONEY_NAMES.with(|s| s.borrow_mut().clear());
    LEDGER_NAMES.with(|s| s.borrow_mut().clear());
    MONEY_ALIASES.with(|s| s.borrow_mut().clear());
    USER_FNS.with(|m| m.borrow_mut().clear());
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
    /// Money-domain value names (2026-10-08 taint gate): await-result
    /// bindings (`const x = await near.call(...)` returns u128 decimal
    /// strings) — a raw `+`/`-` on two of them corrupts the amount.
    // LEDGER tier: names bound to `storageGet(...) ?? ""` reads — may
    // feed SINKS (custody check) but carry NO arithmetic seal.
    static LEDGER_NAMES: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());
    static MONEY_NAMES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Scalar money type aliases seen at statement level: `type Yocto =
    /// string`, `type Bal = Yocto`. The built-in Yocto/Amount/Money trio
    /// is always recognized.
    static MONEY_ALIASES: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Record-typed locals with bigint fields, inferred from the shape
    /// literal in `let rec = storageGet(...) ?? '{"amt":"0",...}'`:
    /// keys whose default is a QUOTED NUMERIC string are bigint fields
    /// (the storageGet ?? record pattern; found via the HTLC contract
    /// 2026-09-01 — `rec.amt + x` lowered to plain numeric + because
    /// dot-access never carried the shape's bigint typing).
    static SHAPE_BIGINT_FIELDS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// `const o = near.input()` handles (JSON API v3, 2026-09-15): property
    /// reads on these names rewrite at compile time to the CACHED-INPUT
    /// getters — `o.name` → (near/json_get_str "name") (nil-on-miss), and
    /// `o.prop ?? fb` dispatches on the fallback type: number fb → the INT
    /// getter (no strToNum ceremony), string fb → the STR getter. Zero
    /// copies: the handle never materializes the input as a string.
    static INPUT_HANDLES: std::cell::RefCell<Vec<String>> =
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
    /// Object-shaped params by NAME (`params`, `ctx`…): their property
    /// reads lower against the CACHED-INPUT contract (near/json_get_str
    /// with a dotted path) instead of the param's dead binding. Before
    /// this registry the read lowered to (json-get-str "to" params) —
    /// params binds (near/json_get_str "params"), a top-level key that's
    /// never in the args JSON — so `params.to` was silently nil at
    /// runtime (documented BUG, fixed 2026-10-07).
    static OBJECT_PARAMS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Top-level `const K = <literal>;` — folded into every use site.
    /// (2026-08-31) a value-define at top level emits a stub (known emitter
    /// limitation), so numeric/string consts INSTEAD substitute inline and
    /// emit nothing. Non-literal top-level consts keep the old path.
    /// TS surface strictness (2026-10-05): every FunctionDeclaration name
    /// (any nesting depth) collected before lowering — bare calls to names
    /// outside this set, the builtin map, or the RLM runtime API are
    /// rejected. The old unknown-name passthrough let brain-written lisp
    /// (reduce/lambda/array…) compile as "TS" and game the checker.
    static USER_FNS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
    static CONST_FOLDS: std::cell::RefCell<Vec<(String, LispVal)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Top-level `const K = <n-literal>;` names — bigint-shaped identifiers
    /// for operator selection (fold value lands in CONST_FOLDS as Str).
    static BIGINT_CONSTS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// True while lowering a function body whose top level bound
    /// __fn_done/__fn_res (the M2 early-return flag-guard). In-loop
    /// `return` rewrites consult it: they must ALSO set the function-level
    /// flags, or a return nested in an inner while only stops that while
    /// and the value vanishes (nested-return bug, 2026-09-11). Nested
    /// lower_block_tail calls see it true and skip their own binding —
    /// a shadowing let would swallow nested returns.
    static FN_FLAGS_BOUND: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
        .map(|l| {
            format!(
                "\n  {:>4} | {}\n       | {}^",
                line,
                l.trim_end(),
                " ".repeat(0)
            )
        })
        .unwrap_or_default()
}

/// Parse TypeScript source and lower it to top-level lisp forms.
pub fn parse_ts(src: &str) -> Result<Vec<LispVal>, String> {
    let allocator = Allocator::default();
    let source_type = SourceType::default()
        .with_typescript(true)
        .with_module(true);
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
    let mut forms = lower_program(&ret.program)?;
    for f in forms.iter_mut() {
        *f = discard_normalize(f);
    }
    Ok(forms)
}

/// Post-lowering normalization (2026-10-06): the __fn_done/__wl_* statement
/// guards wrap arbitrary statement bodies as `(if (= FLAG 0) BODY 0)`.
/// When BODY's TYPE is a str — a bare str-returning call like
/// `poolSwap(amt);`, or a statement-if whose branches type str — the
/// checker's branch unification rejects it as `str ≠ int` even though the
/// statement position discards the value entirely. Fix: rewrite to
/// `(if (= FLAG 0) (begin BODY 0) 0)` — the same discard idiom the loop
/// lowering has always used (`list(vec![Sym("begin"), e, Num(0)])`).
/// Semantics-neutral: effects and evaluation order preserved, value 0.
/// Found live while porting the CLMM pool to TS (bisected via probe files:
/// statement CALLS, nested guard-ifs, and if-branches with str tails all
/// hit the one pattern — one root cause, one rewrite).
fn discard_normalize(v: &LispVal) -> LispVal {
    match v {
        LispVal::List(items) => {
            // don't descend into quoted DATA — it is not code
            if let Some(LispVal::Sym(s)) = items.first() {
                if s == "quote" {
                    return v.clone();
                }
            }
            // rewrite matching guard-wraps
            if items.len() == 4 {
                let is_flag_test = matches!(
                    &items[0],
                    LispVal::Sym(s) if s == "if"
                ) && matches!(
                    &items[1],
                    LispVal::List(t) if t.len() == 3
                        && matches!(&t[0], LispVal::Sym(op) if op == "=")
                        && matches!(&t[1], LispVal::Sym(flag)
                            if flag == "__fn_done" || flag.starts_with("__wl_"))
                        && matches!(&t[2], LispVal::Num(n) if *n == 0)
                ) && matches!(&items[3], LispVal::Num(n) if *n == 0);
                let already_discarding = matches!(
                    &items[2],
                    LispVal::List(b) if !b.is_empty()
                        && matches!(b.first(), Some(LispVal::Sym(s)) if s == "begin")
                        && matches!(b.last(), Some(LispVal::Num(n)) if *n == 0)
                );
                if is_flag_test && !already_discarding {
                    let body = discard_normalize(&items[2]);
                    return LispVal::List(vec![
                        items[0].clone(),
                        items[1].clone(),
                        LispVal::List(vec![LispVal::Sym("begin".into()), body, LispVal::Num(0)]),
                        LispVal::Num(0),
                    ]);
                }
            }
            // recurse
            LispVal::List(items.iter().map(discard_normalize).collect())
        }
        other => other.clone(),
    }
}

/// Parse + retain the ident→offset map (for augmenting downstream errors).
/// Consumes the map the walk just produced — call immediately after a
/// successful `parse_ts` on the SAME thread.
pub fn take_ident_offsets() -> Vec<(String, u32)> {
    IDENT_OFFSETS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Best-effort: name → "line N" hint for error augmentation.
pub fn ts_line_hint(map: &[(String, u32)], src: &str, name: &str) -> Option<String> {
    map.iter().find(|(n, _)| n == name).map(|(_, off)| {
        let (line, _col) = line_col(src, *off);
        format!("{}", line)
    })
}

/// Locate WHICH top-level form a checker error comes from, with its TS
/// source position (2026-10-06). The checker is sequential (defines build
/// the env in order), so the first failing prefix identifies the culprit
/// form exactly — no spans needed on LispVal. Returns
/// "in function 'name' (ts line N)" plus the source line, or None when
/// the whole program checks clean / the culprit can't be named.
///
/// Use on the error path only: O(n) checker passes over a program of n
/// top-level forms. Offsets come from the fresh parse_ts walk (call right
/// after it drains IDENT_OFFSETS).
pub fn locate_form_error(
    exprs: &[LispVal],
    ident_map: &[(String, u32)],
    src: &str,
    orig_err: &str,
) -> Option<String> {
    // find first failing prefix
    let mut culprit: Option<&LispVal> = None;
    for k in 1..=exprs.len() {
        if crate::typing::type_check_program(&exprs[..k], true).is_err() {
            culprit = Some(&exprs[k - 1]);
            break;
        }
    }
    let form = culprit?;
    // extract the defined/exported name: (define (name ...) …) or
    // (export "x" name) — for exports, prefer the referenced define's
    // own name so the hint lands on its definition
    let sym_at = |v: &LispVal| -> Option<String> {
        match v {
            LispVal::Sym(s) => Some(s.clone()),
            _ => None,
        }
    };
    let mut name: Option<String> = None;
    if let LispVal::List(items) = form {
        let head = items.first().and_then(sym_at).unwrap_or_default();
        if head == "define" {
            if let Some(LispVal::List(sig)) = items.get(1) {
                name = sig.first().and_then(sym_at); // (define (name args…) …)
            } else {
                name = items.get(1).and_then(sym_at); // (define name value)
            }
        } else if head == "export" {
            if let Some(LispVal::Str(s)) = items.get(1) {
                name = Some(s.clone()); // exported display name
            }
        }
    }
    let name = name?;
    // prefer the DEFINITION offset (ident_map first-occurrence often points
    // at a call site above the def under hoisted lowering order)
    let line_no = ident_map
        .iter()
        .find(|(n, _)| n == &name)
        .map(|(_, off)| line_col(src, *off).0 as u32);
    let line_no = line_no?;
    let excerpt = src_excerpt(src, line_no);
    Some(format!(
        "in function `{}` (ts line {}) — first error:\n  {}\n{}",
        name,
        line_no,
        orig_err.lines().next().unwrap_or(orig_err),
        excerpt
    ))
}

// ── Program / statements ──────────────────────────────────────────────────

/// RLM runtime API callable from the TS surface (advertised in the
/// cheatsheet COMPLETE EXAMPLE). Runtime defines loaded before eval.
const RLM_RUNTIME_API: &[&str] = &["rlm_set", "rlm_get"];

/// Definition sites (name, byte-offset) recorded during collect_user_fns —
/// unlike IDENT_OFFSETS (first occurrence, often a call above the def in
/// hoisted lowering order) these always land ON the definition for
/// locate_form_error.
thread_local! {
    static FN_DEF_OFFSETS: std::cell::RefCell<Vec<(String, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn note_fn_def(name: &str, span_start: u32) {
    FN_DEF_OFFSETS.with(|m| {
        let mut b = m.borrow_mut();
        if !b.iter().any(|(n, _)| n == name) {
            b.push((name.to_string(), span_start));
        }
    });
}

pub fn take_fn_def_offsets() -> Vec<(String, u32)> {
    FN_DEF_OFFSETS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

fn collect_user_fns(stmts: &[Statement<'_>], out: &mut std::collections::HashSet<String>) {
    for s in stmts {
        match s {
            Statement::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    out.insert(id.name.as_str().to_string());
                    note_fn_def(id.name.as_str(), id.span.start);
                }
                // NOTE: stmts_of() on a non-Block returns the statement
                // itself — recursing through it here was an infinite loop
                // (stack overflow, found by strict_gate_fn_hoisting).
                if let Some(body) = &f.body {
                    collect_user_fns(&body.statements, out);
                }
            }
            Statement::ExportDeclaration(decl) => {
                if let Declaration::FunctionDeclaration(f) = &decl.declaration {
                    if let Some(id) = &f.id {
                        out.insert(id.name.as_str().to_string());
                        note_fn_def(id.name.as_str(), id.span.start);
                    }
                    if let Some(body) = &f.body {
                        collect_user_fns(&body.statements, out);
                    }
                }
                // `export const f = (…) => …` — F3 exported arrow: lowered
                // to a define + export pair and CALLED BY NAME, so the
                // strict-surface gate must whitelist it too.
                if let Declaration::VariableDeclaration(v) = &decl.declaration {
                    collect_arrow_decls(v, out);
                }
            }
            // F3 scoped closures: `const f = (…) => …` locals bind a lambda
            // called by name — register them or the strict gate (2026-10-05)
            // rejects `f(s)` as a lisp passthrough before the F3 lowering
            // ever runs (surfaced when the local F3 line met the pushed
            // strict-surface line in the 2026-10-07 merge).
            Statement::VariableDeclaration(v) => collect_arrow_decls(v, out),
            Statement::BlockStatement(b) => collect_user_fns(&b.body, out),
            Statement::IfStatement(i) => {
                collect_user_fns(stmts_of(&i.consequent), out);
                if let Some(alt) = &i.alternate {
                    collect_user_fns(stmts_of(alt), out);
                }
            }
            Statement::WhileStatement(w) => collect_user_fns(stmts_of(&w.body), out),
            Statement::ForStatement(f) => collect_user_fns(stmts_of(&f.body), out),
            Statement::ForOfStatement(fo) => collect_user_fns(stmts_of(&fo.body), out),
            Statement::DoWhileStatement(d) => collect_user_fns(stmts_of(&d.body), out),
            Statement::SwitchStatement(sw) => {
                for c in &sw.cases {
                    collect_user_fns(&c.consequent, out);
                }
            }
            _ => {}
        }
    }
}

/// Register `const name = (…) => …` declarators (F3 callable arrows).
fn collect_arrow_decls(
    v: &oxc_ast::ast::VariableDeclaration<'_>,
    out: &mut std::collections::HashSet<String>,
) {
    for d in &v.declarations {
        if matches!(d.init, Some(Expression::ArrowFunctionExpression(_))) {
            if let Ok(name) = binding_name(&d.id) {
                out.insert(name);
            }
        }
    }
}

fn user_fn_or_runtime(name: &str) -> bool {
    USER_FNS.with(|f| f.borrow().contains(name)) || RLM_RUNTIME_API.contains(&name)
}

fn lower_program(p: &Program<'_>) -> Result<Vec<LispVal>, String> {
    // strict surface: register all user functions before any lowering
    USER_FNS.with(|m| {
        let mut set = std::collections::HashSet::new();
        collect_user_fns(&p.body, &mut set);
        *m.borrow_mut() = set;
    });
    // TypeScript hoists function declarations: a call may textually precede
    // the helper's definition. Lisp requires define-before-use, so we reorder:
    //   1. top-level consts (module-load-time, source order)
    //   2. non-exported functions (hoisted, source order)
    //   3. everything else (exported defines, exports, top-level exprs) in order
    let mut consts: Vec<LispVal> = Vec::new();
    let mut hoisted: Vec<LispVal> = Vec::new();
    let mut out: Vec<LispVal> = Vec::new();
    // Pre-scan ALL top-level const declarations BEFORE lowering anything:
    // the frontend is single-pass, so a function ABOVE a const referenced
    // it as a bare Sym (no fold, no define → "undefined variable" in the
    // checker). Literals register in CONST_FOLDS now; use-site substitution
    // then works regardless of declaration order. (Non-literal consts
    // still emit (define ...) forms in source order — those were never
    // forward-referenceable and stay that way; found compiling the PLONK
    // verifier, 2026-09-15.)
    for stmt in &p.body {
        if let Statement::VariableDeclaration(v) = stmt {
            for d in &v.declarations {
                if let (Ok(name), Some(init)) = (binding_name(&d.id), d.init.as_ref()) {
                    // `let` is mutable — with top-level loops (M1.5,
                    // 2026-10-05) a folded `let s = 0` went stale inside the
                    // loop body ((+ 0 (* i i)) while set! s wrote a shadow).
                    // Only `const` folds; mutable decls emit real defines.
                    if !matches!(v.kind, oxc_ast::ast::VariableDeclarationKind::Const) {
                        continue;
                    }
                    let mut is_bigint = false;
                    let literal = match init {
                        Expression::NumericLiteral(n) => Some(Num(n.value as i64)),
                        Expression::StringLiteral(s) => Some(Str(s.value.as_str().to_string())),
                        Expression::BooleanLiteral(b) => Some(LispVal::Bool(b.value)),
                        Expression::BigIntLiteral(b) => {
                            is_bigint = true;
                            Some(Str(b
                                .raw
                                .as_ref()
                                .map(|s| s.as_str().trim_end_matches('n').to_string())
                                .unwrap_or_default()))
                        }
                        _ => None,
                    };
                    if let Some(v) = literal {
                        if is_bigint {
                            BIGINT_CONSTS.with(|m| m.borrow_mut().push(name.clone()));
                        }
                        // don't double-register when the main pass reaches
                        // this declaration again (it re-pushes to CONST_FOLDS
                        // — harmless: use-site find() takes the FIRST match,
                        // same value both times)
                        CONST_FOLDS.with(|m| m.borrow_mut().push((name.clone(), v)));
                    }
                }
            }
        }
    }
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
                        let (name, defines) = lower_function(f, true)?;
                        let view = name.starts_with("get_");
                        for d in &defines {
                            out.push(d.clone());
                        }
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
                for d in lower_function(f, false)?.1 {
                    hoisted.push(d);
                }
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
                        Expression::StringLiteral(s) => Some(Str(s.value.as_str().to_string())),
                        Expression::BooleanLiteral(b) => Some(LispVal::Bool(b.value)),
                        // `const FEE_BP = 500n;` — u128 const: folds as a
                        // decimal string AND marks the name bigint-shaped
                        Expression::BigIntLiteral(b) => {
                            is_bigint = true;
                            Some(Str(b
                                .raw
                                .as_ref()
                                .map(|s| s.as_str().trim_end_matches('n').to_string())
                                .unwrap_or_default()))
                        }
                        _ => None,
                    };
                    if let Some(lit) = literal {
                        if is_bigint {
                            BIGINT_CONSTS.with(|m| m.borrow_mut().push(name.clone()));
                        }
                        if matches!(v.kind, oxc_ast::ast::VariableDeclarationKind::Const) {
                            CONST_FOLDS.with(|m| m.borrow_mut().push((name, lit)));
                        } else {
                            // mutable `let x = <literal>` — real define, no fold
                            consts.push(list(vec![Sym("define"), Sym(name), lit]));
                        }
                    } else {
                        consts.push(list(vec![Sym("define"), Sym(name), lower_expr(init)?]));
                    }
                }
            }
            Statement::ExpressionStatement(e) => {
                // effect-position (M2+ 2026-10-05): assignments (incl.
                // property writes o.x = v) are statements here too
                out.push(effect_expr(&e.expression)?);
            }
            Statement::WhileStatement(w) => {
                // M1.5 (2026-10-05): top-level loops (RLM brain code lives
                // here — `let s = 0; for (...) {...}` module scripts).
                out.push(lower_while_form(w, false)?);
            }
            Statement::ForStatement(f) => {
                out.push(lower_for_form(f, false)?);
            }
            Statement::ForOfStatement(fo) => {
                // M1.5+ (2026-10-05): top-level for..of — reuse the
                // function-body machinery (self-contained flag lets);
                // Num(0) tail, view=false. The brain hit this exact gap
                // minutes after the counted-loop ship (t1_sumsq@ts 14:08).
                out.push(lower_prefix_around(
                    std::slice::from_ref(stmt),
                    Num(0),
                    false,
                )?);
            }
            Statement::DoWhileStatement(d) => {
                // M1.5+ (2026-10-05)
                out.push(lower_do_while_form(d, false)?);
            }
            Statement::SwitchStatement(sw) => {
                // M1.5+ (2026-10-05): all-break if-chain
                out.push(lower_switch_form(sw, false)?);
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
                    m.borrow_mut().push((a.id.name.as_str().to_string(), props));
                });
                // (2026-10-08) money aliases: the Yocto/Amount/Money trio,
                // or any alias DEFINED as one (`type Bal = Yocto`).
                {
                    let an = a.id.name.as_str();
                    let money = matches!(an, "Yocto" | "Amount" | "Money")
                        || match &a.type_annotation {
                            oxc_ast::ast::TSType::TSTypeReference(r) => {
                                matches!(&r.type_name,
                                    oxc_ast::ast::TSTypeName::IdentifierReference(id)
                                        if matches!(id.name.as_str(), "Yocto" | "Amount" | "Money")
                                            || MONEY_ALIASES.with(|m| {
                                                m.borrow().iter().any(|x| *x == id.name.as_str())
                                            }))
                            }
                            _ => false,
                        };
                    if money {
                        MONEY_ALIASES.with(|m| m.borrow_mut().push(an.to_string()));
                    }
                }
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
                    let has_default = imp.specifiers.iter().flatten().any(|s| {
                        matches!(
                            s,
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(_)
                        )
                    });
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

/// Lower an async function — async v2 (2026-10-08).
///
/// Awaits anywhere, multiple awaits, `near.all([...])` fanout, payable.
/// CPS split at each await point: the entry runs pre-await statements,
/// snapshots the params into storage and fires await 0; resume k restores
/// the frame, binds await k's results, runs its segment and fires the next
/// await. The LAST resume runs the final segment as the function value.
///
/// Frame = entry params ONLY (the V1-proven `__await:<fn>:<param>` storage
/// path). A `let` local read after an await, or an assignment to a
/// param/pre-await local after an await, is a COMPILE ERROR naming the
/// variable (T4 — locals die at the await boundary; the frame is the only
/// state that crosses). Pre-await reassignment of params is fine: the
/// snapshot happens after the pre-await statements run, so the final
/// values are what the continuations see.
///
/// `await near.call(t, m, args, gas, dep)` and
/// `const [a, b] = await near.all([c1, c2])` both lower to the manual
/// promise shape (promise_create → [promise_and] → ONE promise_then →
/// promise_return) — the same DAG the portfolio fixture hand-writes — so
/// deposits flow through untouched (V1's zero-deposit restriction is gone)
/// and every join has exactly ONE resume reading promise_result(0..n) in
/// dependency order.
///
/// Returns forms: entry define + export, then per await: resume define + export.
fn lower_async_function(f: &TsFunction<'_>) -> Result<Vec<LispVal>, String> {
    const CB_GAS: i64 = 50_000_000_000_000;
    let name =
        f.id.as_ref()
            .map(|i| i.name.as_str().to_string())
            .ok_or("ts_frontend: anonymous async functions unsupported")?;

    // (name, kind): 0 = string, 1 = number, 2 = string[], 3 = object
    // (object = JSON-text binding; numeric props auto-decode on read)
    NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
    OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
    OBJECT_PARAMS.with(|s| s.borrow_mut().clear());
    BIGINT_NAMES.with(|s| s.borrow_mut().clear());
    BIGINT_LOCALS.with(|s| s.borrow_mut().clear());
    STRING_LOCALS.with(|s| s.borrow_mut().clear());
    SHAPE_BIGINT_FIELDS.with(|s| s.borrow_mut().clear());
    MONEY_NAMES.with(|s| s.borrow_mut().clear());
    LEDGER_NAMES.with(|s| s.borrow_mut().clear());
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
        if ann_is_money_alias(p.type_annotation.as_deref()) {
            MONEY_NAMES.with(|s| s.borrow_mut().push(n.clone()));
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
    check_return_contract(&name, stmts, return_ann_money(f))?;
    check_money_sinks(stmts)?;

    // Forward-scan the WHOLE body: bigint lets / string locals / input
    // handles must be registered before any segment lowers (segments lower
    // in CPS order — see scan_bigint_lets).
    scan_bigint_lets(stmts);

    let state_key = format!("__await:{}", name);

    // ── collect top-level await points ──────────────────────────────
    enum Bind {
        Plain,
        Object,
        Positional,
    }
    struct AwaitPoint<'a> {
        idx: usize,
        names: Vec<String>,
        bind: Bind,
        // near.call(...) for Plain/Object, the array literal for Positional
        src_expr: &'a Expression<'a>,
    }
    let mut awaits: Vec<AwaitPoint<'_>> = Vec::new();
    for (i, s) in stmts.iter().enumerate() {
        let Statement::VariableDeclaration(vd) = s else {
            continue;
        };
        if vd.declarations.len() != 1 {
            continue;
        }
        let decl = &vd.declarations[0];
        let Some(Expression::AwaitExpression(ae)) = &decl.init else {
            continue;
        };
        let names = pattern_names(&decl.id)?;
        // (2026-10-08, tightened) Await results seed money ONLY for
        // balance/supply reads — money comes from money sources. An
        // await of `getName` binds a plain string (Value), not Yocto.
        seed_await_money_names(&ae.argument, &names);
        let arg = &ae.argument;
        // near.all([...]) detection — must be the bare member call
        let mut all_arr: Option<&oxc_ast::ast::ArrayExpression<'_>> = None;
        if let Expression::CallExpression(c) = arg {
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                if let Expression::Identifier(obj) = &sm.object {
                    if obj.name == "near" && sm.property.name == "all" {
                        if c.arguments.len() != 1 {
                            return Err(
                                "ts_frontend: near.all takes exactly one array argument".into()
                            );
                        }
                        match c.arguments[0].as_expression() {
                            Some(Expression::ArrayExpression(arr)) => all_arr = Some(arr),
                            _ => {
                                return Err(
                                    "ts_frontend: near.all takes an ARRAY LITERAL of near.call(...) expressions"
                                        .into(),
                                )
                            }
                        }
                    }
                }
            }
        }
        if let Some(arr) = all_arr {
            if !matches!(decl.id, oxc_ast::ast::BindingPattern::ArrayPattern(_)) {
                return Err(
                    "ts_frontend: `await near.all([...])` binds with an array pattern: `const [a, b] = await near.all([...])`"
                        .into(),
                );
            }
            if arr.elements.len() != names.len() {
                return Err(format!(
                    "ts_frontend: near.all — {} name(s) bound but {} call(s) in the array",
                    names.len(),
                    arr.elements.len()
                )
                .into());
            }
            awaits.push(AwaitPoint {
                idx: i,
                names,
                bind: Bind::Positional,
                src_expr: arg,
            });
        } else if names.len() > 1 {
            return Err(
                "ts_frontend: only `await near.all([...])` binds multiple names".into(),
            );
        } else {
            let bind = match &decl.id {
                oxc_ast::ast::BindingPattern::BindingIdentifier(_) => Bind::Plain,
                oxc_ast::ast::BindingPattern::ObjectPattern(_) => Bind::Object,
                _ => {
                    return Err(
                        "ts_frontend: `await near.call(...)` binds a name or object pattern"
                            .into(),
                    )
                }
            };
            awaits.push(AwaitPoint {
                idx: i,
                names,
                bind,
                src_expr: arg,
            });
        }
    }
    if awaits.is_empty() {
        return Err(
            "ts_frontend: async function must contain `const x = await near.call(...);`".into(),
        );
    }

    // ── T4 frame rule ───────────────────────────────────────────────
    // Params cross every await (entry snapshot). Await results are bound
    // at their await and FRAME-PERSISTED for all later continuations (each
    // resume storage_sets its results; later resumes restore them — the
    // join-at-the-end pattern is the point). Plain `let` locals die at the
    // await boundary: reading one after an await, or assigning to a
    // param/pre-await local/await result after an await, is a COMPILE
    // ERROR naming the variable.
    let param_set: Vec<String> = param_names.iter().map(|(n, _)| n.clone()).collect();
    let n_awaits = awaits.len();
    // region 0 = entry (stmts before await 0), region k = resume k-1's stmts
    let mut region_stmts: Vec<&[Statement<'_>]> = Vec::new();
    region_stmts.push(&stmts[0..awaits[0].idx]);
    for k in 0..n_awaits {
        let start = awaits[k].idx + 1;
        let end = if k + 1 < n_awaits {
            awaits[k + 1].idx
        } else {
            stmts.len()
        };
        region_stmts.push(&stmts[start..end]);
    }
    let mut region_decls: Vec<Vec<String>> = Vec::new();
    for rs in &region_stmts {
        let mut d = Vec::new();
        for s in *rs {
            if let Statement::VariableDeclaration(vd) = s {
                for dd in &vd.declarations {
                    d.extend(pattern_names(&dd.id)?);
                }
            }
        }
        region_decls.push(d);
    }
    // await-result owner table: name → await index. Result of await j is
    // live in regions j+1..=n (bound locally at j+1, frame-restored after).
    let mut await_owner: Vec<(String, usize)> = Vec::new();
    for (j, a) in awaits.iter().enumerate() {
        for nm in &a.names {
            await_owner.push((nm.clone(), j));
        }
    }
    let owner_of =
        |n: &str| await_owner.iter().find(|(nm, _)| nm == n).map(|(_, j)| *j);
    for k in 0..region_stmts.len() {
        let mut reads: Vec<String> = Vec::new();
        let mut assigns: Vec<String> = Vec::new();
        for s in region_stmts[k] {
            stmt_walk_idents(s, &mut reads, &mut assigns);
        }
        let decl_here = &region_decls[k];
        // redeclare: a region cannot rebind an await-result name — the
        // resume's let would double-bind the frame restore
        for d in decl_here {
            if owner_of(d).is_some() {
                return Err(format!(
                    "ts_frontend: async `{name}`: cannot redeclare `{d}` — await results are frame-persisted for later continuations; bind a fresh name"
                )
                .into());
            }
        }
        for a in &assigns {
            if owner_of(a).is_some() {
                return Err(format!(
                    "ts_frontend: async `{name}`: `{a}` is an await result — await results are const; bind a new name"
                )
                .into());
            }
        }
        if k >= 1 {
            // assignment targets first (sharper message than a read error)
            for a in &assigns {
                if param_set.contains(a) {
                    return Err(format!(
                        "ts_frontend: async `{name}`: cannot assign to parameter `{a}` after an await — the frame snapshot happens in the entry; make `{a}` final before the first await"
                    )
                    .into());
                }
                if (0..k).any(|j| region_decls[j].contains(a)) {
                    return Err(format!(
                        "ts_frontend: async `{name}`: cannot assign to `{a}` after an await — locals die at the await boundary; declare `{a}` inside this continuation segment"
                    )
                    .into());
                }
            }
        }
        for r in &reads {
            if assigns.contains(r) || decl_here.contains(r) || param_set.contains(r) {
                continue;
            }
            if let Some(j) = owner_of(r) {
                if j + 1 <= k {
                    continue; // bound this resume (j+1==k) or frame-restored (j+1<k)
                }
                return Err(format!(
                    "ts_frontend: async `{name}`: await result `{r}` used before its await runs — move the await earlier or restructure"
                )
                .into());
            }
            if k >= 1 && (0..k).any(|j| region_decls[j].contains(r)) {
                return Err(format!(
                    "ts_frontend: async `{name}`: local `{r}` crosses an await boundary — only parameters and await results survive; make `{r}` a parameter or recompute it after the await"
                )
                .into());
            }
        }
    }

    // ── fire forms: one per await, built once, moved into their body ──
    // (near/call T M A G DEP) → (near/promise_create T M A DEP G)
    let to_pcreate = |items: &[LispVal], ctx: &str| -> Result<LispVal, String> {
        if items.len() != 6 || items[0] != Sym("near/call") {
            return Err(format!(
                "ts_frontend: {ctx} must be near.call(target, method, args, gas, deposit)"
            ));
        }
        // TS surface types deposit as number; the promise ABI is a u128
        // decimal STR (unified 2026-10-07 — see the promise_create typing
        // row). Coerce literals; Str deposits (near.attachedDeposit(),
        // decimal amounts) pass through untouched.
        let dep = match &items[5] {
            LispVal::Num(n) => Str(n.to_string()),
            other => other.clone(),
        };
        Ok(list(vec![
            Sym("near/promise_create"),
            items[1].clone(),
            items[2].clone(),
            items[3].clone(),
            dep,
            items[4].clone(),
        ]))
    };
    let fire = |create: LispVal, cb: &str| -> LispVal {
        list(vec![
            Sym("near/promise_return"),
            list(vec![
                Sym("near/promise_then"),
                create,
                list(vec![Sym("near/current_account_id")]),
                Str(cb.to_string()),
                Str("{}".to_string()),
                Str("0".to_string()),
                Num(CB_GAS),
            ]),
        ])
    };
    let cb_name = |k: usize| -> String {
        if n_awaits == 1 {
            format!("{name}__resume")
        } else {
            format!("{name}__resume{k}")
        }
    };
    let mut fire_forms: Vec<LispVal> = Vec::with_capacity(n_awaits);
    for (k, ap) in awaits.iter().enumerate() {
        match &ap.bind {
            Bind::Positional => {
                let arr = match &ap.src_expr {
                    Expression::CallExpression(c) => match c.arguments[0].as_expression() {
                        Some(Expression::ArrayExpression(arr)) => arr,
                        _ => unreachable!("checked at scan"),
                    },
                    _ => unreachable!("checked at scan"),
                };
                let mut creates: Vec<LispVal> = Vec::new();
                for (j, el) in arr.elements.iter().enumerate() {
                    let Some(elex) = el.as_expression() else {
                        return Err(format!(
                            "ts_frontend: near.all element {j} is empty (`[a, , b]` holes are not calls)"
                        ));
                    };
                    let l = lower_expr(elex)?;
                    let LispVal::List(items) = &l else {
                        return Err(format!(
                            "ts_frontend: near.all element {j} must be near.call(target, method, args, gas, deposit)"
                        ));
                    };
                    creates.push(to_pcreate(
                        items,
                        &format!("near.all element {j}"),
                    )?);
                }
                let joined = list({
                    let mut v = vec![Sym("near/promise_and")];
                    v.extend(creates);
                    v
                });
                fire_forms.push(fire(joined, &cb_name(k)));
            }
            _ => {
                let l = lower_expr(ap.src_expr)?;
                let LispVal::List(items) = l else {
                    return Err(format!(
                        "ts_frontend: await must wrap near.call(target, method, args, gas, deposit) — async `{name}`"
                    ));
                };
                let create = to_pcreate(&items, "await expression")?;
                fire_forms.push(fire(create, &cb_name(k)));
            }
        }
    }

    // ── entry: param reads from tx json, pre-await stmts, snapshot, fire ──
    let save_bindings: Vec<LispVal> = param_names
        .iter()
        .map(|(nm, kind)| {
            let v = if *kind == 1 {
                list(vec![Sym("to-string"), Sym(nm.clone())])
            } else {
                Sym(nm.clone())
            };
            list(vec![
                Sym("near/storage_set"),
                Str(format!("{}:{}", state_key, nm)),
                v,
            ])
        })
        .collect();
    // Tail of the entry = snapshot + fire await 0. Composed THROUGH the
    // pre-await statements (lower_prefix_around) so the first await's
    // near.call arguments can reference pre-await locals.
    let mut entry_tail_items = save_bindings;
    entry_tail_items.push(fire_forms[0].clone());
    let entry_tail = if entry_tail_items.len() == 1 {
        entry_tail_items.pop().unwrap()
    } else {
        let mut b = vec![Sym("begin")];
        b.extend(entry_tail_items);
        list(b)
    };
    // Compose pre-await statements with a tail that may FIRE the next
    // await. Early `return`s inside bind function-level __fn_done/__fn_res
    // locally (mirroring lower_block_tail's flag-guard path) and SUPPRESS
    // the tail-fire — a returned function must not keep the promise chain
    // going. Segment value: __fn_res on early return (done_value), else
    // the tail's value.
    let compose_seg =
        |pre: &[Statement<'_>], tail: LispVal, done_value: LispVal| -> Result<LispVal, String> {
            if pre.is_empty() {
                return Ok(tail);
            }
            if !stmts_have_deep_return(pre) {
                return lower_prefix_around(pre, tail, false);
            }
            let guarded = list(vec![
                Sym("if"),
                list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                tail,
                done_value,
            ]);
            let saved = FN_FLAGS_BOUND.with(|f| f.replace(true));
            let inner = lower_prefix_around_with_return(pre, guarded, false);
            FN_FLAGS_BOUND.with(|f| f.set(saved));
            Ok(list(vec![
                Sym("let"),
                list(vec![
                    list(vec![Sym("__fn_done"), Num(0)]),
                    list(vec![Sym("__fn_res"), list(vec![Sym("quote"), LispVal::Nil])]),
                ]),
                inner?,
            ]))
        };
    let pre = &stmts[0..awaits[0].idx];
    let entry_seq = compose_seg(pre, entry_tail, Sym("__fn_res"))?;
    let entry_body = if param_names.is_empty() {
        entry_seq
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
        list(vec![Sym("let"), list(bindings), entry_seq])
    };
    let entry_define = list(vec![
        Sym("define"),
        list(vec![Sym(name.clone())]),
        entry_body,
    ]);
    let view = name.starts_with("get_");
    let entry_export = list(vec![
        Sym("export"),
        Str(name.clone()),
        Sym(name.clone()),
        if view { Sym("#t") } else { Sym("#f") },
    ]);

    // ── resumes: restore frame + bind await results + segment (+ fire) ──
    let mut out: Vec<LispVal> = vec![entry_define, entry_export];
    for k in 0..n_awaits {
        let cbk = cb_name(k);
        let mut let_bindings: Vec<LispVal> = Vec::new();
        for (nm, kind) in &param_names {
            let getter = list(vec![
                Sym("default"),
                list(vec![
                    Sym("near/storage_get"),
                    Str(format!("{}:{}", state_key, nm)),
                ]),
                Str(String::new()),
            ]);
            let val = if *kind == 1 {
                list(vec![Sym("str->num"), getter])
            } else {
                getter
            };
            let_bindings.push(list(vec![Sym(nm.clone()), val]));
        }
        let ap = &awaits[k];
        for (j, nm) in ap.names.iter().enumerate() {
            let res = list(vec![Sym("near/promise_result"), Num(j as i64)]);
            let v = match ap.bind {
                Bind::Object => {
                    // const {a, b} = await near.call(...) — property reads
                    // on the returned JSON text
                    list(vec![Sym("json-get-str"), Str(nm.clone()), res])
                }
                _ => res, // Plain: index 0; Positional: index = position
            };
            let_bindings.push(list(vec![Sym(nm.clone()), v]));
        }
        // restore every EARLIER await's results (each was storage_set by its
        // own resume; default "" keeps a failed/missing result readable)
        for (j, aj) in awaits.iter().enumerate() {
            if j >= k {
                continue;
            }
            for nm in &aj.names {
                let getter = list(vec![
                    Sym("default"),
                    list(vec![
                        Sym("near/storage_get"),
                        Str(format!("{}:{}", state_key, nm)),
                    ]),
                    Str(String::new()),
                ]);
                let_bindings.push(list(vec![Sym(nm.clone()), getter]));
            }
        }
        let seg = region_stmts[k + 1];
        // Tail = persist THIS await's results for later resumes (the resume's
        // let binds them), then fire the next await — composed THROUGH the
        // segment via lower_prefix_around so the tail sees segment locals
        // (`const dep = ...` before an await is common and legal).
        let mut tail_items: Vec<LispVal> = Vec::new();
        if k + 1 < n_awaits {
            for nm in &ap.names {
                tail_items.push(list(vec![
                    Sym("near/storage_set"),
                    Str(format!("{}:{}", state_key, nm)),
                    Sym(nm.clone()),
                ]));
            }
            tail_items.push(fire_forms[k + 1].clone());
        }
        let tail = match tail_items.len() {
            1 => tail_items.pop().unwrap(),
            0 => Num(0),
            _ => {
                let mut b = vec![Sym("begin")];
                b.extend(tail_items);
                list(b)
            }
        };
        // Segment composition mirrors the entry: early `return`s inside the
        // segment bind LOCAL __fn_done/__fn_res and suppress the tail-fire;
        // the resume exports the last segment's value with a plain-value
        // export (the __resume contract has no flag channel — V1 same).
        let done_value = if k + 1 < n_awaits {
            Num(0)
        } else {
            Sym("__fn_res")
        };
        let body_inner = if seg.is_empty() {
            tail
        } else if !stmts_have_deep_return(seg) {
            lower_prefix_around(seg, tail, false)?
        } else {
            let guarded = list(vec![
                Sym("if"),
                list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                tail,
                done_value,
            ]);
            let saved = FN_FLAGS_BOUND.with(|f| f.replace(true));
            let inner = lower_prefix_around_with_return(seg, guarded, false);
            FN_FLAGS_BOUND.with(|f| f.set(saved));
            list(vec![
                Sym("let"),
                list(vec![
                    list(vec![Sym("__fn_done"), Num(0)]),
                    list(vec![
                        Sym("__fn_res"),
                        list(vec![Sym("quote"), LispVal::Nil]),
                    ]),
                ]),
                inner?,
            ])
        };
        out.push(list(vec![
            Sym("define"),
            list(vec![Sym(cbk.clone())]),
            list(vec![Sym("let"), list(let_bindings), body_inner]),
        ]));
        out.push(list(vec![
            Sym("export"),
            Str(cbk),
            Sym(cb_name(k)),
            Sym("#f"),
        ]));
    }

    Ok(out)
}

/// Every binding NAME in a declaration pattern — flat, positional.
/// `x` → [x]; `{a, b}` → [a, b]; `[p, q]` → [p, q]. Holes/rest/nested
/// patterns are refused (fail-loud, name the syntax).
fn pattern_names(p: &oxc_ast::ast::BindingPattern<'_>) -> Result<Vec<String>, String> {
    use oxc_ast::ast::BindingPattern::*;
    match p {
        BindingIdentifier(b) => Ok(vec![b.name.as_str().to_string()]),
        ObjectPattern(op) => {
            let mut out = Vec::new();
            for prop in &op.properties {
                out.extend(pattern_names(&prop.value)?);
            }
            if let Some(rest) = &op.rest {
                return Err(format!(
                    "ts_frontend: rest element `{}` unsupported in await bindings",
                    match &rest.argument {
                        BindingIdentifier(b) => b.name.as_str().to_string(),
                        _ => "...".to_string(),
                    }
                ));
            }
            Ok(out)
        }
        ArrayPattern(ap) => {
            let mut out = Vec::new();
            for el in &ap.elements {
                match el {
                    Some(e) => out.extend(pattern_names(e)?),
                    None => {
                        return Err(
                            "ts_frontend: near.all bindings cannot have holes (`[a, , b]`)"
                                .into(),
                        )
                    }
                }
            }
            if let Some(rest) = &ap.rest {
                return Err(
                    "ts_frontend: rest elements unsupported in await bindings".into(),
                );
            }
            Ok(out)
        }
        _ => Err("ts_frontend: unsupported await binding pattern".into()),
    }
}

/// Statement-level walker for the T4 await-frame rule: collects identifier
/// READS and bare-identifier ASSIGNMENT/UPDATE targets (recursing through
/// control flow). Arrow bodies are the lambda's own scope — skipped, same
/// policy as expr_idents.
fn stmt_walk_idents(
    s: &Statement<'_>,
    reads: &mut Vec<String>,
    assigns: &mut Vec<String>,
) {
    match s {
        Statement::ExpressionStatement(es) => walk_expr_idents(&es.expression, reads, assigns),
        Statement::VariableDeclaration(vd) => {
            for d in &vd.declarations {
                if let Some(init) = &d.init {
                    walk_expr_idents(init, reads, assigns);
                }
            }
        }
        Statement::BlockStatement(b) => {
            for st in &b.body {
                stmt_walk_idents(st, reads, assigns);
            }
        }
        Statement::IfStatement(i) => {
            walk_expr_idents(&i.test, reads, assigns);
            for st in stmts_of(&i.consequent) {
                stmt_walk_idents(st, reads, assigns);
            }
            if let Some(alt) = &i.alternate {
                for st in stmts_of(alt) {
                    stmt_walk_idents(st, reads, assigns);
                }
            }
        }
        Statement::WhileStatement(w) => {
            walk_expr_idents(&w.test, reads, assigns);
            for st in stmts_of(&w.body) {
                stmt_walk_idents(st, reads, assigns);
            }
        }
        Statement::DoWhileStatement(d) => {
            for st in stmts_of(&d.body) {
                stmt_walk_idents(st, reads, assigns);
            }
            walk_expr_idents(&d.test, reads, assigns);
        }
        Statement::ForStatement(fs) => {
            if let Some(init) = &fs.init {
                match init {
                    oxc_ast::ast::ForStatementInit::VariableDeclaration(vd) => {
                        for dd in &vd.declarations {
                            if let Some(init) = &dd.init {
                                walk_expr_idents(init, reads, assigns);
                            }
                        }
                    }
                    other => {
                        if let Some(e) = other.as_expression() {
                            walk_expr_idents(e, reads, assigns);
                        }
                    }
                }
            }
            if let Some(test) = &fs.test {
                walk_expr_idents(test, reads, assigns);
            }
            if let Some(update) = &fs.update {
                walk_expr_idents(update, reads, assigns);
            }
            for st in stmts_of(&fs.body) {
                stmt_walk_idents(st, reads, assigns);
            }
        }
        Statement::ReturnStatement(r) => {
            if let Some(a) = &r.argument {
                walk_expr_idents(a, reads, assigns);
            }
        }
        _ => {}
    }
}

/// Expression walker: reads via the proven expr_idents, plus a side-walk
/// for assignment/update TARGETS (expr_idents conflates them into reads —
/// the T4 rule wants the sharper "cannot assign" message).
fn walk_expr_idents(e: &Expression<'_>, reads: &mut Vec<String>, assigns: &mut Vec<String>) {
    use oxc_ast::ast::AssignmentTarget;
    expr_idents(e, reads);
    fn aw(e: &Expression<'_>, assigns: &mut Vec<String>) {
        match e {
            Expression::AssignmentExpression(a) => {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                    assigns.push(id.name.as_str().to_string());
                }
                aw(&a.right, assigns);
            }
            Expression::UpdateExpression(u) => {
                if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) =
                    &u.argument
                {
                    assigns.push(id.name.as_str().to_string());
                }
            }
            Expression::ParenthesizedExpression(p) => aw(&p.expression, assigns),
            Expression::BinaryExpression(b) => {
                aw(&b.left, assigns);
                aw(&b.right, assigns);
            }
            Expression::LogicalExpression(l) => {
                aw(&l.left, assigns);
                aw(&l.right, assigns);
            }
            Expression::UnaryExpression(u) => aw(&u.argument, assigns),
            Expression::SequenceExpression(sq) => {
                for x in &sq.expressions {
                    aw(x, assigns);
                }
            }
            Expression::ConditionalExpression(c) => {
                aw(&c.test, assigns);
                aw(&c.consequent, assigns);
                aw(&c.alternate, assigns);
            }
            Expression::CallExpression(c) => {
                aw(&c.callee, assigns);
                for a in &c.arguments {
                    if let Argument::SpreadElement(_) = a {
                        continue;
                    }
                    if let Some(ae) = a.as_expression() {
                        aw(ae, assigns);
                    }
                }
            }
            Expression::StaticMemberExpression(m) => aw(&m.object, assigns),
            Expression::ComputedMemberExpression(m) => {
                aw(&m.object, assigns);
                aw(&m.expression, assigns);
            }
            Expression::TemplateLiteral(t) => {
                for x in &t.expressions {
                    aw(x, assigns);
                }
            }
            _ => {}
        }
    }
    aw(e, assigns);
}

/// Forward scan: register every `let x = <bigint-init>;` in a function
/// body BEFORE lowering. The statement lowering is CPS-style (continuations
/// lower before the statement itself), so registering at the let-site was
/// too late for later statements that reference the binding.
/// STRING_LOCALS uses the same forward scan: `let out = "";` must be
/// marked before the `out + x` binary-+ site lowers.
fn scan_bigint_lets(stmts: &[Statement<'_>]) {
    scan_input_handles(stmts);
    for s in stmts {
        scan_one_bigint_let(s);
    }
}

/// JSON API v3 (2026-09-15): forward-register `const o = near.input()`
/// handles — property reads on these names rewrite to the cached-input
/// getters, so the registration must exist before ANY statement lowers
/// (same CPS-ordering rationale as scan_one_bigint_let). Recurses into
/// blocks/ifs/loops (handles are per-function; lower_function clears).
fn scan_input_handles(stmts: &[Statement<'_>]) {
    for s in stmts {
        match s {
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if let Some(Expression::CallExpression(c)) = &d.init {
                        if let Expression::StaticMemberExpression(sm) = &c.callee {
                            if let Expression::Identifier(oid) = &sm.object {
                                if oid.name == "near" && sm.property.name == "input" {
                                    if let Ok(name) = binding_name(&d.id) {
                                        INPUT_HANDLES.with(|h| {
                                            h.borrow_mut().push(name);
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Statement::BlockStatement(b) => scan_input_handles(&b.body),
            Statement::IfStatement(i) => {
                scan_input_handles(stmts_of(&i.consequent));
                if let Some(alt) = &i.alternate {
                    scan_input_handles(stmts_of(alt));
                }
            }
            Statement::WhileStatement(w) => scan_input_handles(stmts_of(&w.body)),
            _ => {}
        }
    }
}

fn is_input_handle(n: &str) -> bool {
    INPUT_HANDLES.with(|h| h.borrow().iter().any(|x| x == n))
}

/// `near.input()` used as a declaration initializer: the handle is the
/// NAME (registered by scan_input_handles) — the binding itself is a dead
/// nil (property reads rewrite to input getters and never touch it).
fn init_is_input_handle(e: &Expression<'_>) -> bool {
    if let Expression::CallExpression(c) = e {
        if let Expression::StaticMemberExpression(sm) = &c.callee {
            if let Expression::Identifier(oid) = &sm.object {
                return oid.name == "near" && sm.property.name == "input";
            }
        }
    }
    false
}

fn scan_one_bigint_let(s: &Statement<'_>) {
    if let Statement::VariableDeclaration(v) = s {
        for d in &v.declarations {
            let Some(init) = &d.init else { continue };
            // purely-numeric string literals mean ARITHMETIC too (same rule as
            // the + dispatch's literal half, 2026-09-27): `let acc = "0";
            // acc = acc + u128Add(acc, x)` is a u128 accumulator, not a
            // concat — dual membership with STRING_LOCALS is expected; the
            // identifier arm of stringy_nonnumeric excludes BIGINT members.
            let numeric_str_init = matches!(init, Expression::StringLiteral(sl)
                if !sl.value.is_empty()
                    && std::str::from_utf8(sl.value.as_bytes())
                        .ok()
                        .and_then(|s| s.parse::<u128>().ok())
                        .is_some());
            if expr_is_bigint(init) || numeric_str_init {
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
fn lower_function(f: &TsFunction<'_>, exported: bool) -> Result<(String, Vec<LispVal>), String> {
    let name =
        f.id.as_ref()
            .map(|i| i.name.as_str().to_string())
            .ok_or("ts_frontend: anonymous functions unsupported (M1)")?;

    let mut params = Vec::new();
    // (name, kind): 0 = string, 1 = number, 2 = string[], 3 = object
    // (object = JSON-text binding; numeric props auto-decode on read)
    NUM_PARAM_NAMES.with(|s| s.borrow_mut().clear());
    OBJ_PARAM_PROPS.with(|s| s.borrow_mut().clear());
    OBJECT_PARAMS.with(|s| s.borrow_mut().clear());
    BIGINT_NAMES.with(|s| s.borrow_mut().clear());
    BIGINT_LOCALS.with(|s| s.borrow_mut().clear());
    STRING_LOCALS.with(|s| s.borrow_mut().clear());
    SHAPE_BIGINT_FIELDS.with(|s| s.borrow_mut().clear());
    MONEY_NAMES.with(|s| s.borrow_mut().clear());
    LEDGER_NAMES.with(|s| s.borrow_mut().clear());
    INPUT_HANDLES.with(|s| s.borrow_mut().clear());
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
        if ann_is_money_alias(p.type_annotation.as_deref()) {
            MONEY_NAMES.with(|s| s.borrow_mut().push(n.clone()));
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
    check_return_contract(
        &name,
        &body.statements,
        return_ann_money(f),
    )?;
    check_money_sinks(&body.statements)?;

    // forward-scan bigint lets (CPS lowering means let-site registration
    // runs after statements that reference the binding — see scan_bigint_lets)
    scan_bigint_lets(&body.statements);
    // F3: whole-function closure-safety analysis BEFORE lowering — the
    // lowering hoists/reorders statements, so lowering-time checks see
    // assignments/arrows out of order. Static pass sees the full body.
    check_fn_closure_safety(&body.statements)?;

    // view convention: get_* functions' returns become json_return_str
    // (the define tail value alone does not call value_return)
    let view = name.starts_with("get_");

    // Exported contracts read args from the transaction input JSON
    // (json_get_str pattern); `: number` annotations wrap str->num.
    // WASI/P2 mode: exported functions keep REAL params — the P2 _start
    // wrapper passes the stdin input string as the argument (the NEAR
    // near/json_get_str host op does not exist on the OutLayer runtime).
    let wasi_mode = TS_WASI_MODE.with(|m| m.get());
    let mut impl_define: Option<LispVal> = None;
    let expr = if exported && !wasi_mode {
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
            OBJECT_PARAMS.with(|s| s.borrow_mut().clear());
            // Raw-twin entry split (2026-10-03): when every param is
            // `number` and the return is `number`, emit
            //   (define (__impl_f x y) :: int int -> int BODY)
            //   (define (f) (let ((x (str->num ..)) ...) (__impl_f x y)))
            // The impl's annotation feeds fn_int_annotations → the raw-i64
            // twin fires, and the entry's call routes through the twin's
            // untag/retag boundary (call.rs raw-twin fast path). String/
            // array/object/bigint params keep the old inline-let shape.
            // Fail-closed: any body form outside the int subset kills the
            // twin at emit time and everything runs the tagged path, so
            // behavior can only match — never diverge.
            let ret_int = ts_ann_to_lisp(f.return_type.as_ref().map(|v| &**v)) == Some("int");
            if ret_int && param_names.iter().all(|(_, k)| *k == 1) {
                let impl_name = format!("__impl_{}", name);
                let mut impl_sig = vec![Sym(impl_name.clone())];
                for (n, _) in &param_names {
                    impl_sig.push(Sym(n.clone()));
                }
                let mut impl_items = vec![Sym("define"), list(impl_sig), Sym("::".to_string())];
                for _ in &param_names {
                    impl_items.push(Sym("int".to_string()));
                }
                impl_items.push(Sym("->".to_string()));
                impl_items.push(Sym("int".to_string()));
                impl_items.push(inner);
                impl_define = Some(list(impl_items));
                let mut call_items = vec![Sym(impl_name)];
                for (n, _) in &param_names {
                    call_items.push(Sym(n.clone()));
                }
                list(vec![Sym("let"), list(bindings), list(call_items)])
            } else {
                list(vec![Sym("let"), list(bindings), inner])
            }
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
    // The entry define stays under `name` (the export references it). When
    // the impl split fired, the impl define is emitted first so the twin
    // exists before any caller compiles.
    if let Some(impl_def) = impl_define {
        return Ok((name, vec![impl_def, list(define_items)]));
    }
    Ok((name, vec![list(define_items)]))
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

/// Any `return` reachable inside a loop (while/for/for-of), at any nesting
/// depth of ifs/blocks/loops. Such returns can only escape via the
/// function-level __fn_done/__fn_res flags — loop-local __wl_* bindings are
/// shadowed per level, so without the function flags the value vanishes
/// (nested-return bug, 2026-09-11: `while(..){ while(..){ return 77; } }`
/// returned the accumulator instead).
fn has_return_inside_loop(stmts: &[Statement<'_>]) -> bool {
    fn in_loop(s: &Statement<'_>) -> bool {
        match s {
            Statement::WhileStatement(w) => stmts_have_deep_return(stmts_of(&w.body)),
            Statement::DoWhileStatement(d) => stmts_have_deep_return(stmts_of(&d.body)),
            Statement::ForStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            Statement::ForOfStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            Statement::ForInStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            Statement::BlockStatement(b) => b.body.iter().any(in_loop),
            Statement::IfStatement(i) => {
                in_loop(&i.consequent) || i.alternate.as_ref().is_some_and(|a| in_loop(a))
            }
            _ => false,
        }
    }
    stmts.iter().any(in_loop)
}

/// while/for bodies with break/continue/return are not lowered — the
/// exit-protocol machinery exists only for for..of. Hard-error early so
/// the author (or the brain) gets an actionable message instead of
/// silently broken semantics (2026-10-05, M1.5 loops).
fn reject_loop_exits(stmts: &[Statement<'_>]) -> Result<(), String> {
    for s in stmts {
        match s {
            Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
            | Statement::ReturnStatement(_) => {
                return Err("ts_frontend: break/continue/return inside while/for are not supported yet — use a flag variable or recursion".into())
            }
            Statement::BlockStatement(b) => reject_loop_exits(&b.body)?,
            Statement::IfStatement(i) => {
                reject_loop_exits(stmts_of(&i.consequent))?;
                if let Some(alt) = &i.alternate {
                    reject_loop_exits(stmts_of(alt))?;
                }
            }
            Statement::WhileStatement(w) => reject_loop_exits(stmts_of(&w.body))?,
            Statement::ForStatement(f) => reject_loop_exits(stmts_of(&f.body))?,
            Statement::ForOfStatement(fo) => reject_loop_exits(stmts_of(&fo.body))?,
            _ => {}
        }
    }
    Ok(())
}

/// Lower `while (test) { body }` (no break/continue/return — caller must
/// reject_loop_exits first). Body lowers through lower_prefix_around with
/// a nil tail — plain statement chaining, no __fn_done protocol (works at
/// top level AND inside functions).
fn lower_while_form(w: &oxc_ast::ast::WhileStatement<'_>, view: bool) -> Result<LispVal, String> {
    reject_loop_exits(stmts_of(&w.body))?;
    let body = lower_prefix_around(stmts_of(&w.body), Num(0), view)?;
    Ok(list(vec![Sym("while"), truthy(&w.test)?, body]))
}

/// Lower `for (init; test; update) { body }` as
/// `(begin pre... (let* binds (while test body... update)))`.
/// let-scope covers the whole loop; the update runs as the loop's last
/// statement each iteration.
fn lower_for_form(f: &oxc_ast::ast::ForStatement<'_>, view: bool) -> Result<LispVal, String> {
    reject_loop_exits(stmts_of(&f.body))?;
    let mut binds: Vec<LispVal> = Vec::new();
    let mut pre: Vec<LispVal> = Vec::new();
    if let Some(init) = &f.init {
        match init {
            oxc_ast::ast::ForStatementInit::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let ie = d
                        .init
                        .as_ref()
                        .ok_or("ts_frontend: for-loop declaration needs initializer")?;
                    if expr_is_bigint(ie) {
                        BIGINT_LOCALS.with(|s| s.borrow_mut().push(name.clone()));
                    }
                    if expr_is_stringy(ie) || expr_is_str_method_call(ie) {
                        mark_string_local(&name);
                    }
                    binds.push(list(vec![Sym(name), lower_expr(ie)?]));
                }
            }
            other => {
                // INHERIT(Expression) — spread variants, use as_expression()
                if let Some(e) = other.as_expression() {
                    pre.push(effect_expr(e)?);
                } else {
                    return Err("ts_frontend: unsupported for-loop init (M1.5)".into());
                }
            }
        }
    }
    let test = match &f.test {
        Some(e) => truthy(e)?,
        None => Num(1),
    };
    let mut body = lower_prefix_around(stmts_of(&f.body), Num(0), view)?;
    if let Some(u) = &f.update {
        body = list(vec![Sym("begin"), body, effect_expr(u)?]);
    }
    let mut while_form = list(vec![Sym("while"), test, body]);
    if !binds.is_empty() {
        while_form = list(vec![Sym("let*"), list(binds), while_form]);
    }
    let mut seq = vec![Sym("begin")];
    seq.extend(pre);
    seq.push(while_form);
    Ok(list(seq))
}

/// Lower `do { body } while (test);` → (begin body (while test body)).
/// Body lowers twice (pre-run + loop) — semantics-faithful, no exit
/// protocol (reject_loop_exits applies).
fn lower_do_while_form(
    d: &oxc_ast::ast::DoWhileStatement<'_>,
    view: bool,
) -> Result<LispVal, String> {
    reject_loop_exits(stmts_of(&d.body))?;
    let body = lower_prefix_around(stmts_of(&d.body), Num(0), view)?;
    let test = truthy(&d.test)?;
    Ok(list(vec![
        Sym("begin"),
        body.clone(),
        list(vec![Sym("while"), test, body]),
    ]))
}

/// switch with all-break semantics (2026-10-05): lowers to an if-chain of
/// (= disc test) comparisons; default (at most one) becomes the final
/// else. Fallthrough is REJECTED — every non-empty case body must end in
/// break; empty case bodies (case a: case b: …) are rejected too (write
/// the body explicitly). return inside a case is rejected (use a flag
/// variable, assign, then return after the switch).
fn lower_switch_form(
    sw: &oxc_ast::ast::SwitchStatement<'_>,
    view: bool,
) -> Result<LispVal, String> {
    let disc = lower_expr(&sw.discriminant)?;
    let has_default = sw.cases.iter().any(|c| c.test.is_none());
    // validate: exits, fallthrough, empties, dup defaults
    let mut defaults = 0;
    for c in &sw.cases {
        if c.test.is_none() {
            defaults += 1;
            if defaults > 1 {
                return Err("ts_frontend: switch: multiple default cases".into());
            }
        }
        if c.consequent.is_empty() {
            return Err(
                "ts_frontend: switch: empty case body = fallthrough (not supported) — write the body explicitly in each case"
                    .into(),
            );
        }
        for st in &c.consequent {
            if matches!(st, Statement::ReturnStatement(_)) {
                return Err(
                    "ts_frontend: switch: return inside a case is not supported — assign a result variable, break, then return after the switch"
                        .into(),
                );
            }
        }
        let last_break = matches!(c.consequent.last(), Some(Statement::BreakStatement(_)));
        if !last_break {
            return Err(
                "ts_frontend: switch: every case body must end with `break;` (fallthrough not supported)"
                    .into(),
            );
        }
    }
    // build the chain from the LAST case backwards; default becomes else
    let mut chain: Option<LispVal> = None;
    for c in sw.cases.iter().rev() {
        let body_stmts = &c.consequent[..c.consequent.len() - 1]; // strip trailing break
        let body = lower_prefix_around(body_stmts, Num(0), view)?;
        match &c.test {
            Some(t) => {
                let test = lower_expr(t)?;
                let cond = list(vec![Sym("="), disc.clone(), test]);
                let else_arm = chain.take().unwrap_or(Num(0));
                chain = Some(list(vec![Sym("if"), cond, body, else_arm]));
            }
            None => {
                // default: claims the final else position; later
                // (higher-up) cases chain on top of it
                chain = Some(body);
            }
        }
    }
    let _ = has_default;
    Ok(chain.unwrap_or(Num(0)))
}

/// Any `return` anywhere below these statements (loops, ifs, blocks).
fn stmts_have_deep_return(stmts: &[Statement<'_>]) -> bool {
    fn deep(s: &Statement<'_>) -> bool {
        match s {
            Statement::ReturnStatement(_) => true,
            Statement::BlockStatement(b) => b.body.iter().any(deep),
            Statement::IfStatement(i) => {
                deep(&i.consequent) || i.alternate.as_ref().is_some_and(|a| deep(a))
            }
            Statement::WhileStatement(w) => stmts_have_deep_return(stmts_of(&w.body)),
            Statement::DoWhileStatement(d) => stmts_have_deep_return(stmts_of(&d.body)),
            Statement::ForStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            Statement::ForOfStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            Statement::ForInStatement(f) => stmts_have_deep_return(stmts_of(&f.body)),
            _ => false,
        }
    }
    stmts.iter().any(deep)
}

/// Lower a statement list whose value is the tail expression.
fn lower_block_tail(stmts: &[Statement<'_>], view: bool) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(Num(0));
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    // Any `return` in a non-tail statement (directly, in an if branch, or
    // inside a loop at any depth) can only escape through the function-
    // level __fn_done/__fn_res flags — loop-local __wl_* are shadowed per
    // nesting level and a nested return's value vanishes without them
    // (2026-09-11). Tail-statement returns are plain value semantics.
    if !stmts_have_deep_return(init) {
        let tail = lower_tail_stmt(&last[0], view)?;
        return lower_prefix_around(init, tail, view);
    }
    let guarded_tail = |tail: LispVal| {
        list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
            tail,
            fn_exit_form(view),
        ])
    };
    if FN_FLAGS_BOUND.with(|f| f.get()) {
        // An enclosing context (function body, if-branch) already bound the
        // flags — NEVER shadow them: set!s and guards must resolve outward,
        // or a nested return sets the shadow and the value vanishes when
        // this block ends.
        let tail = lower_tail_stmt(&last[0], view)?;
        let body = lower_prefix_around_with_return(init, guarded_tail(tail), view)?;
        return Ok(body);
    }
    // early-return function: flag-guard lowering (M2).
    // __fn_res starts as nil (bottom type — accepts str/num set!s).
    let saved = FN_FLAGS_BOUND.with(|f| f.replace(true));
    let result = (|| {
        let tail = lower_tail_stmt(&last[0], view)?;
        let body = lower_prefix_around_with_return(init, guarded_tail(tail), view)?;
        Ok(list(vec![
            Sym("let"),
            list(vec![
                list(vec![Sym("__fn_done"), Num(0)]),
                list(vec![
                    Sym("__fn_res"),
                    list(vec![Sym("quote"), LispVal::Nil]),
                ]),
            ]),
            body,
        ]))
    })();
    FN_FLAGS_BOUND.with(|f| f.set(saved));
    result
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
            // JSON API v3: `const {..} = near.args<{..}>()` — typed
            // single-pass binding replaces the whole declaration
            if let Some(res) = lower_args_destructuring(v) {
                let binds = res?;
                return lower_prefix_around_with_return(
                    init,
                    list(vec![Sym("let"), list(binds), tail]),
                    view,
                );
            }
            let mut bindings = Vec::new();
            let mut guarded_inits = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                if init_is_input_handle(init_e) {
                    // near.input() handle: dead nil binding (name-level)
                    bindings.push(list(vec![
                        Sym(name),
                        list(vec![Sym("quote"), LispVal::Nil]),
                    ]));
                    continue;
                }
                if expr_is_bigint(init_e) {
                    BIGINT_LOCALS.with(|s| s.borrow_mut().push(name.clone()));
                }
                if expr_is_stringy(init_e) || expr_is_str_method_call(init_e) {
                    mark_string_local(&name);
                }
                check_and_register_money_const(
                    &name,
                    declarator_ann_is_money(d),
                    init_e,
                )?;
                if expr_has_call(init_e) {
                    // Impure initializer — hoist the binding with a nil dummy
                    // (unifies with any type per the checker), then guard the
                    // real init behind __fn_done. The set! establishes the
                    // real type at runtime.
                    // (Bug 2: `const b = writeAndReturn(a)` wrote storage
                    // even after an early return set __fn_done = 1)
                    //
                    // u128 Level 1 (2026-09-15): bigint-shaped initializers
                    // hoist with a "0" dummy instead of nil — the nil poisoned
                    // limb-local eligibility (every store must be u128-pure)
                    // and killed the optimization for the most common shape
                    // (loop accumulators). The dummy is never observed: TDZ
                    // guarantees the guarded set! runs before any read.
                    let lowered_init = lower_expr(init_e)?;
                    let dummy = if init_is_u128_pure(&lowered_init) {
                        Str("0".to_string())
                    } else {
                        list(vec![Sym("quote"), LispVal::Nil])
                    };
                    bindings.push(list(vec![Sym(name.clone()), dummy]));
                    guarded_inits.push(list(vec![
                        Sym("if"),
                        list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                        list(vec![Sym("set!"), Sym(name), lowered_init]),
                        Num(0),
                    ]));
                } else {
                    // pure binding — no guard needed
                    bindings.push(list(vec![Sym(name), lower_expr(init_e)?]));
                }
            }
            if guarded_inits.is_empty() {
                // all pure — same as before
                list(vec![Sym("let"), list(bindings), tail])
            } else {
                // has impure inits — bind nil dummies, then guarded set!s,
                // then the tail
                let mut begin_items = vec![Sym("begin")];
                begin_items.extend(guarded_inits);
                begin_items.push(tail);
                list(vec![Sym("let"), list(bindings), list(begin_items)])
            }
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
            // Commit-at-site (2026-09-17): if the branch's own tail already
            // carries the __fn_res commit (conditional-carrier lowering),
            // do NOT wrap it again — a blanket capture would commit
            // __fn_done on the branch's FALL-THROUGH path too, killing
            // every statement after this if. Direct returns and nested
            // carriers keep the blanket only when the branch is a
            // guaranteed return (its value IS the function result).
            //
            // (2026-10-06) BUT is_commit_form scans the WHOLE branch form:
            // a PREFIX early-return carrier (`{ if (g) return "x";
            // return "B"; }`) also trips the exemption, and the guarded
            // tail's "B" is then discarded in statement position — the
            // return silently vanishes (found via the discard_normalize
            // probe: old code rejected this shape at typecheck, masking
            // the lowering hole). Force the blanket when the branch ENDS
            // in a direct return: the branch is a guaranteed return, and
            // the tail commit survives via the __fn_done guard.
            let then_tail_return = matches!(
                stmts_of(&i.consequent).last(),
                Some(Statement::ReturnStatement(_))
            );
            if (stmt_has_return(&i.consequent) && !is_commit_form(&then_e)) || then_tail_return {
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
                    let alt_tail_return =
                        matches!(stmts_of(alt).last(), Some(Statement::ReturnStatement(_)));
                    if (stmt_has_return(alt) && !is_commit_form(&e)) || alt_tail_return {
                        e = list(vec![
                            Sym("begin"),
                            list(vec![Sym("set!"), Sym("__fn_res"), e]),
                            list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                        ]);
                    }
                    e
                }
                // Same bottom-type idiom as lower_tail_stmt's if-arm: a
                // statement-if with no else is not a numeric 0 value.
                // (2026-09-17: `else { if (c) { return v; } }` reached this
                // None arm with a str-branch inside → str ≠ int false reject)
                None => list(vec![Sym("quote"), LispVal::Nil]),
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
            let mut v = vec![Sym("begin")];
            if has_exits {
                // bind the loop-local flags (the core references them) and
                // guard the whole loop: an earlier bare return must not run it
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    list(vec![
                        Sym("let"),
                        list(vec![
                            list(vec![Sym("__wl_done"), Num(0)]),
                            list(vec![Sym("__wl_brk"), Num(0)]),
                            list(vec![Sym("__wl_ret"), Num(0)]),
                            list(vec![
                                Sym("__wl_res"),
                                list(vec![Sym("quote"), LispVal::Nil]),
                            ]),
                        ]),
                        list(vec![
                            Sym("begin"),
                            core,
                            // loop return feeds the function-level flag too
                            list(vec![
                                Sym("if"),
                                Sym("__wl_ret"),
                                list(vec![
                                    Sym("begin"),
                                    list(vec![Sym("set!"), Sym("__fn_res"), Sym("__wl_res")]),
                                    list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                                    Num(0),
                                ]),
                                Num(0),
                            ]),
                        ]),
                    ]),
                    Num(0),
                ]));
            } else {
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    core,
                    Num(0),
                ]));
            }
            v.push(tail);
            list(v)
        }
        Statement::ForStatement(fr) => {
            let (has_exits, core) = lower_for_parts(fr)?;
            let mut v = vec![Sym("begin")];
            if has_exits {
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    list(vec![
                        Sym("let"),
                        list(vec![
                            list(vec![Sym("__wl_done"), Num(0)]),
                            list(vec![Sym("__wl_brk"), Num(0)]),
                            list(vec![Sym("__wl_ret"), Num(0)]),
                            list(vec![
                                Sym("__wl_res"),
                                list(vec![Sym("quote"), LispVal::Nil]),
                            ]),
                        ]),
                        list(vec![
                            Sym("begin"),
                            core,
                            list(vec![
                                Sym("if"),
                                Sym("__wl_ret"),
                                list(vec![
                                    Sym("begin"),
                                    list(vec![Sym("set!"), Sym("__fn_res"), Sym("__wl_res")]),
                                    list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                                    Num(0),
                                ]),
                                Num(0),
                            ]),
                        ]),
                    ]),
                    Num(0),
                ]));
            } else {
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    core,
                    Num(0),
                ]));
            }
            v.push(tail);
            list(v)
        }
        Statement::ForOfStatement(fo) => {
            let (has_exits, core) = lower_for_of_parts(fo)?;
            let mut v = vec![Sym("begin")];
            if has_exits {
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    list(vec![
                        Sym("let"),
                        list(vec![
                            list(vec![Sym("__wl_done"), Num(0)]),
                            list(vec![Sym("__wl_brk"), Num(0)]),
                            list(vec![Sym("__wl_ret"), Num(0)]),
                            list(vec![
                                Sym("__wl_res"),
                                list(vec![Sym("quote"), LispVal::Nil]),
                            ]),
                        ]),
                        list(vec![
                            Sym("begin"),
                            core,
                            list(vec![
                                Sym("if"),
                                Sym("__wl_ret"),
                                list(vec![
                                    Sym("begin"),
                                    list(vec![Sym("set!"), Sym("__fn_res"), Sym("__wl_res")]),
                                    list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                                    Num(0),
                                ]),
                                Num(0),
                            ]),
                        ]),
                    ]),
                    Num(0),
                ]));
            } else {
                v.push(list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    core,
                    Num(0),
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
fn lower_prefix_around(
    stmts: &[Statement<'_>],
    tail: LispVal,
    view: bool,
) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(tail);
    }
    let (init, last) = stmts.split_at(stmts.len() - 1);
    let inner = match &last[0] {
        // NOTE: WhileStatement/ForStatement have dedicated exit-protocol
        // arms lower in this match (lower_while_parts / lower_for_parts,
        // break/continue/return capable) — top-level loops are the ones
        // handled separately in lower_program (M1.5, 2026-10-05).
        Statement::DoWhileStatement(d) => {
            // M1.5+ (2026-10-05)
            let form = lower_do_while_form(d, view)?;
            list(vec![Sym("begin"), form, tail])
        }
        Statement::SwitchStatement(sw) => {
            // M1.5+ (2026-10-05): if-chain, no exit protocol
            let form = lower_switch_form(sw, view)?;
            list(vec![Sym("begin"), form, tail])
        }
        Statement::VariableDeclaration(v) => {
            // JSON API v3: `const {..} = near.args<{..}>()`
            if let Some(res) = lower_args_destructuring(v) {
                let binds = res?;
                return lower_prefix_around(init, list(vec![Sym("let*"), list(binds), tail]), view);
            }
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                if init_is_input_handle(init_e) {
                    bindings.push(list(vec![
                        Sym(name),
                        list(vec![Sym("quote"), LispVal::Nil]),
                    ]));
                    continue;
                }
                if expr_is_bigint(init_e) {
                    BIGINT_LOCALS.with(|s| s.borrow_mut().push(name.clone()));
                }
                if expr_is_stringy(init_e) || expr_is_str_method_call(init_e) {
                    mark_string_local(&name);
                }
                check_and_register_money_const(
                    &name,
                    declarator_ann_is_money(d),
                    init_e,
                )?;
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
                        list(vec![Sym("__wl_brk"), Num(0)]),
                        list(vec![Sym("__wl_ret"), Num(0)]),
                        list(vec![
                            Sym("__wl_res"),
                            list(vec![Sym("quote"), LispVal::Nil]),
                        ]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_ret"), Num(0)]),
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
                        list(vec![Sym("__wl_brk"), Num(0)]),
                        list(vec![Sym("__wl_ret"), Num(0)]),
                        list(vec![
                            Sym("__wl_res"),
                            list(vec![Sym("quote"), LispVal::Nil]),
                        ]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_ret"), Num(0)]),
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
                        list(vec![Sym("__wl_brk"), Num(0)]),
                        list(vec![Sym("__wl_ret"), Num(0)]),
                        list(vec![
                            Sym("__wl_res"),
                            list(vec![Sym("quote"), LispVal::Nil]),
                        ]),
                    ]),
                    list(vec![
                        Sym("begin"),
                        core,
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_ret"), Num(0)]),
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
    let LispVal::List(items) = v else {
        return false;
    };
    let Some(LispVal::Sym(head)) = items.first() else {
        return false;
    };
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
            let cond = truthy(&i.test)?;
            // Conditional carrier (2026-09-17): `if (c) { return v; }` in a
            // with-return context must commit __fn_res/__fn_done AT the
            // return site — a fall-through path leaves the flags alone. A
            // plain value-if here made the parent's blanket capture commit
            // __fn_done on fall-through (statements after the if never ran).
            // Flags not bound (single-exit value semantics) → plain value-if.
            if i.alternate.is_none()
                && stmt_has_return(&i.consequent)
                && FN_FLAGS_BOUND.with(|f| f.get())
            {
                let commit = if is_commit_form(&then_e) {
                    then_e
                } else {
                    list(vec![
                        Sym("begin"),
                        list(vec![Sym("set!"), Sym("__fn_res"), then_e]),
                        list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]),
                    ])
                };
                return Ok(list(vec![
                    Sym("if"),
                    cond,
                    commit,
                    list(vec![Sym("quote"), LispVal::Nil]),
                ]));
            }
            let else_e = match &i.alternate {
                Some(alt) => lower_block_tail(stmts_of(alt), view)?,
                // Missing else = statement-if, NOT a 0 value: `if (c) { return s; }`
                // lowered to (if c <str> 0) and the checker rejected str ≠ int —
                // legal TS (branch falls through). Nil is bottom (unify unifies it
                // with anything, same idiom as the __fn_res init) so the if types
                // as the then-branch. (2026-09-17, found by cfg differential fuzz)
                None => list(vec![Sym("quote"), LispVal::Nil]),
            };
            Ok(list(vec![Sym("if"), cond, then_e, else_e]))
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
            if let Some(res) = lower_args_destructuring(v) {
                let binds = res?;
                return Ok(list(vec![Sym("let*"), list(binds), Num(0)]));
            }
            let mut bindings = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                let val = if init_is_input_handle(init_e) {
                    list(vec![Sym("quote"), LispVal::Nil])
                } else {
                    lower_expr(init_e)?
                };
                bindings.push(list(vec![Sym(name), val]));
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
                    list(vec![Sym("__wl_brk"), Num(0)]),
                    list(vec![Sym("__wl_ret"), Num(0)]),
                    list(vec![
                        Sym("__wl_res"),
                        list(vec![Sym("quote"), LispVal::Nil]),
                    ]),
                ]),
                list(vec![
                    Sym("begin"),
                    core,
                    list(vec![
                        Sym("if"),
                        Sym("__wl_ret"),
                        exit_result_form(view),
                        Num(0),
                    ]),
                ]),
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
        Statement::BreakStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::ReturnStatement(_) => true,
        Statement::BlockStatement(b) => stmts_have_exit(&b.body),
        Statement::IfStatement(i) => {
            stmt_has_exit(&i.consequent) || i.alternate.as_ref().is_some_and(|a| stmt_has_exit(a))
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
    let fn_bound = FN_FLAGS_BOUND.with(|f| f.get());
    let deep_ret = fn_bound && stmts_have_deep_return(body_stmts);
    let has_exits = stmts_have_exit(body_stmts) || deep_ret;

    // Hoist body declarations (while-core style): bound nil alongside the
    // per-iteration element binding, re-initialized via set! at their source
    // position. A `let j = 0;` in the body used to lower to a dead let —
    // nested whiles referencing j failed with "undefined variable" (2026-09-13).
    let mut hoisted: Vec<(String, LispVal)> = Vec::new();
    for st in body_stmts {
        if let Statement::VariableDeclaration(v) = st {
            for d in &v.declarations {
                let hname = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                hoisted.push((hname, lower_expr(init_e)?));
            }
        }
    }

    // body pieces (exit-aware, same shape as lower_for_parts)
    let mut body_items: Vec<LispVal> = vec![Sym("begin")];
    // continue support: re-arm __wl_done each iteration (see while core).
    // EXIT-MODE ONLY (2026-09-14, gas): dead LocalSet per iteration otherwise
    if has_exits {
        body_items.push(list(vec![Sym("set!"), Sym("__wl_done"), Num(0)]));
    }
    let mut seen_exit = false;
    let mut seen_fn_exit = false;
    for st in body_stmts {
        let piece = if has_exits {
            match st {
                Statement::BreakStatement(_) => list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                    Num(0),
                ]),
                Statement::ContinueStatement(_) => list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    Num(0),
                ]),
                // hoisted declaration: re-init at source position; dead after
                // an exit (matching the while core's skip rule)
                Statement::VariableDeclaration(v) => {
                    let mut re = Vec::new();
                    if !seen_exit && !seen_fn_exit {
                        for d in &v.declarations {
                            let dname = binding_name(&d.id)?;
                            let init = hoisted
                                .iter()
                                .find(|(n, _)| *n == dname)
                                .map(|(_, i)| i.clone())
                                .ok_or("ts_frontend: internal: hoisted decl missing")?;
                            re.push(list(vec![Sym("set!"), Sym(dname), init]));
                        }
                    }
                    re.push(Num(0));
                    let mut items = vec![Sym("begin")];
                    items.extend(re);
                    list(items)
                }
                Statement::ReturnStatement(r) => {
                    let val = match &r.argument {
                        Some(e) => lower_expr(e)?,
                        None => Num(0),
                    };
                    let mut items = vec![
                        list(vec![Sym("set!"), Sym("__wl_res"), val.clone()]),
                        list(vec![Sym("set!"), Sym("__wl_ret"), Num(1)]),
                        list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                        list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                    ];
                    if fn_bound {
                        items.push(list(vec![Sym("set!"), Sym("__fn_res"), val]));
                        items.push(list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]));
                    }
                    items.push(Num(0));
                    let mut v = vec![Sym("begin")];
                    v.extend(items);
                    list(v)
                }
                other => {
                    let e = tail_stmt_as_expr(other)?;
                    if seen_exit || seen_fn_exit {
                        // dead code after an exit (this loop's or a nested
                        // return) — int-pad the branch (e may be set!/while-
                        // typed nil; nil ≠ int breaks the checker's unification)
                        let guarded = if seen_exit && seen_fn_exit {
                            list(vec![
                                Sym("if"),
                                list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                                list(vec![
                                    Sym("if"),
                                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                                    list(vec![Sym("begin"), e, Num(0)]),
                                    Num(0),
                                ]),
                                Num(0),
                            ])
                        } else if seen_exit {
                            list(vec![
                                Sym("if"),
                                list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                                list(vec![Sym("begin"), e, Num(0)]),
                                Num(0),
                            ])
                        } else {
                            list(vec![
                                Sym("if"),
                                list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                                list(vec![Sym("begin"), e, Num(0)]),
                                Num(0),
                            ])
                        };
                        guarded
                    } else {
                        e
                    }
                }
            }
        } else {
            // simple path: hoisted declarations still re-init in place
            match st {
                Statement::VariableDeclaration(v) => {
                    let mut re = Vec::new();
                    for d in &v.declarations {
                        let dname = binding_name(&d.id)?;
                        let init = hoisted
                            .iter()
                            .find(|(n, _)| *n == dname)
                            .map(|(_, i)| i.clone())
                            .ok_or("ts_frontend: internal: hoisted decl missing")?;
                        re.push(list(vec![Sym("set!"), Sym(dname), init]));
                    }
                    re.push(Num(0));
                    let mut items = vec![Sym("begin")];
                    items.extend(re);
                    list(items)
                }
                other => tail_stmt_as_expr(other)?,
            }
        };
        // recursive: a break/return nested in an if ALSO kills the rest of
        // the iteration — top-level-only detection let sibling statements
        // run after a mid-branch break (for-of acc bug, 2026-09-08)
        if stmt_has_exit(st) {
            seen_exit = true;
        }
        if fn_bound && deep_ret_scan(st) {
            seen_fn_exit = true;
        }
        body_items.push(piece);
    }
    body_items.push(list(vec![
        Sym("set!"),
        Sym("__of_i"),
        list(vec![Sym("+"), Sym("__of_i"), Num(1)]),
    ]));
    let body_e = if body_items.len() == 1 {
        Num(0)
    } else {
        list(body_items)
    };

    // per-iteration element binding wraps the body; hoisted declarations
    // bind nil alongside the element (re-armed by set! at source position)
    let mut elem_binds = vec![list(vec![
        Sym(name),
        list(vec![Sym("vec-nth"), Sym("__of_a"), Sym("__of_i")]),
    ])];
    for (hn, init) in &hoisted {
        // u128 Level 1: "0" dummies for u128-pure hoisted inits (see the
        // while-core comment — nil poisons limb-local eligibility)
        let dummy = if init_is_u128_pure(init) {
            Str("0".to_string())
        } else {
            list(vec![Sym("quote"), LispVal::Nil])
        };
        elem_binds.push(list(vec![Sym(hn.clone()), dummy]));
    }
    let body_bound = list(vec![Sym("let"), list(elem_binds), body_e]);

    let test = list(vec![Sym("<"), Sym("__of_i"), Sym("__of_n")]);
    let inner_test = if deep_ret {
        list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
            test,
            list(vec![Sym("="), Num(1), Num(0)]),
        ])
    } else {
        test
    };
    let cond_e = if has_exits {
        list(vec![
            Sym("if"),
            // __wl_brk (break/return, not continue) gates the loop exit
            list(vec![Sym("="), Sym("__wl_brk"), Num(0)]),
            inner_test,
            list(vec![Sym("="), Num(1), Num(0)]),
        ])
    } else {
        inner_test
    };
    Ok((
        has_exits,
        list(vec![
            Sym("let*"),
            list(vec![
                list(vec![Sym("__of_a"), arr_e]),
                list(vec![Sym("__of_i"), Num(0)]),
                list(vec![
                    Sym("__of_n"),
                    list(vec![Sym("vec-length"), Sym("__of_a")]),
                ]),
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
    let fn_bound = FN_FLAGS_BOUND.with(|f| f.get());
    // A return nested in an INNER loop of this body stops this loop only
    // through the function-level flag — the cond must check it, and the
    // body walk must route through the exit machinery (guards) even when
    // this loop itself has no break/return.
    let deep_ret = fn_bound && stmts_have_deep_return(body_stmts);

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

    if !stmts_have_exit(body_stmts) && !deep_ret {
        let mut body_items = vec![Sym("begin")];
        for s in body_stmts {
            match s {
                // Per-iteration re-init AT ITS SOURCE POSITION. Mid-body
                // declarations read state mutated earlier in the SAME
                // iteration (`const s16 = t[16] + C` after an inner loop) —
                // evaluating their inits at body top produced stale/garbage
                // values (fp254 CIOS: every limb wrong; the generalized
                // "values vanish" bug, 2026-09-11).
                Statement::VariableDeclaration(v) => {
                    for d in &v.declarations {
                        let name = binding_name(&d.id)?;
                        let init = hoisted
                            .iter()
                            .find(|(n, _)| *n == name)
                            .map(|(_, i)| i.clone())
                            .ok_or("ts_frontend: internal: hoisted decl missing")?;
                        body_items.push(list(vec![Sym("set!"), Sym(name), init]));
                    }
                }
                other => body_items.push(tail_stmt_as_expr(other)?),
            }
        }
        let body_e = if body_items.len() == 1 {
            Num(0)
        } else {
            list(body_items)
        };
        let while_e = list(vec![Sym("while"), truthy(&w.test)?, body_e]);
        if hoisted.is_empty() {
            return Ok((false, while_e));
        }
        // u128 Level 1 (2026-09-15): u128-pure hoisted inits bind "0" dummies
        // — nil would poison limb-local eligibility for loop accumulators
        // (fib shape). TDZ guarantees the per-iteration set! precedes every
        // read, so the dummy value is never observed.
        let binds: Vec<LispVal> = hoisted
            .iter()
            .map(|(n, init)| {
                let dummy = if init_is_u128_pure(init) {
                    Str("0".to_string())
                } else {
                    list(vec![Sym("quote"), LispVal::Nil])
                };
                list(vec![Sym(n.clone()), dummy])
            })
            .collect();
        return Ok((false, list(vec![Sym("let"), list(binds), while_e])));
    }
    // break/return rewrite — declarations re-init AT THEIR SOURCE POSITION
    // (same rule as the simple path: mid-body inits read same-iteration state)
    let mut body_items = vec![Sym("begin")];
    // continue support (2026-09-13): __wl_done doubles as the skip-rest-of-
    // iteration guard; a continue sets it WITHOUT setting __wl_brk, so the
    // cond keeps looping but the guards skip the tail. Re-arm at iteration
    // start — else a conditional continue would trip the guards forever.
    body_items.push(list(vec![Sym("set!"), Sym("__wl_done"), Num(0)]));
    let mut seen_exit = false;
    let mut seen_fn_exit = false;
    for s in body_stmts {
        if let Statement::VariableDeclaration(v) = s {
            if !seen_exit {
                for d in &v.declarations {
                    let name = binding_name(&d.id)?;
                    let init = hoisted
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map(|(_, i)| i.clone())
                        .ok_or("ts_frontend: internal: hoisted decl missing")?;
                    body_items.push(list(vec![Sym("set!"), Sym(name), init]));
                }
            }
            continue; // position handled above; dead after an exit
        }
        let piece = match s {
            Statement::BreakStatement(_) => list(vec![
                Sym("begin"),
                list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                Num(0),
            ]),
            Statement::ContinueStatement(_) => list(vec![
                Sym("begin"),
                list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                Num(0),
            ]),
            Statement::ReturnStatement(r) => {
                let val = match &r.argument {
                    Some(e) => lower_expr(e)?,
                    None => Num(0),
                };
                // Stop THIS loop via __wl_done; when the function binds the
                // M2 flags, also record the value at function level so a
                // return nested in inner loops escapes (nested-return bug,
                // 2026-09-11).
                let mut items = vec![
                    list(vec![Sym("set!"), Sym("__wl_res"), val.clone()]),
                    list(vec![Sym("set!"), Sym("__wl_ret"), Num(1)]),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                ];
                if fn_bound {
                    items.push(list(vec![Sym("set!"), Sym("__fn_res"), val]));
                    items.push(list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]));
                }
                items.push(Num(0)); // set! types nil — keep the begin int-typed
                let mut v = vec![Sym("begin")];
                v.extend(items);
                list(v)
            }
            other => {
                let e = tail_stmt_as_expr(other)?;
                if seen_exit && seen_fn_exit {
                    // dead code after this loop's own exit OR a nested
                    // return — guard on both flags
                    list(vec![
                        Sym("if"),
                        list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                            e,
                            Num(0),
                        ]),
                        Num(0),
                    ])
                } else if seen_exit {
                    // dead code after break/return in the same iteration — guard
                    list(vec![
                        Sym("if"),
                        list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                        e,
                        Num(0),
                    ])
                } else if seen_fn_exit {
                    // a NESTED loop returned — the rest of this iteration
                    // is dead (the function is returning)
                    list(vec![
                        Sym("if"),
                        list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                        e,
                        Num(0),
                    ])
                } else {
                    e
                }
            }
        };
        // recursive: a break/return nested in an if ALSO kills the rest of
        // the iteration — top-level-only detection let sibling statements
        // run after a mid-branch break (for-of acc bug, 2026-09-08)
        if stmt_has_exit(s) {
            seen_exit = true;
        }
        // fn-done guards only when the fn flags are actually bound — a
        // return in an unbound context sets __wl_done, which the plain
        // exit guard already honors (unbound __fn_done reject, 2026-09-13)
        if fn_bound && deep_ret_scan(s) {
            seen_fn_exit = true;
        }
        body_items.push(piece);
    }
    let body_e = if body_items.len() == 1 {
        Num(0)
    } else {
        list(body_items)
    };
    // cond: stop on this loop's break flag, and on the function-level
    // return flag when a nested return can fire inside this body
    let plain_test = truthy(&w.test)?;
    // false must type-match the test: comparisons lower to bool, but a bare
    // literal (`while (true)` → Num 1) is int — int≠bool if-branches were a
    // checker reject before (2026-09-13). The while emitter's truthiness is
    // tag-aware, so int 0 is a valid false for numeric tests.
    let false_e = if statically_bool(&w.test) {
        list(vec![Sym("="), Num(1), Num(0)]) // bool false
    } else {
        Num(0) // int false — tag-aware truthiness treats 0 as false
    };
    let inner_cond = if deep_ret {
        list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
            plain_test,
            false_e.clone(),
        ])
    } else {
        plain_test
    };
    let cond_e = list(vec![
        Sym("if"),
        // __wl_brk (break/return only — NOT continue) gates the loop exit;
        // __wl_done is re-armed at each body start for continue support.
        list(vec![Sym("="), Sym("__wl_brk"), Num(0)]),
        inner_cond,
        false_e,
    ]);
    // CORE: hoisted bindings + flag-guarded while. Flags themselves are
    // bound by the SURROUNDING context (mid-function continuation guard
    // or the value wrapper) — a return inside the loop must be visible
    // AFTER the loop, so the flags must outlive this let.
    let mut binds = Vec::new();
    for (n, init) in &hoisted {
        // u128 Level 1: "0" dummies for u128-pure hoisted inits (see the
        // simple-path comment — nil poisons limb-local eligibility)
        let dummy = if init_is_u128_pure(init) {
            Str("0".to_string())
        } else {
            list(vec![Sym("quote"), LispVal::Nil])
        };
        binds.push(list(vec![Sym(n.clone()), dummy]));
    }
    let while_e = list(vec![Sym("while"), cond_e, body_e]);
    if binds.is_empty() {
        Ok((true, while_e))
    } else {
        Ok((true, list(vec![Sym("let"), list(binds), while_e])))
    }
}

/// Does this statement contain a `return` at any depth (incl. nested
/// loops)? Drives the function-level exit guard for statements following
/// a may-return statement inside loop bodies.
fn deep_ret_scan(s: &Statement<'_>) -> bool {
    stmts_have_deep_return(std::slice::from_ref(s))
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
            list(vec![Sym("__wl_brk"), Num(0)]),
            list(vec![Sym("__wl_ret"), Num(0)]),
            list(vec![
                Sym("__wl_res"),
                list(vec![Sym("quote"), LispVal::Nil]),
            ]),
        ]),
        list(vec![
            Sym("begin"),
            core,
            list(vec![Sym("if"), Sym("__wl_ret"), Sym("__wl_res"), Num(0)]),
        ]),
    ]))
}

/// Body of a while/for: statements → single begin-expression (side effects).
/// Declarations are HOISTED (while-core style): bound nil in a wrapping let,
/// re-initialized via set! at their source position — a `let j = 0;` inside
/// an if-branch used to lower to a dead `(let ((j 0)) 0)` whose binding
/// vanished, so later statements in the branch (nested whiles etc.) saw
/// "undefined variable j" (2026-09-13).
fn loop_body_expr(stmts: &[Statement<'_>]) -> Result<LispVal, String> {
    if stmts.is_empty() {
        return Ok(Num(0));
    }
    // collect top-level declarations for the wrapper let
    // (u128 Level 1: bigint-shaped inits get "0" dummies — see the while-core
    // comment; nil would poison limb-local eligibility)
    let mut hoisted: Vec<(String, bool)> = Vec::new();
    for s in stmts {
        if let Statement::VariableDeclaration(v) = s {
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let is_big = d.init.as_ref().is_some_and(expr_is_bigint);
                hoisted.push((name, is_big));
            }
        }
    }
    let mut exprs = Vec::new();
    for s in stmts {
        if let Statement::VariableDeclaration(v) = s {
            // re-init at source position (init already validated by hoisting)
            let mut re = Vec::new();
            for d in &v.declarations {
                let name = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                re.push(list(vec![Sym("set!"), Sym(name), lower_expr(init_e)?]));
            }
            re.push(Num(0));
            let mut items = vec![Sym("begin")];
            items.extend(re);
            exprs.push(list(items));
            continue;
        }
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
    let body = if exprs.len() == 1 {
        exprs.into_iter().next().unwrap()
    } else {
        let mut items = vec![Sym("begin")];
        items.extend(exprs);
        list(items)
    };
    if hoisted.is_empty() {
        Ok(body)
    } else {
        let binds: Vec<LispVal> = hoisted
            .into_iter()
            .map(|(n, is_big)| {
                let dummy = if is_big {
                    Str("0".to_string())
                } else {
                    list(vec![Sym("quote"), LispVal::Nil])
                };
                list(vec![Sym(n), dummy])
            })
            .collect();
        Ok(list(vec![Sym("let"), list(binds), body]))
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
            // In-loop return: stop this loop AND, when the function binds
            // the M2 flags, record the value at function level — a return
            // nested in an inner while must not vanish when this loop's
            // value is discarded by the enclosing body (2026-09-11).
            let fn_bound = FN_FLAGS_BOUND.with(|f| f.get());
            let mut items = vec![
                list(vec![Sym("set!"), Sym("__wl_res"), val.clone()]),
                list(vec![Sym("set!"), Sym("__wl_ret"), Num(1)]),
                list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
            ];
            if fn_bound {
                items.push(list(vec![Sym("set!"), Sym("__fn_res"), val]));
                items.push(list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]));
            }
            items.push(Num(0));
            Ok(list(vec![Sym("begin")].into_iter().chain(items).collect()))
        }
        Statement::BreakStatement(_) => Ok(list(vec![
            Sym("begin"),
            list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
            list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
            Num(0),
        ])),
        // continue (2026-09-13): kills the REST OF THE ITERATION only — set
        // __wl_done (the iteration guard the following statements check) but
        // NOT __wl_brk (the loop-exit flag the cond checks). The body-start
        // `(set! __wl_done 0)` reset re-arms the guards next iteration.
        Statement::ContinueStatement(_) => Ok(list(vec![
            Sym("begin"),
            list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
            Num(0),
        ])),
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
                    list(vec![Sym("__wl_brk"), Num(0)]),
                    list(vec![Sym("__wl_ret"), Num(0)]),
                    list(vec![
                        Sym("__wl_res"),
                        list(vec![Sym("quote"), LispVal::Nil]),
                    ]),
                ]),
                list(vec![
                    Sym("begin"),
                    core,
                    list(vec![Sym("if"), Sym("__wl_ret"), Sym("__wl_res"), Num(0)]),
                ]),
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
            list(vec![Sym("__wl_brk"), Num(0)]),
            list(vec![Sym("__wl_ret"), Num(0)]),
            list(vec![
                Sym("__wl_res"),
                list(vec![Sym("quote"), LispVal::Nil]),
            ]),
        ]),
        list(vec![
            Sym("begin"),
            core,
            list(vec![Sym("if"), Sym("__wl_ret"), Sym("__wl_res"), Num(0)]),
        ]),
    ]))
}

/// For → (has_exits, core). Core = (let ((v init)...) (while cond body))
/// with flag-guarded cond when the body can exit; flags bound by caller.
fn lower_for_parts(fr: &oxc_ast::ast::ForStatement<'_>) -> Result<(bool, LispVal), String> {
    use oxc_ast::ast::{AssignmentOperator, ForStatementInit};

    // init: must be a let/const declaration
    let decl = match &fr.init {
        Some(ForStatementInit::VariableDeclaration(v)) => v,
        _ => return Err(
            "ts_frontend: for-loop init must be `let` declarations (e.g. `for (let i = 0; ...)`)"
                .to_string(),
        ),
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
                let one = if matches!(
                    upd.operator,
                    oxc_syntax::operator::UpdateOperator::Increment
                ) {
                    1
                } else {
                    -1
                };
                list(vec![
                    Sym("set!"),
                    Sym(v.clone()),
                    list(vec![Sym("+"), Sym(v), Num(one)]),
                ])
            }
            Expression::AssignmentExpression(asg) => {
                let (v, expr) = lower_assignment(asg)?;
                list(vec![Sym("set!"), Sym(v), expr])
            }
            _ => {
                return Err(
                    "ts_frontend: for-loop update must be `i++`/`i--`/`i = e`/`i += e`".into(),
                )
            }
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
    let fn_bound = FN_FLAGS_BOUND.with(|f| f.get());
    let deep_ret = fn_bound && stmts_have_deep_return(body_stmts);
    let has_exits = stmts_have_exit(body_stmts) || deep_ret;

    // Hoist body declarations (while-core style): bound nil in the outer
    // loop-var let, re-initialized via set! at their source position — a
    // `let j = 0;` in the body used to lower to a dead let whose binding
    // vanished (nested whiles referencing j: "undefined variable", 2026-09-13).
    let mut hoisted: Vec<(String, LispVal)> = Vec::new();
    for s in body_stmts {
        if let Statement::VariableDeclaration(v) = s {
            for d in &v.declarations {
                let hname = binding_name(&d.id)?;
                let init_e = d
                    .init
                    .as_ref()
                    .ok_or("ts_frontend: local declaration needs initializer")?;
                hoisted.push((hname, lower_expr(init_e)?));
            }
        }
    }
    for (hn, init) in &hoisted {
        // u128 Level 1: "0" dummies for u128-pure hoisted inits (see the
        // while-core comment — nil poisons limb-local eligibility)
        let dummy = if init_is_u128_pure(init) {
            Str("0".to_string())
        } else {
            list(vec![Sym("quote"), LispVal::Nil])
        };
        bindings.push(list(vec![Sym(hn.clone()), dummy]));
    }

    let mut body_items: Vec<LispVal> = vec![Sym("begin")];
    // continue support: re-arm __wl_done each iteration (see while core).
    // EXIT-MODE ONLY (2026-09-14, gas): nothing writes __wl_done in a
    // no-exit body, so the re-arm was a dead LocalSet per iteration.
    if has_exits {
        body_items.push(list(vec![Sym("set!"), Sym("__wl_done"), Num(0)]));
    }
    let mut seen_exit = false;
    let mut seen_fn_exit = false;
    for s in body_stmts {
        let piece = if has_exits {
            match s {
                Statement::BreakStatement(_) => list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                    Num(0),
                ]),
                Statement::ContinueStatement(_) => list(vec![
                    Sym("begin"),
                    list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                    Num(0),
                ]),
                // hoisted declaration: re-init at source position; dead after
                // an exit (matching the while core's skip rule)
                Statement::VariableDeclaration(v) => {
                    let mut re = Vec::new();
                    if !seen_exit && !seen_fn_exit {
                        for d in &v.declarations {
                            let dname = binding_name(&d.id)?;
                            let init = hoisted
                                .iter()
                                .find(|(n, _)| *n == dname)
                                .map(|(_, i)| i.clone())
                                .ok_or("ts_frontend: internal: hoisted decl missing")?;
                            re.push(list(vec![Sym("set!"), Sym(dname), init]));
                        }
                    }
                    re.push(Num(0));
                    let mut items = vec![Sym("begin")];
                    items.extend(re);
                    list(items)
                }
                Statement::ReturnStatement(r) => {
                    let val = match &r.argument {
                        Some(e) => lower_expr(e)?,
                        None => Num(0),
                    };
                    let mut items = vec![
                        list(vec![Sym("set!"), Sym("__wl_res"), val.clone()]),
                        list(vec![Sym("set!"), Sym("__wl_ret"), Num(1)]),
                        list(vec![Sym("set!"), Sym("__wl_done"), Num(1)]),
                        list(vec![Sym("set!"), Sym("__wl_brk"), Num(1)]),
                    ];
                    if fn_bound {
                        items.push(list(vec![Sym("set!"), Sym("__fn_res"), val]));
                        items.push(list(vec![Sym("set!"), Sym("__fn_done"), Num(1)]));
                    }
                    items.push(Num(0));
                    let mut v = vec![Sym("begin")];
                    v.extend(items);
                    list(v)
                }
                other => {
                    let e = tail_stmt_as_expr(other)?;
                    if seen_exit && seen_fn_exit {
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            list(vec![
                                Sym("if"),
                                list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                                e,
                                Num(0),
                            ]),
                            Num(0),
                        ])
                    } else if seen_exit {
                        // dead code after break/return in the same iteration
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__wl_done"), Num(0)]),
                            e,
                            Num(0),
                        ])
                    } else if seen_fn_exit {
                        // a NESTED loop returned — the rest of this iteration
                        // is dead (the function is returning)
                        list(vec![
                            Sym("if"),
                            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                            e,
                            Num(0),
                        ])
                    } else {
                        e
                    }
                }
            }
        } else {
            // simple path: hoisted declarations still re-init in place
            match s {
                Statement::VariableDeclaration(v) => {
                    let mut re = Vec::new();
                    for d in &v.declarations {
                        let dname = binding_name(&d.id)?;
                        let init = hoisted
                            .iter()
                            .find(|(n, _)| *n == dname)
                            .map(|(_, i)| i.clone())
                            .ok_or("ts_frontend: internal: hoisted decl missing")?;
                        re.push(list(vec![Sym("set!"), Sym(dname), init]));
                    }
                    re.push(Num(0));
                    let mut items = vec![Sym("begin")];
                    items.extend(re);
                    list(items)
                }
                other => tail_stmt_as_expr(other)?,
            }
        };
        // recursive: a break/return nested in an if ALSO kills the rest of
        // the iteration — top-level-only detection let sibling statements
        // run after a mid-branch break (for-of acc bug, 2026-09-08)
        if stmt_has_exit(s) {
            seen_exit = true;
        }
        // fn-done guards only when the fn flags are actually bound — a
        // return in an unbound context sets __wl_done, which the plain
        // exit guard already honors (unbound __fn_done reject, 2026-09-13)
        if fn_bound && deep_ret_scan(s) {
            seen_fn_exit = true;
        }
        body_items.push(piece);
    }
    // update clause runs after the body; guard it in exit mode so a
    // returned/broken iteration doesn't keep mutating loop vars. __wl_brk
    // (not __wl_done) — a `continue` must still run the update (i++)
    // (2026-09-13), while break/return skip it as before.
    if let Some(u) = update_form {
        if has_exits {
            // inner fn-done guard only when fn flags are in scope — continue-
            // only bodies have has_exits WITHOUT any return, so the guard
            // would reference an unbound __fn_done (checker reject, 2026-09-13)
            let inner = if fn_bound {
                list(vec![
                    Sym("if"),
                    list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
                    list(vec![Sym("begin"), u, Num(0)]),
                    Num(0),
                ])
            } else {
                list(vec![Sym("begin"), u, Num(0)])
            };
            body_items.push(list(vec![
                Sym("if"),
                list(vec![Sym("="), Sym("__wl_brk"), Num(0)]),
                inner,
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
    let plain_test = truthy(test)?;
    // type-align false with the test (see while core, 2026-09-13)
    let false_e = if statically_bool(test) {
        list(vec![Sym("="), Num(1), Num(0)])
    } else {
        Num(0)
    };
    let inner_test = if deep_ret {
        list(vec![
            Sym("if"),
            list(vec![Sym("="), Sym("__fn_done"), Num(0)]),
            plain_test,
            false_e.clone(),
        ])
    } else {
        plain_test
    };
    let cond_e = list(vec![
        Sym("if"),
        // __wl_brk (break/return, not continue) gates the loop exit
        list(vec![Sym("="), Sym("__wl_brk"), Num(0)]),
        inner_test,
        false_e,
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

/// Function-level return value (set by in-loop return rewrites when the
/// M2 flags are bound). json_return_str is idempotent, so wrapping at
/// extraction is safe even when a bare-return site already wrapped at
/// store time.
fn fn_exit_form(view: bool) -> LispVal {
    if view {
        list(vec![Sym("near/json_return_str"), Sym("__fn_res")])
    } else {
        Sym("__fn_res")
    }
}

/// True when `e` already ends in the __fn_done commit (a (set! __fn_done 1)
/// inside the form) — the conditional-carrier lowering marks its commit
/// sites so the blanket branch-capture in lower_prefix_around_with_return
/// never double-wraps them (a double commit would fire __fn_done on
/// fall-through paths and skip post-if statements). (2026-09-17)
fn is_commit_form(e: &LispVal) -> bool {
    match e {
        LispVal::List(items) => {
            if items.len() >= 2
                && items[0] == LispVal::Sym("set!".into())
                && items[1] == LispVal::Sym("__fn_done".into())
            {
                return true;
            }
            // (if c commit nil-hole) — the conditional-carrier shape
            if items[0] == LispVal::Sym("if".into()) && items.len() == 4 {
                return is_commit_form(&items[2]) || is_commit_form(&items[3]);
            }
            // (begin a b c...) — scan elements
            items.iter().skip(1).any(is_commit_form)
        }
        _ => false,
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
            let one = if matches!(
                upd.operator,
                oxc_syntax::operator::UpdateOperator::Increment
            ) {
                1
            } else {
                -1
            };
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
fn lower_assign_form(asg: &oxc_ast::ast::AssignmentExpression<'_>) -> Result<LispVal, String> {
    use oxc_syntax::operator::AssignmentOperator;
    match &asg.left {
        oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) => {
            let (v, expr) = lower_assignment(asg)?;
            Ok(list(vec![Sym("set!"), Sym(v), expr]))
        }
        oxc_ast::ast::AssignmentTarget::StaticMemberExpression(sm) => {
            // M2+ (2026-10-05): `o.x = v` lowers to a REBINDING — objects
            // are immutable JSON-string values, so the property write is
            // (set! o (json-set o "x" <encoded v>)). Single-level only:
            // dotted targets (o.a.b = v) need nested rebuilds — rejected.
            if !matches!(asg.operator, AssignmentOperator::Assign) {
                return Err(
                    "ts_frontend: compound property assignment (o.x += v) not supported — read o.x, add, reassign"
                        .into(),
                );
            }
            // reject dotted chains: sm.object must be a plain identifier
            let (obj_name, path) = match &sm.object {
                Expression::Identifier(id) => (id.name.as_str().to_string(), sm.property.name.as_str().to_string()),
                _ => return Err(
                    "ts_frontend: only single-level property writes (o.x = v) are supported — nested (o.a.b = v) needs a manual jsonSet rebuild"
                        .into(),
                ),
            };
            let val = encode_json_value(&asg.right)?;
            Ok(list(vec![
                Sym("set!"),
                Sym(obj_name.clone()),
                list(vec![Sym("json-set"), Sym(obj_name), Str(path), val]),
            ]))
        }
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
                _ => {
                    return Err("ts_frontend: element writes support only = / += / -= \
                         (compound *= /= %= are plain-variable only; obj.prop \
                         writes need jsonSet)"
                        .into())
                }
            };
            Ok(list(vec![Sym("vec-set!"), obj, idx, val]))
        }
        _ => {
            return Err(
                "ts_frontend: assignment target must be a variable or element access".into(),
            )
        }
    }
}

/// M2 objects: `{ k: v, ... }` → nested `(json-set "{}" "k" <encoded v>)`
/// folds. Objects are JSON-string values: storage/return/interop need no
/// conversion, reads go through near/json_get_str.
fn lower_object_literal(obj: &oxc_ast::ast::ObjectExpression<'_>) -> Result<LispVal, String> {
    let mut acc = Str("{}".to_string());
    for prop in &obj.properties {
        match prop {
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                let key = match &p.key {
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str().to_string(),
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
                if OBJ_PARAM_PROPS.with(|s| s.borrow().iter().any(|(n, _)| n == id.name.as_str())) {
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
        Expression::Identifier(id) => {
            NUM_PARAM_NAMES.with(|s| s.borrow().iter().any(|n| n == id.name.as_str()))
        }
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

/// Collect every plain Identifier NAME appearing in an expression
/// (member property names excluded — only value refs). Used by the F3
/// closure-safety checks: capture detection (T4) and the dispatch-freeze
/// scan on pipeline callback bodies. Deliberately over-approximates
/// (shadowed params count too) — conservative hard errors only.
fn expr_idents(e: &Expression<'_>, out: &mut Vec<String>) {
    match e {
        Expression::Identifier(id) => out.push(id.name.as_str().to_string()),
        Expression::TemplateLiteral(t) => {
            for x in &t.expressions {
                expr_idents(x, out);
            }
        }
        Expression::ArrayExpression(a) => {
            for el in &a.elements {
                if let Some(x) = el.as_expression() {
                    expr_idents(x, out);
                }
            }
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(pp) = prop {
                    expr_idents(&pp.value, out);
                }
            }
        }
        Expression::BinaryExpression(b) => {
            expr_idents(&b.left, out);
            expr_idents(&b.right, out);
        }
        Expression::LogicalExpression(l) => {
            expr_idents(&l.left, out);
            expr_idents(&l.right, out);
        }
        Expression::UnaryExpression(u) => expr_idents(&u.argument, out),
        Expression::UpdateExpression(_) => {}
        Expression::ConditionalExpression(c) => {
            expr_idents(&c.test, out);
            expr_idents(&c.consequent, out);
            expr_idents(&c.alternate, out);
        }
        Expression::ParenthesizedExpression(p) => expr_idents(&p.expression, out),
        Expression::AwaitExpression(a) => expr_idents(&a.argument, out),
        Expression::CallExpression(c) => {
            expr_idents(&c.callee, out);
            for a in &c.arguments {
                if let Some(x) = a.as_expression() {
                    expr_idents(x, out);
                }
            }
        }
        Expression::StaticMemberExpression(m) => expr_idents(&m.object, out),
        Expression::ComputedMemberExpression(m) => {
            expr_idents(&m.object, out);
            expr_idents(&m.expression, out);
        }
        Expression::AssignmentExpression(a) => {
            if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                out.push(id.name.as_str().to_string());
            }
            expr_idents(&a.right, out);
        }
        Expression::ArrowFunctionExpression(_) => {
            // lambda-lifted — its free vars were checked at its own site
        }
        _ => {}
    }
}

fn stmt_exprs<'a>(st: &'a Statement<'a>, acc: &mut Vec<&'a Expression<'a>>) {
    match st {
        Statement::ReturnStatement(r) => {
            if let Some(x) = &r.argument {
                acc.push(x);
            }
        }
        Statement::ExpressionStatement(x) => acc.push(&x.expression),
        Statement::IfStatement(i) => {
            acc.push(&i.test);
            body_or_stmt(&i.consequent, acc);
            if let Some(alt) = &i.alternate {
                body_or_stmt(alt, acc);
            }
        }
        Statement::BlockStatement(b) => stmts_exprs(&b.body, acc),
        Statement::WhileStatement(w) => {
            acc.push(&w.test);
            body_or_stmt(&w.body, acc);
        }
        Statement::ForOfStatement(f) => {
            acc.push(&f.right);
            body_or_stmt(&f.body, acc);
        }
        Statement::VariableDeclaration(v) => {
            for d in &v.declarations {
                if let Some(x) = &d.init {
                    acc.push(x);
                }
            }
        }
        _ => {}
    }
}

fn stmts_exprs<'a>(stmts: &'a [Statement<'a>], acc: &mut Vec<&'a Expression<'a>>) {
    for st in stmts {
        stmt_exprs(st, acc);
    }
}

fn body_or_stmt<'a>(st: &'a Statement<'a>, acc: &mut Vec<&'a Expression<'a>>) {
    match st {
        Statement::BlockStatement(b) => stmts_exprs(&b.body, acc),
        _ => stmt_exprs(st, acc),
    }
}

/// All identifiers an arrow references beyond its params. Expression form
/// walks directly; block form walks its statements' expressions (return/
/// expr-stmt/if/while/for-of/decl inits cover the real body shapes).
fn arrow_body_idents(a: &oxc_ast::ast::ArrowFunctionExpression<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let params: Vec<String> = a
        .params
        .items
        .iter()
        .filter_map(|p| binding_name(&p.pattern).ok())
        .collect();
    if let Some(x) = a.get_expression() {
        expr_idents(x, &mut out);
    } else if let Some(fb) = a.get_function_body() {
        let mut acc = Vec::new();
        stmts_exprs(&fb.statements, &mut acc);
        for x in acc {
            expr_idents(x, &mut out);
        }
    }
    out.retain(|n| !params.contains(n));
    out
}

/// F3 (2026-10-04) — whole-function closure-safety analysis.
///
/// Run BEFORE lowering a function body, over the raw AST — the lowering
/// pipeline hoists/reorders statements (impure-init prologue, early-return
/// prefixes), so inline lowering-time checks see the sets in the wrong
/// order. This static pass sees the complete picture:
///
/// - T4: an arrow capturing a local that is EVER assigned (`x = …`,
///   `x += …`, `x++`) is a wasm landmine (closure cells are not
///   per-invocation; bytecode fixed 2026-08-26, wasm NOT) → hard error
///   naming the variable. Immutable capture is fine (probed end-to-end
///   through wasm + interpreter 2026-10-04).
/// - dispatch-freeze: a pipeline callback (.map/.filter/.reduce) that
///   calls a lambda-initialized local emits an INVALID module (probed
///   2026-10-04) → hard error suggesting inlining.
/// - boundaries: an arrow literal as a direct call argument to a user
///   fn can't cross function boundaries (unknown-function at emit) →
///   hard error.
///
/// Deliberately over-approximate (nested shadowing not tracked) — every
/// hit is a hard error, never a silent pass.
struct ClosureScan {
    assigned: Vec<String>,
    lambda_init_locals: Vec<String>,
    /// (free idents, is_pipeline_callback)
    arrows: Vec<(Vec<String>, bool)>,
    call_arg_arrow: bool,
}

fn scan_expr(e: &Expression<'_>, scan: &mut ClosureScan) {
    match e {
        Expression::AssignmentExpression(a) => {
            if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                scan.assigned.push(id.name.as_str().to_string());
            }
            scan_expr(&a.right, scan);
        }
        Expression::UpdateExpression(u) => {
            if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) =
                &u.argument
            {
                scan.assigned.push(id.name.as_str().to_string());
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            scan.arrows.push((arrow_body_idents(arrow), false));
            // nested content still contributes assignments/other arrows
            if let Some(x) = arrow.get_expression() {
                scan_expr(x, scan);
            } else if let Some(fb) = arrow.get_function_body() {
                scan_stmts_closure(&fb.statements, scan);
            }
        }
        Expression::CallExpression(c) => {
            scan_expr(&c.callee, scan);
            let pipeline = match &c.callee {
                Expression::StaticMemberExpression(sm) => {
                    matches!(sm.property.name.as_str(), "map" | "filter" | "reduce")
                }
                _ => false,
            };
            for arg in &c.arguments {
                let Some(ae) = arg.as_expression() else {
                    continue;
                };
                match ae {
                    Expression::ArrowFunctionExpression(arrow) => {
                        if !pipeline {
                            scan.call_arg_arrow = true;
                        }
                        scan.arrows.push((arrow_body_idents(arrow), pipeline));
                        if let Some(x) = arrow.get_expression() {
                            scan_expr(x, scan);
                        } else if let Some(fb) = arrow.get_function_body() {
                            scan_stmts_closure(&fb.statements, scan);
                        }
                    }
                    _ => scan_expr(ae, scan),
                }
            }
        }
        Expression::BinaryExpression(b) => {
            scan_expr(&b.left, scan);
            scan_expr(&b.right, scan);
        }
        Expression::LogicalExpression(l) => {
            scan_expr(&l.left, scan);
            scan_expr(&l.right, scan);
        }
        Expression::UnaryExpression(u) => scan_expr(&u.argument, scan),
        Expression::ConditionalExpression(cd) => {
            scan_expr(&cd.test, scan);
            scan_expr(&cd.consequent, scan);
            scan_expr(&cd.alternate, scan);
        }
        Expression::ParenthesizedExpression(pe) => scan_expr(&pe.expression, scan),
        Expression::AwaitExpression(a) => scan_expr(&a.argument, scan),
        Expression::TemplateLiteral(t) => {
            for x in &t.expressions {
                scan_expr(x, scan);
            }
        }
        Expression::ArrayExpression(a) => {
            for el in &a.elements {
                if let Some(x) = el.as_expression() {
                    scan_expr(x, scan);
                }
            }
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(pp) = prop {
                    scan_expr(&pp.value, scan);
                }
            }
        }
        Expression::StaticMemberExpression(m) => scan_expr(&m.object, scan),
        Expression::ComputedMemberExpression(m) => {
            scan_expr(&m.object, scan);
            scan_expr(&m.expression, scan);
        }
        _ => {}
    }
}

fn scan_stmts_closure(stmts: &[Statement<'_>], scan: &mut ClosureScan) {
    for st in stmts {
        match st {
            Statement::ExpressionStatement(x) => scan_expr(&x.expression, scan),
            Statement::ReturnStatement(r) => {
                if let Some(x) = &r.argument {
                    scan_expr(x, scan);
                }
            }
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if let (Some(init_e), Ok(name)) = (&d.init, binding_name(&d.id)) {
                        if matches!(init_e, Expression::ArrowFunctionExpression(_)) {
                            scan.lambda_init_locals.push(name.clone());
                        }
                        scan_expr(init_e, scan);
                    }
                }
            }
            _ => {
                // if/while/for-of/blocks — generic statement-expression
                // walker (assignment-statements surface as expression
                // statements in oxc anyway).
                let mut acc = Vec::new();
                stmt_exprs(st, &mut acc);
                for x in acc {
                    scan_expr(x, scan);
                }
            }
        }
    }
}

/// Entry point: run all three F3 checks over a function body.
fn check_fn_closure_safety(stmts: &[Statement<'_>]) -> Result<(), String> {
    let mut scan = ClosureScan {
        assigned: Vec::new(),
        lambda_init_locals: Vec::new(),
        arrows: Vec::new(),
        call_arg_arrow: false,
    };
    scan_stmts_closure(stmts, &mut scan);

    for (idents, is_callback) in &scan.arrows {
        if *is_callback {
            for n in idents {
                if scan.lambda_init_locals.contains(n) {
                    return Err(format!(
                        "ts_frontend: pipeline callback calls lambda-valued local `{n}` — \
dispatch-freeze landmine (emits invalid wasm). Inline the arrow's body or hoist the logic out"
                    ));
                }
            }
        } else {
            for n in idents {
                if scan.assigned.contains(n) {
                    return Err(format!(
                        "ts_frontend: arrow captures mutable local `{n}` — closure-over-set! is \
unsupported on wasm (backend T4: cells not per-invocation). Copy it to a \
fresh const before the arrow, or restructure"
                    ));
                }
            }
        }
    }
    if scan.call_arg_arrow {
        return Err(
            "ts_frontend: arrow as call argument unsupported — first-class \
function values can't cross user-function boundaries (M1)"
                .into(),
        );
    }
    Ok(())
}

/// Does the expression contain any call? Used by lower_prefix_around_with_return
/// to detect impure variable initializers that must be guarded after an early
/// return (Bug 2: `const b = writeAndReturn(a)` wrote storage even when
/// __fn_done was already 1).
fn expr_has_call(e: &Expression<'_>) -> bool {
    match e {
        Expression::CallExpression(_) => true,
        // leaf expressions — no sub-expressions to recurse into
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::Identifier(_) => false,
        // compound expressions — recurse into children
        Expression::TemplateLiteral(t) => t.expressions.iter().any(expr_has_call),
        Expression::ArrayExpression(a) => a
            .elements
            .iter()
            .any(|el| el.as_expression().is_some_and(expr_has_call)),
        Expression::ObjectExpression(o) => o.properties.iter().any(|p| match p {
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(prop) => expr_has_call(&prop.value),
            _ => false,
        }),
        Expression::BinaryExpression(b) => expr_has_call(&b.left) || expr_has_call(&b.right),
        Expression::LogicalExpression(l) => expr_has_call(&l.left) || expr_has_call(&l.right),
        Expression::UnaryExpression(u) => expr_has_call(&u.argument),
        Expression::UpdateExpression(u) => match &u.argument {
            oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(_) => false,
            _ => true, // member-access update targets (xs[i]++) may have call effects
        },
        Expression::AssignmentExpression(a) => expr_has_call(&a.right),
        Expression::ConditionalExpression(c) => {
            expr_has_call(&c.test) || expr_has_call(&c.consequent) || expr_has_call(&c.alternate)
        }
        Expression::ParenthesizedExpression(p) => expr_has_call(&p.expression),
        Expression::StaticMemberExpression(sm) => expr_has_call(&sm.object),
        Expression::ComputedMemberExpression(m) => {
            expr_has_call(&m.object) || expr_has_call(&m.expression)
        }
        Expression::ArrowFunctionExpression(_) => {
            // arrow bodies are lambda-lifted; the closure itself is pure
            // (no side effect at the allocation site)
            false
        }
        Expression::AwaitExpression(a) => expr_has_call(&a.argument),
        _ => true, // unknown expression kind → conservatively assume impure
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
            } else if matches!(&asg.right, Expression::NumericLiteral(_)) && is_string_local(&v) {
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
        // Compound assigns (2026-10-04): *= /= expand to one op application;
        // %= mirrors the binary `%` truncated-mod lowering (lisp `mod` is
        // euclidean — wrong signs for negatives), binding the rhs ONCE via
        // let (JS `x %= f()` runs f() exactly once).
        AssignmentOperator::Multiplication => list(vec![Sym("*"), Sym(v.clone()), rhs]),
        AssignmentOperator::Division => list(vec![Sym("/"), Sym(v.clone()), rhs]),
        AssignmentOperator::Remainder => {
            // lisp `mod` is euclidean (wrong signs for negatives) — mirror
            // the binary `%` truncated form: v - rhs*(v/rhs). Bind-once via
            // let when the rhs is impure (JS `x %= f()` runs f() once);
            // a pure rhs may duplicate freely.
            if expr_has_call(&asg.right) {
                let r = format!("__mod_r_{}", v);
                list(vec![
                    Sym("let"),
                    list(vec![list(vec![Sym(r.clone()), rhs])]),
                    list(vec![
                        Sym("-"),
                        Sym(v.clone()),
                        list(vec![
                            Sym("*"),
                            Sym(r.clone()),
                            list(vec![Sym("/"), Sym(v.clone()), Sym(r)]),
                        ]),
                    ]),
                ])
            } else {
                list(vec![
                    Sym("-"),
                    Sym(v.clone()),
                    list(vec![
                        Sym("*"),
                        rhs.clone(),
                        list(vec![Sym("/"), Sym(v.clone()), rhs]),
                    ]),
                ])
            }
        }
        AssignmentOperator::Exponential => {
            return Err("ts_frontend: **= unsupported — the NEAR typechecker/wasm \
                 emitter have no power builtin (the interpreter-only `expt` \
                 is not in the NEAR builtin set); use a helper with \
                 repeated multiplication"
                .into());
        }
        _ => {
            return Err(
                "ts_frontend: supported assignment operators: = += -= *= /= %= \
                 (obj.prop compound writes stay unsupported — objects are \
                 immutable JSON values; rebuild via jsonSet)"
                    .into(),
            )
        }
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
        // Parenthesized: look through (interp_arg_is_string does the same)
        Expression::ParenthesizedExpression(pe) => expr_is_stringy(&pe.expression),
        // Nullish with a STRING fallback is string-valued — the ubiquitous
        // `near.storageGet(k) ?? ""` shape (g16v verifier, 2026-09-12: a local
        // seeded with it was NOT marked stringy, so `local + STR_CONST`
        // dispatched to numeric + and produced decimal garbage).
        Expression::LogicalExpression(l) if l.operator == LogicalOperator::Coalesce => {
            expr_is_stringy(&l.right)
        }
        Expression::BinaryExpression(b) => {
            b.operator == BinaryOperator::Addition
                && (expr_is_stringy(&b.left) || expr_is_stringy(&b.right))
        }
        Expression::CallExpression(c) => match &c.callee {
            Expression::Identifier(id) => {
                matches!(
                    id.name.as_str(),
                    "toStr" | "toString" | "strCat" | "to_string"
                )
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
            if let Expression::StaticMemberExpression(m) = &c.callee {
                // Methods whose return is NOT a string (numbers/bools/void
                // per the d.ts) must not seed STRING locals: `let ok =
                // near.altBn128PairingCheck(b)` used to mark `ok` stringy,
                // so `${ok}` skipped the to-string wrap and str-cat rendered
                // the raw TAG_NUM as EMPTY (bn254 pairing probe, 2026-09-11).
                if let Expression::Identifier(obj) = &m.object {
                    if member_fn_returns_non_string(obj.name.as_str(), m.property.name.as_str()) {
                        return false;
                    }
                }
            }
            matches!(&c.callee, Expression::StaticMemberExpression(_))
        }
        _ => false,
    }
}

/// near./u128./storage. member methods with non-string returns, per the
/// d.ts surface (`: number`, `: boolean`, `: void`). Keep in sync with
/// ts/lisp-rlm.d.ts.
fn member_fn_returns_non_string(obj: &str, prop: &str) -> bool {
    match obj {
        "near" => matches!(
            prop,
            "iterPrefix"
                | "jsonGetInt"
                | "blockIndex"
                | "attachedDepositHigh"
                | "depositGte"
                | "yieldCreate"
                | "yieldResume"
                | "ed25519Verify"
                | "p256Verify"
                | "altBn128PairingCheck"
                | "bls12381PairingCheck"
                | "prepaidGas"
                | "usedGas"
                | "promiseCreate"
                | "promiseThen"
                | "promiseAnd"
                | "promiseBatchCreate"
                | "promiseBatchThen"
                | "promiseResultsCount"
                | "promiseSucceeded"
                | "storageUsage"
                | "storageHas"
                | "storageHasKey"
                | "storageSet"
                | "storageRemove"
                | "jsonReturnStr"
                | "jsonReturnInt"
                | "transfer"
                | "transferU128"
                | "storeU128"
                | "log"
                | "logNum"
                | "abort"
                | "panic"
                | "callAwait"
                | "call"
                | "promiseBatchActionTransfer"
                | "promiseBatchActionFunctionCall"
                | "promiseBatchActionCreateAccount"
                | "promiseBatchActionTransferToGasKey"
                | "promiseBatchActionAddGasKeyWithFullAccess"
                | "promiseBatchActionAddGasKeyWithFunctionCall"
                | "promiseReturn"
                | "signerAccountPk"
                | "currentAccountId"
        ),
        "u128" => matches!(prop, "lt" | "gt" | "eq" | "isZero" | "toI64"),
        "storage" => matches!(prop, "has" | "hasKey" | "set" | "write" | "del" | "remove"),
        // Math.* always returns a number — without this, `Math.pow(2,16) + 1`
        // type-probed as a string method call and the frontend folded `+`
        // into (str-cat (to-string (expt 2 16)) (to-string 1)) (2026-10-05).
        "Math" => matches!(
            prop,
            "abs" | "max" | "min" | "pow" | "sqrt" | "floor" | "ceil" | "round"
        ),
        _ => false,
    }
}

/// Shared arrow lowering: params list + body value. Used by inline arrows
/// and by `export const f = (x) => ...` (which needs the body spliced into
/// a function-shaped define — `(define f (lambda ...))` exports compile to
/// a stub, only `(define (f x) body)` produces a real entry).
fn arrow_parts(
    a: &oxc_ast::ast::ArrowFunctionExpression<'_>,
) -> Result<(Vec<LispVal>, LispVal), String> {
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
    let export_name = if name == "new_" {
        "new".to_string()
    } else {
        name.to_string()
    };
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
            Expression::LogicalExpression(l) if l.operator == LogicalOperator::Coalesce => {
                e = &l.right
            }
            _ => break,
        }
    }
    let Expression::StringLiteral(sl) = e else {
        return;
    };
    let Ok(name) = binding_name(&d.id) else {
        return;
    };
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
                    "slice"
                        | "substring"
                        | "substr"
                        | "charAt"
                        | "concat"
                        | "toUpperCase"
                        | "toLowerCase"
                        | "trim"
                        | "trimStart"
                        | "trimEnd"
                        | "repeat"
                        | "padStart"
                        | "padEnd"
                        | "at"
                        | "toString"
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

/// Lowered-init form check for hoisted bindings (u128 Level 1, 2026-09-15):
/// a u128 arith op form re-initializes the binding before any read (TDZ),
/// so these hoist with a "0" dummy — keeping limb-local eligibility —
/// instead of the nil that would demote the local function-wide.
fn init_is_u128_pure(init: &LispVal) -> bool {
    if let LispVal::List(items) = init {
        if let Some(LispVal::Sym(head)) = items.first() {
            return matches!(
                head.as_str(),
                "u128/add" | "u128/sub" | "u128/mul" | "u128/div" | "u128/mod"
            ) && items.len() == 3;
        }
    }
    false
}

fn expr_is_bigint(e: &Expression<'_>) -> bool {
    match e {
        Expression::BigIntLiteral(_) => true,
        // `x ?? 0n` — the DEFAULT defines the miss-type: storageGet
        // results are decimal strings in this ABI, so a bigint default
        // makes the whole local bigint-shaped. (HTLC 2026-09-01:
        // `bal + rec.amt` needed this; bal was `storageGet(...) ?? 0n`.)
        Expression::LogicalExpression(l) if l.operator == LogicalOperator::Coalesce => {
            expr_is_bigint(&l.right)
        }
        // `rec.amt` where rec's shape literal has a quoted-numeric default
        Expression::StaticMemberExpression(sm) => {
            let Expression::Identifier(base) = &sm.object else {
                return false;
            };
            let field = sm.property.name.as_str();
            SHAPE_BIGINT_FIELDS.with(|s| {
                s.borrow()
                    .iter()
                    .any(|(b, f)| b == base.name.as_str() && f == field)
            })
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
                    "u128Add" | "u128Sub" | "u128Mul" | "u128Div" | "u128Mod" | "u128FromNum"
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

/// (2026-10-08) Is this expression money-domain? Identifier in
/// MONEY_NAMES (annotated param, await result), a money-source call
/// (deposit/balance reads), or nested raw arithmetic over either.
/// NOTE: u128Xxx(...) calls are deliberately NOT money here — the
/// bigint-shape machinery already routes any `+` touching them to the
/// correct u128/* lowering, and gating them again would reject
/// previously-valid mixed shapes.
fn expr_is_money(e: &Expression<'_>) -> bool {
    match e {
        Expression::Identifier(id) => MONEY_NAMES
            .with(|s| s.borrow().iter().any(|x| *x == id.name.as_str())),
        Expression::ParenthesizedExpression(pe) => expr_is_money(&pe.expression),
        Expression::BinaryExpression(b) => {
            if matches!(b.operator, BinaryOperator::Addition) {
                // `+` with money on ONE side is CONCAT — the result is
                // display TEXT, not money (the contract doctrine: a
                // labeled string is not an amount). Only money+money
                // stays "money" here, and that shape is the raw-
                // arithmetic error caught by the gate — it never
                // flows onward as a value.
                money_arithmetic(&b.left, &b.right)
            } else {
                matches!(
                    b.operator,
                    BinaryOperator::Subtraction
                        | BinaryOperator::Multiplication
                        | BinaryOperator::Division
                        | BinaryOperator::Remainder
                ) && (expr_is_money(&b.left) || expr_is_money(&b.right))
            }
        }
        Expression::CallExpression(c) => match &c.callee {
            Expression::StaticMemberExpression(sm) => {
                matches!(&sm.object, Expression::Identifier(obj) if obj.name == "near")
                    && matches!(
                        sm.property.name.as_str(),
                        "attachedDeposit"
                            | "attachedDepositU128"
                            | "accountBalance"
                            | "loadU128"
                            | "readU128"
                    )
            }
            _ => false,
        },
        _ => false,
    }
}

/// The gate predicate: BOTH operands money-domain. One-sided money `+`
/// stays legal (prefix strings: `"total:" + amt` is concat by design).
fn money_arithmetic<'a, 'b>(
    l: &Expression<'a>,
    r: &Expression<'b>,
) -> bool {
    expr_is_money(l) && expr_is_money(r)
}

/// u128 free-function family that RETURNS a u128 decimal string (the
/// comparison fns return boolean and are excluded). Both spellings count —
/// `u128Add(a, b)` and `u128.add(a, b)` lower to the same builtin, and the
/// money seal must not depend on the author's spelling choice. Found via
/// the TWAP TS twin: a `: Yocto` const initialized from member arithmetic
/// was rejected as "not provable" while the identical free-fn form passed.
fn u128_arith_call(c: &oxc_ast::ast::CallExpression<'_>) -> bool {
    match &c.callee {
        Expression::Identifier(id) => matches!(
            id.name.as_str(),
            "u128Add" | "u128Sub" | "u128Mul" | "u128MulDiv" | "u128Div" | "u128Mod"
        ),
        Expression::StaticMemberExpression(sm) => {
            matches!(&sm.object, Expression::Identifier(obj) if obj.name == "u128")
                && matches!(
                    sm.property.name.as_str(),
                    "add" | "sub" | "mul" | "mulDiv" | "div" | "mod"
                )
        }
        _ => false,
    }
}

/// STRICT: this expression's VALUE is a u128 decimal string — provable
/// money, not display text. Deliberately NO binary ops: `"total:" + amt`
/// is concat (a labeled string), not an amount — that is the whole point
/// of the return-contract check.
fn expr_is_u128_value(e: &Expression<'_>) -> bool {
    match e {
        Expression::ParenthesizedExpression(pe) => expr_is_u128_value(&pe.expression),
        Expression::Identifier(id) => {
            let n = id.name.as_str();
            MONEY_NAMES.with(|s| s.borrow().iter().any(|x| *x == n))
                || BIGINT_NAMES.with(|s| s.borrow().iter().any(|x| *x == n))
                || BIGINT_LOCALS.with(|s| s.borrow().iter().any(|x| *x == n))
        }
        Expression::AwaitExpression(a) => expr_is_u128_value(&a.argument),
        Expression::StringLiteral(s) => s.value.parse::<u128>().is_ok(),
        Expression::ConditionalExpression(c) => {
            expr_is_u128_value(&c.consequent) && expr_is_u128_value(&c.alternate)
        }
        Expression::CallExpression(c) => {
            u128_arith_call(c)
                || matches!(&c.callee, Expression::StaticMemberExpression(sm)
                    if matches!(&sm.object, Expression::Identifier(obj) if obj.name == "near")
                    && matches!(
                        sm.property.name.as_str(),
                        "attachedDeposit" | "attachedDepositU128"
                            | "accountBalance" | "loadU128" | "readU128"
                    ))
        }
        _ => false,
    }
}

/// Does this function's return annotation promise money — either directly
/// (`: Yocto`) or wrapped (`Promise<Yocto>`)? Names the built-in trio or a
/// registered alias.
fn return_ann_money(f: &oxc_ast::ast::Function<'_>) -> bool {
    let Some(t) = f.return_type.as_ref().map(|v| &**v) else {
        return false;
    };
    let is_money_name = |n: &str| {
        matches!(n, "Yocto" | "Amount" | "Money")
            || MONEY_ALIASES.with(|m| m.borrow().iter().any(|x| x == n))
    };
    match &t.type_annotation {
        oxc_ast::ast::TSType::TSTypeReference(r) => {
            let oxc_ast::ast::TSTypeName::IdentifierReference(id) = &r.type_name else {
                return false;
            };
            if id.name.as_str() == "Promise" {
                if let Some(ta) = r.type_arguments.as_ref() {
                    if let Some(first) = ta.params.first() {
                        if let oxc_ast::ast::TSType::TSTypeReference(inner) = first {
                            if let oxc_ast::ast::TSTypeName::IdentifierReference(iid) =
                                &inner.type_name
                            {
                                return is_money_name(iid.name.as_str());
                            }
                        }
                    }
                }
                return false;
            }
            is_money_name(id.name.as_str())
        }
        _ => false,
    }
}

/// Walk a statement slice recursively; invoke `f` on every return argument.
/// Mirrors stmts_have_deep_return's traversal (blocks, if/else, while, for).
fn for_each_return_arg(
    stmts: &[Statement<'_>],
    f: &mut dyn FnMut(&Expression<'_>),
) {
    for s in stmts {
        match s {
            Statement::ReturnStatement(r) => {
                if let Some(e) = r.argument.as_ref() {
                    f(e);
                }
            }
            Statement::BlockStatement(b) => {
                for_each_return_arg(&b.body, f);
            }
            Statement::IfStatement(i) => {
                for_each_return_arg(stmts_of(&i.consequent), f);
                if let Some(alt) = i.alternate.as_ref() {
                    for_each_return_arg(stmts_of(alt), f);
                }
            }
            Statement::WhileStatement(w) => {
                for_each_return_arg(stmts_of(&w.body), f);
            }
            Statement::ForStatement(fo) => {
                for_each_return_arg(stmts_of(&fo.body), f);
            }
            _ => {}
        }
    }
}

/// The method name of a `near.call(target, method, ...)` expression,
/// when it is a string literal. A dynamic method name is `None`: the
/// binding flows as Value (permissive on the unknown).
fn near_call_method<'a>(e: &Expression<'a>) -> Option<&'a str> {
    let Expression::CallExpression(c) = e else {
        return None;
    };
    let Expression::StaticMemberExpression(sm) = &c.callee else {
        return None;
    };
    let Expression::Identifier(obj) = &sm.object else {
        return None;
    };
    if obj.name != "near" || sm.property.name != "call" {
        return None;
    }
    // near.call(target, method, args, gas, deposit) — method is arg 1
    match c.arguments.get(1).and_then(|a| a.as_expression()) {
        Some(Expression::StringLiteral(s)) => Some(s.value.as_str()),
        _ => None,
    }
}

/// Awaited methods whose RESULT is a u128 decimal string — the money
/// SOURCES. NEP-141 balance/supply reads. Every other await result is
/// Value: a `getName` string is not yocto, and returning it as
/// `: Promise<Yocto>` is exactly the lie the contract check kills.
fn await_method_is_money(m: &str) -> bool {
    matches!(
        m,
        "ftBalanceRaw"
            | "ftBalanceOf"
            | "balanceOf"
            | "ftTotalSupply"
            | "ftSupplyFor"
            | "ftStorageBalanceOf"
    )
}

/// Seed MONEY_NAMES for one await binding. near.all binds positionally
/// (name j <-> element j), a plain await binds its one name — and only
/// balance/supply methods seed. This is what makes the registry mean
/// "u128 decimal string" instead of "came from an await".
fn seed_await_money_names(arg: &Expression<'_>, names: &[String]) {
    if let Expression::CallExpression(c) = arg {
        if let Expression::StaticMemberExpression(sm) = &c.callee {
            if let Expression::Identifier(obj) = &sm.object {
                if obj.name == "near" && sm.property.name == "all" {
                    if let Some(args0) = c.arguments.first().and_then(|a| a.as_expression()) {
                        if let Expression::ArrayExpression(arr) = args0 {
                            for (j, n) in names.iter().enumerate() {
                                let money = arr
                                    .elements
                                    .get(j)
                                    .and_then(|el| el.as_expression())
                                    .and_then(near_call_method)
                                    .is_some_and(await_method_is_money);
                                if money {
                                    MONEY_NAMES.with(|s| s.borrow_mut().push(n.clone()));
                                }
                            }
                            return;
                        }
                    }
                }
            }
        }
        if near_call_method(arg).is_some_and(await_method_is_money) {
            for n in names {
                MONEY_NAMES.with(|s| s.borrow_mut().push(n.clone()));
            }
        }
    }
}

/// Pre-pass for check_return_contract: register every
/// `const x = await ...` / destructured-await binding as money so the
/// return walk sees it. Mirrors the AwaitPoint scan's semantics.
fn register_await_names(stmts: &[Statement<'_>]) {
    for s in stmts {
        match s {
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let money_ann = declarator_ann_is_money(d);
                    if money_ann {
                        if let Ok(names) = pattern_names(&d.id) {
                            for n in names {
                                MONEY_NAMES.with(|s| s.borrow_mut().push(n));
                            }
                        }
                    } else if let Some(Expression::AwaitExpression(ae)) = &d.init {
                        if let Ok(names) = pattern_names(&d.id) {
                            seed_await_money_names(&ae.argument, &names);
                        }
                    } else if d
                        .init
                        .as_ref()
                        .is_some_and(|iv| is_storage_read_fallback(iv))
                    {
                        // Ledger boundary (SEAL tier): `const bal =
                        // storageGet(k) ?? "0"` is u128-domain by contract
                        // convention — carries the seal AND the arithmetic rule.
                        if let Ok(names) = pattern_names(&d.id) {
                            for n in names {
                                MONEY_NAMES.with(|s| s.borrow_mut().push(n));
                            }
                        }
                    } else if d
                        .init
                        .as_ref()
                        .is_some_and(|iv| is_boundary_stamp(iv))
                    {
                        // Param-boundary stamp: `const amount =
                        // jsonGetStr("amount") ?? STORAGE_MIN` — the author's
                        // custody declaration. LEDGER tier: transferable to
                        // sinks, NO arithmetic seal — the reader cannot know
                        // the domain (groth16 pulls HEX through the same
                        // jsonGetStr). `: Yocto` is how an author asserts the
                        // money domain and gets the full seal.
                        if let Ok(names) = pattern_names(&d.id) {
                            for n in names {
                                LEDGER_NAMES.with(|s| s.borrow_mut().push(n));
                            }
                        }
                    } else if d
                        .init
                        .as_ref()
                        .is_some_and(|iv| is_ledger_read(iv))
                    {
                        // Ledger tier: `const pn = storageGet(k) ?? ""` —
                        // sink-transferable, NOT arithmetic-sealed.
                        if let Ok(names) = pattern_names(&d.id) {
                            for n in names {
                                LEDGER_NAMES.with(|s| s.borrow_mut().push(n));
                            }
                        }
                    } else if let Some(Expression::CallExpression(c)) = &d.init {
                        // Op-closure inference: `const t = u128Add(a, b)` is
                        // money by the op's signature (Yocto x Yocto -> Yocto).
                        // A bare string literal is Str — NOT money (the
                        // customs seal; its fix is `: Yocto` at birth).
                        if u128_arith_call(c) {
                            if let Ok(names) = pattern_names(&d.id) {
                                for n in names {
                                    MONEY_NAMES.with(|s| s.borrow_mut().push(n));
                                }
                            }
                        }
                    }
                }
            }
            Statement::BlockStatement(b) => register_await_names(&b.body),
            Statement::IfStatement(i) => {
                register_await_names(stmts_of(&i.consequent));
                if let Some(alt) = i.alternate.as_ref() {
                    register_await_names(stmts_of(alt));
                }
            }
            Statement::WhileStatement(w) => register_await_names(stmts_of(&w.body)),
            Statement::ForStatement(fo) => register_await_names(stmts_of(&fo.body)),
            _ => {}
        }
    }
}

/// Return CONTRACT: a function promising `Yocto`/`Amount` (or a Promise of
/// one) must return a provable u128 value — await result, u128 arithmetic
/// call, deposit/balance read, or a decimal string literal. Labeled strings
/// (`"total:" + x`) are display text, not money: annotate `: Promise<string>`
/// instead. This is the annotation made checkable.
fn check_return_contract(
    fname: &str,
    body_stmts: &[Statement<'_>],
    ret_money: bool,
) -> Result<(), String> {
    if !ret_money {
        return Ok(());
    }
    // The lowering registers await-result names AFTER this check runs (the
    // AwaitPoint scan at lowering time) — pre-register them here so
    // `return a` after `const a = await ...` is provably money. The later
    // registration pushing the same names again is harmless (membership).
    register_await_names(body_stmts);
    let mut bad: Option<String> = None;
    for_each_return_arg(body_stmts, &mut |e| {
        if bad.is_none() && !expr_is_u128_value(e) {
            bad = Some(match e {
                Expression::BinaryExpression(b)
                    if matches!(
                        b.operator,
                        oxc_ast::ast::BinaryOperator::Addition
                            | oxc_ast::ast::BinaryOperator::Subtraction
                            | oxc_ast::ast::BinaryOperator::Multiplication
                            | oxc_ast::ast::BinaryOperator::Division
                            | oxc_ast::ast::BinaryOperator::Remainder
                    ) && money_arithmetic(&b.left, &b.right) =>
                    // the return IS money-op-money — the canonical raw-
                    // arithmetic error is the precise diagnosis; the
                    // contract check just caught it earlier (entry, not
                    // lowering). Match the taint lint's wording verbatim.
                    "raw arithmetic on money values (Yocto/Amount) corrupts u128 decimal strings — `+` concatenates, other ops use i64 math. Use the u128* family: u128Add(a, b), u128Sub(a, b), ...".into(),
                Expression::BinaryExpression(b)
                    if matches!(b.operator, oxc_ast::ast::BinaryOperator::Addition) =>
                    "concatenation (`+` with a label) — annotate the fn `: Promise<string>`, or return the raw u128 value".into(),
                _ => "not a provable u128 value — return an await result, u128Add(...), a deposit/balance read, or a decimal string".into(),
            });
        }
    });
    match bad {
        Some(why) => Err(format!(
            "ts_frontend: `{fname}` promises a money type (Yocto/Amount) but its return is {why}"
        )),
        None => Ok(()),
    }
}

/// `const d: Yocto = ...` — a money-annotated declaration must initialize
/// from a provable u128 value, and the name joins the taint registry.
fn check_and_register_money_const(
    name: &str,
    ann_is_money: bool,
    init: &Expression<'_>,
) -> Result<(), String> {
    if !ann_is_money {
        return Ok(());
    }
    if !expr_is_u128_value(init) && !is_boundary_stamp(init) {
        return Err(format!(
            "ts_frontend: `const {name}` is annotated as a money type but its initializer is not a provable u128 value — use an await result, u128Add(...), a deposit/balance read, or a decimal string"
        ));
    }
    MONEY_NAMES.with(|s| s.borrow_mut().push(name.to_string()));
    Ok(())
}

/// Walk every CallExpression in a statement tree — statements, nested
/// blocks/if/loops, and expression positions (init, conditions, args).
fn for_each_stmt_call<'b>(
    stmts: &'b [Statement<'b>],
    f: &mut dyn FnMut(&oxc_ast::ast::CallExpression<'b>),
) {
    use oxc_ast::ast::Expression;
    fn walk_expr<'b>(
        e: &Expression<'b>,
        f: &mut dyn FnMut(&oxc_ast::ast::CallExpression<'b>),
    ) {
        match e {
            Expression::CallExpression(c) => {
                f(c);
                for a in &c.arguments {
                    if let Some(x) = a.as_expression() {
                        walk_expr(x, f);
                    }
                }
            }
            Expression::StaticMemberExpression(sm) => walk_expr(&sm.object, f),
            Expression::ComputedMemberExpression(cm) => walk_expr(&cm.object, f),
            Expression::AwaitExpression(a) => walk_expr(&a.argument, f),
            Expression::ParenthesizedExpression(p) => walk_expr(&p.expression, f),
            Expression::BinaryExpression(b) => {
                walk_expr(&b.left, f);
                walk_expr(&b.right, f);
            }
            Expression::LogicalExpression(l) => {
                walk_expr(&l.left, f);
                walk_expr(&l.right, f);
            }
            Expression::UnaryExpression(u) => walk_expr(&u.argument, f),
            Expression::ConditionalExpression(c) => {
                walk_expr(&c.test, f);
                walk_expr(&c.consequent, f);
                walk_expr(&c.alternate, f);
            }
            Expression::TemplateLiteral(t) => {
                for x in &t.expressions {
                    walk_expr(x, f);
                }
            }
            Expression::ArrayExpression(a) => {
                for el in &a.elements {
                    if let Some(x) = el.as_expression() {
                        walk_expr(x, f);
                    }
                }
            }
            Expression::ObjectExpression(o) => {
                for prop in &o.properties {
                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(op) = prop {
                        walk_expr(&op.value, f);
                    }
                }
            }
            Expression::SequenceExpression(s) => {
                for x in &s.expressions {
                    walk_expr(x, f);
                }
            }
            Expression::AssignmentExpression(a) => walk_expr(&a.right, f),
            _ => {}
        }
    }
    for s in stmts {
        match s {
            Statement::ExpressionStatement(es) => walk_expr(&es.expression, f),
            Statement::VariableDeclaration(v) => {
                for d in &v.declarations {
                    if let Some(init) = &d.init {
                        walk_expr(init, f);
                    }
                }
            }
            Statement::ReturnStatement(r) => {
                if let Some(x) = r.argument.as_ref() {
                    walk_expr(x, f);
                }
            }
            Statement::IfStatement(i) => {
                walk_expr(&i.test, f);
                for_each_stmt_call(stmts_of(&i.consequent), f);
                if let Some(alt) = i.alternate.as_ref() {
                    match alt {
                        oxc_ast::ast::Statement::BlockStatement(b) => {
                            for_each_stmt_call(&b.body, f)
                        }
                        other => {
                            let sl = std::slice::from_ref(other);
                            for_each_stmt_call(sl, f);
                        }
                    }
                }
            }
            Statement::BlockStatement(b) => for_each_stmt_call(&b.body, f),
            Statement::WhileStatement(w) => {
                walk_expr(&w.test, f);
                for_each_stmt_call(stmts_of(&w.body), f);
            }
            Statement::ForStatement(fo) => {
                if let Some(oxc_ast::ast::ForStatementInit::VariableDeclaration(vd)) = &fo.init {
                    for d in &vd.declarations {
                        if let Some(init) = &d.init {
                            walk_expr(init, f);
                        }
                    }
                }
                if let Some(test) = &fo.test {
                    walk_expr(test, f);
                }
                if let Some(upd) = &fo.update {
                    walk_expr(upd, f);
                }
                for_each_stmt_call(stmts_of(&fo.body), f);
            }
            _ => {}
        }
    }
}

/// The two ABIs where untyped strings enter the contract: the PARAM
/// boundary (jsonGet) and the LEDGER boundary (storageGet — u128 keys
/// by contract convention, `?? "0"` for the empty ledger).
fn boundary_reader_name<'a>(e: &Expression<'a>) -> Option<&'a str> {
    match e {
        Expression::CallExpression(c) => {
            if let Expression::Identifier(id) = &c.callee {
                if id.name == "jsonGet" {
                    return Some("jsonGet");
                }
            }
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                if let Expression::Identifier(obj) = &sm.object {
                    if obj.name == "near" {
                        if sm.property.name == "jsonGet" {
                            return Some("jsonGet");
                        }
                        if sm.property.name == "jsonGetStr" {
                            return Some("jsonGetStr");
                        }
                        if sm.property.name == "storageGet" {
                            return Some("storageGet");
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// `near.storageGet(k) ?? "0"` — the LEDGER-read idiom. The fallback
/// must be a digit-parsable literal: `?? "0"` declares a u128 ledger,
/// while `?? ""` declares optional TEXT (JSON rows, comma buffers) —
/// those do NOT carry the seal.
fn is_storage_read_fallback(e: &Expression<'_>) -> bool {
    match e {
        Expression::AwaitExpression(a) => is_storage_read_fallback(&a.argument),
        Expression::ParenthesizedExpression(pe) => is_storage_read_fallback(&pe.expression),
        Expression::LogicalExpression(l)
            if l.operator == oxc_ast::ast::LogicalOperator::Coalesce =>
        {
            boundary_reader_name(&l.left) == Some("storageGet")
                && match &l.right {
                    Expression::BigIntLiteral(_) => true,
                    Expression::StringLiteral(s) => s.value.parse::<u128>().is_ok(),
                    _ => false,
                }
        }
        Expression::CallExpression(_) => boundary_reader_name(e) == Some("storageGet"),
        _ => false,
    }
}

/// `near.storageGet(...) ?? ""` (or a bare read) — the contract's OWN
/// storage, whose ledger keys are u128 by convention and whose missing
/// case the contract already guards (`ERR_NO_POOL`-style aborts). This
/// is the LEDGER tier: values may feed SINKS (the custody check) but
/// carry NO arithmetic seal — `a + b` between two such reads stays a
/// raw-arithmetic error. The u128-typed readers (`loadU128`/`readU128`)
/// are the migration path to the full seal.
fn is_ledger_read(e: &Expression<'_>) -> bool {
    match e {
        Expression::AwaitExpression(a) => is_ledger_read(&a.argument),
        Expression::ParenthesizedExpression(pe) => is_ledger_read(&pe.expression),
        Expression::LogicalExpression(l)
            if l.operator == oxc_ast::ast::LogicalOperator::Coalesce =>
        {
            boundary_reader_name(&l.left) == Some("storageGet")
                && matches!(&l.right, Expression::StringLiteral(_))
        }
        Expression::CallExpression(_) => boundary_reader_name(e) == Some("storageGet"),
        _ => false,
    }
}

/// The sanctioned BOUNDARY STAMP (typed-surface rule 5, in TS form):
/// `const amt: Yocto = jsonGet("amt", p)` — the ABI param reader, or
/// `const bal = near.storageGet(k) ?? "0"` — the ledger boundary. A
/// cross-boundary value cannot be PROVEN at compile time; the author
/// stamps it, and the stamp is greppable for audit. Anything else
/// (arbitrary calls, template concat, numbers) is NOT stampable:
/// u128 is not i64, and display text is not an amount.
fn is_boundary_stamp(e: &Expression<'_>) -> bool {
    match e {
        Expression::AwaitExpression(a) => is_boundary_stamp(&a.argument),
        Expression::ParenthesizedExpression(pe) => is_boundary_stamp(&pe.expression),
        Expression::LogicalExpression(l)
            if l.operator == oxc_ast::ast::LogicalOperator::Coalesce =>
        {
            match boundary_reader_name(&l.left) {
                // param boundary: the read is the author's custody point —
                // the fallback choice is theirs too (STORAGE_MIN etc.)
                Some("jsonGet") | Some("jsonGetStr") => true,
                // ledger boundary: digit-parsable fallback declares a
                // u128 key; `?? ""` is optional TEXT and does not stamp
                Some("storageGet") => match &l.right {
                    Expression::BigIntLiteral(_) => true,
                    Expression::StringLiteral(s) => s.value.parse::<u128>().is_ok(),
                    _ => false,
                },
                _ => false,
            }
        }
        Expression::CallExpression(_) => boundary_reader_name(e).is_some(),
        _ => false,
    }
}

/// SINK check: `near.transferU128(to, amount)` — the amount slot moves
/// real value on chain, so it demands a provable u128 (the customs
/// seal). An unbranded `const s = "5"` is Str/Value, not Yocto — the
/// fix is the annotation at birth: `const s: Yocto = "5"`.
fn check_money_sinks(stmts: &[Statement<'_>]) -> Result<(), String> {
    // Same pre-pass as the return contract: annotate-birth and await/op
    // registrations happen at lowering, AFTER this entry check.
    register_await_names(stmts);
    let bad: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    for_each_stmt_call(stmts, &mut |c| {
        if bad.borrow().is_some() {
            return;
        }
        let Expression::StaticMemberExpression(sm) = &c.callee else {
            return;
        };
        let Expression::Identifier(obj) = &sm.object else {
            return;
        };
        if obj.name != "near" || sm.property.name != "transferU128" || c.arguments.len() != 2 {
            return;
        }
        let Some(amt) = c.arguments.get(1).and_then(|a| a.as_expression()) else {
            return;
        };
        let amt_ok = expr_is_u128_value(amt)
            || is_boundary_stamp(amt)
            || is_ledger_read(amt)
            || matches!(&amt, Expression::Identifier(id)
                if LEDGER_NAMES.with(|s| s.borrow().iter().any(|x| *x == id.name.as_str())));
        if !amt_ok {
            *bad.borrow_mut() = Some(
                "transferU128 amount must be a provable u128 value (await result, u128Add(...), deposit/balance read, a `: Yocto`-annotated const, a storageGet ledger read, or jsonGet at the ABI boundary) — an unannotated string is not checked money".into(),
            );
        }
    });
    match bad.into_inner() {
        Some(why) => Err(format!("ts_frontend: {why}")),
        None => Ok(()),
    }
}

/// `const d: Yocto = ...` — the declarator's id annotation names a money
/// alias. BindingPattern has no type_annotation field; the idiom mirrors
/// the type-alias walk at the top of the file (ann → TSTypeAnnotation).
fn declarator_ann_is_money(d: &oxc_ast::ast::VariableDeclarator<'_>) -> bool {
    // oxc 0.147: the annotation lives on the DECLARATOR (estree
    // VariableDeclaratorId hoist), not on the binding identifier.
    ann_is_money_alias(d.type_annotation.as_ref().map(|v| &**v))
}

/// `d: Yocto` / `x: Amount` / `m: Money` — the annotation names a money
/// alias (the built-in trio, or an alias defined as one).
fn ann_is_money_alias(t: Option<&oxc_ast::ast::TSTypeAnnotation<'_>>) -> bool {
    let Some(a) = t else { return false };
    match &a.type_annotation {
        oxc_ast::ast::TSType::TSTypeReference(r) => match &r.type_name {
            oxc_ast::ast::TSTypeName::IdentifierReference(id) => {
                let n = id.name.as_str();
                matches!(n, "Yocto" | "Amount" | "Money")
                    || MONEY_ALIASES.with(|m| m.borrow().iter().any(|x| x == n))
            }
            _ => false,
        },
        _ => false,
    }
}

fn lower_expr(e: &Expression<'_>) -> Result<LispVal, String> {
    match e {
        Expression::NumericLiteral(n) => Ok(Num(n.value as i64)),
        Expression::StringLiteral(s) => Ok(Str(s.value.as_str().to_string())),
        Expression::BooleanLiteral(b) => Ok(LispVal::Bool(b.value)),
        Expression::NullLiteral(_) => Ok(LispVal::Nil),
        // u128-style digits-as-string. oxc raw for `1000n` is "1000n" —
        // strip the suffix: a stray 'n' would trap the u128/* parsers.
        Expression::BigIntLiteral(b) => Ok(Str(b
            .raw
            .as_ref()
            .map(|s| s.as_str().trim_end_matches('n').to_string())
            .unwrap_or_default())),
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
                    other => items.push(lower_expr(
                        other
                            .as_expression()
                            .ok_or("ts_frontend: unsupported array element (M1)")?,
                    )?),
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
                        // JSON API v3 (2026-09-15): property read on a
                        // near.input() handle → the CACHED-INPUT getter,
                        // dot-path preserved, NIL-ON-MISS (unlike legacy
                        // o.k on plain strings — "" contract). `o.prop ?? fb`
                        // fires its fallback; bare `o.prop` on a miss yields
                        // nil (to-string renders "nil" — visible, not silent).
                        if is_input_handle(id.name.as_str()) {
                            let dotted: Vec<&str> = path.iter().rev().map(|s| s.as_str()).collect();
                            if dotted.len() == 1 {
                                return Ok(list(vec![
                                    Sym("near/json_get_str"),
                                    Str(dotted.join(".")),
                                ]));
                            }
                            // Nested handle path `o.a.b.c`: top key via the
                            // input getter (its span is a full JSON value —
                            // buffer-compatible), remainder via the buffer
                            // dot-path scanner. Buffer reads are ""-on-miss
                            // (legacy contract) — `??` on nested paths is
                            // rejected in the Coalesce arm below.
                            let top = dotted[0].to_string();
                            let rest = dotted[1..].join(".");
                            return Ok(list(vec![
                                Sym("json-get-str"),
                                Str(rest),
                                list(vec![Sym("near/json_get_str"), Str(top)]),
                            ]));
                        }
                        // Object-param property read (`params.to`): the
                        // param binds a DEAD nil — its entry prologue binds
                        // (near/json_get_str "params"), a top-level args
                        // key that never exists — so reads go through the
                        // cached input getter with the dotted path. Same
                        // nil-on-miss contract as the input handles above.
                        // (BUG fix 2026-10-07: this read used to lower to
                        // (json-get-str "to" params) → silent nil at
                        // runtime while compiling clean.) Number-typed
                        // leaf fields keep the auto str->num decode the
                        // old dead-binding arm performed.
                        if OBJECT_PARAMS
                            .with(|s| s.borrow().iter().any(|x| x == id.name.as_str()))
                        {
                            let dotted: Vec<&str> = path.iter().rev().map(|s| s.as_str()).collect();
                            let get = if dotted.len() == 1 {
                                list(vec![
                                    Sym("near/json_get_str"),
                                    Str(dotted.join(".")),
                                ])
                            } else {
                                let top = dotted[0].to_string();
                                let rest = dotted[1..].join(".");
                                list(vec![
                                    Sym("json-get-str"),
                                    Str(rest),
                                    list(vec![Sym("near/json_get_str"), Str(top)]),
                                ])
                            };
                            let is_num_leaf = dotted.len() == 1
                                && OBJ_PARAM_PROPS.with(|s| {
                                    s.borrow()
                                        .iter()
                                        .find(|(n, _)| n == id.name.as_str())
                                        .and_then(|(_, props)| {
                                            props
                                                .iter()
                                                .find(|(k, _)| k.as_str() == dotted[0])
                                                .map(|(_, num)| *num)
                                        })
                                })
                                .unwrap_or(false);
                            return Ok(if is_num_leaf {
                                list(vec![Sym("str->num"), get])
                            } else {
                                get
                            });
                        }
                    }
                    let recv = lower_expr(base)?;
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
                                sl.value.is_empty() || sl.value.bytes().any(|b| !b.is_ascii_digit())
                            }
                            Expression::TemplateLiteral(_) => true,
                            // STRING LOCALS concat too (pool.ts bigDiv, found
                            // 2026-09-27): `out = out + u128Div(...)` with
                            // `let out = ""` — out is a string local, but the
                            // identifier shape fell through every arm →
                            // u128/add(out, …) → parse("") trap. Excluded:
                            // bigint locals — a `let acc = "0"` u128
                            // accumulator is BOTH a string local and a
                            // bigint local, and there the + means arithmetic.
                            Expression::Identifier(id) => {
                                is_string_local(id.name.as_str())
                                    && !BIGINT_LOCALS
                                        .with(|m| m.borrow().iter().any(|x| *x == id.name.as_str()))
                                    && !BIGINT_NAMES
                                        .with(|m| m.borrow().iter().any(|x| *x == id.name.as_str()))
                                    && !BIGINT_CONSTS
                                        .with(|m| m.borrow().iter().any(|x| *x == id.name.as_str()))
                            }
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
                        return Ok(list(vec![Sym("not"), list(vec![Sym("u128/eq"), l, r])]))
                    }
                    _ => {
                        return Err(format!(
                            "ts_frontend: operator {:?} not supported on bigint operands",
                            b.operator
                        ))
                    }
                }
            }
            // (2026-10-08) Money-taint gate: raw arithmetic on TWO
            // money-domain values corrupts u128 decimal strings — `+`
            // concatenates (balances merge instead of summing), - * / %
            // use i64 math (truncation). Fires only when both sides are
            // money (MONEY_NAMES idents = await results + annotated
            // params/locals, deposit/balance reads, nested raw money
            // arith). Bigint-shaped operands dispatched above to u128/*;
            // one-sided money `+` stays legal (prefix concat: "total:" +
            // amt is the DSL's display idiom).
            if matches!(
                b.operator,
                BinaryOperator::Addition
                    | BinaryOperator::Subtraction
                    | BinaryOperator::Multiplication
                    | BinaryOperator::Division
                    | BinaryOperator::Remainder
            ) && money_arithmetic(&b.left, &b.right)
            {
                return Err(
                    "ts_frontend: raw arithmetic on money values (Yocto/Amount) corrupts u128 decimal strings — `+` concatenates, other ops use i64 math. Use the u128* family: u128Add(a, b), u128Sub(a, b), ...".into(),
                );
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
            //
            // 2026-09-12: top-level `const K = "…"` identifiers count too —
            // CONST_FOLDS substitutes them at LOWER time, but this dispatch
            // runs BEFORE the substitution and only saw a bare Identifier
            // (→ false → numeric +). The g16v verifier built a 198B multiexp
            // buffer instead of 288B from exactly this: `(storageGet() ??
            // "") + ONE_HEX` evaluated as num + num → decimal garbage.
            if b.operator == BinaryOperator::Addition {
                let side_is_string_local = |e: &Expression| match e {
                    Expression::Identifier(id) => {
                        is_string_local(id.name.as_str())
                            || CONST_FOLDS.with(|m| {
                                m.borrow().iter().any(|(k, v)| {
                                    k == id.name.as_str() && matches!(v, LispVal::Str(_))
                                })
                            })
                    }
                    _ => false,
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
                    list(vec![Sym("*"), bsym.clone(), list(vec![Sym("/"), a, bsym])]),
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
                BinaryOperator::Exponential => {
                    // No power builtin exists in the NEAR typechecker/wasm
                    // emitter (the interpreter-only `expt` is not in the
                    // NEAR builtin set — probe E, 2026-10-04). Hard error,
                    // never a silent wrong lowering.
                    return Err("ts_frontend: `**` unsupported — no power builtin in the \
                         NEAR typechecker/wasm emitter (interpreter-only `expt` is \
                         not in the NEAR builtin set); use a helper with repeated \
                         multiplication"
                        .into());
                }
                _ => {
                    return Err(
                        "ts_frontend: exponent/assign-ops in expressions not supported".into(),
                    )
                }
            };
            Ok(list(vec![
                Sym(op),
                lower_expr(&b.left)?,
                lower_expr(&b.right)?,
            ]))
        }
        Expression::LogicalExpression(l) => {
            // && / ||: JS VALUE semantics (2026-10-04) — `a || b` yields a
            // when a is truthy else b; `a && b` yields b when a is truthy
            // else a. The left operand is let-bound so it evaluates AT MOST
            // once (`f() || x` runs f exactly once); the branch test is the
            // RAW bound value — emit_cond_branch truthiness (falsy = {Bool
            // false, Nil, Num 0}; STR — including "" — is truthy, the
            // documented M2 boundary mirroring `if (s)`). Replaces the old
            // always-0/1 boolean coercion.
            let lv_and = |t: &str, a_val: LispVal, b_val: LispVal| {
                list(vec![
                    Sym("let"),
                    list(vec![list(vec![Sym(t.to_string()), a_val])]),
                    list(vec![
                        Sym("if"),
                        Sym(t.to_string()),
                        b_val,
                        Sym(t.to_string()),
                    ]),
                ])
            };
            Ok(match l.operator {
                LogicalOperator::And => lv_and(
                    &format!("__lv_and{}", l.span.start),
                    lower_expr(&l.left)?,
                    lower_expr(&l.right)?,
                ),
                LogicalOperator::Or => {
                    let t = format!("__lv_or{}", l.span.start);
                    list(vec![
                        Sym("let"),
                        list(vec![list(vec![Sym(t.clone()), lower_expr(&l.left)?])]),
                        list(vec![
                            Sym("if"),
                            Sym(t.clone()),
                            Sym(t),
                            lower_expr(&l.right)?,
                        ]),
                    ])
                }
                // `a ?? b` — value-level nil-handling: (default a b).
                // JSON API v3 (2026-09-15): `handle.prop ?? fb` dispatches on
                // the FALLBACK's literal type — number fb → the INT getter
                // (typed read, no strToNum ceremony), string fb → the STR
                // getter. Both input getters are nil-on-miss so `default`
                // fires exactly when JS `??` would (missing key).
                LogicalOperator::Coalesce => {
                    if let Expression::StaticMemberExpression(sm) = &l.left {
                        let mut root = &sm.object;
                        let mut path = vec![sm.property.name.as_str().to_string()];
                        loop {
                            match root {
                                Expression::StaticMemberExpression(inner) => {
                                    path.push(inner.property.name.as_str().to_string());
                                    root = &inner.object;
                                }
                                _ => break,
                            }
                        }
                        if let Expression::Identifier(id) = root {
                            if is_input_handle(id.name.as_str()) {
                                let dotted: Vec<&str> =
                                    path.iter().rev().map(|s| s.as_str()).collect();
                                if dotted.len() > 1 {
                                    // nested handle path + ?? (2026-09-15):
                                    // top key via the input getter, rest via
                                    // the NIL-ON-MISS buffer op (json-get-str?)
                                    // so the fallback fires on a miss. Number
                                    // fallbacks parse the span (str->num over
                                    // the default).
                                    let top = dotted[0].to_string();
                                    let rest = dotted[1..].join(".");
                                    let read = list(vec![
                                        Sym("json-get-str?"),
                                        Str(rest),
                                        list(vec![Sym("near/json_get_str"), Str(top)]),
                                    ]);
                                    return Ok(match &l.right {
                                        Expression::NumericLiteral(n) => list(vec![
                                            Sym("str->num"),
                                            list(vec![
                                                Sym("default"),
                                                read,
                                                Str(format!("{}", n.value)),
                                            ]),
                                        ]),
                                        Expression::StringLiteral(s) => list(vec![
                                            Sym("default"),
                                            read,
                                            Str(s.value.to_string()),
                                        ]),
                                        _ => {
                                            return Err(
                                                "ts_frontend: `handle.a.b ?? fb` — fallback must be a string or number literal"
                                                    .into(),
                                            )
                                        }
                                    });
                                }
                                let dotted = dotted.join(".");
                                match &l.right {
                                    Expression::NumericLiteral(n) => {
                                        let fb = n.value as i64;
                                        return Ok(list(vec![
                                            Sym("default"),
                                            list(vec![
                                                Sym("near/json_get_int"),
                                                Str(dotted),
                                            ]),
                                            Num(fb),
                                        ]));
                                    }
                                    Expression::StringLiteral(s) => {
                                        return Ok(list(vec![
                                            Sym("default"),
                                            list(vec![
                                                Sym("near/json_get_str"),
                                                Str(dotted),
                                            ]),
                                            Str(s.value.to_string()),
                                        ]));
                                    }
                                    _ => {
                                        return Err(
                                            "ts_frontend: `handle.prop ?? fb` — fallback must be a string or number literal (v3)"
                                                .into(),
                                        )
                                    }
                                }
                            }
                        }
                    }
                    list(vec![
                        Sym("default"),
                        lower_expr(&l.left)?,
                        lower_expr(&l.right)?,
                    ])
                }
            })
        }
        Expression::UnaryExpression(u) => match u.operator {
            UnaryOperator::LogicalNot => {
                // JS parity (2026-09-27): falsy = {Bool false, Nil, Num 0, ""}.
                // Tag-aware `if` alone misses "" — a string (even empty) is
                // STR-tagged → truthy, so `if (!name)` with name === "" never
                // fired. The naive `!"" → (strLength x) == 0` alternative is
                // WRONG for numbers: str-len is an untagged `>> 32` with no
                // type check, and a num's payload is its value, so
                // strLength(5) = 0 → !5 would be true.
                //
                // Lowering: bind the operand once (side effects must run
                // exactly once — nested !!x shadowing via let is safe, same
                // mechanism as __wl_* loop locals), then
                //   not(x) = if (if x false true) true (= x "")
                // The inner tag-aware if handles {false, Nil, Num 0}; the
                // outer arm catches exactly "". `(= x "")` is safe on ANY
                // operand: = compiles to __h_val_eq (structural), which
                // returns false on tag mismatch WITHOUT trapping
                // (const_fold.rs eq(): numeric fast-path raw i64.eq is
                // equally exact across {Num, Nil, Bool} tag words; helper
                // path tag-mismatch → 0). So (= 5 "") → false → !5 → false,
                // matching JS.
                //
                // Known leftover (deliberate): `if (s)` with s === "" still
                // takes the then-branch — wrapping every condition would add
                // the empty-string test to all branches of all contracts
                // (gas + code size) for a rare pattern. See GAPS.md.
                Ok(list(vec![
                    Sym("let"),
                    list(vec![list(vec![Sym("__not_tmp"), lower_expr(&u.argument)?])]),
                    list(vec![
                        Sym("if"),
                        list(vec![
                            Sym("if"),
                            Sym("__not_tmp"),
                            list(vec![Sym("="), Num(1), Num(0)]),
                            list(vec![Sym("="), Num(1), Num(1)]),
                        ]),
                        list(vec![Sym("="), Num(1), Num(1)]),
                        list(vec![Sym("="), Sym("__not_tmp"), Str(String::new())]),
                    ]),
                ]))
            }
            UnaryOperator::UnaryNegation => {
                Ok(list(vec![Sym("-"), Num(0), lower_expr(&u.argument)?]))
            }
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
                                let e2 = a
                                    .as_expression()
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
                        ("Math", "abs")
                        | ("Math", "max")
                        | ("Math", "min")
                        | ("Math", "sqrt")
                        | ("Math", "floor")
                        | ("Math", "round") => {
                            let op = sm.property.name.as_str();
                            if c.arguments.is_empty() {
                                return Err(format!(
                                    "ts_frontend: Math.{} needs at least one argument",
                                    op
                                ));
                            }
                            let mut items = vec![Sym(op)];
                            for a in &c.arguments {
                                let e2 = a
                                    .as_expression()
                                    .ok_or("ts_frontend: unsupported Math argument (M1)")?;
                                items.push(lower_expr(e2)?);
                            }
                            return Ok(list(items));
                        }
                        ("Math", "pow") => {
                            // Math.pow(a, b) → (expt a b): expt is the
                            // runtime's power builtin; the old fallthrough
                            // minted an unknown Math/pow symbol instead.
                            if c.arguments.len() != 2 {
                                return Err(
                                    "ts_frontend: Math.pow takes exactly two arguments (M1)".into(),
                                );
                            }
                            let mut items = vec![Sym("expt")];
                            for a in &c.arguments {
                                let e2 = a
                                    .as_expression()
                                    .ok_or("ts_frontend: unsupported Math.pow argument (M1)")?;
                                items.push(lower_expr(e2)?);
                            }
                            return Ok(list(items));
                        }
                        ("Math", "ceil") => {
                            // Math.ceil(x) → (ceiling x) — the runtime builtin
                            // is spelled `ceiling`.
                            if c.arguments.is_empty() {
                                return Err(
                                    "ts_frontend: Math.ceil needs at least one argument (M1)"
                                        .into(),
                                );
                            }
                            let mut items = vec![Sym("ceiling")];
                            for a in &c.arguments {
                                let e2 = a
                                    .as_expression()
                                    .ok_or("ts_frontend: unsupported Math.ceil argument (M1)")?;
                                items.push(lower_expr(e2)?);
                            }
                            return Ok(list(items));
                        }
                        ("Math", other) => {
                            // Hard-error instead of minting Math/<name>
                            // symbols no backend knows (t4_pow@ts root
                            // cause — 0/38 on "unknown 'Math/pow'").
                            return Err(format!(
                                "ts_frontend: Math.{other} not supported \
                                 (M1: abs/max/min/pow/sqrt/floor/ceil/round)"
                            ));
                        }
                        ("JSON", "stringify") => {
                            if c.arguments.len() != 1 {
                                return Err(
                                    "ts_frontend: JSON.stringify takes exactly one value (M1)"
                                        .into(),
                                );
                            }
                            let e2 = c.arguments[0]
                                .as_expression()
                                .ok_or("ts_frontend: unsupported JSON.stringify argument (M1)")?;
                            return Ok(list(vec![Sym("json-quote"), lower_expr(e2)?]));
                        }
                        ("JSON", "stringifyArr") => {
                            if c.arguments.len() != 1 {
                                return Err(
                                    "ts_frontend: JSON.stringifyArr takes exactly one array (M1)"
                                        .into(),
                                );
                            }
                            let e2 = c.arguments[0].as_expression().ok_or(
                                "ts_frontend: unsupported JSON.stringifyArr argument (M1)",
                            )?;
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
                                "ts_frontend: JSON.parse not needed — tx args arrive parsed (use near.input() + property reads, or near.args<T>())"
                                    .into(),
                            );
                        }
                        // ── JSON API v3 (2026-09-15): the input handle ──
                        // near.input() is the NAME-level handle: `const o =
                        // near.input()` registers o (scan_input_handles) and
                        // property reads rewrite to the cached-input getters;
                        // the DECL path nil-binds the name (dead binding).
                        // Bare VALUE uses (`return near.input()`) lower to the
                        // real (near/input) op — the full args JSON as a
                        // tagged string, per the d.ts `input(): string`
                        // contract. (2026-09-17: this arm used to swallow ALL
                        // call sites to dead nil — `return near.input()`
                        // silently returned nil, breaking tour2_input.)
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
                        | "repeat"
                        | "padStart"
                        | "padEnd"
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
                        // repeat/pad (2026-09-13): expr_returns_str_method already
                        // listed them for + typing, but no lowering existed —
                        // "0".repeat(5) died in callee_name ("nested member
                        // chains not in M1"). str-repeat is a native wasm
                        // builtin; pads lower to repeat+slice in let-position.
                        "repeat" => Ok(list(vec![Sym("str-repeat"), recv, arg(0)?])),
                        "padStart" => {
                            let len = arg(0)?;
                            let pad = if argc >= 2 {
                                arg(1)?
                            } else {
                                Str(String::from(" "))
                            };
                            // (let ((need (- len (str-length s))))
                            //   (if (<= need 0) s (str-cat (str-slice (str-repeat pad len) 0 need) s)))
                            Ok(list(vec![
                                Sym("let"),
                                list(vec![list(vec![
                                    Sym("__pad_need"),
                                    list(vec![
                                        Sym("-"),
                                        len.clone(),
                                        list(vec![Sym("str-length"), recv.clone()]),
                                    ]),
                                ])]),
                                list(vec![
                                    Sym("if"),
                                    list(vec![Sym("<="), Sym("__pad_need"), Num(0)]),
                                    recv.clone(),
                                    list(vec![
                                        Sym("str-cat"),
                                        list(vec![
                                            Sym("str-slice"),
                                            list(vec![Sym("str-repeat"), pad, len]),
                                            Num(0),
                                            Sym("__pad_need"),
                                        ]),
                                        recv,
                                    ]),
                                ]),
                            ]))
                        }
                        "padEnd" => {
                            let len = arg(0)?;
                            let pad = if argc >= 2 {
                                arg(1)?
                            } else {
                                Str(String::from(" "))
                            };
                            Ok(list(vec![
                                Sym("let"),
                                list(vec![list(vec![
                                    Sym("__pad_need"),
                                    list(vec![
                                        Sym("-"),
                                        len.clone(),
                                        list(vec![Sym("str-length"), recv.clone()]),
                                    ]),
                                ])]),
                                list(vec![
                                    Sym("if"),
                                    list(vec![Sym("<="), Sym("__pad_need"), Num(0)]),
                                    recv.clone(),
                                    list(vec![
                                        Sym("str-cat"),
                                        recv,
                                        list(vec![
                                            Sym("str-slice"),
                                            list(vec![Sym("str-repeat"), pad, len]),
                                            Num(0),
                                            Sym("__pad_need"),
                                        ]),
                                    ]),
                                ]),
                            ]))
                        }
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
                            return Err(format!("ts_frontend: {} takes exactly one callback", op));
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
                                "ts_frontend: reduce takes a callback and an initial value".into(),
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
                                    list(vec![Sym("vec-push"), Sym(name), lower_expr(e2)?]),
                                ]));
                            }
                            return Err("ts_frontend: push target must be a plain variable".into());
                        }
                        _ => {} // push handled; pipeline members were matched above
                        _ => {} // fall through to the generic path
                    }
                }
            }
            // ── near.* special forms (2026-09-30) ─────────────────────────
            // depositGte(<bigint literal>) — compile-time u128 → (lo64, hi64)
            // split. Nobody should hand-split a u128 (crossport: the 0.012N
            // fee pair was machine-verified because hand-computing it is a
            // footgun). Two-number form stays as-is.
            if let Expression::StaticMemberExpression(sm) = &c.callee {
                if let (Expression::Identifier(o), "depositGte") =
                    (&sm.object, sm.property.name.as_str())
                {
                    if o.name == "near" && c.arguments.len() == 1 {
                        let a = c.arguments[0]
                            .as_expression()
                            .ok_or("ts_frontend: bad depositGte arg")?;
                        if let Expression::BigIntLiteral(b) = a {
                            let raw = b
                                .raw
                                .as_ref()
                                .map(|s| s.as_str().trim_end_matches('n'))
                                .unwrap_or_default();
                            let v = raw.parse::<u128>().map_err(|_| {
                                "ts_frontend: depositGte bigint literal out of u128 range"
                                    .to_string()
                            })?;
                            return Ok(list(vec![
                                Sym("near/deposit-gte"),
                                Num((v & u64::MAX as u128) as i64),
                                Num((v >> 64) as i64),
                            ]));
                        }
                        return Err("ts_frontend: depositGte takes (lo64, hi64) numbers or ONE \
                             bigint literal, e.g. depositGte(12000000000000000000000n)"
                            .into());
                    }
                }
            }
            // near.db.* (2026-10-07): MUST run before callee_name — the
            // callee `near.db.key` is a 2-level member chain, which
            // callee_name rejects ("nested member chains not in M1").
            // near.db.<method>(…) desugars onto the PROVEN storage family
            // — pure frontend aliasing, checker/emitter/runtime untouched:
            //   key(k) → (near/storage_get k)     opt str; `?? "d"` unwraps
            //   put(k,v) → (near/storage_set k v) string in, string at rest
            //   has(k) → (near/storage_has k)     bool
            //   del(k) → (near/storage_remove k)  nil
            //   keys(p) → the storage_cleaner drain loop
            //     (examples/storage_cleaner.lisp shape — nil-guard exits,
            //     NOT truthiness: numeric 0 is truthy in this lisp),
            //     compiled to a vec via (list) + vec-push. Rides the
            //     ENGINE-level storage-iter builtins (host fns 36/38,
            //     tests/test_storage_iter.rs) — the PROTOCOL's
            //     storage_iter_* host ABI is deprecated (near-vm-runner
            //     0.37.3 answers HostError::Deprecated) and is never
            //     emitted.
            if let Expression::StaticMemberExpression(sm2) = &c.callee {
                if let Expression::StaticMemberExpression(db2) = &sm2.object {
                    if let Expression::Identifier(ns_o) = &db2.object {
                    if ns_o.name == "near" && db2.property.name.as_str() == "db" {
                            let method = sm2.property.name.as_str();
                            let mut db_args = Vec::new();
                            for a in &c.arguments {
                                db_args.push(
                                    a.as_expression()
                                        .ok_or("ts_frontend: bad near.db argument (M1)")?,
                                );
                            }
                            return match (method, db_args.len()) {
                                ("key", 1) => Ok(list(vec![
                                    Sym("near/storage_get"),
                                    lower_expr(db_args[0])?,
                                ])),
                                ("put", 2) => Ok(list(vec![
                                    Sym("near/storage_set"),
                                    lower_expr(db_args[0])?,
                                    lower_expr(db_args[1])?,
                                ])),
                                ("has", 1) => Ok(list(vec![
                                    Sym("near/storage_has"),
                                    lower_expr(db_args[0])?,
                                ])),
                                ("del", 1) => Ok(list(vec![
                                    Sym("near/storage_remove"),
                                    lower_expr(db_args[0])?,
                                ])),
                                ("keys", 1) => {
                                    let kv = lower_expr(db_args[0])?;
                                    Ok(list(vec![
                                        Sym("let*"),
                                        list(vec![
                                            list(vec![
                                                Sym("__dbk_it"),
                                                list(vec![
                                                    Sym("storage-iter-prefix"),
                                                    kv,
                                                ]),
                                            ]),
                                            list(vec![
                                                Sym("__dbk_cur"),
                                                list(vec![
                                                    Sym("storage-iter-next"),
                                                    Sym("__dbk_it"),
                                                ]),
                                            ]),
                                            list(vec![
                                                Sym("__dbk_acc"),
                                                list(vec![Sym("list")]),
                                            ]),
                                        ]),
                                        list(vec![
                                            Sym("begin"),
                                            list(vec![
                                                Sym("while"),
                                                list(vec![
                                                    Sym("!="),
                                                    Sym("__dbk_cur"),
                                                    LispVal::Nil,
                                                ]),
                                                list(vec![
                                                    Sym("begin"),
                                                    list(vec![
                                                        Sym("set!"),
                                                        Sym("__dbk_acc"),
                                                        list(vec![
                                                            Sym("vec-push"),
                                                            Sym("__dbk_acc"),
                                                            Sym("__dbk_cur"),
                                                        ]),
                                                    ]),
                                                    list(vec![
                                                        Sym("set!"),
                                                        Sym("__dbk_cur"),
                                                        list(vec![
                                                            Sym("storage-iter-next"),
                                                            Sym("__dbk_it"),
                                                        ]),
                                                    ]),
                                                    Num(0),
                                                ]),
                                            ]),
                                            Sym("__dbk_acc"),
                                        ]),
                                    ]))
                                }
                                ("key", _) => {
                                    Err("ts_frontend: near.db.key takes (key)".into())
                                }
                                ("put", _) => {
                                    Err("ts_frontend: near.db.put takes (key, value)".into())
                                }
                                ("has", _) => {
                                    Err("ts_frontend: near.db.has takes (key)".into())
                                }
                                ("del", _) => {
                                    Err("ts_frontend: near.db.del takes (key)".into())
                                }
                                ("keys", _) => {
                                    Err("ts_frontend: near.db.keys takes (prefix)".into())
                                }
                                _ => Err(format!(
                                    "ts_frontend: unknown near.db method `{method}` — key/put/has/del/keys"
                                )),
                            };
                        }
                    }
                }
            }
            let head = callee_name(&c.callee)?;
            // blockTimestampNum() → RAW numeric ns (alias for the host op
            // — near/block_timestamp_num doesn't exist)
            // ── ergonomics-v2 builtins (2026-10-07) ─────────────────────
            // assert(cond, msg) → (if cond 0 (near/panic msg)). Special
            // form (not a function): near/panic : str → any makes ANY
            // typed branch unify. Statement position discards the 0.
            if head == "assert" {
                if c.arguments.len() != 2 {
                    return Err("ts_frontend: assert takes exactly (cond, msg)".into());
                }
                let a0 = c.arguments[0]
                    .as_expression()
                    .ok_or("ts_frontend: bad assert cond")?;
                let a1 = c.arguments[1]
                    .as_expression()
                    .ok_or("ts_frontend: bad assert msg")?;
                return Ok(list(vec![
                    Sym("if"),
                    lower_expr(a0)?,
                    Num(0),
                    list(vec![Sym("near/panic"), lower_expr(a1)?]),
                ]));
            }
            // near.event(name, {…}) → (near/log (json-quote …)). Host op
            // exists (near/log, fn 28); object literal already encodes to
            // JSON text via lower_object_literal. Standard event JSON:
            // {"event":name,"data":{…}}.
            if head == "near/event" {
                if c.arguments.len() != 2 {
                    return Err(
                        "ts_frontend: near.event takes exactly (name, objectLiteral)".into(),
                    );
                }
                let a1 = c.arguments[1]
                    .as_expression()
                    .ok_or("ts_frontend: bad near.event payload")?;
                let payload = match a1 {
                    Expression::ObjectExpression(_) => lower_expr(a1)?,
                    _ => {
                        return Err(
                            "ts_frontend: near.event payload must be an object literal".into()
                        )
                    }
                };
                let name_e = c.arguments[0]
                    .as_expression()
                    .ok_or("ts_frontend: bad near.event name")?;
                // payload is ALREADY encoded JSON text (lower_object_literal
                // → json-set chain) — json-set embeds it raw as the "data"
                // value. json-quote wraps the assembled object text.
                let name_v = lower_expr(name_e)?;
                return Ok(list(vec![
                    Sym("near/log"),
                    list(vec![
                        Sym("json-quote"),
                        list(vec![
                            Sym("json-set"),
                            list(vec![
                                Sym("json-set"),
                                Str("{}".to_string()),
                                Str("event".to_string()),
                                list(vec![Sym("json-quote"), name_v]),
                            ]),
                            Str("data".to_string()),
                            payload,
                        ]),
                    ]),
                ]));
            }
            let head = if head == "near/block_timestamp_num" {
                "near/block_timestamp".to_string()
            } else {
                head
            };
            let head = map_builtin_call(&head);
            // strict surface: a BARE-IDENTIFIER callee whose RAW name
            // survives every mapping unchanged (raw == final head) and is
            // neither a user-defined function nor RLM runtime API is a
            // lisp passthrough attempt — reject (2026-10-05). Raw-name
            // comparison matters: callee_name already applies
            // map_global_fn (strToNum → str->num), so comparing
            // post-map heads false-positived known builtins. Member-call
            // results (near.storageSet → near/storage_set) are Identifier-
            // free and exempt: near.<member> lowering is its own
            // (deliberately generic) path.
            if let Expression::Identifier(id) = &c.callee {
                let raw = id.name.as_str().to_string();
                if raw == head && !user_fn_or_runtime(&raw) {
                    return Err(format!(
                        "ts_frontend: unknown function `{}` is not a TS-surface builtin (see cheatsheet) and not defined in this program; do NOT write lisp names",
                        raw
                    ));
                }
            }
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
            // blockTimestamp(): STRING (ns ~1.8e18 exceed JS safe ints —
            // arithmetic on a `number` silently loses precision). Raw numeric
            // variant stays available as near.blockTimestampNum().
            if head == "near/block_timestamp" && c.arguments.is_empty() {
                return Ok(list(vec![Sym("to-string"), list(items)]));
            }
            // nil-trap shield: haystack ops fed a bare miss-able getter
            // (nil-on-miss) get a (default x "") wrap — strSlice(nil,0,10)
            // and strIndexOf(nil,…) silently misbehave otherwise. Users who
            // `??` explicitly are unaffected (the wrap sees a non-bare arg).
            if matches!(head.as_str(), "str-slice" | "str-index-of" | "str-split")
                && items.len() >= 2
            {
                if let Some(a) = c.arguments[0].as_expression() {
                    if expr_is_missable_getter(a) {
                        let shielded =
                            list(vec![Sym("default"), items[1].clone(), Str(String::new())]);
                        items[1] = shielded;
                    }
                }
            }
            if head == "json-get" {
                // TS surface keeps its documented jsonGet(key, json) order
                // (ts/lisp-rlm.d.ts); the lisp op is (json-get <json> "key")
                // since the 2026-10-07 unification — swap the 2-arg form at
                // lowering. 1-arg jsonGet(key) passes through untouched.
                if items.len() == 3 {
                    items.swap(1, 2);
                }
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
                let raw = c
                    .arguments
                    .get(2)
                    .and_then(|a| a.as_expression())
                    .ok_or("ts_frontend: bad jsonSet 3rd arg")?;
                // jsonQuote(x) / {object-literal} / jsonSet(...) already
                // produce encoded text — splice raw.
                let already = match raw {
                    Expression::CallExpression(ic) => match &ic.callee {
                        Expression::Identifier(id) => {
                            matches!(id.name.as_str(), "jsonQuote" | "jsonSet")
                        }
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
            Ok(list(vec![Sym("lambda"), list(params), body_val]))
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
                                "u128/gt"
                                    | "u128/lt"
                                    | "u128/gte"
                                    | "u128/lte"
                                    | "u128/eq"
                                    | "u128/is-zero"
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
                            "u128/gt"
                                | "u128/lt"
                                | "u128/gte"
                                | "u128/lte"
                                | "u128/eq"
                                | "u128/is-zero"
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
    // `while (true)` / `if (x === true)` — literal booleans are bool (the
    // exit-path cond wrapper picks its false_e by this predicate; a missed
    // literal made bool≠int branches and the checker rejected while(true)
    // with break/continue — test_continue_keyword regression, fixed again
    // 2026-10-05).
    let bool_lit = matches!(e, Expression::BooleanLiteral(_));
    matches!(e, Expression::LogicalExpression(_))
        || is_not
        || bool_lit
        || bool_call
        || matches!(e, Expression::BooleanLiteral(_))
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
        // Pass the value RAW to the (if …) — the lisp `if`/`while` emitters
        // use tag-aware truthiness (emit_cond_branch: falsy = {Bool false,
        // Nil, Num 0}), so any tagged value branches correctly.
        //
        // The old `(!= x 0)` wrapper did a NUMERIC compare: an identifier
        // holding a BOOL was compared as bool≠num → always true, so
        // `const take = r < 2; if (take)` in a loop took the then-arm every
        // iteration (probeB: 60 instead of 24 — Poseidon's partial rounds
        // hashed wrong from exactly this, 2026-09-11).
        lower_expr(e)
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
            let mapped = map_member_fn(&obj, s.property.name.as_str());
            // Typo hole: near.<member> lowers generically to near/<snake>,
            // so an unknown member compiles fine and traps at RUNTIME
            // (or worse, silently no-ops). Warn at compile time instead
            // (2026-09-30). Generic lowering stays — the d.ts surface is a
            // convention, not a whitelist (near.returnStr works pre-d.ts).
            if obj == "near" && s.property.name.as_str() == "event" {
                // documented surface (d.ts + KNOWN_NEAR_MEMBERS); lowering
                // is the near/event special form in the call path below
            } else if obj == "near" && !known_near_member(s.property.name.as_str()) {
                eprintln!(
                    "[warn] ts_frontend: near.{} is not in the known host surface — \
                     lowering to `{}`. If that op doesn't exist, this traps at \
                     runtime. (typo? check ts/lisp-rlm.d.ts)",
                    s.property.name.as_str(),
                    mapped
                );
            }
            Ok(mapped)
        }
        _ => Err("ts_frontend: callee must be an identifier or member (M1)".into()),
    }
}

/// The known near.* member surface — DATA, not a hand-rolled matches!().
/// Enforced byte-for-byte against ts/lisp-rlm.d.ts's `declare const near`
/// block by tests/ts_surface_dts_parity.rs (both directions). Adding a
/// builtin means: d.ts entry + this table in the same commit, or the
/// parity test fails. Keep order alphabetical within each group.
pub const KNOWN_NEAR_MEMBERS: &[&str] = &[
    // namespaces (near.db.* — methods gated by KNOWN_DB_MEMBERS below)
    "db",
    // events
    "event",
    // input/args
    "jsonGetStr",
    "jsonGetInt",
    "jsonGetArr",
    "jsonArr",
    "jsonGet",
    "jsonSet",
    "jsonQuote",
    "jsonExtract",
    "jsonReturnStr",
    "jsonReturnInt",
    // identity / env
    "predecessorAccountId",
    "signerAccountId",
    "currentAccountId",
    "signerAccountPk",
    "blockIndex",
    "blockHeight",
    "blockTimestamp",
    "blockTimestampNum",
    "prepaidGas",
    "usedGas",
    "storageUsage",
    "accountBalance",
    "attachedDeposit",
    "attachedDepositLow",
    "attachedDepositHigh",
    "attachedDepositU128",
    "depositGte",
    "input",
    // storage
    "storageGet",
    // numeric storage (tagged-i64 keys — Q32 CLMM math; 2026-10-09)
    "storeNum",
    "loadNum",
    "storageSet",
    "storageRemove",
    "storageHas",
    "storageHasKey",
    "iterPrefix",
    "iterNext",
    // returns / control
    "returnStr",
    "log",
    "logNum",
    "panic",
    "abort",
    // money
    "transfer",
    "transferU128",
    "storeU128",
    "readU128",
    "loadU128",
    // hashes / crypto (precompiles)
    "sha256",
    "keccak256",
    "keccak512",
    "ripemd160",
    "randomSeed",
    "hexDecode",
    "hexEncode",
    "sha256Hash",
    "keccak256Hash",
    "ed25519Verify",
    "ecrecover",
    "vrfGenerate",
    "schnorrVerify",
    "schnorrSign",
    "schnorrSignPk",
    "schnorrPubkey",
    "schnorrPubkey33",
    "p256Verify",
    "altBn128PairingCheck",
    "altBn128G1Sum",
    "altBn128G1Multiexp",
    "altBn128G2Sum",
    "bls12381PairingCheck",
    "bls12381P1Sum",
    "bls12381P2Sum",
    "bls12381G1Multiexp",
    "bls12381G2Multiexp",
    "bls12381MapFpToG1",
    "bls12381MapFp2ToG2",
    "bls12381P1Decompress",
    "bls12381P2Decompress",
    "blsG1Sum",
    "blsG2Sum",
    // promises
    "all",
    "promiseCreate",
    "promiseThen",
    "promiseAnd",
    "promiseReturn",
    "promiseResult",
    "promiseSucceeded",
    "promiseResultsCount",
    "promiseBatchCreate",
    "promiseBatchThen",
    "promiseBatchActionTransfer",
    "promiseBatchActionFunctionCall",
    "promiseBatchActionFunctionCallWeight",
    "promiseBatchActionCreateAccount",
    "promiseBatchActionDeployGlobalContract",
    "promiseBatchActionDeployGlobalContractByAccountId",
    "promiseBatchActionUseGlobalContract",
    "promiseBatchActionUseGlobalContractByAccountId",
    "promiseBatchActionAddKeyWithFullAccess",
    "promiseBatchActionAddGasKeyWithFullAccess",
    "promiseBatchActionAddKeyWithFunctionCall",
    "promiseBatchActionAddGasKeyWithFunctionCall",
    "promiseBatchActionTransferToGasKey",
    "promiseBatchActionDeleteKey",
    "promiseBatchActionStake",
    "promiseBatchActionDeleteAccount",
    "promiseYieldCreate",
    "promiseYieldResume",
    "yieldCreate",
    "yieldResume",
    "callAwait",
    "call",
];

/// The known near.* member surface (keep in sync with ts/lisp-rlm.d.ts —
/// parity is ENFORCED by tests/ts_surface_dts_parity.rs; the warning below
/// only exists to catch typos, not to close the door on undocumented host
/// ops).
fn known_near_member(p: &str) -> bool {
    KNOWN_NEAR_MEMBERS.contains(&p)
}

/// Known near.db.* method surface — parity-enforced against the d.ts
/// `db: { … }` block by tests/ts_surface_dts_parity.rs (both directions),
/// same rule as KNOWN_NEAR_MEMBERS.
pub const KNOWN_DB_MEMBERS: &[&str] = &["del", "has", "key", "keys", "put"];

fn known_db_member(p: &str) -> bool {
    KNOWN_DB_MEMBERS.contains(&p)
}

/// Bare getter calls that yield NIL on a miss (jsonGetStr / near.jsonGetStr /
/// near.storageGet) — first-arg shield candidates for the haystack ops.
fn expr_is_missable_getter(e: &Expression<'_>) -> bool {
    match e {
        Expression::CallExpression(cc) => match &cc.callee {
            Expression::StaticMemberExpression(s) => {
                matches!(
                    (&s.object, s.property.name.as_str()),
                    (Expression::Identifier(o), "jsonGetStr" | "storageGet") if o.name == "near"
                )
            }
            Expression::Identifier(id) => id.name.as_str() == "jsonGetStr",
            _ => false,
        },
        _ => false,
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
    // outlayer.* members → kebab-case lisp ops (outlayer/storage-get etc).
    // The near.* family uses snake_case host names, but the outlayer ops
    // are kebab — snake() here produced outlayer/storage_get (nonexistent).
    if obj == "outlayer" {
        // storageGet -> storage-get (kebab, matching the lisp op names).
        // The near.* family uses snake_case host names, but the outlayer
        // ops are kebab — snake() here produced outlayer/storage_get
        // (nonexistent op).
        let mut out = String::new();
        for ch in prop.chars() {
            if ch.is_uppercase() {
                out.push('-');
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        return format!("outlayer/{}", out);
    }
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
            "jsonExtract" => Some("json-extract-input"),
            "sha256Hash" => Some("sha256-hash"),
            "hexDecode" => Some("hex-decode"),
            "hexEncode" => Some("hex-encode"),
            "schnorrVerify" => Some("schnorr-verify"),
            "schnorrSign" => Some("schnorr-sign"),
            "schnorrSignPk" => Some("schnorr-sign-pk"),
            "schnorrPubkey" => Some("schnorr-pubkey"),
            "schnorrPubkey33" => Some("schnorr-pubkey33"),
            "vrfGenerate" => Some("vrf-generate"),
            "ed25519Verify" => Some("ed25519-verify"),
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
        "strToNum" | "Number" | "parseInt" | "parseFloat" => "str->num",
        "toStr" | "toString" => "to-string",
        "jsonGet" => "json-get",
        "jsonGetStr" => "json-get-str",
        "jsonExtract" => "json-extract-input",
        "strSplit" => "str-split",
        "strJoin" => "str-join",
        // near_* free functions (d.ts-declared since 2026-08-30, unmapped
        // until the dts-parity test caught them 2026-10-05)
        "near_storage_get" => "near/storage_get",
        "near_storage_set" => "near/storage_set",
        "near_predecessor_account_id" => "near/predecessor_account_id",
        "hexDecode" => "hex-decode",
        "hexEncode" => "hex-encode",
        "sha256Hash" => "sha256-hash",
        "schnorrVerify" => "schnorr-verify",
        "schnorrSign" => "schnorr-sign",
        "schnorrSignPk" => "schnorr-sign-pk",
        "schnorrPubkey" => "schnorr-pubkey",
        "schnorrPubkey33" => "schnorr-pubkey33",
        "vrfGenerate" => "vrf-generate",
        "ed25519Verify" => "ed25519-verify",
        "httpPost" => "http-post",
        "httpGet" => "http-get",
        "jsonSet" => "json-set",
        "jsonQuote" => "json-quote",
        // u128-precision arithmetic (decimal-string ABI, both runtimes)
        "u128Add" => "u128/add",
        "u128Sub" => "u128/sub",
        "u128Mul" => "u128/mul",
        "u128MulDiv" => "u128/muldiv",
        "u128Div" => "u128/div",
        "u128Mod" => "u128/mod",
        "u128Lt" => "u128/lt",
        "u128Gt" => "u128/gt",
        "u128Eq" => "u128/eq",
        "u128IsZero" => "u128/is-zero",
        "u128FromNum" => "u128/from-i64",
        "u128ToNum" => "u128/to-i64",
        // numeric intrinsics (Q32 CLMM surface, 2026-10-09): camelCase TS
        // spellings mapping to the lisp builtins — raw==head would trip
        // the lisp-passthrough gate, and that's the point.
        "mulDiv" => "muldiv",
        "intSqrt" => "isqrt",
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
                    oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str().to_string(),
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
                oxc_ast::ast::TSTypeName::IdentifierReference(id) => id.name.as_str().to_string(),
                _ => return None, // qualified names — not a local alias
            };
            TYPE_ALIASES.with(|m| {
                m.borrow()
                    .iter()
                    .find(|(k, _)| *k == name)
                    .and_then(|(_, props)| {
                        if props.first().map(|(k, _)| k == SCALAR_ALIAS_MARK).unwrap_or(false) {
                            // scalar alias (`type Money = string`) — a plain
                            // param, not an object shape
                            None
                        } else {
                            Some(props.clone())
                        }
                    })
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
                oxc_ast::ast::TSTypeName::IdentifierReference(id) => Some(id.name.as_str()),
                _ => None,
            };
            let known = name.map(|n| TYPE_ALIASES.with(|m| m.borrow().iter().any(|(k, _)| k == n)));
            return !known.unwrap_or(false);
        }
    }
    false
}

/// Marker first-element for SCALAR type aliases (`type Money = string`):
/// param_object_props sees this and resolves the alias to None — the param
/// is a plain str/int, NOT a kind-3 object param (an empty-props kind-3
/// used to register a dead object binding and corrupt the entry ABI).
const SCALAR_ALIAS_MARK: &str = "\u{0}scalar";

/// Shape of a `type X = { prop: string; num: number; ... }` alias.
fn alias_props(a: &oxc_ast::ast::TSTypeAliasDeclaration<'_>) -> Vec<(String, bool)> {
    match &a.type_annotation {
        // `type Money = string` (u128-decimal ABI hover docs, 2026-10-07):
        // SCALAR aliases record as marker ([] + is_scalar) so
        // param_object_props resolves the ref to None (plain str param)
        // instead of kind-3-empty (which used to register a dead
        // object-param binding and corrupt the entry ABI).
        oxc_ast::ast::TSType::TSStringKeyword(_) => vec![(SCALAR_ALIAS_MARK.to_string(), false)],
        oxc_ast::ast::TSType::TSNumberKeyword(_) => vec![(SCALAR_ALIAS_MARK.to_string(), false)],
        oxc_ast::ast::TSType::TSBigIntKeyword(_) => vec![(SCALAR_ALIAS_MARK.to_string(), false)],
        oxc_ast::ast::TSType::TSTypeLiteral(lit) => {
            let mut props = Vec::new();
            for m in &lit.members {
                if let oxc_ast::ast::TSSignature::TSPropertySignature(sig) = m {
                    if let oxc_ast::ast::PropertyKey::StaticIdentifier(id) = &sig.key {
                        let is_num = matches!(
                            sig.type_annotation.as_ref().map(|t| &t.type_annotation),
                            Some(oxc_ast::ast::TSType::TSNumberKeyword(_))
                        );
                        props.push((id.name.as_str().to_string(), is_num));
                    }
                }
            }
            props
        }
        _ => Vec::new(),
    }
}

/// Register an object param's shape for read-time numeric decoding and
/// encode-time raw embedding (side-channel, cleared with NUM_PARAM_NAMES
/// after body lowering).
fn register_obj_param(name: &str, props: Vec<(String, bool)>) {
    OBJECT_PARAMS.with(|s| s.borrow_mut().push(name.to_string()));
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
        // TS booleans are first-class Bool values (2026-09-30): literals
        // lower as Bool, comparisons/str-contains already type Bool, and
        // the checker's `=`/`!=` are total on mixed pairs — so
        // `ch ? x.includes(y) : false` unifies instead of dying with
        // "branch types disagree: bool ≠ int". The old Num(1|0) decree
        // made every ternary mixing a bool expression with a literal
        // fail, and forced `: boolean` annotations to lie (:: int).
        TSType::TSBooleanKeyword(_) => Some("bool"),
        _ => None,
    }
}

fn binding_name(p: &oxc_ast::ast::BindingPattern<'_>) -> Result<String, String> {
    use oxc_ast::ast::BindingPattern::*;
    match p {
        BindingIdentifier(b) => Ok(b.name.as_str().to_string()),
        ObjectPattern(_) => {
            // JSON API v3: the ONLY destructuring form is
            // `const {a, b} = near.args<{...}>()` — handled by
            // lower_args_destructuring before binding_name is reached.
            Err(
                "ts_frontend: destructuring only supported for `const {..} = near.args<{..}>()`"
                    .into(),
            )
        }
        _ => Err("ts_frontend: destructuring patterns not in M1".into()),
    }
}

/// JSON API v3 (2026-09-15): `const {a, b, n} = near.args<{a: string, b:
/// string, n: number}>()` — single-pass typed arg binding. One
/// json-extract-input scan for ALL keys; number-typed fields wrap
/// str->num (extract yields raw span strings). Returns the lisp bindings,
/// or None when d is not an args-destructuring declaration.
fn lower_args_destructuring(
    v: &oxc_ast::ast::VariableDeclaration<'_>,
) -> Option<Result<Vec<LispVal>, String>> {
    let d = v.declarations.first()?;
    let init = d.init.as_ref()?;
    let Expression::CallExpression(c) = init else {
        return None;
    };
    let Expression::StaticMemberExpression(sm) = &c.callee else {
        return None;
    };
    let Expression::Identifier(oid) = &sm.object else {
        return None;
    };
    if !(oid.name == "near" && sm.property.name == "args") {
        return None;
    }
    if !c.arguments.is_empty() {
        return Some(Err(
            "ts_frontend: near.args takes its shape from the type parameter only".into(),
        ));
    }
    // ObjectPattern with the field names; types from the type argument
    let oxc_ast::ast::BindingPattern::ObjectPattern(op) = &d.id else {
        return Some(Err("ts_frontend: near.args<T>() binds with an object pattern: `const {a, b} = near.args<{a: string, b: number}>()`".into()));
    };
    let mut fields: Vec<String> = Vec::new();
    for p in &op.properties {
        // BindingProperty is a plain struct in oxc 0.147
        let Ok(n) = binding_name(&p.value) else {
            return Some(Err(
                "ts_frontend: args pattern must be plain identifiers".into()
            ));
        };
        let key = match &p.key {
            oxc_ast::ast::PropertyKey::StaticIdentifier(k) => k.name.as_str().to_string(),
            _ => return Some(Err("ts_frontend: args keys must be static".into())),
        };
        let _ = n;
        fields.push(key);
    }
    // types from the type argument (TSTypeLiteral)
    let Some(targs) = c.type_arguments.as_ref() else {
        return Some(Err(
            "ts_frontend: near.args needs a type parameter: near.args<{a: string, n: number}>()"
                .into(),
        ));
    };
    let Some(targ) = targs.params.first() else {
        return Some(Err("ts_frontend: near.args needs a type parameter".into()));
    };
    let oxc_ast::ast::TSType::TSTypeLiteral(tl) = &targ else {
        return Some(Err(
            "ts_frontend: near.args type parameter must be an inline object literal type".into(),
        ));
    };
    let mut num_fields = Vec::new();
    for m in &tl.members {
        let oxc_ast::ast::TSSignature::TSPropertySignature(ps) = m else {
            return Some(Err("ts_frontend: args type must be plain properties".into()));
        };
        let key = match &ps.key {
            oxc_ast::ast::PropertyKey::StaticIdentifier(k) => k.name.as_str().to_string(),
            _ => return Some(Err("ts_frontend: args type keys must be static".into())),
        };
        let is_num = matches!(
            ps.type_annotation.as_ref().map(|a| &a.type_annotation),
            Some(oxc_ast::ast::TSType::TSNumberKeyword(_))
        );
        if is_num {
            num_fields.push(key);
        }
    }
    // build the bindings: one extract + per-field vec-nth (+ str->num for numbers)
    let keys: Vec<String> = fields.clone();
    if keys.is_empty() {
        return Some(Err("ts_frontend: near.args needs at least one field".into()));
    }
    if keys.len() > 8 {
        return Some(Err(
            "ts_frontend: near.args supports at most 8 fields (jsonExtract cap)".into(),
        ));
    }
    let mut extract_items = vec![Sym("json-extract-input")];
    for k in &keys {
        extract_items.push(Str(k.clone()));
    }
    let tmp = "__args_v3".to_string();
    let mut bindings = vec![list(vec![Sym(tmp.clone()), list(extract_items)])];
    for (i, k) in fields.iter().enumerate() {
        let nth = list(vec![Sym("vec-nth"), Sym(tmp.clone()), Num(i as i64)]);
        let val = if num_fields.contains(k) {
            list(vec![Sym("str->num"), nth])
        } else {
            nth
        };
        bindings.push(list(vec![Sym(k.clone()), val]));
    }
    // NOTE: callers MUST bind with let* — field inits reference __args_v3
    // bound in the same clause group (plain let evaluates inits in the
    // outer scope — the "undefined variable __args_v3" trap)
    Some(Ok(bindings))
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
        // 2026-09-30: print real Bool atoms. The lisp parser reads
        // true/false back as Bool (parser.rs) — the old "1"/"0" print
        // silently demoted every TS boolean literal to Num across the
        // source-text boundary, so `: boolean` and ternary bool branches
        // could never unify ("branch types disagree: bool ≠ int").
        LispVal::Bool(b) => if *b { "true" } else { "false" }.into(),
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
        // strict surface (2026-10-05): unknown bare calls are rejected, so
        // the helper is DEFINED here — its call-site still records an
        // ident offset for the line-hint machinery.
        let src = "function undefined_helper(x: number): number { return x; }\nexport function new_() {\n  let x = 1\n  let y = undefined_helper(x)\n  return y\n}\n";
        let r = super::parse_ts(src).expect("parses");
        assert!(!r.is_empty());
        let map = super::take_ident_offsets();
        eprintln!("MAP: {:?}", map);
        assert!(
            map.iter().any(|(n, _)| n == "undefined_helper"),
            "undefined_helper should be in the ident map"
        );
        let line = super::ts_line_hint(&map, src, "undefined_helper").expect("hint");
        assert_eq!(line, "4");
    }
}
