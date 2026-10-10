//! Compile-time gas fill (2026-10-09): `(gas default)` at a promise gas
//! slot expands to a static conservative gas literal BEFORE any check or
//! emission runs, so the gate, type checker and emitter all see a plain
//! `LispVal::Num` — one rewrite point, no parity split.
//!
//! Fill model = the checker's callback-floor model, which is validated
//! against real-VM replay (examples/ab_runner.rs: predicted 14.43 Tgas vs
//! ~1.2 Tgas static + 10 Tgas attached): receipt base + per-host-op
//! minimum over the callback's transitive same-program closure, times a
//! 2x headroom (measured replay margin < 1.5x). Lower bounds are sound —
//! loops only add runtime cost, never remove it.
//!
//! Conservative scope: fills ONLY same-program callbacks, i.e.
//! `promise_then` whose account argument is exactly
//! `(near/current_account_id)` and whose method is a define in this file.
//! We do not own remote callees (docs/resource-flow.md), so their floor is
//! unknowable and `(gas default)` there is a hard error — never a silent
//! guess that could underfund a receipt.
//!
//! `default` is a lone symbol, NOT a language binding: the canonical form
//! is `(gas default)`; a bare `default` at a gas slot keeps failing the
//! type checker as an undefined variable, exactly as before.

use crate::types::LispVal;
use crate::typing::checker::{collect_bodies, collect_top_defns, hosts_closure_of};
use std::collections::{HashMap, HashSet};

/// Receipt base: receive + execute any receipt. Same constant and provenance
/// as the checker's callback-floor warning (checker.rs CB_FLOOR_BASE).
const FILL_BASE: i128 = 1_000_000_000_000;
/// Per host-op minimum, >= realistic instruction cost (CB_FLOOR_PER_HOST).
const FILL_PER_HOST: i128 = 10_000_000_000;
/// Safety headroom over the static floor (replay margin measured < 1.5x).
const FILL_HEADROOM: i128 = 2;
/// Defensive cap: NEAR's per-transaction gas limit.
const FILL_CAP: i128 = 300_000_000_000_000;

/// Expand every `(gas default)` in-place. Hard error on any `default` we
/// cannot provably fill (remote callee, undefined local method, non-literal
/// method name). Returns Ok when no site remains that is not a plain value.
pub fn expand_gas_defaults(exprs: &mut [LispVal]) -> Result<(), String> {
    let defns = collect_top_defns(exprs);
    let bodies = collect_bodies(exprs);
    let mut errs: Vec<String> = Vec::new();
    for e in exprs.iter_mut() {
        walk_fill(e, &bodies, &defns, &mut errs);
    }
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("\n"))
    }
}

fn walk_fill(
    e: &mut LispVal,
    bodies: &HashMap<String, Vec<LispVal>>,
    defns: &HashMap<String, usize>,
    errs: &mut Vec<String>,
) {
    if let LispVal::List(items) = e {
        // Binder/introducing forms: never themselves a resource op; descend
        // past the head so shadowed names and nested bodies are covered.
        let binder = matches!(
            items.first(),
            Some(LispVal::Sym(s))
                if s == "define" || s == "let" || s == "let*" || s == "lambda" || s == "while" || s == "loop"
        );
        if binder {
            for it in items.iter_mut().skip(1) {
                walk_fill(it, bodies, defns, errs);
            }
            return;
        }
        let head_is_then = matches!(items.first(), Some(LispVal::Sym(s)) if s == "near/promise_then");
        if head_is_then && items.len() == 7 {
            // Exact-locality: the account argument IS (near/current_account_id).
            let local = matches!(
                items.get(2),
                Some(LispVal::List(a))
                    if a.len() == 1 && matches!(a.first(), Some(LispVal::Sym(s)) if s == "near/current_account_id")
            );
            let mname = match items.get(3) {
                Some(LispVal::Str(s)) => Some(s.clone()),
                _ => None,
            };
            let wants_default = matches!(
                items.get(6),
                Some(LispVal::List(g))
                    if g.len() == 2
                        && matches!(g.first(), Some(LispVal::Sym(s)) if s == "gas")
                        && matches!(g.get(1), Some(LispVal::Sym(d)) if d == "default")
            );
            if wants_default {
                if !local {
                    errs.push(
                        "error: (gas default) is only valid for same-program callbacks — a remote callee's floor is unknowable and a silent fill could underfund the receipt; pass an explicit gas value"
                            .to_string(),
                    );
                } else {
                    match mname {
                        Some(m) if defns.contains_key(&m) => {
                            let mut memo = HashMap::new();
                            let mut stack = Vec::new();
                            let hosts = hosts_closure_of(&m, bodies, defns, &mut memo, &mut stack);
                            let fill =
                                ((FILL_BASE + FILL_PER_HOST * hosts) * FILL_HEADROOM).min(FILL_CAP);
                            items[6] = LispVal::Num(fill as i64);
                        }
                        Some(m) => errs.push(format!(
                            "error: (gas default) callback '{}' is not defined in this program — there is no floor to fill from; define it or pass an explicit gas value",
                            m
                        )),
                        None => errs.push(
                            "error: (gas default) callback method must be a string literal"
                                .to_string(),
                        ),
                    }
                }
            }
        }
        for it in items.iter_mut() {
            walk_fill(it, bodies, defns, errs);
        }
    }
}
