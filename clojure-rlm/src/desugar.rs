//! Desugar CljVal → lisp_rlm_wasm::LispVal
//!
//! Handles:
//! - defn, fn with [] params
//! - #(...) %1 %2 %& anon fn
//! - -> and ->> threading macros
//! - when, when-not, cond, if-not, not-empty
//! - Vectors → (vec ...) or quoted lists
//! - Keywords → :keyword strings
//! - Maps → (hash-map ...) calls
//! - Sets → (hash-set ...) calls

use lisp_rlm_wasm::LispVal;
use crate::parser::CljVal;

pub fn desugar(val: &CljVal) -> LispVal {
    match val {
        CljVal::Nil => LispVal::Nil,
        CljVal::Bool(b) => LispVal::Bool(*b),
        CljVal::Num(n) => LispVal::Num(*n as i64),
        CljVal::Str(s) => LispVal::Str(s.clone()),
        CljVal::Keyword(k) => LispVal::Str(format!(":{}", k)),
        CljVal::Sym(s) => LispVal::Sym(s.clone()),

        CljVal::Vec(items) => {
            let desugared: Vec<LispVal> = items.iter().map(desugar).collect();
            // (vec item1 item2 ...) — runtime constructs a vector value
            let mut args = vec![LispVal::Sym("vec".into())];
            args.extend(desugared);
            LispVal::List(args)
        }

        CljVal::Map(pairs) => {
            let mut args = vec![LispVal::Sym("hash-map".into())];
            for (k, v) in pairs {
                args.push(desugar(k));
                args.push(desugar(v));
            }
            LispVal::List(args)
        }

        CljVal::Set(items) => {
            let mut args = vec![LispVal::Sym("hash-set".into())];
            for item in items {
                args.push(desugar(item));
            }
            LispVal::List(args)
        }

        CljVal::List(items) => {
            if items.is_empty() {
                return LispVal::List(vec![]);
            }
            // Check for special forms
            if let CljVal::Sym(head) = &items[0] {
                match head.as_str() {
                    "defn" => return desugar_defn(items),
                    "fn" => return desugar_fn(items),
                    "defn-" => return desugar_defn(items), // private, treat same for now
                    "def" => return desugar_def(items),
                    "let" => return desugar_let(items),
                    "when" => return desugar_when(items),
                    "when-not" => return desugar_when_not(items),
                    "if-not" => return desugar_if_not(items),
                    "cond" => return desugar_cond(items),
                    "->" | "some->" => return desugar_thread_first(items),
                    "->>" | "some->>" => return desugar_thread_last(items),
                    "not-empty" => return desugar_not_empty(items),
                    "assoc" | "dissoc" | "get" | "update" | "contains?" => {
                        // Pass through, just desugar args
                    }
                    _ => {}
                }
            }
            // Default: desugar all items as a function call
            let desugared: Vec<LispVal> = items.iter().map(desugar).collect();
            LispVal::List(desugared)
        }

        CljVal::AnonFn(body) => desugar_anon_fn(body),
    }
}

// (defn name [params...] body...)
// → (define (name params...) body...)
fn desugar_defn(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    // items[0] = defn, items[1] = name, items[2] = [params], items[3..] = body
    let name = match &items[1] {
        CljVal::Sym(s) => s.clone(),
        _ => return LispVal::List(items.iter().map(desugar).collect()),
    };

    let params = match &items[2] {
        CljVal::Vec(v) => v.iter().map(|p| desugar(p)).collect::<Vec<_>>(),
        _ => return LispVal::List(items.iter().map(desugar).collect()),
    };

    // Body: wrap multiple expressions in begin
    let body = if items.len() > 4 {
        let body_exprs: Vec<LispVal> = items[3..].iter().map(desugar).collect();
        let mut b = vec![LispVal::Sym("begin".into())];
        b.extend(body_exprs);
        LispVal::List(b)
    } else if items.len() == 4 {
        desugar(&items[3])
    } else {
        LispVal::Nil
    };

    let mut sig = vec![LispVal::Sym(name)];
    sig.extend(params);
    LispVal::List(vec![
        LispVal::Sym("define".into()),
        LispVal::List(sig),
        body,
    ])
}

// (fn [params...] body...)
// → (lambda (params...) body...)
fn desugar_fn(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let params = match &items[1] {
        CljVal::Vec(v) => v.iter().map(|p| desugar(p)).collect(),
        _ => return LispVal::List(items.iter().map(desugar).collect()),
    };

    let body = if items.len() > 3 {
        let body_exprs: Vec<LispVal> = items[2..].iter().map(desugar).collect();
        { let mut b = vec![LispVal::Sym("begin".into())]; b.extend(body_exprs); LispVal::List(b) }
    } else {
        desugar(&items[2])
    };

    LispVal::List(vec![
        LispVal::Sym("lambda".into()),
        LispVal::List(params),
        body,
    ])
}

// (def name expr)
fn desugar_def(items: &[CljVal]) -> LispVal {
    if items.len() != 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    LispVal::List(vec![
        LispVal::Sym("define".into()),
        desugar(&items[1]),
        desugar(&items[2]),
    ])
}

// (let [x 1 y 2] body...)
// → (let ((x 1) (y 2)) body...)
fn desugar_let(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let bindings = match &items[1] {
        CljVal::Vec(v) => {
            // pairs: [x 1 y 2] → ((x 1) (y 2))
            let mut pairs = Vec::new();
            let mut i = 0;
            while i + 1 < v.len() {
                pairs.push(LispVal::List(vec![desugar(&v[i]), desugar(&v[i + 1])]));
                i += 2;
            }
            pairs
        }
        _ => return LispVal::List(items.iter().map(desugar).collect()),
    };

    let body = if items.len() > 3 {
        let body_exprs: Vec<LispVal> = items[2..].iter().map(desugar).collect();
        { let mut b = vec![LispVal::Sym("begin".into())]; b.extend(body_exprs); LispVal::List(b) }
    } else {
        desugar(&items[2])
    };

    LispVal::List(vec![
        LispVal::Sym("let".into()),
        LispVal::List(bindings),
        body,
    ])
}

// (when test body...) → (if test (begin body...))
fn desugar_when(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let test = desugar(&items[1]);
    let body = if items.len() > 3 {
        let body_exprs: Vec<LispVal> = items[2..].iter().map(desugar).collect();
        { let mut b = vec![LispVal::Sym("begin".into())]; b.extend(body_exprs); LispVal::List(b) }
    } else {
        desugar(&items[2])
    };
    LispVal::List(vec![LispVal::Sym("if".into()), test, body])
}

// (when-not test body...) → (if (not test) (begin body...))
fn desugar_when_not(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let test = LispVal::List(vec![LispVal::Sym("not".into()), desugar(&items[1])]);
    let body = if items.len() > 3 {
        let body_exprs: Vec<LispVal> = items[2..].iter().map(desugar).collect();
        { let mut b = vec![LispVal::Sym("begin".into())]; b.extend(body_exprs); LispVal::List(b) }
    } else {
        desugar(&items[2])
    };
    LispVal::List(vec![LispVal::Sym("if".into()), test, body])
}

// (if-not test then else?) → (if (not test) then else?)
fn desugar_if_not(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let test = LispVal::List(vec![LispVal::Sym("not".into()), desugar(&items[1])]);
    let then = desugar(&items[2]);
    let else_ = if items.len() > 3 { desugar(&items[3]) } else { LispVal::Nil };
    LispVal::List(vec![LispVal::Sym("if".into()), test, then, else_])
}

// (cond test1 expr1 test2 expr2 ... :else default)
fn desugar_cond(items: &[CljVal]) -> LispVal {
    if items.len() < 3 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    let mut i = 1;
    let mut result = None::<LispVal>;
    while i + 1 < items.len() {
        let test = &items[i];
        let expr = &items[i + 1];
        let is_else = match test {
            CljVal::Keyword(k) => k == "else",
            CljVal::Sym(s) => s == "else",
            _ => false,
        };
        if is_else {
            result = Some(desugar(expr));
            break;
        }
        result = Some(LispVal::List(vec![
            LispVal::Sym("if".into()),
            desugar(test),
            desugar(expr),
            result.unwrap_or(LispVal::Nil),
        ]));
        i += 2;
    }
    result.unwrap_or(LispVal::Nil)
}

// (-> x (f a) (g b c)) → (g (f x a) b c)
fn desugar_thread_first(items: &[CljVal]) -> LispVal {
    if items.len() < 2 {
        return LispVal::Nil;
    }
    let mut acc = desugar(&items[1]);
    for item in items[2..].iter() {
        acc = thread_into(acc, item, false);
    }
    acc
}

// (->> x (f a) (g b c)) → (g b c (f a x))
fn desugar_thread_last(items: &[CljVal]) -> LispVal {
    if items.len() < 2 {
        return LispVal::Nil;
    }
    let mut acc = desugar(&items[1]);
    for item in items[2..].iter() {
        acc = thread_into(acc, item, true);
    }
    acc
}

fn thread_into(acc: LispVal, form: &CljVal, last: bool) -> LispVal {
    match form {
        CljVal::List(items) if !items.is_empty() => {
            let func = desugar(&items[0]);
            let args: Vec<LispVal> = items[1..].iter().map(desugar).collect();
            if last {
                let mut all_args = args;
                all_args.push(acc);
                { let mut c = vec![func]; c.extend(all_args); LispVal::List(c) }
            } else {
                // Insert acc as second arg (after function)
                let mut all_args = vec![acc];
                all_args.extend(args);
                { let mut c = vec![func]; c.extend(all_args); LispVal::List(c) }
            }
        }
        // (-> x f) shorthand — just (f x)
        CljVal::Sym(s) => {
            if last {
                LispVal::List(vec![LispVal::Sym(s.clone()), acc])
            } else {
                LispVal::List(vec![LispVal::Sym(s.clone()), acc])
            }
        }
        _ => LispVal::List(vec![acc, desugar(form)]),
    }
}

// (not-empty x) → (if (empty? x) nil x)
fn desugar_not_empty(items: &[CljVal]) -> LispVal {
    if items.len() != 2 {
        return LispVal::List(items.iter().map(desugar).collect());
    }
    LispVal::List(vec![
        LispVal::Sym("if".into()),
        LispVal::List(vec![LispVal::Sym("empty?".into()), desugar(&items[1])]),
        LispVal::Nil,
        desugar(&items[1]),
    ])
}

// #(* %1 %2) → (lambda (%1 %2) (* %1 %2))
// Detects % (=%1), %1..%9, %&
fn desugar_anon_fn(body: &[CljVal]) -> LispVal {
    let mut max_arg = 0usize;
    let mut has_rest = false;

    // Scan for %N and %&
    fn scan(val: &CljVal, max: &mut usize, rest: &mut bool) {
        match val {
            CljVal::Sym(s) => {
                if s == "%" {
                    *max = (*max).max(1);
                } else if s == "%&" {
                    *rest = true;
                } else if s.starts_with('%') {
                    if let Ok(n) = s[1..].parse::<usize>() {
                        *max = (*max).max(n);
                    }
                }
            }
            CljVal::List(items) | CljVal::Vec(items) | CljVal::AnonFn(items) => {
                for item in items { scan(item, max, rest); }
            }
            CljVal::Map(pairs) => {
                for (k, v) in pairs { scan(k, max, rest); scan(v, max, rest); }
            }
            CljVal::Set(items) => {
                for item in items { scan(item, max, rest); }
            }
            _ => {}
        }
    }

    for item in body {
        scan(item, &mut max_arg, &mut has_rest);
    }

    // Build param list
    let mut params: Vec<LispVal> = (1..=max_arg)
        .map(|i| LispVal::Sym(format!("%{}", i)))
        .collect();
    if has_rest {
        params.push(LispVal::Sym("%&".into()));
    }

    // Body items are the raw contents of #(...) — treat as a function call form
    // #(* % 2) → body = [Sym("*"), Sym("%"), Num(2)] → desugar as (* %1 2)
    let desugared_body: Vec<LispVal> = body.iter().map(|b| {
        let mut d = desugar(b);
        rename_bare_percent(&mut d);
        d
    }).collect();

    let body_expr = if desugared_body.len() == 1 {
        desugared_body.into_iter().next().unwrap()
    } else {
        // Multiple items = function call (first is fn, rest are args)
        LispVal::List(desugared_body)
    };

    LispVal::List(vec![
        LispVal::Sym("lambda".into()),
        LispVal::List(params),
        body_expr,
    ])
}

fn rename_bare_percent(val: &mut LispVal) {
    if let LispVal::Sym(s) = val {
        if s == "%" {
            *s = "%1".into();
        }
    }
    if let LispVal::List(items) = val {
        for item in items.iter_mut() {
            rename_bare_percent(item);
        }
    }
}
