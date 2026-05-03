use lisp_rlm_wasm::EvalState;
use lisp_rlm_wasm::*;

fn run_program(code: &str, env: &mut Env, state: &mut EvalState) -> Result<String, String> {
    let result = lisp_rlm_wasm::program::run_program(
        &parse_all(code)?, env, state
    )?;
    Ok(result.to_string())
}

fn eval_str(code: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    run_program(code, &mut env, &mut state).unwrap_or_else(|e| format!("ERROR: {}", e))
}

fn eval_str_with_stdlib(code: &str) -> String {
    let mut env = Env::new();
    let mut state = EvalState::new();
    if let Some(scode) = get_stdlib_code("math") {
        if let Ok(exprs) = parse_all(scode) {
            let _ = lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state);
        }
    }
    if let Some(scode) = get_stdlib_code("list") {
        if let Ok(exprs) = parse_all(scode) {
            let _ = lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state);
        }
    }
    if let Some(scode) = get_stdlib_code("string") {
        if let Ok(exprs) = parse_all(scode) {
            let _ = lisp_rlm_wasm::program::run_program(&exprs, &mut env, &mut state);
        }
    }
    run_program(code, &mut env, &mut state).unwrap_or_else(|e| format!("ERROR: {}", e))
}

// === Basic construction ===

#[test]
fn test_vec_literal() {
    assert_eq!(eval_str("(vec 1 2 3)"), "[1 2 3]");
}

#[test]
fn test_vec_empty() {
    assert_eq!(eval_str("(vec)"), "[]");
}

#[test]
fn test_vec_single() {
    assert_eq!(eval_str("(vec 42)"), "[42]");
}

#[test]
fn test_vec_nested() {
    assert_eq!(eval_str("(vec (vec 1 2) (vec 3 4))"), "[[1 2] [3 4]]");
}

#[test]
fn test_vec_mixed_types() {
    assert_eq!(eval_str("(vec 1 \"hello\" true nil)"), "[1 \"hello\" true nil]");
}

// === vec? predicate ===

#[test]
fn test_vec_predicate() {
    assert_eq!(eval_str("(vec? (vec 1 2))"), "true");
    assert_eq!(eval_str("(vec? (list 1 2))"), "false");
    assert_eq!(eval_str("(vec? 42)"), "false");
    assert_eq!(eval_str("(vec? nil)"), "false");
}

// === vec-nth ===

#[test]
fn test_vec_nth() {
    assert_eq!(eval_str("(vec-nth (vec 10 20 30) 0)"), "10");
    assert_eq!(eval_str("(vec-nth (vec 10 20 30) 1)"), "20");
    assert_eq!(eval_str("(vec-nth (vec 10 20 30) 2)"), "30");
}

#[test]
fn test_vec_nth_out_of_bounds() {
    assert_eq!(eval_str("(vec-nth (vec 10 20 30) 3)"), "nil");
    assert_eq!(eval_str("(vec-nth (vec 10 20 30) -1)"), "nil");
}

// === vec-assoc (immutable update) ===

#[test]
fn test_vec_assoc() {
    assert_eq!(eval_str("(vec-assoc (vec 10 20 30) 1 99)"), "[10 99 30]");
}

#[test]
fn test_vec_assoc_immutable() {
    // Original vec is unchanged — use grouped let bindings
    assert_eq!(
        eval_str("(let ((v (vec 10 20 30)) (v2 (vec-assoc v 1 99))) (list (vec-nth v 1) (vec-nth v2 1)))"),
        "(20 99)"
    );
}

#[test]
fn test_vec_assoc_out_of_bounds() {
    assert_eq!(eval_str("(vec-assoc (vec 10 20 30) 5 99)"), "nil");
}

// === vec-len ===

#[test]
fn test_vec_len() {
    assert_eq!(eval_str("(vec-len (vec 1 2 3))"), "3");
    assert_eq!(eval_str("(vec-len (vec))"), "0");
    assert_eq!(eval_str("(vec-len (vec 1))"), "1");
}

#[test]
fn test_vec_len_non_vec() {
    assert_eq!(eval_str("(vec-len 42)"), "0");
}

// === vec-conj (append) ===

#[test]
fn test_vec_conj() {
    assert_eq!(eval_str("(vec-conj (vec 1 2) 3)"), "[1 2 3]");
}

#[test]
fn test_vec_conj_empty() {
    // conj onto nil creates a vec
    assert_eq!(eval_str("(vec-conj nil 1)"), "[1]");
}

#[test]
fn test_vec_conj_nested() {
    assert_eq!(eval_str("(vec-conj (vec (vec 1)) (vec 2))"), "[[1] [2]]");
}

// === vec-contains? ===

#[test]
fn test_vec_contains() {
    assert_eq!(eval_str("(vec-contains? (vec 1 2 3) 2)"), "true");
    assert_eq!(eval_str("(vec-contains? (vec 1 2 3) 5)"), "false");
}

#[test]
fn test_vec_contains_non_vec() {
    assert_eq!(eval_str("(vec-contains? 42 1)"), "false");
}

#[test]
fn test_vec_contains_string() {
    assert_eq!(eval_str("(vec-contains? (vec \"a\" \"b\" \"c\") \"b\")"), "true");
}

// === vec-slice ===

#[test]
fn test_vec_slice() {
    assert_eq!(eval_str("(vec-slice (vec 0 1 2 3 4) 1 3)"), "[1 2]");
}

#[test]
fn test_vec_slice_full() {
    assert_eq!(eval_str("(vec-slice (vec 0 1 2 3 4) 0 5)"), "[0 1 2 3 4]");
}

#[test]
fn test_vec_slice_empty() {
    assert_eq!(eval_str("(vec-slice (vec 0 1 2 3 4) 2 2)"), "[]");
}

#[test]
fn test_vec_slice_clamped() {
    // Out of bounds indices are clamped
    assert_eq!(eval_str("(vec-slice (vec 0 1 2) 0 10)"), "[0 1 2]");
}

#[test]
fn test_vec_slice_invalid_args() {
    assert_eq!(eval_str("(vec-slice 42 0 1)"), "nil");
}

// === HOF: map on vec ===

#[test]
fn test_map_on_vec() {
    assert_eq!(
        eval_str_with_stdlib("(map (fn (x) (+ x 1)) (vec 1 2 3))"),
        "[2 3 4]"
    );
}

#[test]
fn test_filter_on_vec() {
    assert_eq!(
        eval_str_with_stdlib("(filter (fn (x) (> x 2)) (vec 1 2 3 4 5))"),
        "[3 4 5]"
    );
}

#[test]
fn test_reduce_on_vec() {
    assert_eq!(
        eval_str_with_stdlib("(reduce + 0 (vec 1 2 3 4 5))"),
        "15"
    );
}

// === Type checking ===

#[test]
fn test_check_vec() {
    assert_eq!(eval_str("(check (vec 1 2 3) :vec)"), "[1 2 3]");
}

#[test]
fn test_check_vec_fail() {
    let result = eval_str("(check (list 1 2 3) :vec)");
    assert!(result.starts_with("ERROR: type mismatch: expected :vec, got :list"));
}

#[test]
fn test_matches_vec() {
    assert_eq!(eval_str("(matches? (vec 1 2) :vec)"), "true");
    assert_eq!(eval_str("(matches? (list 1 2) :vec)"), "false");
}

#[test]
fn test_valid_type_vec() {
    assert_eq!(eval_str("(valid-type? :vec)"), r#"":vec""#);
}
