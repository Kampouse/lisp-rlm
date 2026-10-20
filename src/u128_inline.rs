//! Trivial u128 wrapper inlining (TASK-u128-native-chain, Phase 1 — layer 2).
//!
//! A `(define (f p1..pn) (u128/op p.. "lit"..))` wrapper — params each used
//! exactly once, in declaration order, extras are literals (shape shared
//! with the checker via `trivial_u128_wrapper_shape`) — is, semantically,
//! exactly the bare u128 op. But as a CALL it is a tagged-convention
//! boundary: every activation re-parses its decimal-string args and
//! re-renders its result, even when caller and callee both run u128 math
//! (pool-a's step spends ~24 parses/step this way; see limb_locals.rs).
//!
//! This pass rewrites matching CALL SITES `(f e1..en)` (arity must equal
//! the wrapper's param count) to the direct op form BEFORE type check and
//! emission, so:
//!   * the limb-eligibility pre-scan sees op forms — wrapper results become
//!     limb-pair locals (native between ops, zero parse/render);
//!   * operands share the caller's per-activation parse memo;
//!   * the wrapper function itself stays defined (value uses, non-matching
//!     arities, and reflection keep the old path) — tree shaking removes it
//!     when nothing calls it anymore.
//!
//! Semantics preserved EXACTLY (the contract for phase 1):
//!   * argument evaluation order is unchanged — Syms appear in param order
//!     in the op form, so arg exprs evaluate in the same left-to-right
//!     order as the call; each arg expr occurs exactly once (the shape
//!     forbids duplicate params);
//!   * TRAP OBSERVABILITY: inside `(try ...)` / `assert-raises` subtrees a
//!     wrapper call's internal traps are NOT catchable today (callee bodies
//!     compile with an empty try stack, so their ops use the hard-trap
//!     variant). Inlining would silently make them catchable — so those
//!     subtrees are left untouched.
//!   * wrappers whose name is EVER used as a value (bare Sym), shadowed
//!     (let/lambda/letrec param, set! target), or defined more than once
//!     are never rewritten.
//!
//! The pass never fails: unknown shapes are simply left alone.

use crate::types::LispVal;
use crate::typing::checker::trivial_u128_wrapper_shape;
use std::collections::{HashMap, HashSet};

/// Kill/rewrite plan derived from the top-level defines.
struct WrapperSet {
    /// name → (params, body) for fns with the trivial wrapper shape
    fns: HashMap<String, (Vec<String>, LispVal)>,
}

impl WrapperSet {
    fn collect(exprs: &[LispVal]) -> Self {
        let mut last: HashMap<String, (Vec<String>, LispVal)> = HashMap::new();
        let mut dup: HashSet<String> = HashSet::new();
        for e in exprs {
            let LispVal::List(items) = e else { continue };
            if !matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                continue;
            }
            let Some(LispVal::List(header)) = items.get(1) else {
                continue;
            };
            let Some(LispVal::Sym(name)) = header.first() else {
                continue;
            };
            let params: Vec<String> = header[1..]
                .iter()
                .filter_map(|p| match p {
                    LispVal::Sym(n) => Some(n.clone()),
                    _ => None,
                })
                .collect();
            let Some(body) = items.get(2) else { continue };
            if last.insert(name.clone(), (params, body.clone())).is_some() {
                dup.insert(name.clone());
            }
        }
        let mut fns = HashMap::new();
        for (name, (params, body)) in last {
            if dup.contains(&name) {
                continue; // redefined — last-wins semantics unclear, stand down
            }
            if trivial_u128_wrapper_shape(&params, &body).is_some() {
                fns.insert(name, (params, body));
            }
        }
        WrapperSet { fns }
    }
}

/// Does `name` occur as a bare Sym in a NON-head position (a value use)?
fn any_value_use(e: &LispVal, name: &str, shadowed: &HashSet<String>) -> bool {
    match e {
        LispVal::Sym(n) => n == name && !shadowed.contains(name),
        LispVal::List(items) if !items.is_empty() => {
            // quote is data, not a use
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "quote") {
                return false;
            }
            // binding forms extend the shadow set
            let mut inner = shadowed.clone();
            if let Some(LispVal::Sym(h)) = items.first() {
                match h.as_str() {
                    "let" | "let*" | "letrec" => {
                        if let Some(LispVal::List(bs)) = items.get(1) {
                            for b in bs {
                                if let LispVal::List(p) = b {
                                    if let Some(LispVal::Sym(n)) = p.first() {
                                        if n == name {
                                            // shadowed binding: body uses are
                                            // not uses of the wrapper — but the
                                            // INIT still is (outer scope)
                                        }
                                        inner.insert(n.clone());
                                    }
                                }
                            }
                        }
                    }
                    "lambda" | "fn" => {
                        if let Some(LispVal::List(ps)) = items.get(1) {
                            for p in ps {
                                if let LispVal::Sym(n) = p {
                                    inner.insert(n.clone());
                                }
                            }
                        }
                    }
                    "define" => {
                        if let Some(LispVal::List(header)) = items.get(1) {
                            for p in &header[1..] {
                                if let LispVal::Sym(n) = p {
                                    inner.insert(n.clone());
                                }
                            }
                        }
                    }
                    "set!" | "=" => {
                        // a store TO the wrapper name is a value use of the
                        // name (rebinding) — treat conservatively
                        if let Some(LispVal::Sym(n)) = items.get(1) {
                            if n == name && !shadowed.contains(name) {
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            // head position of a call is not a value use
            let start = if matches!(items.first(), Some(LispVal::Sym(_))) {
                1
            } else {
                0
            };
            items[start..]
                .iter()
                .any(|x| any_value_use(x, name, &inner))
                // let inits are outer-scope uses
                || (matches!(items.first(), Some(LispVal::Sym(h)) if h == "let" || h == "let*" || h == "letrec")
                    && items
                        .get(1)
                        .and_then(|b| match b {
                            LispVal::List(bs) => Some(
                                bs.iter()
                                    .filter_map(|p| match p {
                                        LispVal::List(pp) if pp.len() == 2 => Some(&pp[1]),
                                        _ => None,
                                    })
                                    .any(|x| any_value_use(x, name, shadowed)),
                            ),
                            _ => None,
                        })
                        .unwrap_or(false))
        }
        _ => false,
    }
}

/// Depth-first rewrite. `shadowed` = names bound in enclosing scopes;
/// `in_try` = inside a try/assert-raises subtree (no rewrites: trap
/// observability through the call boundary must stay exactly as-is).
fn rewrite(
    e: LispVal,
    ws: &WrapperSet,
    shadowed: &HashSet<String>,
    in_try: bool,
    n: &mut usize,
) -> LispVal {
    let LispVal::List(items) = e else { return e };
    if items.is_empty() {
        return LispVal::List(items);
    }
    let head_sym = match items.first() {
        Some(LispVal::Sym(s)) => s.clone(),
        _ => {
            // not headed by a symbol — rewrite children (e.g. ((lambda ...) args))
            return LispVal::List(
                items
                    .into_iter()
                    .map(|x| rewrite(x, ws, shadowed, in_try, n))
                    .collect(),
            );
        }
    };
    match head_sym.as_str() {
        // data — never touched
        "quote" => LispVal::List(items),
        // trap observability zones — leave the whole subtree untouched
        "try" | "assert-raises" => LispVal::List(items),
        "let" | "let*" | "letrec" => {
            let mut out = vec![items[0].clone()];
            let mut inner = shadowed.clone();
            if let Some(LispVal::List(bs)) = items.get(1) {
                let mut nbs = Vec::new();
                for b in bs {
                    match b {
                        LispVal::List(p) if p.len() == 2 => {
                            // init evaluates in the OUTER scope
                            let init = rewrite(p[1].clone(), ws, shadowed, in_try, n);
                            if let LispVal::Sym(bn) = &p[0] {
                                inner.insert(bn.clone());
                            }
                            nbs.push(LispVal::List(vec![p[0].clone(), init]));
                        }
                        other => nbs.push(other.clone()),
                    }
                }
                out.push(LispVal::List(nbs));
            } else {
                out.push(items[1].clone());
            }
            for x in &items[2..] {
                out.push(rewrite(x.clone(), ws, &inner, in_try, n));
            }
            LispVal::List(out)
        }
        "lambda" | "fn" => {
            let mut inner = shadowed.clone();
            if let Some(LispVal::List(ps)) = items.get(1) {
                for p in ps {
                    if let LispVal::Sym(pn) = p {
                        inner.insert(pn.clone());
                    }
                }
            }
            let mut out = items.clone();
            for x in &mut out[2..] {
                *x = rewrite(x.clone(), ws, &inner, in_try, n);
            }
            LispVal::List(out)
        }
        "define" => {
            let mut inner = shadowed.clone();
            if let Some(LispVal::List(header)) = items.get(1) {
                for p in &header[1..] {
                    if let LispVal::Sym(pn) = p {
                        inner.insert(pn.clone());
                    }
                }
            }
            let mut out = items.clone();
            if out.len() >= 3 {
                out[2] = rewrite(out[2].clone(), ws, &inner, in_try, n);
            }
            LispVal::List(out)
        }
        "set!" | "=" => {
            // rewrite the VALUE only (target stays a symbol)
            let mut out = items.clone();
            if out.len() >= 3 {
                out[2] = rewrite(out[2].clone(), ws, shadowed, in_try, n);
            }
            LispVal::List(out)
        }
        _ => {
            // candidate wrapper call?
            if !in_try && !shadowed.contains(&head_sym) {
                if let Some((params, body)) = ws.fns.get(&head_sym) {
                    if items.len() == params.len() + 1 {
                        // rewrite args first (inner wrapper calls)
                        let args: Vec<LispVal> = items[1..]
                            .iter()
                            .map(|x| rewrite(x.clone(), ws, shadowed, in_try, n))
                            .collect();
                        let LispVal::List(bitems) = body else {
                            unreachable!("wrapper body shape checked at collect")
                        };
                        let new_items: Vec<LispVal> = bitems[1..]
                            .iter()
                            .map(|a| match a {
                                LispVal::Sym(p) => {
                                    let pos = params.iter().position(|q| q == p).unwrap();
                                    args[pos].clone()
                                }
                                lit => lit.clone(),
                            })
                            .collect();
                        *n += 1;
                        let mut out = vec![bitems[0].clone()];
                        out.extend(new_items);
                        return LispVal::List(out);
                    }
                }
            }
            // generic form: rewrite all children
            LispVal::List(
                items
                    .into_iter()
                    .map(|x| rewrite(x, ws, shadowed, in_try, n))
                    .collect(),
            )
        }
    }
}

/// Inline trivial u128 wrapper calls across the whole program. Returns the
/// number of call sites rewritten (0 = untouched program).
pub fn inline_trivial_u128(exprs: &mut [LispVal]) -> usize {
    let ws = WrapperSet::collect(exprs);
    if ws.fns.is_empty() {
        return 0;
    }
    // conservative kill: any value use / shadow-rebind / set!-to of a
    // wrapper name disqualifies that name everywhere
    let names: Vec<String> = ws.fns.keys().cloned().collect();
    let mut live = ws.fns.clone();
    for name in &names {
        let killed = exprs
            .iter()
            .any(|e| any_value_use(e, name, &HashSet::new()));
        if killed {
            live.remove(name);
        }
    }
    if live.is_empty() {
        return 0;
    }
    let live_ws = WrapperSet { fns: live };
    let mut n = 0usize;
    for e in exprs.iter_mut() {
        *e = rewrite(e.clone(), &live_ws, &HashSet::new(), false, &mut n);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_all;

    fn inlined(src: &str) -> (Vec<LispVal>, usize) {
        let mut exprs = parse_all(src).unwrap();
        let n = inline_trivial_u128(&mut exprs);
        (exprs, n)
    }

    #[test]
    fn inlines_pool_a_wrappers() {
        let src = r#"(define (add a b) (u128/add a b))
(define (f x) (add x "3"))"#;
        let (e, n) = inlined(src);
        assert_eq!(n, 1);
        let s = format!("{}", e[1]);
        assert!(s.contains("u128/add"), "call became op form: {}", s);
        assert!(!s.contains("(add x"), "no residual wrapper call: {}", s);
    }

    #[test]
    fn nested_inline_and_order() {
        let src = r#"(define (mdiv a b d) (u128/muldiv a b d))
(define (add a b) (u128/add a b))
(define (f q w) (mdiv q (add w "1") "2"))"#;
        let (e, n) = inlined(src);
        assert_eq!(n, 2, "both call sites rewritten");
        let s = format!("{}", e[2]);
        assert!(
            s.contains("(u128/muldiv q (u128/add w \"1\") \"2\")"),
            "nested + arg order: {}",
            s
        );
    }

    #[test]
    fn arity_mismatch_untouched() {
        let src = r#"(define (add a b) (u128/add a b))
(define (f x) (add x))"#;
        let (_, n) = inlined(src);
        assert_eq!(n, 0, "wrong-arity call keeps the error path");
    }

    #[test]
    fn value_use_disqualifies() {
        let src = r#"(define (add a b) (u128/add a b))
(define (f xs) (map add xs))"#;
        let (e, n) = inlined(src);
        assert_eq!(n, 0);
        assert!(format!("{}", e[1]).contains("map"));
    }

    #[test]
    fn shadowing_disqualifies() {
        // let-shadowed head: call targets the local binding, not the wrapper
        let src = r#"(define (add a b) (u128/add a b))
(define (f g) (let ((add g)) (add 1 2)))"#;
        let (_, n) = inlined(src);
        assert_eq!(n, 0, "shadowed head not rewritten");
    }

    #[test]
    fn try_subtree_untouched() {
        // trap observability: wrapper overflow under try stays a call
        let src = r#"(define (add a b) (u128/add a b))
(define (f x) (try (add x "1") (catch e "neg")))"#;
        let (e, n) = inlined(src);
        assert_eq!(n, 0, "try subtree left alone");
        assert!(format!("{}", e[1]).contains("(add x \"1\")"));
    }

    #[test]
    fn let_inits_and_body_rewrite() {
        let src = r#"(define (sub a b) (u128/sub a b))
(define (f sqp gb y)
  (let* ((w (sub gb sqp)) (t (if (u128/lt w y) w y)))
    (sub sqp t)))"#;
        let (e, n) = inlined(src);
        assert_eq!(n, 2);
        let s = format!("{}", e[1]);
        assert!(s.contains("(u128/sub gb sqp)"));
        assert!(s.contains("(u128/sub sqp t)"));
    }

    #[test]
    fn quote_data_untouched() {
        let src = r#"(define (add a b) (u128/add a b))
(define (f) (quote (add 1 2)))"#;
        let (_, n) = inlined(src);
        assert_eq!(n, 0);
    }
}
