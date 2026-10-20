//! u128 native-chain 2B: storage-site resolution (checker + emit, ONE source).
//!
//! Design (soundness argument — do not weaken without re-reading):
//! - Literal-key READS are Native carriers. The emit-side read path sniffs
//!   the first value byte: NUL ⇒ 17-byte binary cell (0x00 + 16B LE) ⇒
//!   hydrate limbs; anything else ⇒ the SAME runtime strict-decimal parse
//!   the tagged path uses ⇒ identical result-or-trap for arbitrary content.
//!   Therefore no cell-content inference is needed for read safety — a
//!   "wrongly hot" read is semantically identical, only slower.
//! - WRITES choose codec by the value's provenance at the emit site:
//!   limb-borne value ⇒ binary cell (saves the to_str render AND shrinks
//!   storage bytes 38→17); tagged value ⇒ decimal bytes exactly as phase 1
//!   (json-fed `start`, audit cells). Dynamic-key writes always decimal —
//!   which is what keeps `paid:`/`left:` human-audit cells decimal (charter).
//! - Mixed formats in one cell across time are fine: reads self-describe.
//!   Old-code-reads-new-state is the ONLY hazard (decimal parse of a NUL
//!   byte fails) → redeploy discipline, not a code-level fallback.
use crate::LispVal;

pub(crate) enum StorageSite {
    /// (key_expr, value_expr) — `(near/storage_set k v)` possibly through
    /// an arity-2 wrapper.
    Write(Box<LispVal>, Box<LispVal>),
    /// (key_expr, default_expr_opt) — `(near/storage_get k)` possibly
    /// through `default` / an arity-1-or-2 wrapper.
    Read(Box<LispVal>, Option<Box<LispVal>>),
}

/// Resolve a form to a storage READ/WRITE site, seeing through `default`
/// and flat user wrappers (sput/sget shape). Shared by the checker's
/// Native-carrier rule and the emitter's codec selection — single source.
pub(crate) fn storage_site(
    e: &LispVal,
    wrappers: &std::collections::HashMap<String, (Vec<String>, LispVal)>,
) -> Option<StorageSite> {
    let LispVal::List(items) = e else {
        return None;
    };
    let LispVal::Sym(h) = items.first()? else {
        return None;
    };
    let args: Vec<LispVal> = items[1..].to_vec();
    match h.as_str() {
        "near/storage_get" | "near/storage_read" => {
            if args.len() != 1 {
                return None;
            }
            Some(StorageSite::Read(Box::new(args[0].clone()), None))
        }
        "near/storage_set" | "near/storage_write" => {
            if args.len() != 2 {
                return None;
            }
            Some(StorageSite::Write(
                Box::new(args[0].clone()),
                Box::new(args[1].clone()),
            ))
        }
        "default" => {
            // (default (sget k) d)
            if args.len() == 2 {
                if let Some(StorageSite::Read(k, _)) = storage_site(&args[0], wrappers) {
                    return Some(StorageSite::Read(k, Some(Box::new(args[1].clone()))));
                }
            }
            None
        }
        _ => {
            // user wrapper: (define (sget k d) (default (near/storage_get k) d))
            let (ps, body) = wrappers.get(h)?;
            if ps.is_empty() || ps.len() != args.len() {
                return None;
            }
            let substituted = substitute_syms(body, ps, &args);
            storage_site(&substituted, wrappers)
        }
    }
}

/// True when the key expr is a string literal (hot-cell candidate).
pub(crate) fn literal_key(k: &LispVal) -> Option<&str> {
    match k {
        LispVal::Str(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Hot LIMB-read predicate (scan seeding, sniff arm, checker carrier rule).
/// Literal key AND the default — when present — is a non-empty digit
/// literal. A non-digit default ("" above all) makes the site's result
/// "cell-or-default-text"; the hydrate's len==0 guard treats the resolved
/// "" as invalid and traps (probes pa/pc, 2026-10-13). Digit defaults are
/// safe: miss → "0" parses decimal; hit → cell sniffs. No default → the
/// raw cell itself.
pub(crate) fn hot_limb_read(
    e: &LispVal,
    wrappers: &std::collections::HashMap<String, (Vec<String>, LispVal)>,
) -> bool {
    match storage_site(e, wrappers) {
        Some(StorageSite::Read(k, d)) => {
            if literal_key(&k).is_none() {
                return false;
            }
            match d {
                None => true,
                Some(dd) => matches!(
                    dd.as_ref(),
                    LispVal::Str(s)
                        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
                ),
            }
        }
        _ => false,
    }
}

/// Recursive program-walk: does ANY form in `e` classify as a storage site
/// with a literal key? (The pipeline's hot-mode probe must see inside
/// define bodies — reads hide under `(define (main) …)`, and a top-level
/// scan alone classified every real program cold. 2026-10-13, 2B.)
pub(crate) fn any_hot_read_in(
    e: &LispVal,
    wrappers: &std::collections::HashMap<String, (Vec<String>, LispVal)>,
) -> bool {
    if let Some(crate::typing::storage_prov::StorageSite::Read(k, _)) = storage_site(e, wrappers) {
        if literal_key(&k).is_some() {
            return true;
        }
    }
    match e {
        LispVal::List(items) => items.iter().any(|x| any_hot_read_in(x, wrappers)),
        _ => false,
    }
}

/// Textual substitution of param syms with arg exprs (no capture worry:
/// wrapper bodies are flat call forms in this analysis).
fn substitute_syms(e: &LispVal, ps: &[String], args: &[LispVal]) -> LispVal {
    match e {
        LispVal::Sym(n) => {
            if let Some(i) = ps.iter().position(|p| p == n) {
                args[i].clone()
            } else {
                e.clone()
            }
        }
        LispVal::List(items) => {
            LispVal::List(items.iter().map(|x| substitute_syms(x, ps, args)).collect())
        }
        _ => e.clone(),
    }
}
