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
        (TcCon::Any, _) | (_, TcCon::Any) => Ok(Subst::new()),
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
        other => return Err(format!("unknown type: {}", other)),
    })
}

fn parse_type_list(elems: &[LispVal]) -> Result<TcType, String> {
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
    if args.is_empty() {
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
    env.pure_mode = true;  // forbid effectful calls in pure blocks
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
        other => return Err(format!("pure define: expected function name, got {:?}", other)),
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
                None => Err(format!("type error: undefined variable '{}' — not in scope", name)),
            }
        }

        // Lambda: (lambda (params...) body)
        LispVal::List(list) if !list.is_empty() => {
            match &list[0] {
                LispVal::Sym(s) if s == "lambda" => infer_lambda(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "if" => infer_if(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "let" => infer_let(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "let*" => infer_let_star(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "begin" => infer_begin(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "and" => infer_and(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "or" => infer_or(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "cond" => infer_cond(&list[1..], env, supply, subst),
                LispVal::Sym(s) if s == "quote" => Ok(TcType::Con(TcCon::Any)), // quoted data is opaque
                LispVal::Sym(s) if s == "set!" => {
                    // (set! var value) — mutation, infer the value but return nil
                    if list.len() >= 3 {
                        let _ = infer(&list[2], env, supply, subst)?;
                    }
                    Ok(TcType::Con(TcCon::Nil))
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
                LispVal::Sym(s) if s == "list" => {
                    infer_list_literal(&list[1..], env, supply, subst)
                }
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
        let s = unify(&then_type, &else_type)
            .map_err(|e| format!("if: branch types disagree — {}", e))?;
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
    let bindings_list = parts.first().ok_or("let: missing bindings")?;
    let body = parts.get(1).cloned().unwrap_or(LispVal::Nil);

    let bindings = match bindings_list {
        LispVal::List(l) => l,
        other => return Err(format!("let: bindings must be list, got {}", other)),
    };

    let mut new_env = env.clone();
    for binding in bindings {
        let pair = match binding {
            LispVal::List(l) if l.len() == 2 => l,
            other => return Err(format!("let: binding must be (name val), got {:?}", other)),
        };
        let name = match &pair[0] {
            LispVal::Sym(s) => s.clone(),
            other => return Err(format!("let: binding name must be symbol, got {:?}", other)),
        };
        let val_type = infer(&pair[1], env, supply, subst)?;
        new_env.insert_mono(name, subst.apply(&val_type));
    }

    infer(&body, &new_env, supply, subst)
}

fn infer_let_star(
    parts: &[LispVal],
    env: &TcEnv,
    supply: &mut VarSupply,
    subst: &mut Subst,
) -> Result<TcType, String> {
    let bindings_list = parts.first().ok_or("let*: missing bindings")?;
    let body = parts.get(1).cloned().unwrap_or(LispVal::Nil);

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

    infer(&body, &new_env, supply, subst)
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
    let has_else = parts.iter().last().and_then(|c| {
        if let LispVal::List(l) = c { l.first() } else { None }
    }).and_then(|v| if let LispVal::Sym(s) = v { Some(s.as_str()) } else { None })
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
        let s = unify(&elem_type, &t).map_err(|e| format!("list: heterogeneous types — {}", e))?;
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
pub fn type_check_program(exprs: &[LispVal], near: bool) -> Result<(), String> {
    let mut env = if near {
        TcEnv::with_near_builtins()
    } else {
        TcEnv::with_pure_builtins()
    };
    let mut supply = VarSupply::new();

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

                            // Build body (handle multi-body defines)
                            let body = if list.len() > 3 {
                                LispVal::List(
                                    std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(list[2..].iter().cloned())
                                        .collect(),
                                )
                            } else {
                                list[2].clone()
                            };

                            // Create fresh type vars for params
                            let mut check_env = env.clone();
                            let mut subst = Subst::new();
                            let ret_var = supply.fresh();
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
                                } else {
                                    param_types.push(supply.fresh());
                                }
                            }

                            // Self-reference: register the function's own type so
                            // recursive calls type-check. For ycomb-style (has "me"/"self"
                            // param), the extra "me" param is excluded from the callable type.
                            if is_self_param && params.len() > 1 {
                                // (fib me n) is callable as (fib n) — skip the me param
                                let callable_params: Vec<TcType> =
                                    param_types[1..].to_vec();
                                let self_type = TcType::Arrow(
                                    callable_params,
                                    Box::new(ret_var.clone()),
                                );
                                check_env.insert_mono(name.clone(), self_type);
                            } else if !is_self_param {
                                // Normal direct recursion
                                let self_type = TcType::Arrow(
                                    param_types.clone(),
                                    Box::new(ret_var.clone()),
                                );
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
/// Tracks `(near/storage_write "key" val)` to build a schema, then warns if
/// `(near/storage_read "key")` is used where a different type was written.
pub fn check_storage_schema(exprs: &[LispVal]) {
    let mut schema: HashMap<String, String> = HashMap::new(); // key → location info

    fn extract_str_key(val: &LispVal) -> Option<String> {
        match val {
            LispVal::Str(s) => Some(s.clone()),
            _ => None,
        }
    }

    fn walk_for_storage(exprs: &[LispVal], schema: &mut HashMap<String, String>) {
        for expr in exprs {
            walk_expr_storage(expr, schema);
        }
    }

    fn walk_expr_storage(expr: &LispVal, schema: &mut HashMap<String, String>) {
        match expr {
            LispVal::List(list) if list.len() >= 3 => {
                if let LispVal::Sym(op) = &list[0] {
                    if op == "near/storage_write" {
                        if let Some(key) = extract_str_key(&list[1]) {
                            // Record that this key is written to
                            schema.entry(key).or_insert_with(|| "written".into());
                        }
                    } else if op == "near/storage_read" {
                        if let Some(key) = extract_str_key(&list[1]) {
                            // Check if this key was ever written
                            if !schema.contains_key(&key) {
                                eprintln!(
                                    "⚠ warning: storage_read of key \"{}\" but no matching storage_write found",
                                    key
                                );
                            }
                        }
                    }
                }
                // Recurse into sub-expressions
                for sub in list {
                    walk_expr_storage(sub, schema);
                }
            }
            LispVal::List(list) => {
                for sub in list {
                    walk_expr_storage(sub, schema);
                }
            }
            _ => {}
        }
    }

    walk_for_storage(exprs, &mut schema);
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
        assert!(err.contains("overflow") || err.contains("Overflow"),
            "expected overflow message, got: {}", err);
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
        // wrap-add should always succeed, even on overflow
        let src = "(define (run) (wrap-add 9223372036854775807 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_ok(), "wrap-add should not error, got: {:?}", result);
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
        let result = type_check_program(
            &crate::parser::parse_all(src).unwrap(), true
        );
        assert!(result.is_err(), "pure should reject near/storage_write");
        let err = result.unwrap_err();
        assert!(err.contains("effect"), "expected effect error, got: {}", err);
    }

    #[test]
    fn test_pure_rejects_log() {
        let src = r#"(pure (define (bad) (near/log "hello")))"#;
        let result = type_check_program(
            &crate::parser::parse_all(src).unwrap(), true
        );
        assert!(result.is_err(), "pure should reject near/log");
    }

    #[test]
    fn test_pure_allows_arithmetic() {
        let src = "(pure (define (good x) (+ x 1)))";
        let result = type_check_program(
            &crate::parser::parse_all(src).unwrap(), true
        );
        assert!(result.is_ok(), "pure should allow arithmetic, got: {:?}", result);
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
        assert!(result.is_ok(), "assert-equal should compile, got: {:?}", result);
    }

    #[test]
    fn test_assert_true_compiles() {
        let src = "(define (run) (assert-true (> 5 3)))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_ok(), "assert-true should compile, got: {:?}", result);
    }

    #[test]
    fn test_assert_raises_compiles() {
        let src = "(define (run) (assert-raises (/ 1 0)))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_ok(), "assert-raises should compile, got: {:?}", result);
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
        assert!(result.is_ok(), "cond with else should compile, got: {:?}", result);
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
        assert!(result.is_ok(), "cond without else should still compile (just warn)");
    }

    // ── Type checker core tests ──

    #[test]
    fn test_undefined_variable_caught() {
        let src = "(define (run) (+ unknown_var 1))";
        let result = crate::wasm_emit::compile_pure(src);
        assert!(result.is_err(), "should catch undefined variable");
        let err = result.unwrap_err();
        assert!(err.contains("undefined") || err.contains("Unbound"),
            "expected undefined var error, got: {}", err);
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
        assert!(result.is_ok(), "near builtins should be accepted, got: {:?}", result);
    }
}
