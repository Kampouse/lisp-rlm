//! Clojure-syntax desugaring pass.
//!
//! Translates Clojure-style forms into the internal lisp-rlm representation
//! that the compiler already understands. Applied after parsing, before
//! compilation. Both old `(define ...)` and new `(defn ...)` work side by side.
//!
//! Supported transformations:
//!   (defn name [params...] body...)    → (define (name params...) body...)
//!   (defn name doc-string [params...] body...) → (define (name params...) body...)
//!   (def name expr)                    → (define name expr)
//!   (let [x 1 y 2] body...)           → (let ((x 1) (y 2)) body...)
//!   (fn [params...] body...)           → (fn (params...) body...)
//!   (lambda [params...] body...)       → (lambda (params...) body...)
//!   [a b c] in param position          → (a b c) — Vec → List for params

use crate::types::LispVal;

/// Desugar all top-level forms in place.
pub fn desugar(exprs: &mut [LispVal]) {
    for expr in exprs.iter_mut() {
        desugar_expr(expr);
    }
}

fn desugar_expr(expr: &mut LispVal) {
    match expr {
        LispVal::List(items) => {
            if items.is_empty() {
                return;
            }

            // Check head symbol
            if let LispVal::Sym(head) = items[0].clone() {
                match head.as_str() {
                    "defn" | "defn-" => {
                        desugar_defn(items, head == "defn-");
                    }
                    "def" => {
                        desugar_def(items);
                    }
                    "let" | "let*" => {
                        desugar_let(items);
                    }
                    "loop" => {
                        desugar_let(items);
                    }
                    "fn" | "lambda" => {
                        desugar_fn_params(items);
                    }
                    _ => {
                        // Recurse into sub-expressions
                        for item in items.iter_mut() {
                            desugar_expr(item);
                        }
                    }
                }
            } else {
                for item in items.iter_mut() {
                    desugar_expr(item);
                }
            }
        }
        LispVal::Vec(items) => {
            for item in items.iter_mut() {
                desugar_expr(item);
            }
        }
        _ => {}
    }
}

/// (defn name [params...] body...) → (define (name params...) body...)
/// (defn name doc-string [params...] body...) → (define (name params...) body...)
/// Also handles: (defn [params...] name body...)  — swapped form
fn desugar_defn(items: &mut Vec<LispVal>, _private: bool) {
    // items[0] = "defn", items[1..] = rest
    if items.len() < 3 {
        return;
    }

    let mut name_idx = 1;
    let mut param_idx = 2;

    // Handle swapped form: (defn [params] name body...)
    if matches!(&items[1], LispVal::Vec(v) if !v.is_empty())
        || matches!(&items[1], LispVal::List(v) if !v.is_empty())
    {
        // items[1] looks like a param vector, check if items[2] is a symbol (the name)
        if let LispVal::Sym(_) = &items[2] {
            param_idx = 1;
            name_idx = 2;
        }
    }

    let name = items[name_idx].clone();

    // Skip doc-string if present (a string literal at param_idx position)
    if param_idx + 1 < items.len() {
        if let LispVal::Str(_) = &items[param_idx] {
            param_idx += 1;
        }
    }

    if param_idx >= items.len() {
        return;
    }

    // Convert [params] to (params)
    let params = vec_to_list(&items[param_idx]);
    let body_items: Vec<LispVal> = items[param_idx + 1..].to_vec();

    // Build (define (name params...) body...)
    let define_list = if body_items.len() <= 1 {
        let body = body_items.into_iter().next().unwrap_or(LispVal::Nil);
        vec![
            LispVal::Sym("define".into()),
            LispVal::List({
                let mut sig = vec![name];
                match params {
                    LispVal::List(ps) => sig.extend(ps),
                    LispVal::Vec(ps) => sig.extend(ps),
                    other => sig.push(other),
                }
                sig
            }),
            body,
        ]
    } else {
        // Multiple body forms → implicit begin
        vec![
            LispVal::Sym("define".into()),
            LispVal::List({
                let mut sig = vec![name];
                match params {
                    LispVal::List(ps) => sig.extend(ps),
                    LispVal::Vec(ps) => sig.extend(ps),
                    other => sig.push(other),
                }
                sig
            }),
            LispVal::List({
                let mut begin = vec![LispVal::Sym("begin".into())];
                begin.extend(body_items);
                begin
            }),
        ]
    };

    // Desugar body recursively
    let mut result = LispVal::List(define_list);
    desugar_expr(&mut result);
    *items = if let LispVal::List(r) = result {
        r
    } else {
        vec![result]
    };
}

/// (def name expr) → (define name expr)
fn desugar_def(items: &mut Vec<LispVal>) {
    if items.len() < 3 {
        return;
    }
    items[0] = LispVal::Sym("define".into());
    // Recurse into the value expression
    for item in items.iter_mut() {
        desugar_expr(item);
    }
}

/// (let [x 1 y 2] body...) → (let ((x 1) (y 2)) body...)
/// Also accepts already-paired form: (let ((x 1) (y 2)) body...)
fn desugar_let(items: &mut Vec<LispVal>) {
    // items[0] = "let", items[1] = [bindings...], items[2..] = body
    if items.len() < 3 {
        return;
    }

    // Check if bindings are already in paired form ((x 1) (y 2))
    // Each element should be a 2-element list — if so, skip conversion
    let already_paired = match &items[1] {
        LispVal::List(bs) | LispVal::Vec(bs) => bs
            .iter()
            .all(|b| matches!(b, LispVal::List(p) if p.len() == 2)),
        _ => false,
    };

    if !already_paired {
        let bindings = vec_to_pairs(&items[1]);
        items[1] = bindings;
    }

    // Recurse into binding values (the second element of each pair)
    if let LispVal::List(pairs) = &mut items[1] {
        for pair in pairs.iter_mut() {
            if let LispVal::List(p) = pair {
                if p.len() == 2 {
                    desugar_expr(&mut p[1]);
                }
            }
        }
    }

    // Recurse into body
    for item in items.iter_mut().skip(2) {
        desugar_expr(item);
    }
}

/// (fn [params...] body...) → (fn (params...) body...)
/// (lambda [params...] body...) → (lambda (params...) body...)
fn desugar_fn_params(items: &mut Vec<LispVal>) {
    if items.len() < 3 {
        return;
    }
    // Convert [params] to (params)
    items[1] = vec_to_list(&items[1]);
    // Recurse into body
    for item in items.iter_mut() {
        desugar_expr(item);
    }
}

/// Convert a LispVal::Vec to LispVal::List, leaving List as-is.
fn vec_to_list(val: &LispVal) -> LispVal {
    match val {
        LispVal::Vec(items) => {
            let list_items: Vec<LispVal> =
                items.iter().map(|item| vec_to_list_inner(item)).collect();
            LispVal::List(list_items)
        }
        LispVal::List(items) => LispVal::List(items.clone()),
        other => other.clone(),
    }
}

fn vec_to_list_inner(val: &LispVal) -> LispVal {
    match val {
        LispVal::Vec(items) => LispVal::List(items.iter().map(vec_to_list_inner).collect()),
        other => other.clone(),
    }
}

/// Convert flat bindings vector [x 1 y 2] → ((x 1) (y 2))
fn vec_to_pairs(val: &LispVal) -> LispVal {
    let items = match val {
        LispVal::Vec(items) => items,
        LispVal::List(items) => items,
        _ => return val.clone(),
    };

    let mut pairs = Vec::new();
    let mut i = 0;
    while i + 1 < items.len() {
        pairs.push(LispVal::List(vec![
            vec_to_list_inner(&items[i]),
            vec_to_list_inner(&items[i + 1]),
        ]));
        i += 2;
    }
    LispVal::List(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_all;

    fn run(src: &str) -> Result<String, String> {
        let mut exprs = parse_all(src)?;
        desugar(&mut exprs);
        let mut env = crate::types::Env::new();
        let mut state = crate::types::EvalState::new();
        let result = crate::program::run_program(&exprs, &mut env, &mut state)?;
        Ok(result.to_string())
    }

    #[test]
    fn test_defn_basic() {
        let r = run("(defn add [a b] (+ a b)) (add 3 4)").unwrap();
        assert_eq!(r, "7");
    }

    #[test]
    fn test_defn_fibonacci() {
        let r = run(r#"
            (defn fib [n]
              (if (<= n 1) n
                (+ (fib (- n 1)) (fib (- n 2)))))
            (fib 10)
        "#)
        .unwrap();
        assert_eq!(r, "55");
    }

    #[test]
    fn test_def_basic() {
        let r = run("(def x 42) x").unwrap();
        assert_eq!(r, "42");
    }

    #[test]
    fn test_let_bindings() {
        let r = run("(let [x 10 y 20] (+ x y))").unwrap();
        assert_eq!(r, "30");
    }

    #[test]
    fn test_fn_anonymous() {
        let r = run("((fn [a b] (* a b)) 6 7)").unwrap();
        assert_eq!(r, "42");
    }

    #[test]
    fn test_defn_multi_body() {
        let r = run(r#"
            (defn greet [name]
              (println "hello")
              (+ 1 2))
            (greet "world")
        "#)
        .unwrap();
        assert_eq!(r, "3");
    }

    #[test]
    fn test_old_define_still_works() {
        let r = run("(define (square x) (* x x)) (square 5)").unwrap();
        assert_eq!(r, "25");
    }

    #[test]
    fn test_mixed_syntax() {
        let r = run(r#"
            (defn add [a b] (+ a b))
            (define (mul x y) (* x y))
            (let [x (add 3 4)
                  y (mul 2 3)]
              (+ x y))
        "#)
        .unwrap();
        assert_eq!(r, "13");
    }

    #[test]
    fn test_loop_recur() {
        let r = run(r#"
            (loop [i 0 acc 0]
              (if (>= i 10) acc
                (recur (+ i 1) (+ acc i))))
        "#)
        .unwrap();
        assert_eq!(r, "45");
    }

    #[test]
    fn test_loop_inside_defn() {
        let r = run(r#"
            (defn count-to [n]
              (loop [i 0 acc 0]
                (if (>= i n) acc
                  (recur (+ i 1) (+ acc i)))))
            (count-to 10)
        "#)
        .unwrap();
        assert_eq!(r, "45");
    }

    #[test]
    fn test_defn_no_space_name_brackets() {
        // fib[] parses as fib + [] — zero params, no crash
        let r = run(r#"
            (defn fib[]
              (if true 42 0))
            (fib)
        "#)
        .unwrap();
        assert_eq!(r, "42");
    }

    #[test]
    fn test_defn_swapped_params_name() {
        // (defn [params] name body...) — params before name
        let r = run(r#"
            (defn [x y] my-add
              (+ x y))
            (my-add 3 4)
        "#)
        .unwrap();
        assert_eq!(r, "7");
    }
}
