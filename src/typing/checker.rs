//! Bidirectional type checker for the pure subset.
//!
//! Usage:
//! ```lisp
//! (pure (define (f x y) :: int -> int -> int
//!   (+ x (* y 2))))
//! ```
//!
//! The `pure` form extracts the type signature, checks the body against it,
//! and only registers the define if type-checking passes.

use super::types::{Scheme, TcCon, TcEnv, TcType};
use crate::types::LispVal;

/// Returns true for effectful operations that are forbidden in pure blocks.
fn is_effectful(name: &str) -> bool {
    name.starts_with("near/")
        || name.starts_with("json")
        || name == "print"
        || name == "println"
        || name == "set!"
}

// ---------------------------------------------------------------------------
// Substitution & Unification
// ---------------------------------------------------------------------------

/// A substitution: maps type variables to types.
#[derive(Clone, Debug, Default)]
struct Subst(HashMap<u32, TcType>);

impl Subst {
    fn new() -> Self {
        Subst(HashMap::new())
    }

    fn apply(&self, ty: &TcType) -> TcType {
        match ty {
            TcType::Var(id) => match self.0.get(id) {
                Some(t) => self.apply(t),
                None => ty.clone(),
            },
            TcType::Con(c) => TcType::Con(self.apply_con(c)),
            TcType::Arrow(args, ret) => TcType::Arrow(
                args.iter().map(|a| self.apply(a)).collect(),
                Box::new(self.apply(ret)),
            ),
            TcType::Forall(vars, body) => {
                // Don't substitute bound vars
                TcType::Forall(vars.clone(), Box::new(self.apply(body)))
            }
        }
    }

    fn apply_con(&self, con: &TcCon) -> TcCon {
        match con {
            TcCon::List(t) => TcCon::List(Box::new(self.apply(t))),
            TcCon::Map(k, v) => TcCon::Map(Box::new(self.apply(k)), Box::new(self.apply(v))),
            TcCon::Tuple(ts) => TcCon::Tuple(ts.iter().map(|t| self.apply(t)).collect()),
            other => other.clone(),
        }
    }

    #[allow(dead_code)]
    fn apply_scheme(&self, scheme: &Scheme) -> Scheme {
        // Don't substitute bound vars
        Scheme {
            vars: scheme.vars.clone(),
            ty: self.apply(&scheme.ty),
        }
    }

    fn compose(self, other: Subst) -> Subst {
        let mut combined = Subst::new();
        // Apply self to all of other's values
        for (k, v) in other.0 {
            combined.0.insert(k, self.apply(&v));
        }
        // Add self's bindings
        for (k, v) in self.0 {
            combined.0.entry(k).or_insert(v);
        }
        combined
    }

    fn insert(&mut self, var: u32, ty: TcType) {
        self.0.insert(var, ty);
    }
}

/// Unification result.
type UnifyResult = Result<Subst, String>;

/// Unify two types, producing a substitution or an error.
fn unify(t1: &TcType, t2: &TcType) -> UnifyResult {
    match (t1, t2) {
        // Var with Var
        (TcType::Var(a), TcType::Var(b)) if a == b => Ok(Subst::new()),

        // Any with anything — escape hatch for untypeable constructs
        (TcType::Con(TcCon::Any), _) | (_, TcType::Con(TcCon::Any)) => Ok(Subst::new()),

        // Var with anything — occurs check
        (TcType::Var(a), t) | (t, TcType::Var(a)) => {
            if occurs(*a, t) {
                Err(format!("infinite type: 't{} = {}", a, t))
            } else {
                let mut s = Subst::new();
                s.insert(*a, t.clone());
                Ok(s)
            }
        }

        // Nil is bottom — unifies with anything (early-return guard pattern)
        (TcType::Con(TcCon::Nil), _) | (_, TcType::Con(TcCon::Nil)) => Ok(Subst::new()),

        // Constructor matching
        (TcType::Con(c1), TcType::Con(c2)) => unify_con(c1, c2),

        // Arrow matching
        (TcType::Arrow(args1, ret1), TcType::Arrow(args2, ret2)) => {
            if args1.len() != args2.len() {
                return Err(format!(
                    "arity mismatch: {} args vs {} args",
                    args1.len(),
                    args2.len()
                ));
            }
            let mut subst = Subst::new();
            for (a, b) in args1.iter().zip(args2.iter()) {
                let sa = apply_subst(&subst, a);
                let sb = apply_subst(&subst, b);
                let s = unify(&sa, &sb)?;
                subst = s.compose(subst);
            }
            let r1 = apply_subst(&subst, ret1);
            let r2 = apply_subst(&subst, ret2);
            let s = unify(&r1, &r2)?;
            Ok(s.compose(subst))
        }

        _ => Err(format!("type mismatch: {} ≠ {}", t1, t2)),
    }
}

fn unify_con(c1: &TcCon, c2: &TcCon) -> UnifyResult {
    match (c1, c2) {
        (TcCon::Nil, TcCon::Nil) => Ok(Subst::new()),
        (TcCon::Bool, TcCon::Bool) => Ok(Subst::new()),
        (TcCon::Int, TcCon::Int) => Ok(Subst::new()),
        (TcCon::Float, TcCon::Float) => Ok(Subst::new()),
        (TcCon::Num, TcCon::Num) => Ok(Subst::new()),
        (TcCon::Str, TcCon::Str) => Ok(Subst::new()),
        (TcCon::Sym, TcCon::Sym) => Ok(Subst::new()),
        (TcCon::Ptr, TcCon::Ptr) => Ok(Subst::new()),
        // Ptr and Num are the same i64 at runtime (raw untagged) — unifiable so
        // address literals/offsets can flow into mem-get/mem-set!/ptr-add, which
        // validate protected regions at runtime. (2026-08-29)
        (TcCon::Ptr, TcCon::Num) | (TcCon::Num, TcCon::Ptr) => Ok(Subst::new()),
        (TcCon::Ptr, TcCon::Int) | (TcCon::Int, TcCon::Ptr) => Ok(Subst::new()),
        (TcCon::Any, _) | (_, TcCon::Any) => Ok(Subst::new()),
        // Option types: (opt T) unifies with itself (inner must agree) and
        // with nil (the empty branch). A bare T does NOT unify with (opt T) —
        // force the nil case through (default x fallback).
        (TcCon::Opt(a), TcCon::Opt(b)) => unify(a, b),
        (TcCon::Opt(_), TcCon::Nil) | (TcCon::Nil, TcCon::Opt(_)) => Ok(Subst::new()),
        (TcCon::Num, TcCon::Int) | (TcCon::Int, TcCon::Num) => Ok(Subst::new()),
        (TcCon::Num, TcCon::Float) | (TcCon::Float, TcCon::Num) => Ok(Subst::new()),
        (TcCon::List(a), TcCon::List(b)) => unify(a, b),
        (TcCon::Map(k1, v1), TcCon::Map(k2, v2)) => {
            let s1 = unify(k1, k2)?;
            let _k2_sub = apply_subst(&s1, k2);
            let v1_sub = apply_subst(&s1, v1);
            let v2_sub = apply_subst(&s1, v2);
            let s2 = unify(&v1_sub, &v2_sub)?;
            Ok(s2.compose(s1))
        }
        (TcCon::Tuple(ts1), TcCon::Tuple(ts2)) => {
            if ts1.len() != ts2.len() {
                return Err(format!(
                    "tuple length mismatch: {} vs {}",
                    ts1.len(),
                    ts2.len()
                ));
            }
            let mut subst = Subst::new();
            for (a, b) in ts1.iter().zip(ts2.iter()) {
                let sa = apply_subst(&subst, a);
                let sb = apply_subst(&subst, b);
                let s = unify(&sa, &sb)?;
                subst = s.compose(subst);
            }
            Ok(subst)
        }
        _ => Err(format!(
            "type mismatch: {} ≠ {}",
            TcType::Con(c1.clone()),
            TcType::Con(c2.clone())
        )),
    }
}

fn occurs(var: u32, ty: &TcType) -> bool {
    match ty {
        TcType::Var(v) => *v == var,
        TcType::Con(c) => occurs_con(var, c),
        TcType::Arrow(args, ret) => args.iter().any(|a| occurs(var, a)) || occurs(var, ret),
        TcType::Forall(vars, body) => {
            if vars.contains(&var) {
                false // bound variable, not free
            } else {
                occurs(var, body)
            }
        }
    }
}

fn occurs_con(var: u32, con: &TcCon) -> bool {
    match con {
        TcCon::List(t) => occurs(var, t),
        TcCon::Map(k, v) => occurs(var, k) || occurs(var, v),
        TcCon::Tuple(ts) => ts.iter().any(|t| occurs(var, t)),
        _ => false,
    }
}

fn apply_subst(subst: &Subst, ty: &TcType) -> TcType {
    subst.apply(ty)
}

// ---------------------------------------------------------------------------
// Fresh type variable supply
// ---------------------------------------------------------------------------

struct VarSupply {
    next: u32,
}

impl VarSupply {
    fn new() -> Self {
        VarSupply { next: 1000 } // Start high to avoid conflicts with builtin schemes
    }

    fn fresh(&mut self) -> TcType {
        let id = self.next;
        self.next += 1;
        TcType::Var(id)
    }
}

// ---------------------------------------------------------------------------
// Type parsing from LispVal annotations
// ---------------------------------------------------------------------------

/// Parse a type annotation like `int -> int -> int` from a LispVal.
/// The annotation is a flat list: (int -> int -> int)
/// or nested: ((int int) -> int)
pub fn parse_type_annotation(ann: &LispVal) -> Result<TcType, String> {
    match ann {
        LispVal::Sym(s) => parse_type_sym(s),
        LispVal::List(elems) => parse_type_list(elems),
        other => Err(format!(
            "type annotation: expected symbol or list, got {}",
            other
        )),
    }
}

fn parse_type_sym(s: &str) -> Result<TcType, String> {
    Ok(match s {
        "nil" | ":nil" => TcType::Con(TcCon::Nil),
        "bool" | ":bool" => TcType::Con(TcCon::Bool),
        "int" | ":int" | "i64" => TcType::Con(TcCon::Int),
        "float" | ":float" | "f64" => TcType::Con(TcCon::Float),
        "num" | ":num" | "number" => TcType::Con(TcCon::Num),
        "str" | ":str" | "string" => TcType::Con(TcCon::Str),
        "sym" | ":sym" | "symbol" => TcType::Con(TcCon::Sym),
        "any" | ":any" => TcType::Con(TcCon::Any),
        "ptr" | ":ptr" | "pointer" => TcType::Con(TcCon::Ptr),
        other => return Err(format!("unknown type: {}", other)),
    })
}

pub fn parse_type_list(elems: &[LispVal]) -> Result<TcType, String> {
    if elems.is_empty() {
        return Err("empty type annotation".into());
    }

    // Check for (list T) form
    if let LispVal::Sym(s) = &elems[0] {
        match s.as_str() {
            "list" | ":list" => {
                if elems.len() != 2 {
                    return Err(format!("(list T) expects 1 arg, got {}", elems.len() - 1));
                }
                let inner = parse_type_annotation(&elems[1])?;
                return Ok(TcType::Con(TcCon::List(Box::new(inner))));
            }
            "opt" | ":opt" | "option" | ":option" => {
                if elems.len() != 2 {
                    return Err(format!("(opt T) expects 1 arg, got {}", elems.len() - 1));
                }
                let inner = parse_type_annotation(&elems[1])?;
                return Ok(TcType::Con(TcCon::Opt(Box::new(inner))));
            }
            "map" | ":map" => {
                if elems.len() != 3 {
                    return Err(format!("(map K V) expects 2 args, got {}", elems.len() - 1));
                }
                let k = parse_type_annotation(&elems[1])?;
                let v = parse_type_annotation(&elems[2])?;
                return Ok(TcType::Con(TcCon::Map(Box::new(k), Box::new(v))));
            }
            "tuple" | ":tuple" => {
                let inner: Result<Vec<TcType>, String> =
                    elems[1..].iter().map(parse_type_annotation).collect();
                return Ok(TcType::Con(TcCon::Tuple(inner?)));
            }
            _ => {}
        }
    }

    // Arrow type: split on "->"
    let arrow_positions: Vec<usize> = elems
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, LispVal::Sym(s) if s == "->"))
        .map(|(i, _)| i)
        .collect();

    if arrow_positions.is_empty() {
        // No arrow — try as a single type
        if elems.len() == 1 {
            return parse_type_annotation(&elems[0]);
        }
        return Err(format!("type annotation: unexpected list {:?}", elems));
    }

    // Parse as: arg1 arg2 ... -> ret
    // Last arrow separates args from return
    let last_arrow = *arrow_positions.last().unwrap();
    let ret_slice: Vec<LispVal> = elems[last_arrow + 1..].to_vec();
    let ret = parse_type_annotation(ret_slice.first().ok_or("arrow type: missing return type")?)?;

    // Everything before last arrow could be multiple arrows (curried)
    // For now, treat everything before last -> as arg types
    let args: Result<Vec<TcType>, String> = elems[..last_arrow]
        .iter()
        .filter(|e| !matches!(e, LispVal::Sym(s) if s == "->"))
        .map(parse_type_annotation)
        .collect();

    let args = args?;
    if args.is_empty() && last_arrow != 0 {
        // nilary arrow `-> ret` is fine (last_arrow == 0);
        // otherwise there were stray types before a leading ->
        return Err("arrow type: missing argument types".into());
    }

    Ok(TcType::Arrow(args, Box::new(ret)))
}

// ---------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------

/// Result of type-checking a pure define.
pub struct PureCheckResult {
    pub name: String,
    pub inferred_type: TcType,
}

/// Check a block of `pure` define forms with a shared type environment.
///
/// Each define is type-checked in order. Inferred types from earlier defines
/// are added to the environment so later defines can reference them:
///
/// ```lisp
/// (pure
///   (define (double x) (* x 2))      ;; inferred: num → num
///   (define (quadruple x) (double (double x))))  ;; sees double's type
/// ```
///
/// Input: a slice of define forms (each is a `LispVal::List` starting with "define").
/// Returns Ok with all results if every form type-checks, or the first error.
pub fn check_pure_block(forms: &[&LispVal]) -> Result<Vec<PureCheckResult>, String> {
    let mut env = TcEnv::with_pure_builtins();
    env.pure_mode = true; // forbid effectful calls in pure blocks
    let mut supply = VarSupply::new();
    let mut results = Vec::new();

    for form in forms {
        let list = match form {
            LispVal::List(l) => l,
            other => return Err(format!("pure: expected list, got {}", other)),
        };

        if list.is_empty() {
            return Err("pure: empty define form".into());
        }

        match &list[0] {
            LispVal::Sym(s) if s == "define" => {}
            other => return Err(format!("pure: expected define, got {}", other)),
        }

        // Extract name + params + type annotation + body
        let result = check_define_in_env(list, &mut env, &mut supply)?;

        // Add the inferred type to the shared environment for later forms
        env.insert_mono(result.name.clone(), result.inferred_type.clone());
        results.push(result);
    }

    Ok(results)
}

/// Check a single define form using an existing type environment (for pure blocks).
fn check_define_in_env(
    list: &[LispVal],
    env: &mut TcEnv,
    supply: &mut VarSupply,
) -> Result<PureCheckResult, String> {
    match list.get(1) {
        Some(LispVal::List(sig)) => {
            // (define (f x y) [:: type] body)
            check_function_define_in_env(sig, &list[2..], env, supply)
        }
        Some(LispVal::Sym(_name)) => {
            // (define name [:: type] expr)
            check_value_define_in_env(&list[1..], env, supply)
        }
        other => Err(format!("pure define: unexpected form {:?}", other)),
    }
}

/// Check a function define in an existing environment.
fn check_function_define_in_env(
    sig: &[LispVal],
    rest: &[LispVal],
    env: &mut TcEnv,
    supply: &mut VarSupply,
) -> Result<PureCheckResult, String> {
    let name = match sig.first() {
        Some(LispVal::Sym(s)) => s.clone(),
        other => {
            return Err(format!(
                "pure define: expected function name, got {:?}",
                other
            ))
        }
    };

    let params: Vec<String> = sig[1..]
        .iter()
        .map(|v| match v {
            LispVal::Sym(s) => s.clone(),
            other => format!("_{}", other),
        })
        .collect();

    // Parse type annotation and body
    let (annotated_type, body) = if rest.len() >= 3 {
        match &rest[0] {
            LispVal::Sym(s) if s == "::" => {
                if rest.len() < 3 {
                    return Err("pure define: missing body after type annotation".into());
                }
                let body = rest.last().cloned().unwrap();
                let type_parts: Vec<LispVal> = rest[1..rest.len() - 1].to_vec();
                let ann_type = parse_type_annotation(&LispVal::List(type_parts))?;
                (Some(ann_type), body)
            }
            _ => {
                let body = rest.last().cloned().unwrap_or(LispVal::Nil);
                (None, body)
            }
        }
    } else if rest.len() >= 1 {
        match &rest[0] {
            LispVal::Sym(s) if s == "::" => {
                return Err("pure define: missing type annotation after ::".into());
            }
            _ => {
                let body = rest[0].clone();
                (None, body)
            }
        }
    } else {
        return Err("pure define: missing body".into());
    };

    // Create a child scope: params shadow outer names
    let mut check_env = env.clone();
    let mut subst = Subst::new();

    let param_types: Vec<TcType> = if let Some(ref ann_ty) = annotated_type {
        match ann_ty {
            TcType::Arrow(args, ret) => {
                if args.len() != params.len() {
                    return Err(format!(
                        "pure define {}: annotation has {} params, function has {}",
                        name,
                        args.len(),
                        params.len()
                    ));
                }
                let self_type = TcType::Arrow(args.clone(), ret.clone());
                check_env.insert_mono(name.clone(), self_type);
                args.clone()
            }
            other => {
                return Err(format!(
                    "pure define {}: expected arrow type, got {}",
                    name, other
                ));
            }
        }
    } else {
        let ret_var = supply.fresh();
        let arg_vars: Vec<TcType> = params.iter().map(|_| supply.fresh()).collect();
        let self_type = TcType::Arrow(arg_vars.clone(), Box::new(ret_var));
        check_env.insert_mono(name.clone(), self_type);
        arg_vars
    };

    for (p, t) in params.iter().zip(param_types.iter()) {
        check_env.insert_mono(p.clone(), t.clone());
    }

    // Infer body type
    let body_type = infer(&body, &check_env, supply, &mut subst)?;

    let resolved_params: Vec<TcType> = param_types.iter().map(|t| subst.apply(t)).collect();
    let resolved_ret = subst.apply(&body_type);
    let inferred = TcType::Arrow(resolved_params.clone(), Box::new(resolved_ret.clone()));

    // Check against annotation if provided
    if let Some(ann_ty) = annotated_type {
        let s = unify(&inferred, &ann_ty)
            .map_err(|e| format!("pure define {}: type error — {}", name, e))?;
        subst = s.compose(subst);
    }

    let final_type = subst.apply(&inferred);

    Ok(PureCheckResult {
        name,
        inferred_type: final_type,
    })
}

/// Check a value define in an existing environment.
fn check_value_define_in_env(
    parts: &[LispVal],
    env: &mut TcEnv,
    supply: &mut VarSupply,
) -> Result<PureCheckResult, String> {
    let name = match parts.first() {
        Some(LispVal::Sym(s)) => s.clone(),
        other => return Err(format!("pure define: expected name, got {:?}", other)),
    };

    let (annotated_type, body) = if parts.len() >= 4 {
        match &parts[1] {
            LispVal::Sym(s) if s == "::" => {
                let ann_type = parse_type_annotation(&parts[2])?;
                let body = parts[3].clone();
                (Some(ann_type), body)
            }
            _ => {
                let body = parts[1].clone();
                (None, body)
            }
        }
    } else if parts.len() >= 2 {
        match &parts[1] {
            LispVal::Sym(s) if s == "::" => {
                return Err("pure define: missing type after ::".into());
            }
            _ => {
                let body = parts[1].clone();
                (None, body)
            }
        }
    } else {
        return Err("pure define: missing body".into());
    };

    let mut subst = Subst::new();
    let body_type = infer(&body, env, supply, &mut subst)?;

    if let Some(ann_ty) = annotated_type {
        let s = unify(&body_type, &ann_ty)
            .map_err(|e| format!("pure define {}: type error — {}", name, e))?;
        subst = s.compose(subst);
    }

    let final_type = subst.apply(&body_type);

    Ok(PureCheckResult {
        name,
        inferred_type: final_type,
    })
}

/// Check a `pure` define form (single form, backward compatible).
///
/// Expected input: the args to `pure` — a single define form.
/// `(pure (define (f x y) :: int -> int -> int (body)))`
///
/// Returns Ok(PureCheckResult) if the body type-checks against the annotation,
/// or Err with a human-readable type error.
pub fn check_pure_define(args: &[LispVal]) -> Result<PureCheckResult, String> {
    let define_form = args.first().ok_or("pure: expected a define form")?;

    // Extract define parts
    let list = match define_form {
        LispVal::List(l) => l,
        other => return Err(format!("pure: expected list, got {}", other)),
    };

    if list.is_empty() {
        return Err("pure: empty define form".into());
    }

    // Must start with "define"
    match &list[0] {
        LispVal::Sym(s) if s == "define" => {}
        other => return Err(format!("pure: expected define, got {}", other)),
    }

    // Two forms:
    // (define (f params...) :: type body)
    // (define name :: type expr)
    match list.get(1) {
        Some(LispVal::List(sig)) => {
            // (define (f x y) :: type body)
            check_function_define(sig, &list[2..])
        }
        Some(LispVal::Sym(_name)) => {
            // Simple binding: (define x :: type expr)
            check_value_define(&list[1..])
        }
        other => Err(format!("pure define: unexpected form {:?}", other)),
    }
}

fn check_function_define(sig: &[LispVal], rest: &[LispVal]) -> Result<PureCheckResult, String> {
    // sig = [name, param1, param2, ...]
    let name = match sig.first() {
        Some(LispVal::Sym(s)) => s.clone(),
        other => {
            return Err(format!(
                "pure define: expected function name, got {:?}",
                other
            ))
        }
    };

    let params: Vec<String> = sig[1..]
        .iter()
        .map(|v| match v {
            LispVal::Sym(s) => s.clone(),
            other => format!("_{}", other),
        })
        .collect();

    // rest contains: [:: type-parts... body]
    // Type parts are individual symbols: int -> int -> int
    // We need to collect from after :: until we find the body (last element or a list)
    let (annotated_type, body) = if rest.len() >= 3 {
        match &rest[0] {
            LispVal::Sym(s) if s == "::" => {
                // Find where the type annotation ends and the body begins
                // The body is the last element (or first non-type-looking element)
                // Strategy: take everything between :: and the last element as type
                if rest.len() < 3 {
                    return Err("pure define: missing body after type annotation".into());
                }
                // Last element is the body
                let body = rest.last().cloned().unwrap();
                // Everything between :: and body is the type
                let type_parts: Vec<LispVal> = rest[1..rest.len() - 1].to_vec();
                let ann_type = parse_type_annotation(&LispVal::List(type_parts))?;
                (Some(ann_type), body)
            }
            _ => {
                // No annotation — infer. Body is last element.
                let body = rest.last().cloned().unwrap_or(LispVal::Nil);
                (None, body)
            }
        }
    } else if rest.len() >= 1 {
        match &rest[0] {
            LispVal::Sym(s) if s == "::" => {
                return Err("pure define: missing type annotation after ::".into());
            }
            _ => {
                let body = rest[0].clone();
                (None, body)
            }
        }
    } else {
        return Err("pure define: missing body".into());
    };

    // Set up typing environment
    let mut env = TcEnv::with_pure_builtins();
    env.pure_mode = true;
    let mut supply = VarSupply::new();

    // Add params with fresh type vars or from annotation
    let param_types: Vec<TcType> = if let Some(ref ann_ty) = annotated_type {
        match ann_ty {
            TcType::Arrow(args, ret) => {
                if args.len() != params.len() {
                    return Err(format!(
                        "pure define {}: annotation has {} params, function has {}",
                        name,
                        args.len(),
                        params.len()
                    ));
                }
                // Put the function itself in scope for self-reference
                let self_type = TcType::Arrow(args.clone(), ret.clone());
                env.insert_mono(name.clone(), self_type);
                args.clone()
            }
            other => {
                return Err(format!(
                    "pure define {}: expected arrow type, got {}",
                    name, other
                ));
            }
        }
    } else {
        // No annotation — give the function a fresh type var for the return
        let ret_var = supply.fresh();
        let arg_vars: Vec<TcType> = params.iter().map(|_| supply.fresh()).collect();
        let self_type = TcType::Arrow(arg_vars.clone(), Box::new(ret_var));
        env.insert_mono(name.clone(), self_type);
        arg_vars
    };

    for (p, t) in params.iter().zip(param_types.iter()) {
        env.insert_mono(p.clone(), t.clone());
    }

    // Infer the body type
    let mut subst = Subst::new();
    let body_type = infer(&body, &env, &mut supply, &mut subst)?;

    // Build the full function type
    let resolved_params: Vec<TcType> = param_types.iter().map(|t| subst.apply(t)).collect();
    let resolved_ret = subst.apply(&body_type);
    let inferred = TcType::Arrow(resolved_params.clone(), Box::new(resolved_ret.clone()));

    // Check against annotation if provided
    if let Some(ann_ty) = annotated_type {
        let s = unify(&inferred, &ann_ty)
            .map_err(|e| format!("pure define {}: type error — {}", name, e))?;
        subst = s.compose(subst);
    }

    let final_type = subst.apply(&inferred);

    Ok(PureCheckResult {
        name,
        inferred_type: final_type,
    })
}

fn check_value_define(parts: &[LispVal]) -> Result<PureCheckResult, String> {
    let name = match parts.first() {
        Some(LispVal::Sym(s)) => s.clone(),
        other => return Err(format!("pure define: expected name, got {:?}", other)),
    };

    let (annotated_type, body) = if parts.len() >= 4 {
        match &parts[1] {
            LispVal::Sym(s) if s == "::" => {
                let ann_type = parse_type_annotation(&parts[2])?;
                let body = parts[3].clone();
                (Some(ann_type), body)
            }
            _ => {
                let body = parts[1].clone();
                (None, body)
            }
        }
    } else if parts.len() >= 2 {
        let body = parts[1].clone();
        (None, body)
    } else {
        return Err("pure define: missing value".into());
    };

    let env = TcEnv::with_pure_builtins();
    let mut supply = VarSupply::new();
    let mut subst = Subst::new();

    let inferred = infer(&body, &env, &mut supply, &mut subst)?;

    if let Some(ann_ty) = annotated_type {
        let s = unify(&inferred, &ann_ty)
            .map_err(|e| format!("pure define {}: type error — {}", name, e))?;
        subst = s.compose(subst);
    }

    let final_type = subst.apply(&inferred);

    Ok(PureCheckResult {
        name,
        inferred_type: final_type,
    })
}

// ---------------------------------------------------------------------------
// Inference (synthesize mode)
// ---------------------------------------------------------------------------

fn infer(
    expr: &LispVal,
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    match expr {
        // Literals
        LispVal::Nil => Ok(TcType::Con(TcCon::Nil)),
        LispVal::Bool(_) => Ok(TcType::Con(TcCon::Bool)),
        LispVal::Num(_) => Ok(TcType::Con(TcCon::Int)),
        LispVal::U64(_) => Ok(TcType::Con(TcCon::Int)), // U64 is an integer
        LispVal::Float(_) => Ok(TcType::Con(TcCon::Float)),
        LispVal::Str(_) => Ok(TcType::Con(TcCon::Str)),
        LispVal::Sym(s) if s.starts_with(':') => Ok(TcType::Con(TcCon::Sym)), // keywords
        LispVal::BuiltinFn(_) => Ok(TcType::Con(TcCon::Any)), // builtin fn is callable
        LispVal::Tagged { .. } => Ok(TcType::Con(TcCon::Any)), // tagged value is opaque data

        // Symbol lookup
        LispVal::Sym(name) => {
            match env.get(name) {
                Some(scheme) => {
                    // Instantiate the scheme: replace quantified vars with fresh ones
                    Ok(instantiate(scheme, supply))
                }
                None => Err(format!(
                    "type error: undefined variable '{}' — not in scope",
                    name
                )),
            }
        }

        // Lambda: (lambda (params...) body)
        LispVal::List(list) if !list.is_empty() => {
            match &list[0] {
                LispVal::Sym(s) if s == "lambda" || s == "fn" => {
                    infer_lambda(&list[1..], env, supply, subst)
                }
                LispVal::Sym(s) if s == "if" => infer_if(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "default" => {
                    // (default e1 e2) — e1 maybe-nil, e2 the fallback.
                    // Collapses (opt T) → T; forces the nil case to be handled.
                    let e1 = list.get(1).ok_or("default: missing value")?;
                    let e2 = list.get(2).ok_or("default: missing fallback")?;
                    let inner = supply.fresh();
                    let t1 = infer(e1, env, supply, subst)?;
                    let s1 = unify(&t1, &TcType::Con(TcCon::Opt(Box::new(inner.clone()))))
                        .map_err(|e| format!("default: value is not maybe-nil — {}", e))?;
                    *subst = s1.compose(subst.clone());
                    let t2 = infer(e2, env, supply, subst)?;
                    let inner_sub = subst.apply(&inner);
                    let s2 = unify(&inner_sub, &t2)
                        .map_err(|e| format!("default: fallback type disagrees — {}", e))?;
                    *subst = s2.compose(subst.clone());
                    Ok(subst.apply(&t2))
                }
                LispVal::Sym(s) if s == "let" => infer_let(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "let*" => infer_let_star(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "while" => {
                    // (while cond body...) — loop, evaluates to nil.
                    // Note: desugar rewrites dotimes into let+while BEFORE type
                    // checking, so this also covers dotimes.
                    if list.len() >= 3 {
                        let _ = infer(&list[1], env, supply, subst)?;
                        for expr in &list[2..] {
                            let _ = infer(expr, env, supply, subst)?;
                        }
                    }
                    Ok(TcType::Con(TcCon::Nil))
                }
                LispVal::Sym(s) if s == "try" => {
                    // (try body (catch var handler...)) — the body is inferred
                    // LENIENTLY: a type error inside the body becomes a caught
                    // runtime error on the wasm surface (guarded fallible op →
                    // catch jump), so we type it Any instead of rejecting.
                    // The catch var binds the error message (string); the whole
                    // form types as Any (body and handler may differ).
                    if list.len() == 3 {
                        if let LispVal::List(cl) = &list[2] {
                            if cl.len() >= 3 && cl[0] == LispVal::Sym("catch".into()) {
                                if let LispVal::Sym(var) = &cl[1] {
                                    let _ = infer(&list[1], env, supply, subst);
                                    let mut henv = env.clone();
                                    henv.insert_mono(var.clone(), TcType::Con(TcCon::Str));
                                    for h in &cl[2..] {
                                        let _ = infer(h, &henv, supply, subst);
                                    }
                                    let _ = var; // silence unused if bind is non-consuming
                                }
                            }
                        }
                    }
                    Ok(TcType::Con(TcCon::Any))
                }
                LispVal::Sym(s) if s == "begin" => infer_begin(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "and" => infer_and(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "or" => infer_or(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "cond" => infer_cond(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "quote" => Ok(TcType::Con(TcCon::Any)), // quoted data is opaque
                LispVal::Sym(s) if s == "set!" => {
                    // (set! var value) — mutation, infer the value but return nil
                    if env.pure_mode {
                        return Err(
                            "effect error: cannot call 'set!' inside pure — it has side effects"
                                .into(),
                        );
                    }
                    if list.len() >= 3 {
                        let _ = infer(&list[2], env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::Nil))
                }
                LispVal::Sym(s) if s == "loop" => {
                    // (loop ((var init) ...) body...) — TCO loop, type as body
                    // Introduce loop vars into scope
                    if list.len() >= 3 {
                        let mut loop_env = env.clone();
                        if let LispVal::List(bindings) = &list[1] {
                            for b in bindings {
                                if let LispVal::List(p) = b {
                                    if p.len() >= 2 {
                                        let _ = infer(&p[1], env, supply, subst)?;
                                        if let LispVal::Sym(name) = &p[0] {
                                            // Add loop var as monomorphic Any
                                            loop_env
                                                .insert_mono(name.clone(), TcType::Con(TcCon::Any));
                                        }
                                    }
                                }
                            }
                        }
                        // Infer body expressions with loop vars in scope
                        let mut last_ty = TcType::Con(TcCon::Any);
                        for expr in &list[2..] {
                            last_ty = infer(expr, &loop_env, supply, subst)?;
                        }
                        Ok(last_ty)
                    } else {
                        Ok(TcType::Con(TcCon::Any))
                    }
                }
                LispVal::Sym(s) if s == "recur" => {
                    // (recur val ...) — recursive call inside loop
                    // Just infer args for side effects, return Any (it's a jump, not a value)
                    for arg in &list[1..] {
                        let _ = infer(arg, env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::Any))
                }
                LispVal::Sym(s) if s == "assert-equal" => {
                    // (assert-equal expected actual) — infer both, return nil
                    if list.len() >= 3 {
                        let _ = infer(&list[1], env, supply, subst)?;
                        let _ = infer(&list[2], env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::Nil))
                }
                LispVal::Sym(s) if s == "assert-true" || s == "assert-raises" => {
                    if list.len() >= 2 {
                        let _ = infer(&list[1], env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::Nil))
                }
                // json-get 1-arg form: (json-get "key") scans the input
                // buffer (NEAR input() / outlayer stdin). The emitter
                // supports both arities; the typing env only carries the
                // 2-arg signature. Result is dynamic (num or str) → Any.
                LispVal::Sym(s) if s == "json-get" && list.len() == 2 => {
                    let t = infer(&list[1], env, supply, subst)?;
                    let su = unify(&subst.apply(&t), &TcType::Con(TcCon::Str))
                        .map_err(|e| format!("in call (json-get ...): {}", e))?;
                    *subst = su.compose(subst.clone());
                    Ok(TcType::Con(TcCon::Any))
                }
                LispVal::Sym(s) if s == "list" => {
                    infer_list_literal(&list[1..], env, supply, subst)
                }
                // Structural equality/inequality: reflexive α → α → bool.
                // Interpreter compares (= / !=) structurally on any values
                // (nums, bools, strings, lists); the num-only env scheme
                // would reject (= "a" "b") / (!= (list 1) (list 1)).
                // bool ↔ int mixes are allowed: bools ARE tagged ints and
                // `(= ok 1)` is a truthiness probe (surface tour 2 ctx
                // fixture, 2026-09-01 — near/deposit-gte returns bool).
                LispVal::Sym(s) if (s == "=" || s == "!=") && list.len() == 3 => {
                    let t1 = infer(&list[1], env, supply, subst)?;
                    let t2 = infer(&list[2], env, supply, subst)?;
                    let a1 = subst.apply(&t1);
                    let a2 = subst.apply(&t2);
                    let su = match unify(&a1, &a2) {
                        Ok(su) => su,
                        Err(_) => {
                            // mixed pair: OK — `=` is TOTAL at runtime.
                            // The emitter compiles = to __h_val_eq
                            // (structural; tag mismatch → false, no trap)
                            // or the raw-compare num fast path (exact
                            // across {Num, Nil, Bool} tag words). Any
                            // mixed base-type pair therefore types as
                            // Bool with no substitution — TS `!x` relies
                            // on this: it lowers to
                            // (if (if x …) … (= x "")) where x may be
                            // bool/num/str/array (2026-09-27).
                            Subst::new()
                        }
                    };
                    *subst = su.compose(subst.clone());
                    Ok(TcType::Con(TcCon::Bool))
                }
                // Variadic arithmetic desugar: interpreter left-folds
                // (+ a b c ...) = (+ (+ a b) c) for + - * / min max; mod is
                // binary (extras dropped). Typing env is binary-only, so
                // rewrite n-ary calls to nested binaries BEFORE inference.
                LispVal::Sym(op)
                    if matches!(op.as_str(), "+" | "-" | "*" | "/" | "min" | "max")
                        && list.len() > 3 =>
                {
                    let mut folded = list[1].clone();
                    for next in &list[2..] {
                        folded = LispVal::List(vec![list[0].clone(), folded, next.clone()]);
                    }
                    infer(&folded, env, supply, subst)
                }
                LispVal::Sym(op) if op == "mod" && list.len() > 3 => infer(
                    &LispVal::List(vec![list[0].clone(), list[1].clone(), list[2].clone()]),
                    env,
                    supply,
                    subst,
                ),
                LispVal::Sym(s) if s == "dict" => {
                    // Variadic key-val pairs: infer each arg but don't enforce arity
                    for arg in &list[1..] {
                        let _ = infer(arg, env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any)))))
                }
                _ => infer_application(list, env, supply, subst),
            }
        }

        // Empty list
        LispVal::List(_) => Ok(TcType::Con(TcCon::List(Box::new(supply.fresh())))),

        // Vec — infer as (:vec T)
        LispVal::Vec(_) => Ok(TcType::Con(TcCon::Any)),

        // Maps, lambdas from env — treat as opaque
        LispVal::Lambda { .. }
        | LispVal::CaseLambda { .. }
        | LispVal::Macro { .. }
        | LispVal::Map(_)
        | LispVal::Recur(_)
        | LispVal::Memoized { .. }
        | LispVal::Delay { .. } => Ok(TcType::Con(TcCon::Any)),
    }
}

/// Instantiate a type scheme by replacing quantified vars with fresh ones.
fn instantiate(scheme: &Scheme, supply: &mut VarSupply) -> TcType {
    if scheme.vars.is_empty() {
        return scheme.ty.clone();
    }

    let mut mapping = HashMap::new();
    for &v in &scheme.vars {
        mapping.insert(v, supply.fresh());
    }

    substitute(&scheme.ty, &mapping)
}

fn substitute(ty: &TcType, mapping: &HashMap<u32, TcType>) -> TcType {
    match ty {
        TcType::Var(id) => mapping.get(id).cloned().unwrap_or_else(|| ty.clone()),
        TcType::Con(c) => TcType::Con(substitute_con(c, mapping)),
        TcType::Arrow(args, ret) => TcType::Arrow(
            args.iter().map(|a| substitute(a, mapping)).collect(),
            Box::new(substitute(ret, mapping)),
        ),
        TcType::Forall(vars, body) => {
            // Only substitute free vars
            let mut filtered = mapping.clone();
            for v in vars {
                filtered.remove(v);
            }
            TcType::Forall(vars.clone(), Box::new(substitute(body, &filtered)))
        }
    }
}

fn substitute_con(con: &TcCon, mapping: &HashMap<u32, TcType>) -> TcCon {
    match con {
        TcCon::List(t) => TcCon::List(Box::new(substitute(t, mapping))),
        TcCon::Map(k, v) => TcCon::Map(
            Box::new(substitute(k, mapping)),
            Box::new(substitute(v, mapping)),
        ),
        TcCon::Tuple(ts) => TcCon::Tuple(ts.iter().map(|t| substitute(t, mapping)).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Inference helpers for special forms
// ---------------------------------------------------------------------------

fn infer_lambda(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    let params_list = parts.first().ok_or("lambda: missing params")?;
    let body = parts.get(1).cloned().unwrap_or(LispVal::Nil);

    let (param_names, _rest) = parse_lambda_params(params_list)?;

    // Each param gets a fresh type variable
    let mut new_env = env.clone();
    let mut param_types = Vec::new();
    for name in &param_names {
        let t = supply.fresh();
        param_types.push(t.clone());
        new_env.insert_mono(name.clone(), t);
    }

    let body_type = infer(&body, &new_env, supply, subst)?;
    Ok(TcType::Arrow(
        param_types.iter().map(|t| subst.apply(t)).collect(),
        Box::new(subst.apply(&body_type)),
    ))
}

fn parse_lambda_params(val: &LispVal) -> Result<(Vec<String>, Option<String>), String> {
    match val {
        LispVal::List(elems) => {
            let mut params = Vec::new();
            let mut rest = None;
            let mut seen_amp = false;
            for e in elems {
                match e {
                    LispVal::Sym(s) if s == "&rest" => seen_amp = true,
                    LispVal::Sym(s) if seen_amp => {
                        rest = Some(s.clone());
                        seen_amp = false;
                    }
                    LispVal::Sym(s) => params.push(s.clone()),
                    _ => return Err("lambda param must be symbol".into()),
                }
            }
            Ok((params, rest))
        }
        LispVal::Sym(s) => Ok((vec![], Some(s.clone()))), // (lambda args body)
        _ => Err("lambda params must be list".into()),
    }
}

fn infer_if(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    let cond = parts.first().ok_or("if: missing condition")?;
    let then_branch = parts.get(1).ok_or("if: missing then")?;
    let else_branch = parts.get(2);

    // Check condition is bool-ish (we allow any for truthy)
    let _cond_type = infer(cond, env, supply, subst)?;

    let then_type = infer(then_branch, env, supply, subst)?;

    if let Some(else_expr) = else_branch {
        let else_type = infer(else_expr, env, supply, subst)?;
        // Unify branches
        let s = unify(&then_type, &else_type).map_err(|e| {
            // Render both branches (truncated) so the offending source is
            // identifiable in a large module without a source map.
            let trunc = |v: &LispVal| -> String {
                let s = v.to_string();
                if s.chars().count() > 120 {
                    let t: String = s.chars().take(120).collect();
                    format!("{t}…")
                } else {
                    s
                }
            };
            format!(
                "if: branch types disagree — {} (then: {}, else: {})",
                e,
                trunc(then_branch),
                trunc(else_expr)
            )
        })?;
        *subst = s.compose(subst.clone());
    }

    Ok(subst.apply(&then_type))
}

fn infer_let(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    // The WASM emitter treats `let` and `let*` IDENTICALLY — sequential
    // bindings, later ones see earlier ones (wasm_emit let lowering).
    // The checker used to infer `let` with parallel scoping, rejecting
    // valid programs (wallet.lisp: `sig_end` "undefined", 2026-10).
    // Delegate so the checker always matches emitter truth.
    infer_let_star(parts, env, supply, subst)
}

fn infer_let_star(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    let bindings_list = parts.first().ok_or("let*: missing bindings")?;

    let bindings = match bindings_list {
        LispVal::List(l) => l,
        other => return Err(format!("let*: bindings must be list, got {}", other)),
    };

    let mut new_env = env.clone();
    for binding in bindings {
        let pair = match binding {
            LispVal::List(l) if l.len() == 2 => l,
            other => return Err(format!("let*: binding must be (name val), got {:?}", other)),
        };
        let name = match &pair[0] {
            LispVal::Sym(s) => s.clone(),
            other => {
                return Err(format!(
                    "let*: binding name must be symbol, got {:?}",
                    other
                ))
            }
        };
        let val_type = infer(&pair[1], &new_env, supply, subst)?;
        new_env.insert_mono(name, subst.apply(&val_type));
    }

    // multi-statement body: check ALL (2026-09-02 same fix as infer_let)
    if parts.len() < 2 {
        return Ok(TcType::Con(TcCon::Nil));
    }
    if parts.len() == 2 {
        return infer(&parts[1], &new_env, supply, subst);
    }
    let mut last_ty = TcType::Con(TcCon::Nil);
    for stmt in &parts[1..] {
        last_ty = infer(stmt, &new_env, supply, subst)?;
    }
    Ok(last_ty)
}

fn infer_begin(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    if parts.is_empty() {
        return Ok(TcType::Con(TcCon::Nil));
    }
    // Type-check all, return last
    let mut last_ty = TcType::Con(TcCon::Nil);
    for part in parts {
        last_ty = infer(part, env, supply, subst)?;
    }
    Ok(last_ty)
}

fn infer_and(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    if parts.is_empty() {
        return Ok(TcType::Con(TcCon::Bool));
    }
    let mut last = TcType::Con(TcCon::Bool);
    for p in parts {
        last = infer(p, env, supply, subst)?;
    }
    Ok(last)
}

fn infer_or(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    if parts.is_empty() {
        return Ok(TcType::Con(TcCon::Bool));
    }
    let mut last = TcType::Con(TcCon::Bool);
    for p in parts {
        last = infer(p, env, supply, subst)?;
    }
    Ok(last)
}

fn infer_cond(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    if parts.is_empty() {
        return Ok(TcType::Con(TcCon::Nil));
    }
    // Check exhaustiveness: last clause should have "else" as condition
    let has_else = parts
        .iter()
        .last()
        .and_then(|c| {
            if let LispVal::List(l) = c {
                l.first()
            } else {
                None
            }
        })
        .and_then(|v| {
            if let LispVal::Sym(s) = v {
                Some(s.as_str())
            } else {
                None
            }
        })
        .map(|s| s == "else" || s == "true" || s == "t")
        .unwrap_or(false);
    if !has_else {
        eprintln!("⚠ warning: cond without else — may return nil when no branch matches");
    }
    let mut result_type: Option<TcType> = None;
    for clause in parts {
        let pair = match clause {
            LispVal::List(l) if l.len() >= 2 => l,
            _ => continue,
        };
        // Skip type-checking the condition for else/true/t — they're always-true sentinels
        let cond_sym = match &pair[0] {
            LispVal::Sym(s) if s == "else" || s == "true" || s == "t" => None,
            other => Some(other),
        };
        if let Some(cond) = cond_sym {
            let _cond_type = infer(cond, env, supply, subst)?;
        }
        let branch_type = infer(&pair[1], env, supply, subst)?;
        match result_type {
            None => result_type = Some(branch_type),
            Some(ref rt) => {
                let s = unify(rt, &branch_type)
                    .map_err(|e| format!("cond: branch types disagree — {}", e))?;
                *subst = s.compose(subst.clone());
            }
        }
    }
    Ok(result_type.unwrap_or(TcType::Con(TcCon::Nil)))
}

fn infer_list_literal(
    elems: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    if elems.is_empty() {
        let a = supply.fresh();
        return Ok(TcType::Con(TcCon::List(Box::new(a))));
    }

    let first_type = infer(&elems[0], env, supply, subst)?;
    let mut elem_type = subst.apply(&first_type);

    for elem in &elems[1..] {
        let t = infer(elem, env, supply, subst)?;
        let t = subst.apply(&t);
        // Heterogeneous literals: the interpreter is dynamically typed and
        // accepts mixed lists (e.g. (list 1 (list 2 3))). Downgrade to
        // (list any) instead of rejecting — trace-equivalence principle: the
        // wasm surface accepts what the interpreter accepts.
        let s = match unify(&elem_type, &t) {
            Ok(s) => s,
            Err(_) => {
                return Ok(TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any)))));
            }
        };
        *subst = s.compose(subst.clone());
        elem_type = subst.apply(&elem_type);
    }

    Ok(TcType::Con(TcCon::List(Box::new(elem_type))))
}

fn infer_application(
    list: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    let func = &list[0];
    let args = &list[1..];

    // Effect check: reject effectful calls in pure mode
    if env.pure_mode {
        if let LispVal::Sym(name) = func {
            if is_effectful(name) {
                return Err(format!(
                    "effect error: cannot call '{}' inside pure — it has side effects",
                    name
                ));
            }
        }
    }

    // Variadic builtins: accept any arity
    if let LispVal::Sym(name) = func {
        // str-cat: variadic STRINGS-ONLY concat (emitter + interp are
        // variadic; the old binary Arrow type rejected legal 3+ arg forms
        // — matches the wasm_emit arm's n-arg single-alloc path).
        // Non-str args still hard-error (wasm untag assumes TAG_STR).
        if name == "str-cat" {
            for arg in args {
                let t = infer(arg, env, supply, subst)?;
                if unify(&t, &TcType::Con(TcCon::Str)).is_err() {
                    return Err("in call (str-cat ...): args must all be str".into());
                }
            }
            return Ok(TcType::Con(TcCon::Str));
        }
        // json-extract-input / json-extract: variadic (1..=8 str keys →
        // array of raw span strings; the emitter enforces the 8-key cap)
        if name == "json-extract-input" || name == "json-extract" {
            if args.is_empty() {
                return Err(format!("in call ({name} ...): needs at least 1 key"));
            }
            for arg in args.iter().skip(if name == "json-extract" { 1 } else { 0 }) {
                let t = infer(arg, env, supply, subst)?;
                if unify(&t, &TcType::Con(TcCon::Str)).is_err() {
                    return Err(format!("in call ({name} ...): keys must be str"));
                }
            }
            return Ok(TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any)))));
        }
        // len: POLYMORPHIC — interp accepts BOTH str and list (and any
        // array-ish value); the env carried two conflicting registrations
        // (list→int generic at types.rs:800, str-narrow at 1512) so
        // (len (str-concat "a" "b")) failed the checker while interp ran
        // it. Found via the source-level differential fuzzer, 2026-09-01.
        if name == "len" && args.len() == 1 {
            let t = infer(&args[0], env, supply, subst)?;
            let el = supply.fresh();
            let list_t = match el {
                TcType::Var(vid) => TcType::Con(TcCon::List(Box::new(TcType::Var(vid)))),
                other => other,
            };
            let is_list = unify(&t, &list_t).is_ok();
            let is_str = unify(&t, &TcType::Con(TcCon::Str)).is_ok();
            if !is_list && !is_str {
                return Err(format!(
                    "in call (len ...): type mismatch: {} ≠ str/list",
                    t
                ));
            }
            return Ok(TcType::Con(TcCon::Int));
        }
        // vec-length: same polymorphic treatment as len — the TS frontend
        // lowers `.length` (arrays AND strings) to vec-length, the wasm
        // emitter's `len` arm handles TAG_STR + TAG_ARRAY, and the interp
        // "length" builtin accepts both. The old list-only Arrow type
        // rejected `S.length` on any string (surface tour 2, 2026-09-01).
        if name == "vec-length" && args.len() == 1 {
            let t = infer(&args[0], env, supply, subst)?;
            let el = supply.fresh();
            let list_t = match el {
                TcType::Var(vid) => TcType::Con(TcCon::List(Box::new(TcType::Var(vid)))),
                other => other,
            };
            let is_list = unify(&t, &list_t).is_ok();
            let is_str = unify(&t, &TcType::Con(TcCon::Str)).is_ok();
            if !is_list && !is_str {
                return Err(format!(
                    "in call (vec-length ...): type mismatch: {} ≠ str/list",
                    t
                ));
            }
            return Ok(TcType::Con(TcCon::Int));
        }
        if name == "str-concat" || name == "string-append" || name == "str" {
            for arg in args {
                let _ = infer(arg, env, supply, subst)?;
            }
            return Ok(TcType::Con(TcCon::Str));
        }
        // borsh-serialize: variadic (schema_name + field values) → nil
        if name == "borsh-serialize" {
            for arg in args {
                let _ = infer(arg, env, supply, subst)?;
            }
            return Ok(TcType::Con(TcCon::Nil));
        }
        // borsh-deserialize: (schema_name bytes) → any
        if name == "borsh-deserialize" {
            for arg in args {
                let _ = infer(arg, env, supply, subst)?;
            }
            return Ok(TcType::Con(TcCon::Any));
        }
        // array: variadic constructor → list of any
        if name == "array" || name == "list" {
            for arg in args {
                let _ = infer(arg, env, supply, subst)?;
            }
            return Ok(TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any)))));
        }
        // `+` on strings → concat. The interpreter's `+` IS polymorphic
        // (str+str concatenates; see interp "length" parity and the TS
        // dialect's `acc + "S"` idiom), but the typing env is num-only —
        // TS-side string concat that survived lowering (var + var, where a
        // var is string-typed at runtime) hard-errored "num ≠ str".
        // Str+str → str; anything else keeps the num arrow.
        if name == "+" && args.len() == 2 {
            let tl = infer(&args[0], env, supply, subst)?;
            let tr = infer(&args[1], env, supply, subst)?;
            // CONCRETE str only: an unbound type var unifies with Str, which
            // made `(+ (f) 1)` on a not-yet-bound fn return Str and broke
            // near/store's num param (test_near_counter regression 2026-09-01).
            let is_concrete_str = |t: &TcType| matches!(subst.apply(t), TcType::Con(TcCon::Str));
            let l_str = is_concrete_str(&tl);
            let r_str = is_concrete_str(&tr);
            if l_str || r_str {
                // coerce the non-str side through to-string at runtime is the
                // emitter's job; here just accept the mix and return str
                return Ok(TcType::Con(TcCon::Str));
            }
            // fall through to the env arrow (num → num → num)
            let su = unify(&subst.apply(&tl), &TcType::Con(TcCon::Num))
                .map_err(|e| format!("in call (+ ...): {}", e))?;
            *subst = su.compose(subst.clone());
            let su2 = unify(&subst.apply(&tr), &TcType::Con(TcCon::Num))
                .map_err(|e| format!("in call (+ ...): {}", e))?;
            *subst = su2.compose(subst.clone());
            return Ok(TcType::Con(TcCon::Num));
        }
        // `=` is polymorphic at runtime: numeric eq, string eq, and bool eq
        // (bools are tagged ints; the interp's `=` compares tagged values).
        // The typing env is num-only — `(= (deposit-gte 0 0) 1)` (surface
        // tour 2 ctx fixture, 2026-09-01) errored "bool ≠ int". Accept
        // same-type pairs of num|str|bool → bool.
        if name == "=" && args.len() == 2 {
            let tl = infer(&args[0], env, supply, subst)?;
            let tr = infer(&args[1], env, supply, subst)?;
            let tl = subst.apply(&tl);
            let tr = subst.apply(&tr);
            for t in [
                &TcType::Con(TcCon::Num),
                &TcType::Con(TcCon::Str),
                &TcType::Con(TcCon::Bool),
            ] {
                if unify(&tl, t).is_ok() && unify(&tr, t).is_ok() {
                    return Ok(TcType::Con(TcCon::Bool));
                }
            }
            // bool ↔ int mix: bools are tagged ints, `(= ok 1)` is a
            // truthiness probe (surface tour 2 ctx, 2026-09-01)
            let tl_bool = unify(&tl, &TcType::Con(TcCon::Bool)).is_ok();
            let tr_bool = unify(&tr, &TcType::Con(TcCon::Bool)).is_ok();
            if (tl_bool && unify(&tr, &TcType::Con(TcCon::Num)).is_ok())
                || (tr_bool && unify(&tl, &TcType::Con(TcCon::Num)).is_ok())
            {
                return Ok(TcType::Con(TcCon::Bool));
            }
            // mixed/unknown: fall through to the num arrow for a precise error
        }
    }

    let func_type = infer(func, env, supply, subst)?;

    // Create arg types
    let mut arg_types = Vec::new();
    for arg in args {
        let t = infer(arg, env, supply, subst)?;
        arg_types.push(subst.apply(&t));
    }

    let return_type = supply.fresh();
    let call_type = TcType::Arrow(arg_types, Box::new(return_type.clone()));

    let s = unify(&subst.apply(&func_type), &call_type).map_err(|e| {
        // Try to make a nice error message
        let func_str = match func {
            LispVal::Sym(name) => name.clone(),
            other => other.to_string(),
        };
        format!("in call ({} ...): {}", func_str, e)
    })?;
    *subst = s.compose(subst.clone());

    Ok(subst.apply(&return_type))
}

// ---------------------------------------------------------------------------
// Helpers
//---------------------------------------------------------------------------

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Whole-program type check pass (used by WASM compiler)
// ---------------------------------------------------------------------------

/// Run a lightweight type-check pass over all top-level define forms.
///
/// This catches:
/// - Undefined variables
/// - Arity mismatches in function calls
/// - Type mismatches (e.g., passing str where int expected)
/// - Heterogeneous list literals
///
/// For `(pure (define ...))` forms, the existing `check_pure_define` /
/// `check_pure_block` is used (which also validates annotations).
/// For bare `(define ...)` forms, we run inference-only checking — no
/// annotation required, but still catches undefined vars and arity errors.
///
/// `near` flag selects between the pure-only builtin env and the NEAR host
/// builtin env (which knows about `near/input`, `near/storage_write`, etc.)
///
/// Returns `Ok(())` if all forms type-check, or `Err(msg)` with the first error.
/// Promise single-use lint (design: docs/promise-single-use.md, Tier 1).
///
/// A promise handle is move-only by intent: `near/promise_then`,
/// `near/promise_return`, `near/promise_and` are CONSUMING positions. The
/// host does not invalidate a consumed promise — a second consumer
/// registers a second callback that ALSO fires (silent double-pay).
/// This lint makes that a compile error, defn-local, naming both lines.
///
/// NOT consuming: `near/promise_batch_action_*` — batch actions legally
/// accumulate on one handle (create -> action* -> then -> return is the
/// canonical ft2.lisp idiom).
///
/// Runs PRE-desugar on source-faithful shapes. Lines are resolved against
/// the raw source: the i-th walked occurrence of an op name corresponds to
/// the i-th token occurrence (pre-order walk == source order; the parser is
/// sequential). If the two counts ever disagree (quoted data, reader
/// macros), line labels are dropped rather than misreported.

const PROMISE_PRODUCERS: &[&str] = &[
    "near/promise_batch_create",
    "near/promise_create",
    "near/promise_and",
    "near/promise_then",
];

const PROMISE_CONSUMERS: &[&str] = &[
    "near/promise_then",
    "near/promise_return",
    "near/promise_and",
];

/// Abstract promise-flow value. Handles carry a unique origin id assigned at
/// their creating expression; tuples carry elementwise values; everything
/// else is opaque (top). Joins are all-equal-or-Opaque: ambiguity degrades
/// to Opaque, which records no flow — the near-mock trap stays the backstop.
/// Soundness: we only ever ADD detections for statically provable flows, so
/// no legal program can be falsely rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AVal {
    Handle(u32),
    Tuple(Vec<AVal>),
    Opaque,
}

/// Join a set of possible values: all-equal keeps the value, any divergence
/// collapses to Opaque.
fn join_vals(vs: &[AVal]) -> AVal {
    if vs.len() == 1 {
        return vs[0].clone();
    }
    if vs.iter().all(|v| *v == vs[0]) {
        return vs[0].clone();
    }
    AVal::Opaque
}

fn origin_ids(v: &AVal, out: &mut Vec<u32>) {
    match v {
        AVal::Handle(h) => out.push(*h),
        AVal::Tuple(vs) => {
            for v in vs {
                origin_ids(v, out);
            }
        }
        AVal::Opaque => {}
    }
}

/// One consuming use of a handle origin.
#[derive(Clone, Debug)]
struct PromiseUse {
    defn: String,
    /// source-level symbol text at the consuming position (display only)
    name: String,
    op: String,
    /// consuming-op occurrence within the program (single-pass source order)
    occurrence: usize,
    origin: u32,
}

/// A call to a known defn with computed argument values.
#[derive(Clone, Debug)]
struct CallEdge {
    caller: String,
    callee: String,
    vals: Vec<AVal>,
}

/// Per-defn consumption summary from one full program pass.
#[derive(Default)]
struct Summary {
    uses: Vec<PromiseUse>,
    calls: Vec<CallEdge>,
}

/// Origin registry: creating defn for each handle origin.
#[derive(Default)]
struct OriginInfo {
    creator: String,
}

/// Static promise-single-use gate (2026-10-08, v3 origin flow).
///
/// Model: every producer/consuming call mints a fresh handle origin. Each
/// pass evaluates expressions to AVals, records consuming uses per origin,
/// and records call edges carrying argument values (tuples flow
/// elementwise through car/cdr). A fixpoint joins caller-provided values
/// into callee param envs by re-passing the whole program until param
/// values stabilize; origin ids are round-stable (counter reset per round,
/// deterministic traversal), so the fixpoint terminates.
///
/// Per-origin transitive consumption is counted from the origin's creator
/// defn through onward edges, with memoization and an on-stack cycle guard
/// (self-recursive forwarding terminates; per-iteration re-consume of a
/// freshly minted handle stays the near-mock trap's job — statically
/// unbounded).
///
/// Rejections:
/// - local: 2+ consuming uses of one origin inside its creator defn
///   (byte-stable message shape)
/// - across calls: transitive total of one origin reaches 2+
///
/// Boundaries: params with divergent values from multiple call sites join
/// to Opaque; unknown callees, computed non-tuple expressions and shadowed
/// lambda params stay Opaque — no flow recorded, runtime trap backstops.
fn promise_op_lines(source: Option<&str>) -> std::collections::HashMap<&str, Vec<usize>> {
    // Token-scan the source: byte offset of every occurrence of each op name.
    // With no source text (exprs-only path) the map stays empty and every
    // label degrades to "line ?" - the rejection itself still fires.
    let mut op_lines: std::collections::HashMap<&str, Vec<usize>> =
        std::collections::HashMap::new();
    if let Some(source) = source {
        for op in PROMISE_CONSUMERS {
            let mut lines = Vec::new();
            let bytes = source.as_bytes();
            let pat = op.as_bytes();
            let mut i = 0;
            while i + pat.len() <= bytes.len() {
                if &bytes[i..i + pat.len()] == pat {
                    // token boundary: not surrounded by identifier chars
                    let before_ok = i == 0
                        || !bytes[i - 1].is_ascii_alphanumeric()
                            && bytes[i - 1] != b'/'
                            && bytes[i - 1] != b'-'
                            && bytes[i - 1] != b'_';
                    let after = bytes.get(i + pat.len()).copied().unwrap_or(b' ');
                    let after_ok = !after.is_ascii_alphanumeric()
                        && after != b'-'
                        && after != b'_'
                        && after != b'/';
                    if before_ok && after_ok {
                        let line = 1 + source[..i].bytes().filter(|b| *b == b'\n').count();
                        lines.push(line);
                        i += pat.len();
                        continue;
                    }
                }
                i += 1;
            }
            op_lines.insert(op, lines);
        }
    }
    op_lines
}

pub fn check_promise_single_use(source: Option<&str>, exprs: &[LispVal]) -> Result<(), String> {
    let op_lines = promise_op_lines(source);

    // Pre-pass: top-level function defines, for call-edge resolution.
    // Shape: (define (name p1 p2 ...) BODY...) — params are the header's
    // trailing siblings, not a nested list.
    let mut defns: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        if items.len() >= 3 {
                            let ps: Vec<String> = header[1..]
                                .iter()
                                .filter_map(|p| match p {
                                    LispVal::Sym(s) => Some(s.clone()),
                                    _ => None,
                                })
                                .collect();
                            defns.insert(name.clone(), ps);
                        }
                    }
                }
            }
        }
    }

    // One full program pass. Records uses and call edges per defn.
    // `param_vals` feeds joined caller values into param envs.
    fn pass(
        exprs: &[LispVal],
        defns: &std::collections::HashMap<String, Vec<String>>,
        param_vals: &std::collections::HashMap<(String, usize), Vec<AVal>>,
        global_env: &mut std::collections::HashMap<String, AVal>,
        occurrence: &mut std::collections::HashMap<String, usize>,
    ) -> (std::collections::HashMap<String, Summary>, std::collections::HashMap<u32, String>) {
        let mut summaries: std::collections::HashMap<String, Summary> =
            std::collections::HashMap::new();
        let mut creators: std::collections::HashMap<u32, String> =
            std::collections::HashMap::new();
        let mut next_origin: u32 = 0;
        // Occurrence indices are single-pass source order; reset per pass so
        // fixpoint rounds never inherit (and inflate) the previous round's
        // counters — stale indices rendered as `line ?`.
        occurrence.clear();

        // Unified walker: records uses/call edges AND returns the abstract
        // value of the expression. Every sub-expression is walked exactly
        // once (special forms return early), so occurrence indices follow
        // single-pass source order and no use can be skipped or duplicated.
        #[allow(clippy::too_many_arguments)]
        fn walkv(
            val: &LispVal,
            cur_defn: &str,
            env: &mut std::collections::HashMap<String, AVal>,
            global_env: &std::collections::HashMap<String, AVal>,
            summary: &mut Summary,
            occurrence: &mut std::collections::HashMap<String, usize>,
            next_origin: &mut u32,
            creators: &mut std::collections::HashMap<u32, String>,
            defns: &std::collections::HashMap<String, Vec<String>>,
        ) -> AVal {
            let list = match val {
                LispVal::List(items) if !items.is_empty() => items,
                LispVal::Sym(name) => {
                    return env
                        .get(name)
                        .or_else(|| global_env.get(name))
                        .cloned()
                        .unwrap_or(AVal::Opaque)
                }
                _ => return AVal::Opaque,
            };
            let head = match &list[0] {
                LispVal::Sym(s) => s.as_str(),
                _ => {
                    for it in &list[1..] {
                        walkv(it, cur_defn, env, global_env, summary, occurrence, next_origin, creators, defns);
                    }
                    return AVal::Opaque;
                }
            };

            // (let* / let BINDINGS BODY...) — sequential binding scope.
            if matches!(head, "let*" | "let") {
                let mut saved: Vec<(String, AVal)> = Vec::new();
                if let Some(LispVal::List(bindings)) = list.get(1) {
                    for b in bindings {
                        if let LispVal::List(bi) = b {
                            if bi.len() >= 2 {
                                // The binding VALUE is walked (recording any
                                // uses/edges inside it) AND yields the value.
                                let v = walkv(
                                    &bi[1], cur_defn, env, global_env, summary, occurrence,
                                    next_origin, creators, defns,
                                );
                                if let LispVal::Sym(name) = &bi[0] {
                                    let prev =
                                        env.insert(name.clone(), v).unwrap_or(AVal::Opaque);
                                    saved.push((name.clone(), prev));
                                }
                            }
                        }
                    }
                }
                let mut last = AVal::Opaque;
                for it in &list[2..] {
                    last = walkv(
                        it, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    );
                }
                for (n, v) in saved {
                    env.insert(n, v);
                }
                return last;
            }

            // (lambda PARAMS BODY...) / (fn ...) — params shadow env.
            if matches!(head, "lambda" | "fn") {
                let mut saved: Vec<(String, AVal)> = Vec::new();
                if let Some(LispVal::List(params)) = list.get(1) {
                    for p in params {
                        if let LispVal::Sym(n) = p {
                            let prev =
                                env.insert(n.clone(), AVal::Opaque).unwrap_or(AVal::Opaque);
                            saved.push((n.clone(), prev));
                        }
                    }
                }
                for it in &list[2..] {
                    walkv(
                        it, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    );
                }
                for (n, v) in saved {
                    env.insert(n, v);
                }
                return AVal::Opaque;
            }

            // Consuming call: record uses of handle-valued consuming args,
            // walk every arg exactly once, result is a fresh handle.
            if PROMISE_CONSUMERS.contains(&head) {
                let idx = *occurrence.get(head).unwrap_or(&0);
                occurrence.insert(head.to_string(), idx + 1);
                // Indices are relative to list[1..] (arg 0 = the promise).
                let consume_args: &[usize] = match head {
                    "near/promise_then" => &[0],
                    "near/promise_return" => &[0],
                    "near/promise_and" => &[0, 1, 2, 3, 4, 5, 6, 7],
                    _ => &[],
                };
                for (ai, arg) in list[1..].iter().enumerate() {
                    let v = walkv(
                        arg, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    );
                    if consume_args.contains(&ai) {
                        if let AVal::Handle(id) = v {
                            // Creator attribution happens ONLY at minting
                            // sites below. A consumer whose defn textually
                            // precedes its producer must not steal it, or
                            // per-origin totals root at the wrong defn.
                            let name = match arg {
                                LispVal::Sym(n) => n.clone(),
                                _ => String::new(),
                            };
                            summary.uses.push(PromiseUse {
                                defn: cur_defn.to_string(),
                                name,
                                op: head.to_string(),
                                occurrence: idx,
                                origin: id,
                            });
                        }
                    }
                }
                let id = *next_origin;
                *next_origin += 1;
                creators.entry(id).or_insert_with(|| cur_defn.to_string());
                return AVal::Handle(id);
            }

            // Known-defn call: record the edge with walked argument values.
            if defns.contains_key(head) {
                let mut vals = Vec::new();
                for arg in &list[1..] {
                    vals.push(walkv(
                        arg, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    ));
                }
                summary.calls.push(CallEdge {
                    caller: cur_defn.to_string(),
                    callee: head.to_string(),
                    vals,
                });
                return AVal::Opaque;
            }

            // Producer call: walk args (recording), result is a fresh handle.
            if PROMISE_PRODUCERS.contains(&head) {
                for arg in &list[1..] {
                    walkv(
                        arg, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    );
                }
                let id = *next_origin;
                *next_origin += 1;
                creators.entry(id).or_insert_with(|| cur_defn.to_string());
                return AVal::Handle(id);
            }

            // (list ...) — tuple of walked element values.
            if head == "list" {
                let mut vs = Vec::new();
                for arg in &list[1..] {
                    vs.push(walkv(
                        arg, cur_defn, env, global_env, summary, occurrence, next_origin,
                        creators, defns,
                    ));
                }
                return AVal::Tuple(vs);
            }

            // (car X)/(first X), (cdr X)/(rest X) — peel walked tuple values.
            if (head == "car" || head == "first" || head == "cdr" || head == "rest")
                && list.len() == 2
            {
                let v = walkv(
                    &list[1], cur_defn, env, global_env, summary, occurrence, next_origin,
                    creators, defns,
                );
                return match v {
                    AVal::Tuple(vs) => {
                        if head == "car" || head == "first" {
                            vs.first().cloned().unwrap_or(AVal::Opaque)
                        } else if vs.len() <= 1 {
                            AVal::Opaque
                        } else {
                            AVal::Tuple(vs[1..].to_vec())
                        }
                    }
                    _ => AVal::Opaque,
                };
            }

            // Any other call/form: walk children (recording nested uses),
            // value is opaque.
            for it in &list[1..] {
                walkv(
                    it, cur_defn, env, global_env, summary, occurrence, next_origin, creators,
                    defns,
                );
            }
            AVal::Opaque
        }

        for e in exprs {
            if let LispVal::List(items) = e {
                if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                    // (define (name p1 ...) BODY...) — function defn; params
                    // start at their joined value (Opaque on early rounds).
                    if let Some(LispVal::List(header)) = items.get(1) {
                        if let Some(LispVal::Sym(name)) = header.first() {
                            if items.len() >= 3 {
                                let mut env: std::collections::HashMap<String, AVal> =
                                    std::collections::HashMap::new();
                                for (pos, p) in header[1..].iter().enumerate() {
                                    if let LispVal::Sym(n) = p {
                                        let joined = param_vals
                                            .get(&(name.clone(), pos))
                                            .map(|v| join_vals(v))
                                            .unwrap_or(AVal::Opaque);
                                        env.insert(n.clone(), joined);
                                    }
                                }
                                let sum = summaries.entry(name.clone()).or_default();
                                for body in &items[2..] {
                                    walkv(
                                        body,
                                        name,
                                        &mut env,
                                        global_env,
                                        sum,
                                        occurrence,
                                        &mut next_origin,
                                        &mut creators,
                                        defns,
                                    );
                                }
                                continue;
                            }
                        }
                    }
                    // (define name value) — global value binding.
                    if !matches!(items.get(1), Some(LispVal::List(_))) {
                        if let (Some(LispVal::Sym(n)), Some(v)) = (items.get(1), items.get(2)) {
                            let mut env0: std::collections::HashMap<String, AVal> =
                                std::collections::HashMap::new();
                            let mut sink = Summary::default();
                            let av = walkv(
                                v,
                                "",
                                &mut env0,
                                global_env,
                                &mut sink,
                                occurrence,
                                &mut next_origin,
                                &mut creators,
                                defns,
                            );
                            global_env.insert(n.clone(), av);
                            continue;
                        }
                    }
                }
            }
            // Top-level non-define expression.
            let mut env: std::collections::HashMap<String, AVal> =
                std::collections::HashMap::new();
            let sum = summaries.entry(String::new()).or_default();
            walkv(
                e,
                "",
                &mut env,
                global_env,
                sum,
                occurrence,
                &mut next_origin,
                &mut creators,
                defns,
            );
        }
        (summaries, creators)
    }

    // Fixpoint: re-pass the whole program until param values stabilize.
    // Origin ids are round-stable (counter reset per pass, deterministic
    // traversal), so identity is preserved across rounds.
    let mut param_vals: std::collections::HashMap<(String, usize), Vec<AVal>> =
        std::collections::HashMap::new();
    let mut global_env: std::collections::HashMap<String, AVal> =
        std::collections::HashMap::new();
    let mut occurrence: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let (mut summaries, mut creators) =
        pass(exprs, &defns, &param_vals, &mut global_env, &mut occurrence);
    for _round in 0..100 {
        let mut next: std::collections::HashMap<(String, usize), Vec<AVal>> =
            std::collections::HashMap::new();
        for sum in summaries.values() {
            for c in &sum.calls {
                for (pos, v) in c.vals.iter().enumerate() {
                    if *v == AVal::Opaque {
                        continue;
                    }
                    let slot = next.entry((c.callee.clone(), pos)).or_default();
                    if !slot.contains(v) {
                        slot.push(v.clone());
                    }
                }
            }
        }
        if next == param_vals {
            break;
        }
        param_vals = next;
        let (s2, c2) = pass(exprs, &defns, &param_vals, &mut global_env, &mut occurrence);
        summaries = s2;
        creators = c2;
    }

    // ---- per-origin transitive consumption (final summaries) ----
    let mut body_uses: std::collections::HashMap<(String, u32), Vec<PromiseUse>> =
        std::collections::HashMap::new();
    let mut uses_of: std::collections::HashMap<u32, Vec<PromiseUse>> =
        std::collections::HashMap::new();
    for sum in summaries.values() {
        for u in &sum.uses {
            body_uses
                .entry((u.defn.clone(), u.origin))
                .or_default()
                .push(u.clone());
            uses_of.entry(u.origin).or_default().push(u.clone());
        }
    }
    // onward[(defn, origin)] = edges (callee, pos) leaving that defn with
    // that origin flowing into the callee's param slot.
    let mut onward: std::collections::HashMap<(String, u32), Vec<(String, usize)>> =
        std::collections::HashMap::new();
    for sum in summaries.values() {
        for c in &sum.calls {
            for (pos, v) in c.vals.iter().enumerate() {
                let mut ids = Vec::new();
                origin_ids(v, &mut ids);
                for id in ids {
                    onward
                        .entry((c.caller.clone(), id))
                        .or_default()
                        .push((c.callee.clone(), pos));
                }
            }
        }
    }

    // G(defn, pos, origin): consumption of `origin` entering through that
    // param = body uses in that defn + onward edges. Memoized; on-stack
    // keys contribute 0 (cycle guard).
    fn g_count(
        key: (String, usize, u32),
        defns: &std::collections::HashMap<String, Vec<String>>,
        body_uses: &std::collections::HashMap<(String, u32), Vec<PromiseUse>>,
        onward: &std::collections::HashMap<(String, u32), Vec<(String, usize)>>,
        memo: &mut std::collections::HashMap<(String, usize, u32), usize>,
        stack: &mut Vec<(String, usize, u32)>,
    ) -> usize {
        if let Some(n) = memo.get(&key) {
            return *n;
        }
        if stack.contains(&key) {
            return 0;
        }
        stack.push(key.clone());
        let (defn, _pos, origin) = key.clone();
        let mut total = body_uses
            .get(&(defn.clone(), origin))
            .map(|v| v.len())
            .unwrap_or(0);
        if let Some(edges) = onward.get(&(defn.clone(), origin)) {
            for (callee, pos) in edges {
                if let Some(ps) = defns.get(callee) {
                    if *pos < ps.len() {
                        total += g_count(
                            (callee.clone(), *pos, origin),
                            defns,
                            body_uses,
                            onward,
                            memo,
                            stack,
                        );
                    }
                }
            }
        }
        stack.pop();
        memo.insert(key, total);
        total
    }

    // total(origin) = creator-body uses + creator onward edges' G.
    let mut memo: std::collections::HashMap<(String, usize, u32), usize> =
        std::collections::HashMap::new();
    let mut totals: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut sorted_origins: Vec<u32> = creators.keys().copied().collect();
    sorted_origins.sort();
    for id in &sorted_origins {
        let creator = creators.get(id).cloned().unwrap_or_default();
        let mut total = body_uses
            .get(&(creator.clone(), *id))
            .map(|v| v.len())
            .unwrap_or(0);
        if let Some(edges) = onward.get(&(creator.clone(), *id)) {
            for (callee, pos) in edges {
                if let Some(ps) = defns.get(callee) {
                    if *pos < ps.len() {
                        let mut stack: Vec<(String, usize, u32)> = Vec::new();
                        total += g_count(
                            (callee.clone(), *pos, *id),
                            &defns,
                            &body_uses,
                            &onward,
                            &mut memo,
                            &mut stack,
                        );
                    }
                }
            }
        }
        totals.insert(*id, total);
    }

    let mut errs: Vec<String> = Vec::new();
    let mut flagged: std::collections::HashSet<u32> = Default::default();

    // Bucket 1 (byte-stable): 2+ uses of one origin inside its creator
    // defn, sites rendered without defn clauses in source order.
    let mut b1_ids: Vec<u32> = Vec::new();
    for id in &sorted_origins {
        let creator = creators.get(id).cloned().unwrap_or_default();
        if body_uses
            .get(&(creator.clone(), *id))
            .map(|v| v.len())
            .unwrap_or(0)
            >= 2
        {
            b1_ids.push(*id);
        }
    }
    for id in b1_ids {
        flagged.insert(id);
        let creator = creators.get(&id).cloned().unwrap_or_default();
        let uses = &body_uses[&(creator.clone(), id)];
        let name = uses
            .iter()
            .map(|u| u.name.clone())
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| format!("promise#{}", id));
        let sites: Vec<String> = uses
            .iter()
            .map(|u| {
                format!(
                    "{} ({})",
                    u.op.trim_start_matches("near/"),
                    label_of_occ(&op_lines, u.op.as_str(), u.occurrence)
                )
            })
            .collect();
        errs.push(format!(
            "error: promise '{}' consumed {} times ({}) — a promise handle is single-use; both callbacks would fire on chain",
            name,
            uses.len(),
            sites.join(", ")
        ));
    }

    // Bucket 2: transitive total of one origin reaches 2+ (across calls).
    for id in &sorted_origins {
        if flagged.contains(id) {
            continue;
        }
        let total = totals.get(id).copied().unwrap_or(0);
        if total < 2 {
            continue;
        }
        let all_uses = uses_of.get(id).cloned().unwrap_or_default();
        if all_uses.is_empty() {
            continue;
        }
        let name = all_uses
            .iter()
            .map(|u| u.name.clone())
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| format!("promise#{}", id));
        let sites = render_sites_occ(all_uses, &op_lines);
        errs.push(format!(
            "error: promise '{}' consumed {} times across calls ({}) — a promise handle is single-use; both callbacks would fire on chain",
            name,
            total,
            sites.join(", ")
        ));
    }

    errs.sort();
    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs.join("\n"))
    }
}

/// Line label for one consuming-op occurrence.
// ===== resource flow gate (arity / gas / money domain / deposit) =====
// House invariants: checker-only, Err not panic, no new host fns, and the
// soundness rule — hard reject ONLY what is provably wrong; everything
// model-dependent degrades to a warning (eprintln, same precedent as the
// cond-without-else warning) or silence.
//
// Slot maps transcribed from src/wasm_emit/call_near_promise.rs emitter
// arms (a[i] indexes = list[i+1] in checker coords). `near/call` sugar has
// gas at emitter slot 3 / deposit at 4 — the REVERSE of near/promise_then.
//
// Gas units are source literals (10000000000000 = 10 Tgas).

/// Conservative callback floor model (gas units). Documented as a MODEL,
/// never a proof: fires as a warning, never a hard error.
const CB_FLOOR_BASE: i128 = 1_000_000_000_000; // 1 Tgas: receive + exec any receipt
const CB_FLOOR_PER_HOST: i128 = 10_000_000_000; // 10 Ggas per host op (>= realistic mins)
/// Mainnet per-transaction prepaid gas cap.
const GAS_TX_CAP: i128 = 300_000_000_000_000; // 300 Tgas
const U128_MAX_DEC: &str = "340282366920938463463374607431768211455";

/// (expected_arity, gas_slot, amount_slot) in emitter coords.
fn resource_op_info(op: &str) -> (Option<usize>, Option<usize>, Option<usize>) {
    match op {
        "near/promise_then" => (Some(6), Some(5), Some(4)),
        "near/promise_create" => (Some(5), Some(4), Some(3)),
        "near/promise_batch_create" => (Some(1), None, None),
        "near/promise_batch_then" => (Some(2), None, None),
        "near/promise_batch_action_function_call" => (Some(5), Some(4), Some(3)),
        "near/promise_batch_action_function_call_weight" => (None, Some(4), Some(3)),
        "near/promise_batch_action_transfer" => (Some(2), None, Some(1)),
        "near/transfer" => (Some(2), None, Some(1)),
        "near/transfer_u128" => (Some(2), None, Some(1)),
        "near/call" => (Some(5), Some(3), Some(4)),
        _ => (None, None, None),
    }
}

/// u128-domain producers (string-decimal money domain, helpers.rs).
fn is_u128_producer(op: &str) -> bool {
    matches!(
        op,
        "u128/add"
            | "u128/sub"
            | "u128/mul"
            | "u128/muldiv"
            | "u128/div"
            | "u128/mod"
            | "u128/from-i64"
            | "u128/from_i64"
            | "u128/from_yocto"
            | "u128/from_str"
            | "u128/new"
            | "u128/store"
    )
}

/// u128 -> i64 truncators: result leaves the money domain.
fn is_u128_truncator(op: &str) -> bool {
    matches!(op, "u128/to-i64" | "u128/to_i64" | "u128/checked_to_i64" | "u128/fit_i64")
}

fn str_as_u128(s: &str) -> bool {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let t = s.trim_start_matches('0');
    let max = U128_MAX_DEC.trim_start_matches('0');
    t.len() < max.len() || (t.len() == max.len() && t <= max)
}

/// Abstract money domain.
#[derive(Clone, PartialEq, Debug)]
enum MVal {
    /// Provable u128-domain value (decimal literal, u128 family, deposit).
    U128,
    /// One specific `attached_deposit` call site (identity for the
    /// double-attach spend check).
    Deposit(u32),
    /// u128 truncated to i64 — legal i64, corruption at a money sink.
    TruncI64,
    /// The contract's own account id — (near/current_account_id).
    SelfAcct,
    /// Anything else. No opinion.
    Opaque,
}

fn mjoin(a: MVal, b: MVal) -> MVal {
    if a == b {
        a
    } else {
        MVal::Opaque
    }
}

/// Top-level function defines: name -> param count.
fn collect_top_defns(exprs: &[LispVal]) -> std::collections::HashMap<String, usize> {
    let mut defns = std::collections::HashMap::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        defns.insert(name.clone(), header.len().saturating_sub(1));
                    }
                }
            }
        }
    }
    defns
}

/// Defn bodies (top-level function defines only).
fn collect_bodies(exprs: &[LispVal]) -> std::collections::HashMap<String, Vec<LispVal>> {
    let mut bodies = std::collections::HashMap::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        if header.len() >= 2 && items.len() >= 3 {
                            bodies.insert(name.clone(), items[2..].to_vec());
                        }
                    }
                }
            }
        }
    }
    bodies
}

fn count_hosts(val: &LispVal, acc: &mut i128) {
    if let LispVal::List(items) = val {
        if let Some(LispVal::Sym(h)) = items.first() {
            if h.starts_with("near/") || h.starts_with("u128/") {
                *acc += 1;
            }
        }
        let start = if matches!(items.first(), Some(LispVal::Sym(_))) { 1 } else { 0 };
        for it in &items[start..] {
            count_hosts(it, acc);
        }
    }
}

fn collect_callees(val: &LispVal, defns: &std::collections::HashMap<String, usize>, out: &mut std::collections::HashSet<String>) {
    if let LispVal::List(items) = val {
        if let Some(LispVal::Sym(h)) = items.first() {
            if defns.contains_key(h) {
                out.insert(h.clone());
            }
        }
        let start = if matches!(items.first(), Some(LispVal::Sym(_))) { 1 } else { 0 };
        for it in &items[start..] {
            collect_callees(it, defns, out);
        }
    }
}

/// Static gas-literal floor of a defn closure: sum of attached-gas literals
/// + transitive callees' sums, each defn counted once (cycle-safe). This is
/// a LOWER bound on the happy-path attach total — every listed site must
/// execute at least once for the method to do its job.
fn gas_floor_of(
    defn: &str,
    bodies: &std::collections::HashMap<String, Vec<LispVal>>,
    defns: &std::collections::HashMap<String, usize>,
    memo: &mut std::collections::HashMap<String, i128>,
    stack: &mut Vec<String>,
) -> i128 {
    if let Some(v) = memo.get(defn) {
        return *v;
    }
    if stack.iter().any(|s| s == defn) {
        return 0;
    }
    stack.push(defn.to_string());
    let mut total = 0i128;
    if let Some(bs) = bodies.get(defn) {
        for b in bs {
            count_gas_literals(b, &mut total);
        }
        let mut callees = std::collections::HashSet::new();
        for b in bs {
            collect_callees(b, defns, &mut callees);
        }
        for c in callees {
            total += gas_floor_of(&c, bodies, defns, memo, stack);
        }
    }
    stack.pop();
    memo.insert(defn.to_string(), total);
    total
}

/// Sum attached-gas literals inside an expression tree.
fn count_gas_literals(val: &LispVal, acc: &mut i128) {
    if let LispVal::List(items) = val {
        if let Some(LispVal::Sym(h)) = items.first() {
            let (_, gas_slot, _) = resource_op_info(h);
            if let Some(g) = gas_slot {
                if let Some(LispVal::Num(n)) = items.get(g + 1) {
                    if *n > 0 {
                        *acc += *n as i128;
                    }
                }
            }
        }
        let start = if matches!(items.first(), Some(LispVal::Sym(_))) { 1 } else { 0 };
        for it in &items[start..] {
            count_gas_literals(it, acc);
        }
    }
}

/// Static host-op floor of a defn closure (each defn counted once).
fn hosts_closure_of(
    defn: &str,
    bodies: &std::collections::HashMap<String, Vec<LispVal>>,
    defns: &std::collections::HashMap<String, usize>,
    memo: &mut std::collections::HashMap<String, i128>,
    stack: &mut Vec<String>,
) -> i128 {
    if let Some(v) = memo.get(defn) {
        return *v;
    }
    if stack.iter().any(|s| s == defn) {
        return 0;
    }
    stack.push(defn.to_string());
    let mut hosts = 0i128;
    if let Some(bs) = bodies.get(defn) {
        for b in bs {
            count_hosts(b, &mut hosts);
        }
        let mut callees = std::collections::HashSet::new();
        for b in bs {
            collect_callees(b, defns, &mut callees);
        }
        for c in callees {
            hosts += hosts_closure_of(&c, bodies, defns, memo, stack);
        }
    }
    stack.pop();
    memo.insert(defn.to_string(), hosts);
    hosts
}

fn fmt_tgas(g: i128) -> String {
    if g % 1_000_000_000_000 == 0 {
        format!("{} Tgas", g / 1_000_000_000_000)
    } else {
        format!("{} gas", g)
    }
}

/// Line label reusing the promise gate's scanner (consuming ops only;
/// other ops degrade to `line ?` — same convention as the exprs path).
fn res_line(
    op_lines: &std::collections::HashMap<&str, Vec<usize>>,
    op: &str,
    nth: usize,
) -> String {
    match op_lines.get(op).and_then(|l| l.get(nth)) {
        Some(l) => format!("line {}", l),
        None => "line ?".to_string(),
    }
}

/// Shared context for the check walker.
struct ResCx<'a> {
    defns: &'a std::collections::HashMap<String, usize>,
    bodies: &'a std::collections::HashMap<String, Vec<LispVal>>,
    param_u128: &'a std::collections::HashMap<(String, usize), MVal>,
    op_lines: &'a std::collections::HashMap<&'a str, Vec<usize>>,
    seen: std::collections::HashMap<String, usize>,
    errs: Vec<String>,
    warns: Vec<String>,
    /// attached_deposit identity -> number of attach sites it reached.
    deposit_sites: std::collections::HashMap<u32, usize>,
    next_deposit: u32,
    /// storage money stamps: key literal -> () — a key written with a
    /// provable u128 value is a money key; reads of it seed U128.
    money_keys: std::collections::HashSet<String>,
}

/// Pass 1: compute per-(defn, pos) param provenance (U128 or not) by walking
/// each defn body with unknown params as Opaque and recording arg values at
/// call sites; joined all-equal-or-Opaque across sites. Cross-defn recursion
/// is one level deep (v1 boundary — documented): param provenance reflects
/// DIRECT call sites only.
fn seed_pass(
    exprs: &[LispVal],
    defns: &std::collections::HashMap<String, usize>,
) -> std::collections::HashMap<(String, usize), MVal> {
    fn swalk(
        val: &LispVal,
        env: &mut std::collections::HashMap<String, MVal>,
        global_env: &std::collections::HashMap<String, MVal>,
        param_u128: &mut std::collections::HashMap<(String, usize), MVal>,
        defns: &std::collections::HashMap<String, usize>,
    ) -> MVal {
        let list = match val {
            LispVal::List(items) if !items.is_empty() => items,
            LispVal::Sym(n) => return env
                .get(n)
                .or_else(|| global_env.get(n))
                .cloned()
                .unwrap_or(MVal::Opaque),
            LispVal::Str(s) => {
                return if str_as_u128(s) {
                    MVal::U128
                } else {
                    MVal::Opaque
                };
            }
            _ => return MVal::Opaque,
        };
        let head = match &list[0] {
            LispVal::Sym(s) => s.as_str(),
            _ => return MVal::Opaque,
        };
        if matches!(head, "let*" | "let") {
            let mut saved: Vec<(String, MVal)> = Vec::new();
            if let Some(LispVal::List(bindings)) = list.get(1) {
                for b in bindings {
                    if let LispVal::List(bi) = b {
                        if bi.len() >= 2 {
                            let v = swalk(&bi[1], env, global_env, param_u128, defns);
                            if let LispVal::Sym(n) = &bi[0] {
                                let prev =
                                    env.insert(n.clone(), v).unwrap_or(MVal::Opaque);
                                saved.push((n.clone(), prev));
                            }
                        }
                    }
                }
            }
            let mut last = MVal::Opaque;
            for it in &list[2..] {
                last = swalk(it, env, global_env, param_u128, defns);
            }
            for (n, v) in saved {
                env.insert(n, v);
            }
            return last;
        }
        if matches!(head, "+" | "-" | "*" | "/" | "%" | "mod") {
            let mut ret = MVal::Opaque;
            for a in &list[1..] {
                let v = swalk(a, env, global_env, param_u128, defns);
                ret = if ret == MVal::Opaque { v } else { mjoin(ret, v) };
            }
            return ret;
        }
        if is_u128_producer(head) {
            for a in &list[1..] {
                swalk(a, env, global_env, param_u128, defns);
            }
            return MVal::U128;
        }
        if is_u128_truncator(head) {
            for a in &list[1..] {
                swalk(a, env, global_env, param_u128, defns);
            }
            return MVal::TruncI64;
        }
        if head == "near/attached_deposit" || head == "near/current_account_id" {
            return if head == "near/attached_deposit" {
                MVal::U128
            } else {
                MVal::SelfAcct
            };
        }
        if defns.contains_key(head) {
            for (pos, a) in list[1..].iter().enumerate() {
                let v = swalk(a, env, global_env, param_u128, defns);
                if let Some(p) = defns.get(head) {
                    if pos < *p {
                        let key = (head.to_string(), pos);
                        let joined = match param_u128.get(&key) {
                            None => v,
                            Some(prev) => mjoin(prev.clone(), v),
                        };
                        param_u128.insert(key, joined);
                    }
                }
            }
            return MVal::Opaque;
        }
        let mut ret = MVal::Opaque;
        for a in &list[1..] {
            let v = swalk(a, env, global_env, param_u128, defns);
            if v != MVal::Opaque {
                ret = mjoin(ret, v);
            }
        }
        ret
    }

    let mut param_u128: std::collections::HashMap<(String, usize), MVal> =
        std::collections::HashMap::new();
    let mut global_env: std::collections::HashMap<String, MVal> =
        std::collections::HashMap::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        if header.len() == 1 {
                            // (define name value)
                            if items.len() >= 3 {
                                let mut env = std::collections::HashMap::new();
                                let v = swalk(
                                    &items[2],
                                    &mut env,
                                    &global_env,
                                    &mut param_u128,
                                    defns,
                                );
                                global_env.insert(name.clone(), v);
                            }
                        } else {
                            // function define: walk the whole body
                            let mut env = std::collections::HashMap::new();
                            for b in &items[2..] {
                                swalk(b, &mut env, &global_env, &mut param_u128, defns);
                            }
                        }
                        continue;
                    }
                }
            }
            // top-level non-define
            let mut env = std::collections::HashMap::new();
            swalk(e, &mut env, &global_env, &mut param_u128, defns);
        }
    }
    param_u128
}

/// Pass 2 walker: enforces the provable checks, collects model-dependent
/// warnings. `cur_defn` is "" at top level.
fn cwalk(
    val: &LispVal,
    env: &mut std::collections::HashMap<String, MVal>,
    global_env: &std::collections::HashMap<String, MVal>,
    cx: &mut ResCx,
    cur_defn: &str,
) -> MVal {
    let list = match val {
        LispVal::List(items) if !items.is_empty() => items,
        LispVal::Sym(n) => {
            return env
                .get(n)
                .or_else(|| global_env.get(n))
                .cloned()
                .unwrap_or(MVal::Opaque)
        }
        LispVal::Str(s) => {
            return if str_as_u128(s) {
                MVal::U128
            } else {
                MVal::Opaque
            };
        }
        _ => return MVal::Opaque,
    };
    let head = match &list[0] {
        LispVal::Sym(s) => s.as_str(),
        _ => {
            for it in list.iter() {
                cwalk(it, env, global_env, cx, cur_defn);
            }
            return MVal::Opaque;
        }
    };

    // (let* / let BINDINGS BODY...)
    if matches!(head, "let*" | "let") {
        let mut saved: Vec<(String, MVal)> = Vec::new();
        if let Some(LispVal::List(bindings)) = list.get(1) {
            for b in bindings {
                if let LispVal::List(bi) = b {
                    if bi.len() >= 2 {
                        let v = cwalk(&bi[1], env, global_env, cx, cur_defn);
                        if let LispVal::Sym(n) = &bi[0] {
                            let prev = env.insert(n.clone(), v).unwrap_or(MVal::Opaque);
                            saved.push((n.clone(), prev));
                        }
                    }
                }
            }
        }
        let mut last = MVal::Opaque;
        for it in &list[2..] {
            last = cwalk(it, env, global_env, cx, cur_defn);
        }
        for (n, v) in saved {
            env.insert(n, v);
        }
        return last;
    }

    // Raw arithmetic on a provable u128 — the TS money-taint rule, Lisp side.
    if matches!(head, "+" | "-" | "*" | "/" | "%" | "mod") {
        let mut any_u128 = false;
        let mut ret = MVal::Opaque;
        for a in &list[1..] {
            let v = cwalk(a, env, global_env, cx, cur_defn);
            if v == MVal::U128 {
                any_u128 = true;
            }
            ret = if ret == MVal::Opaque { v } else { mjoin(ret, v) };
        }
        if any_u128 {
            cx.errs.push(format!(
                "error: raw arithmetic on a u128 money value corrupts its decimal string — use the u128 family, e.g. (u128/add a b) — {}",
                res_line(cx.op_lines, "__arith__", 0) // degrades to line ?
            ));
            return MVal::U128; // result stays in the (corrupt) domain for sinks
        }
        return ret;
    }

    if is_u128_producer(head) {
        for a in &list[1..] {
            cwalk(a, env, global_env, cx, cur_defn);
        }
        return MVal::U128;
    }
    if is_u128_truncator(head) {
        for a in &list[1..] {
            cwalk(a, env, global_env, cx, cur_defn);
        }
        return MVal::TruncI64;
    }
    if head == "near/attached_deposit" || head == "near/attached_deposit_u128" {
        // Each deposit-reader call site mints a distinct identity: two sites
        // = two independent reads (legal). One identity flowing into 2+
        // attach slots = provable double-spend of the attached deposit.
        // (Checker typings: attached_deposit : () -> int,
        // attached_deposit_u128 : () -> str — the u128-domain reader.)
        let id = cx.next_deposit;
        cx.next_deposit += 1;
        return MVal::Deposit(id);
    }
    if head == "near/current_account_id" {
        return MVal::SelfAcct;
    }

    // resource-constrained ops
    let (want_arity, gas_slot, amt_slot) = resource_op_info(head);
    if want_arity.is_some() || gas_slot.is_some() || amt_slot.is_some() {
        let idx = cx.seen.entry(head.to_string()).or_insert(0);
        let this_idx = *idx;
        *idx += 1;
        if let Some(w) = want_arity {
            if list.len() - 1 != w {
                cx.errs.push(format!(
                    "error: {} takes {} argument{}, got {} — {}",
                    head,
                    w,
                    if w == 1 { "" } else { "s" },
                    list.len() - 1,
                    res_line(cx.op_lines, head, this_idx)
                ));
                for a in &list[1..] {
                    cwalk(a, env, global_env, cx, cur_defn);
                }
                return MVal::Opaque;
            }
        }
        // gas slot: literal sanity
        if let Some(g) = gas_slot {
            if let Some(arg) = list.get(g + 1) {
                if let LispVal::Num(n) = arg {
                    let gv = *n as i128;
                    if gv <= 0 {
                        cx.warns.push(format!(
                            "warning: {} attaches {} gas — a zero/negative gas receipt cannot execute (the parent tx still commits) — {}",
                            head,
                            gv,
                            res_line(cx.op_lines, head, this_idx)
                        ));
                    } else if gv > GAS_TX_CAP {
                        cx.errs.push(format!(
                            "error: {} attaches {} — over the {} per-transaction limit — {}",
                            head,
                            fmt_tgas(gv),
                            fmt_tgas(GAS_TX_CAP),
                            res_line(cx.op_lines, head, this_idx)
                        ));
                    }
                }
            }
        }
        // amount slot: money domain
        if let Some(m) = amt_slot {
            if let Some(arg) = list.get(m + 1) {
                let v = cwalk(arg, env, global_env, cx, cur_defn);
                if v == MVal::TruncI64 {
                    cx.errs.push(format!(
                        "error: {} amount was truncated to i64 (u128/to-i64) — yocto-scale values overflow i64; keep the u128 domain — {}",
                        head,
                        res_line(cx.op_lines, head, this_idx)
                    ));
                }
                if let MVal::Deposit(id) = v {
                    let n = cx.deposit_sites.entry(id).or_insert(0);
                    *n += 1;
                    if *n == 2 {
                        cx.errs.push(format!(
                            "error: the value of one (near/attached_deposit) is attached to two promise operations — the second attach spends funds the tx does not have — {}",
                            res_line(cx.op_lines, head, this_idx)
                        ));
                    }
                }
                match arg {
                    LispVal::Str(s) if !s.is_empty() && !str_as_u128(s) => {
                        cx.errs.push(format!(
                            "error: {} amount '{}' is not a decimal string — the u128 parse traps at runtime — {}",
                            head,
                            s,
                            res_line(cx.op_lines, head, this_idx)
                        ));
                    }
                    LispVal::Sym(_) if v == MVal::Opaque => {
                        // unknown ident at a money slot: no opinion (could be
                        // a param seeded U128 via another path — pass1 handles
                        // what is provable)
                    }
                    _ => {}
                }
            }
        }
    }

    // storage money stamps: (storage_write "key" <u128>) marks "key";
    // reads of a stamped literal key seed U128. Sound direction: only
    // literal keys are tracked (computed keys degrade to no opinion).
    if (head == "near/storage_write" || head == "near/storage_set") && list.len() == 3 {
        let v = cwalk(&list[2], env, global_env, cx, cur_defn);
        if let (LispVal::Str(k), true) = (&list[1], matches!(v, MVal::U128)) {
            cx.money_keys.insert(k.clone());
        }
        return MVal::Opaque;
    }
    if (head == "near/storage_get" || head == "near/storage_read") && list.len() == 2 {
        if let LispVal::Str(k) = &list[1] {
            if cx.money_keys.contains(k) {
                return MVal::U128;
            }
        }
        let v = cwalk(&list[1], env, global_env, cx, cur_defn);
        return match v {
            MVal::U128 => MVal::U128, // computed key proven to be money
            _ => MVal::Opaque,
        };
    }

    // same-program callback: existence (warning) + floor vs attached (warning)
    if head == "near/promise_then" {
        // checker coords: list[1]=promise list[2]=account list[3]=method
        // list[4]=args list[5]=amount list[6]=gas
        let acct_v = match list.get(2) {
            Some(a) => cwalk(a, env, global_env, cx, cur_defn),
            None => MVal::Opaque,
        };
        if let (Some(LispVal::Str(mname)), true) = (list.get(3), acct_v == MVal::SelfAcct) {
            let this_idx = cx.seen.get(head).copied().unwrap_or(1).saturating_sub(1);
            if !cx.defns.contains_key(mname) {
                cx.warns.push(format!(
                    "warning: promise_then callback '{}' is not defined in this program — on mainnet that receipt dies with MethodNotFound (the parent tx still commits) — {}",
                    mname,
                    res_line(cx.op_lines, head, this_idx)
                ));
            } else {
                // floor model (warning only): receipt base + one minimum per
                // host op in the callback's transitive same-program closure.
                let mut memo = std::collections::HashMap::new();
                let mut stack = Vec::new();
                let hosts =
                    hosts_closure_of(mname, cx.bodies, cx.defns, &mut memo, &mut stack);
                let cb_floor = CB_FLOOR_BASE + hosts * CB_FLOOR_PER_HOST;
                if let Some(LispVal::Num(n)) = list.get(6) {
                    let attached = *n as i128;
                    if attached > 0 && cb_floor > attached {
                        cx.warns.push(format!(
                            "warning: callback '{}' static floor is ~{} (receipt base + {} host-op minimums) but only {} is attached — the callback may exhaust its gas — {}",
                            mname,
                            fmt_tgas(cb_floor),
                            hosts,
                            fmt_tgas(attached),
                            res_line(cx.op_lines, head, this_idx)
                        ));
                    }
                }
            }
        }
    }

    // defn call: walk args; provenance comes from the pass-1 param seeds
    if cx.defns.contains_key(head) {
        for a in &list[1..] {
            cwalk(a, env, global_env, cx, cur_defn);
        }
        return MVal::Opaque;
    }

    // generic form: walk children, join non-opaque results
    let mut ret = MVal::Opaque;
    for a in &list[1..] {
        let v = cwalk(a, env, global_env, cx, cur_defn);
        if v != MVal::Opaque {
            ret = mjoin(ret, v);
        }
    }
    ret
}

/// Resource flow gate. Runs on BOTH pipelines next to the promise
/// single-use gate. Provable violations -> Err; model-dependent findings
/// -> eprintln warnings.
pub fn check_resource_flow(source: Option<&str>, exprs: &[LispVal]) -> Result<(), String> {
    let op_lines = promise_op_lines(source);
    let defns = collect_top_defns(exprs);
    let bodies = collect_bodies(exprs);
    let param_u128 = seed_pass(exprs, &defns);

    let mut cx = ResCx {
        defns: &defns,
        bodies: &bodies,
        param_u128: &param_u128,
        op_lines: &op_lines,
        seen: std::collections::HashMap::new(),
        errs: Vec::new(),
        warns: Vec::new(),
        deposit_sites: std::collections::HashMap::new(),
        next_deposit: 0,
        money_keys: std::collections::HashSet::new(),
    };

    // walk each defn body with params seeded from pass-1 provenance
    for e in exprs {
        if let LispVal::List(items) = e {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        if header.len() > 1 {
                            let mut env: std::collections::HashMap<String, MVal> =
                                std::collections::HashMap::new();
                            let np = header.len() - 1;
                            for (pos, p) in header[1..].iter().enumerate() {
                                if pos < np {
                                    if let LispVal::Sym(n) = p {
                                        let seeded = param_u128
                                            .get(&(name.clone(), pos))
                                            .cloned()
                                            .unwrap_or(MVal::Opaque);
                                        env.insert(n.clone(), seeded);
                                    }
                                }
                            }
                            for b in &items[2..] {
                                cwalk(b, &mut env, &mut std::collections::HashMap::new(), &mut cx, name);
                            }
                        } else if items.len() >= 3 {
                            // (define name value)
                            let mut env = std::collections::HashMap::new();
                            let mut g: std::collections::HashMap<String, MVal> =
                                std::collections::HashMap::new();
                            cwalk(&items[2], &mut env, &mut g, &mut cx, name);
                        }
                        continue;
                    }
                }
            }
            // top-level non-define
            let mut env = std::collections::HashMap::new();
            cwalk(e, &mut env, &mut std::collections::HashMap::new(), &mut cx, "");
        }
    }

    // per-defn closure gas floor vs the tx cap (static lower bound)
    {
        let mut memo = std::collections::HashMap::new();
        let mut stack = Vec::new();
        for name in defns.keys() {
            let total = gas_floor_of(name, &bodies, &defns, &mut memo, &mut stack);
            if total > GAS_TX_CAP {
                cx.errs.push(format!(
                    "error: method '{}' attaches at least {} in static gas literals across its call closure — over the {} per-transaction limit",
                    name,
                    fmt_tgas(total),
                    fmt_tgas(GAS_TX_CAP)
                ));
            }
        }
    }

    for w in &cx.warns {
        eprintln!("{}", w);
    }
    if cx.errs.is_empty() {
        Ok(())
    } else {
        cx.errs.sort();
        Err(cx.errs.join("\n"))
    }
}


fn label_of_occ(
    op_lines: &std::collections::HashMap<&str, Vec<usize>>,
    op: &str,
    occurrence: usize,
) -> String {
    match op_lines.get(op).and_then(|l| l.get(occurrence)) {
        Some(l) => format!("line {l}"),
        None => "line ?".to_string(),
    }
}

/// Render sites as "op in defn (line N)"; defn omitted at top level so
/// single-defn messages stay in the established shape. Sorted + deduped.
fn render_sites_occ(
    mut sites: Vec<PromiseUse>,
    op_lines: &std::collections::HashMap<&str, Vec<usize>>,
) -> Vec<String> {
    sites.sort_by(|a, b| (&a.defn, &a.op, a.occurrence).cmp(&(&b.defn, &b.op, b.occurrence)));
    sites.dedup_by(|a, b| a.defn == b.defn && a.op == b.op && a.occurrence == b.occurrence);
    sites
        .iter()
        .map(|u| {
            let where_clause = if u.defn.is_empty() {
                String::new()
            } else {
                format!(" in {}", u.defn)
            };
            format!(
                "{}{} ({})",
                u.op.trim_start_matches("near/"),
                where_clause,
                label_of_occ(op_lines, u.op.as_str(), u.occurrence)
            )
        })
        .collect()
}


pub fn type_check_program(exprs: &[LispVal], near: bool) -> Result<(), String> {
    let mut env = if near {
        TcEnv::with_near_builtins()
    } else {
        TcEnv::with_pure_builtins()
    };
    let mut supply = VarSupply::new();

    // Pre-register every top-level function-define name as a fully
    // polymorphic placeholder so forward references (mutual recursion like
    // even?/odd?) type-check. The ordered pass below re-registers each name
    // with its precise inferred type, shadowing the placeholder.
    // Value defines (define name value) get the same treatment — a const
    // referenced by a function defined ABOVE it previously failed with
    // "undefined variable" (found compiling the PLONK verifier, where
    // ZERO_BE_HEX sits below its first consumer after function reordering).
    for expr in exprs {
        if let LispVal::List(items) = expr {
            if matches!(items.first(), Some(LispVal::Sym(s)) if s == "define") {
                if let Some(LispVal::List(header)) = items.get(1) {
                    if let Some(LispVal::Sym(name)) = header.first() {
                        env.insert(
                            name.clone(),
                            Scheme {
                                vars: vec![0],
                                ty: TcType::Var(0),
                            },
                        );
                    }
                }
                // Value define: (define NAME value) — NAME is a Sym at [1]
                if let Some(LispVal::Sym(name)) = items.get(1) {
                    env.insert(
                        name.clone(),
                        Scheme {
                            vars: vec![0],
                            ty: TcType::Var(0),
                        },
                    );
                }
            }
        }
    }

    for expr in exprs {
        let list = match expr {
            LispVal::List(l) => l,
            _ => continue, // skip non-lists (comments, atoms, etc.)
        };

        if list.is_empty() {
            continue;
        }

        match &list[0] {
            // (pure (define ...) ...) — use existing checker
            LispVal::Sym(s) if s == "pure" => {
                let define_forms: Vec<&LispVal> = list[1..]
                    .iter()
                    .filter(|f| {
                        if let LispVal::List(l) = f {
                            matches!(l.first(), Some(LispVal::Sym(s)) if s == "define")
                        } else {
                            false
                        }
                    })
                    .collect();

                if define_forms.is_empty() {
                    // Single form: (pure (define ...))
                    let results = crate::typing::check_pure_define(&list[1..])?;
                    env.insert_mono(results.name, results.inferred_type);
                } else {
                    // Block: (pure (define ...) (define ...))
                    let results = crate::typing::check_pure_block(&define_forms)?;
                    for r in results {
                        env.insert_mono(r.name, r.inferred_type);
                    }
                }
            }

            // (define (name params...) body) — inference-only check
            LispVal::Sym(s) if s == "define" && list.len() >= 3 => {
                match &list[1] {
                    // Function define
                    LispVal::List(sig) if !sig.is_empty() => {
                        if let LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..]
                                .iter()
                                .map(|v| match v {
                                    LispVal::Sym(s) => s.clone(),
                                    other => format!("_{}", other),
                                })
                                .collect();

                            // Build body (handle multi-body defines; skip `::` annotations)
                            let (ann_parts, body_items) =
                                crate::helpers::split_define_annotation(&list[2..]);
                            let annotated_type = match &ann_parts {
                                Some(parts) => {
                                    Some(parse_type_annotation(&LispVal::List(parts.clone()))?)
                                }
                                None => None,
                            };
                            let body = if body_items.len() > 1 {
                                LispVal::List(
                                    std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(body_items.iter().cloned())
                                        .collect(),
                                )
                            } else {
                                body_items.first().cloned().unwrap_or(LispVal::Nil)
                            };

                            // Annotation drives param types when present
                            if let Some(TcType::Arrow(args, _ret)) = &annotated_type {
                                if args.len() != params.len() {
                                    return Err(format!(
                                        "define {}: annotation has {} params, function has {}",
                                        name,
                                        args.len(),
                                        params.len()
                                    ));
                                }
                            } else if let Some(other) = &annotated_type {
                                return Err(format!(
                                    "define {}: expected arrow type annotation, got {}",
                                    name, other
                                ));
                            }

                            // Create fresh type vars for params
                            let mut check_env = env.clone();
                            let mut subst = Subst::new();
                            let ret_var = match &annotated_type {
                                Some(TcType::Arrow(_, ret)) => (**ret).clone(),
                                _ => supply.fresh(),
                            };
                            let any_ty = TcType::Con(TcCon::Any);
                            let mut param_types: Vec<TcType> = Vec::with_capacity(params.len());

                            // Detect ycomb-style self-call: if the first param name
                            // is "me" or "self", it's a defunctionalized self-ref.
                            // Give it `any` type to avoid infinite type errors.
                            let self_param = params.first().map(|s| s.as_str()).unwrap_or("");
                            let is_self_param = self_param == "me" || self_param == "self";

                            for (i, _p) in params.iter().enumerate() {
                                if i == 0 && is_self_param {
                                    param_types.push(any_ty.clone());
                                } else if let Some(TcType::Arrow(args, _)) = &annotated_type {
                                    param_types.push(args[i].clone());
                                } else {
                                    param_types.push(supply.fresh());
                                }
                            }

                            // Self-reference: register the function's own type so
                            // recursive calls type-check. For ycomb-style (has "me"/"self"
                            // param), the extra "me" param is excluded from the callable type.
                            if is_self_param && params.len() > 1 {
                                // (fib me n) is callable as (fib n) — skip the me param
                                let callable_params: Vec<TcType> = param_types[1..].to_vec();
                                let self_type =
                                    TcType::Arrow(callable_params, Box::new(ret_var.clone()));
                                check_env.insert_mono(name.clone(), self_type);
                            } else if !is_self_param {
                                // Normal direct recursion
                                let self_type =
                                    TcType::Arrow(param_types.clone(), Box::new(ret_var.clone()));
                                check_env.insert_mono(name.clone(), self_type);
                            } else {
                                // Single-param ycomb: (f me) — unlikely but handle gracefully
                                check_env.insert_mono(name.clone(), any_ty.clone());
                            }

                            for (p, t) in params.iter().zip(param_types.iter()) {
                                check_env.insert_mono(p.clone(), t.clone());
                            }

                            // Infer body — errors propagate as compile errors
                            let _body_type = infer(&body, &check_env, &mut supply, &mut subst)?;

                            // Verify body against the annotation's return type
                            if let Some(TcType::Arrow(_, ann_ret)) = &annotated_type {
                                let inferred_ret = subst.apply(&_body_type);
                                let declared_ret = subst.apply(ann_ret);
                                unify(&inferred_ret, &declared_ret)
                                    .map_err(|e| format!("define {}: type error — {}", name, e))?;
                            }

                            // Register inferred type for later defines
                            let resolved_params: Vec<TcType> =
                                param_types.iter().map(|t| subst.apply(t)).collect();
                            let resolved_ret = subst.apply(&ret_var);
                            env.insert_mono(
                                name.clone(),
                                TcType::Arrow(resolved_params, Box::new(resolved_ret)),
                            );
                        }
                    }
                    // Value define: (define name value)
                    LispVal::Sym(name) => {
                        let value = &list[2];
                        let mut subst = Subst::new();
                        let inferred = infer(value, &env, &mut supply, &mut subst)?;
                        env.insert_mono(name.clone(), subst.apply(&inferred));
                    }
                    _ => {}
                }
            }

            // Skip exports, memory declarations, borsh-schema, etc.
            _ => {}
        }
    }

    Ok(())
}

/// Scan all expressions for storage key consistency.
/// Tracks `(near/storage_write "key" val)` and `(near/store_num key val)` to
/// build a schema, then warns if reads don't match writes.
pub fn check_storage_schema(exprs: &[LispVal]) {
    let mut schema: HashMap<String, String> = HashMap::new(); // key → "written" | "read"
                                                              // t2: also track string keys used alongside numeric keys for collision detection
    let mut string_keys: HashMap<String, ()> = HashMap::new(); // bare string keys seen

    fn extract_str_key(val: &LispVal) -> Option<String> {
        match val {
            LispVal::Str(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// t2: Extract a numeric literal key from a LispVal.
    fn extract_num_key(val: &LispVal) -> Option<i64> {
        match val {
            LispVal::Num(n) => Some(*n),
            _ => None,
        }
    }

    // ── t5: Best-effort static evaluation of simple arithmetic expressions ──
    // Handles: literals, (+ a b), (* a b), (shl a b), (bor a b), (- a b)
    fn eval_const(expr: &LispVal) -> Option<i64> {
        match expr {
            LispVal::Num(n) => Some(*n),
            LispVal::List(list) if list.len() >= 3 => {
                if let LispVal::Sym(op) = &list[0] {
                    let a = eval_const(&list[1])?;
                    let b = eval_const(&list[2])?;
                    match op.as_str() {
                        "+" => Some(a.wrapping_add(b)),
                        "*" => a.checked_mul(b),
                        "shl" => {
                            if b >= 0 && b < 64 {
                                Some(a.wrapping_shl(b as u32))
                            } else {
                                None
                            }
                        }
                        "bor" => Some(a | b),
                        "-" => Some(a.wrapping_sub(b)),
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// t2: Get a numeric key from an expression — tries literal first, then
    /// t5 static eval, then gives up (returns None silently).
    fn resolve_num_key(key_expr: &LispVal) -> Option<i64> {
        // Direct literal first
        if let Some(n) = extract_num_key(key_expr) {
            return Some(n);
        }
        // t5: try static evaluation of arithmetic expressions
        eval_const(key_expr)
    }

    fn walk_for_storage(
        exprs: &[LispVal],
        schema: &mut HashMap<String, String>,
        string_keys: &mut HashMap<String, ()>,
    ) {
        for expr in exprs {
            walk_expr_storage(expr, schema, string_keys);
        }
    }

    fn walk_expr_storage(
        expr: &LispVal,
        schema: &mut HashMap<String, String>,
        string_keys: &mut HashMap<String, ()>,
    ) {
        match expr {
            LispVal::List(list) if !list.is_empty() => {
                if let LispVal::Sym(op) = &list[0] {
                    // ── String-keyed storage ──
                    if (op == "near/storage_write" || op == "near/storage_set") && list.len() >= 3 {
                        if let Some(key) = extract_str_key(&list[1]) {
                            schema
                                .entry(key.clone())
                                .or_insert_with(|| "written".into());
                            string_keys.insert(key, ());
                        }
                    } else if (op == "near/storage_read" || op == "near/storage_get")
                        && list.len() >= 2
                    {
                        if let Some(key) = extract_str_key(&list[1]) {
                            if !schema.contains_key(&key) {
                                eprintln!(
                                    "⚠ warning: storage_read of key \"{}\" but no matching storage_write found",
                                    key
                                );
                            }
                        }
                    }
                    // ── t2: Numeric-keyed storage ──
                    else if op == "near/store_num" && list.len() >= 3 {
                        if let Some(n) = resolve_num_key(&list[1]) {
                            let num_key = format!("num:{}", n);
                            schema
                                .entry(num_key.clone())
                                .or_insert_with(|| "written".into());
                            // t2: detect collision with string key of same digits
                            let bare = format!("{}", n);
                            if string_keys.contains_key(&bare) {
                                eprintln!(
                                    "⚠ warning: numeric key {} used alongside string key \"{}\" — potential confusion",
                                    n, bare
                                );
                            }
                        }
                    } else if op == "near/load_num" && list.len() >= 2 {
                        if let Some(n) = resolve_num_key(&list[1]) {
                            let num_key = format!("num:{}", n);
                            if !schema.contains_key(&num_key) {
                                eprintln!(
                                    "⚠ warning: load_num of key {} but no matching store_num found",
                                    n
                                );
                            }
                            // t2: detect collision with string key
                            let bare = format!("{}", n);
                            if string_keys.contains_key(&bare) {
                                eprintln!(
                                    "⚠ warning: numeric key {} used alongside string key \"{}\" — potential confusion",
                                    n, bare
                                );
                            }
                        }
                    }
                }
                // Recurse into sub-expressions
                for sub in list {
                    walk_expr_storage(sub, schema, string_keys);
                }
            }
            _ => {}
        }
    }

    walk_for_storage(exprs, &mut schema, &mut string_keys);

    // ── t5: Check for collisions among statically-evaluated numeric keys ──
    // We already stored all resolved numeric keys in schema as "num:N".
    // Check that no two different source expressions resolved to the same key.
    // (This is implicitly handled: if two different expressions produce the same
    //  "num:N", they both insert into the same schema entry — no duplication error.
    //  But we can scan for duplicate static key values that came from different
    //  expressions by re-walking and tracking expression→key mappings.)
    let mut const_keys: Vec<(i64, String)> = Vec::new(); // (resolved_key, source_snippet)
    fn collect_const_keys(exprs: &[LispVal], const_keys: &mut Vec<(i64, String)>) {
        for expr in exprs {
            collect_const_keys_expr(expr, const_keys);
        }
    }
    fn collect_const_keys_expr(expr: &LispVal, const_keys: &mut Vec<(i64, String)>) {
        if let LispVal::List(list) = expr {
            if list.len() >= 3 {
                if let LispVal::Sym(op) = &list[0] {
                    if op == "near/store_num" || op == "near/load_num" {
                        if let Some(n) = eval_const(&list[1]) {
                            let snippet = format!("{}", list[1]);
                            const_keys.push((n, snippet));
                        }
                    }
                }
            }
            for sub in list {
                collect_const_keys_expr(sub, const_keys);
            }
        }
    }
    collect_const_keys(exprs, &mut const_keys);
    // Check for collisions: same numeric key from different expressions
    let mut seen: HashMap<i64, String> = HashMap::new();
    for (key, snippet) in &const_keys {
        if let Some(prev) = seen.get(key) {
            if prev != snippet {
                eprintln!(
                    "⚠ warning: storage key collision — key {} produced by both {} and {}",
                    key, prev, snippet
                );
            }
        } else {
            seen.insert(*key, snippet.clone());
        }
    }
}

/// t3: Warn when `set!` is used in a value position where its nil return
/// is likely unintentional. Specifically checks:
/// - `set!` as the last expression in a `define` body (function returns nil)
/// - `set!` in an `if` branch (either then or else)
/// - `set!` in a `cond` branch body
/// - `set!` as the last expression in a `begin` block that's used as a value
pub fn check_set_value_positions(exprs: &[LispVal]) {
    for expr in exprs {
        walk_set_check(expr, ValueContext::Other);
    }
}

/// Tracks whether we're in a context where the expression's value matters.
#[derive(Clone, Copy, PartialEq)]
enum ValueContext {
    /// The expression is the last in a define body — its value IS the function result
    DefineBody,
    /// Inside an if/cond branch — value matters
    BranchValue,
    /// Inside a begin block as the last expression — value propagates
    BeginTail,
    /// Other positions — value may or may not matter
    Other,
}

fn walk_set_check(expr: &LispVal, ctx: ValueContext) {
    if let LispVal::List(list) = expr {
        if list.is_empty() {
            return;
        }

        // Check if this is a (set! ...) form
        if let LispVal::Sym(s) = &list[0] {
            if s == "set!" {
                // Warn if set! is in a value position
                if ctx == ValueContext::DefineBody
                    || ctx == ValueContext::BranchValue
                    || ctx == ValueContext::BeginTail
                {
                    let var_name = list
                        .get(1)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into());
                    eprintln!(
                        "⚠ warning: set! returns nil — result used as value in {} position (var: {})",
                        match ctx {
                            ValueContext::DefineBody => "function body (function returns nil)",
                            ValueContext::BranchValue => "branch",
                            ValueContext::BeginTail => "begin block",
                            _ => "expression",
                        },
                        var_name
                    );
                }
                // Still recurse into the value expression
                if list.len() >= 3 {
                    walk_set_check(&list[2], ValueContext::Other);
                }
                return;
            }

            // (if cond then else) — both branches are value positions
            if s == "if" && list.len() >= 3 {
                walk_set_check(&list[1], ValueContext::Other); // condition
                walk_set_check(&list[2], ValueContext::BranchValue); // then
                if list.len() >= 4 {
                    walk_set_check(&list[3], ValueContext::BranchValue); // else
                }
                return;
            }

            // (cond (cond1 val1) (cond2 val2) ...) — each branch value
            if s == "cond" {
                for clause in &list[1..] {
                    if let LispVal::List(cl) = clause {
                        if cl.len() >= 2 {
                            walk_set_check(&cl[0], ValueContext::Other); // condition
                            walk_set_check(&cl[1], ValueContext::BranchValue); // value
                        }
                    }
                }
                return;
            }

            // (begin ...) — last expression is the value
            if s == "begin" {
                for (i, sub) in list[1..].iter().enumerate() {
                    let is_last = i + 1 == list.len() - 1;
                    let sub_ctx = if is_last { ctx } else { ValueContext::Other };
                    walk_set_check(sub, sub_ctx);
                }
                return;
            }

            // (define (name params...) body) — body is a define-body context
            if s == "define" {
                if list.len() >= 3 {
                    if let LispVal::List(_sig) = &list[1] {
                        // Function define: body is define-body context
                        if list.len() > 3 {
                            // Multi-body: wrapped in begin
                            for (i, sub) in list[2..].iter().enumerate() {
                                let is_last = i + 1 == list.len() - 2;
                                let sub_ctx = if is_last {
                                    ValueContext::DefineBody
                                } else {
                                    ValueContext::Other
                                };
                                walk_set_check(sub, sub_ctx);
                            }
                        } else {
                            walk_set_check(&list[2], ValueContext::DefineBody);
                        }
                    } else {
                        // Value define: (define name value)
                        walk_set_check(&list[2], ValueContext::Other);
                    }
                }
                return;
            }

            // (let ((var val)) body) — body is value context
            if s == "let" || s == "let*" {
                if list.len() >= 3 {
                    if let LispVal::List(bindings) = &list[1] {
                        for b in bindings {
                            if let LispVal::List(pair) = b {
                                if pair.len() >= 2 {
                                    walk_set_check(&pair[1], ValueContext::Other);
                                }
                            }
                        }
                    }
                    walk_set_check(&list[2], ctx);
                }
                return;
            }

            // (loop ((var init) ...) body...) — last body expr is value
            if s == "loop" && list.len() >= 3 {
                if let LispVal::List(bindings) = &list[1] {
                    for b in bindings {
                        if let LispVal::List(p) = b {
                            if p.len() >= 2 {
                                walk_set_check(&p[1], ValueContext::Other);
                            }
                        }
                    }
                }
                for sub in &list[2..] {
                    walk_set_check(sub, ctx);
                }
                return;
            }

            // (lambda (params) body) or (fn (params) body) — body inherits define-body-like context
            if (s == "lambda" || s == "fn") && list.len() >= 3 {
                walk_set_check(&list[2], ValueContext::DefineBody);
                return;
            }
        }

        // Generic recursion for other list forms
        for sub in list {
            walk_set_check(sub, ValueContext::Other);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Checked arithmetic tests ──

    #[test]
    fn test_checked_add_overflows() {
        // (+ 9223372036854775807 1) should fail at compile time (const eval)
        let src = "(define (run) (+ 9223372036854775807 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "expected overflow error, got success");
        let err = result.unwrap_err();
        assert!(
            err.contains("overflow") || err.contains("Overflow"),
            "expected overflow message, got: {}",
            err
        );
    }

    #[test]
    fn test_checked_sub_overflows() {
        // (- 0 1) should be fine (returns -1), but (- -9223372036854775808 1) overflows
        let src = "(define (run) (- -9223372036854775808 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "expected underflow error, got success");
    }

    #[test]
    fn test_checked_mul_overflows() {
        let src = "(define (run) (* 9223372036854775807 2))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "expected overflow error, got success");
    }

    #[test]
    fn test_wrap_add_no_overflow() {
        // wrap-add should always succeed, even when the wrapped result
        // leaves the tagged payload range (wrap is its explicit contract).
        let src = "(define (run) (wrap-add 576460752303423488 576460752303423488))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "wrap-add should not error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_checked_add_refuses_unrepresentable_literal() {
        // Money-safety: literals outside [-2^60, 2^60) silently corrupted
        // at emission before (i64::MAX compiled to -1). Now: hard error.
        let src = "(define (run) (+ 9223372036854775807 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "unrepresentable literal must be refused");
        let err = result.unwrap_err();
        // Refusal may come from either guard: compile-time fold
        // ("arithmetic overflow at compile time") or literal emission
        // ("exceeds tagged range"). Both are safe refusals.
        assert!(
            err.contains("exceeds tagged range") || err.contains("overflow"),
            "error should name overflow/refusal, got: {}",
            err
        );
    }

    #[test]
    fn test_normal_add_no_overflow() {
        let src = "(define (run) (+ 1 2))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_ok(), "normal add should work, got: {:?}", result);
    }

    // ── Effect tracking tests ──

    #[test]
    fn test_pure_rejects_storage_write() {
        let src = r#"(pure (define (bad) (near/storage_write "k" "v")))"#;
        let result = type_check_program(&crate::parser::parse_all(src).unwrap(), true);
        assert!(result.is_err(), "pure should reject near/storage_write");
        let err = result.unwrap_err();
        assert!(
            err.contains("effect"),
            "expected effect error, got: {}",
            err
        );
    }

    #[test]
    fn test_pure_rejects_log() {
        let src = r#"(pure (define (bad) (near/log "hello")))"#;
        let result = type_check_program(&crate::parser::parse_all(src).unwrap(), true);
        assert!(result.is_err(), "pure should reject near/log");
    }

    #[test]
    fn test_pure_allows_arithmetic() {
        let src = "(pure (define (good x) (+ x 1)))";
        let result = type_check_program(&crate::parser::parse_all(src).unwrap(), true);
        assert!(
            result.is_ok(),
            "pure should allow arithmetic, got: {:?}",
            result
        );
    }

    // ── Storage schema tests ──

    #[test]
    fn test_storage_schema_warns_on_orphan_read() {
        // reading a key that was never written should produce a warning on stderr
        let src = r#"
            (define (run) (near/storage_read "orphan_key"))
        "#;
        // Can't easily capture eprintln in unit tests, but the function should not panic
        let exprs = crate::parser::parse_all(src).unwrap();
        check_storage_schema(&exprs);
        // If we got here, it didn't crash
    }

    // ── Assert forms tests ──

    #[test]
    fn test_assert_equal_compiles() {
        let src = "(define (run) (assert-equal 1 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "assert-equal should compile, got: {:?}",
            result
        );
    }

    #[test]
    fn test_assert_true_compiles() {
        let src = "(define (run) (assert-true (> 5 3)))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "assert-true should compile, got: {:?}",
            result
        );
    }

    #[test]
    fn test_assert_raises_compiles() {
        let src = "(define (run) (assert-raises (/ 1 0)))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "assert-raises should compile, got: {:?}",
            result
        );
    }

    // ── Cond exhaustiveness tests ──

    #[test]
    fn test_cond_with_else_no_warning() {
        // This should compile without issues
        let src = r#"
            (define (classify x)
              (cond
                ((< x 0) "negative")
                ((> x 0) "positive")
                (else "zero")))
        "#;
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "cond with else should compile, got: {:?}",
            result
        );
    }

    #[test]
    fn test_cond_without_else_still_compiles() {
        // No else — warning but not error
        let src = r#"
            (define (maybe x)
              (cond
                ((> x 0) "positive")))
        "#;
        let result = crate::wasm_emit::compile_pure(src);
        assert!(
            result.is_ok(),
            "cond without else should still compile (just warn)"
        );
    }

    // ── Type checker core tests ──

    #[test]
    fn test_undefined_variable_caught() {
        let src = "(define (run) (+ unknown_var 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "should catch undefined variable");
        let err = result.unwrap_err();
        assert!(
            err.contains("undefined") || err.contains("Unbound"),
            "expected undefined var error, got: {}",
            err
        );
    }

    #[test]
    fn test_arity_mismatch_caught() {
        let src = "(define (f x) (+ x 1)) (define (run) (f 1 2))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "should catch arity mismatch");
    }

    #[test]
    fn test_near_builtin_accepted() {
        let src = r#"(define (hello) (near/return_str "hi"))"#;
        let result = crate::wasm_emit::compile_near(src);
        assert!(
            result.is_ok(),
            "near builtins should be accepted, got: {:?}",
            result
        );
    }
}
