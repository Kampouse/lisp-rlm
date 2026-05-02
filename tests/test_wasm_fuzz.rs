//! Differential fuzz harness: ClosureVM vs tagged WASM emitter.
//!
//! Compiles a Lisp program with both:
//!   1. The ClosureVM interpreter (lisp_eval)
//!   2. The WASM emitter (compile_fuzz → wasmtime)
//!
//! Then compares results. The WASM emitter uses 3-bit tagged values:
//!   bottom 3 bits = type tag, upper 61 bits = payload
//!   TAG_NUM=0, TAG_BOOL=1, TAG_FNREF=2, TAG_CLOSURE=3, TAG_NIL=4, TAG_STR=5

use lisp_rlm_wasm::types::{Env, EvalState, LispVal};
use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::wasm_emit::compile_fuzz;

// Tag constants (must match wasm_emit.rs)
const TAG_BITS: i64 = 3;
const TAG_NUM: i64 = 0;
const TAG_BOOL: i64 = 1;
const TAG_NIL: i64 = 4;
const TAG_STR: i64 = 5;

const TEMP_MEM: usize = 64;

/// Decode a tagged i64 from WASM memory into a LispVal for comparison.
fn decode_tagged(raw: i64) -> Option<LispVal> {
    let tag = raw & 0x7;
    let payload = raw >> TAG_BITS; // arithmetic shift preserves sign
    match tag {
        TAG_NUM => Some(LispVal::Num(payload)),
        TAG_BOOL => Some(LispVal::Bool(payload != 0)),
        TAG_NIL => Some(LispVal::Nil),
        TAG_STR => Some(LispVal::Str(format!("STR_TAGGED_{}", payload))),
        2 => Some(LispVal::Sym(format!("fnref_{}", payload))),
        3 => Some(LispVal::Sym(format!("closure_{}", payload))),
        _ => None,
    }
}

/// Convert a ClosureVM LispVal to a comparable tagged i64.
/// Returns None for values that can't round-trip through i64 (strings, lists, etc.).
fn lispval_to_tagged(val: &LispVal) -> Option<i64> {
    match val {
        LispVal::Num(n) => Some((*n << TAG_BITS) | TAG_NUM),
        LispVal::Bool(b) => Some(((if *b { 1i64 } else { 0 }) << TAG_BITS) | TAG_BOOL),
        LispVal::Nil => Some(TAG_NIL),
        // These can't round-trip through i64 tagging
        LispVal::Str(_) | LispVal::List(_) | LispVal::Lambda { .. }
        | LispVal::Float(_) | LispVal::Map(_) | LispVal::BuiltinFn(_)
        | LispVal::Macro { .. } | LispVal::CaseLambda { .. }
        | LispVal::Recur(_) | LispVal::Memoized { .. }
        | LispVal::Tagged { .. } | LispVal::Sym(_) => None,
    }
}

/// Set up wasmtime with all NEAR host function stubs, run the WASM module,
/// and return the tagged i64 stored at TEMP_MEM.
#[cfg(not(target_arch = "wasm32"))]
fn run_wasm_fuzz(wasm: &[u8]) -> Result<i64, String> {
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    // Check if module imports memory (NEAR mode) or declares it internally
    let needs_imported_memory = module.imports().any(|i| i.module() == "env" && i.name() == "memory");

    if needs_imported_memory {
        let memory = Memory::new(&mut store, MemoryType::new(4, None))
            .map_err(|e| format!("memory: {}", e))?;
        linker.define(&store, "env", "memory", memory)
            .map_err(|e| format!("link memory: {}", e))?;
    }

    // Define noop stubs for every non-memory import, matching exact signatures.
    for import in module.imports() {
        if import.module() == "env" && import.name() != "memory" {
            let ty = import.ty();
            if let wasmtime::ExternType::Func(func_ty) = ty {
                let params: Vec<ValType> = func_ty.params().collect();
                let results: Vec<ValType> = func_ty.results().collect();
                let result_count = results.len();
                let ft = FuncType::new(&engine, params, results);
                let stub = Func::new(&mut store, ft, move |_, _, ret| {
                    for i in 0..result_count {
                        ret[i] = Val::I64(0);
                    }
                    Ok(())
                });
                linker.define(&store, "env", import.name(), stub)
                    .map_err(|e| format!("link {}: {}", import.name(), e))?;
            }
        }
    }

    let instance = linker.instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {}", e))?;

    // Get memory from instance export (works for both internal and imported memory)
    let memory = instance.get_memory(&mut store, "memory")
        .ok_or_else(|| "no memory export".to_string())?;

    // Call "run" export
    let run = instance.get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| format!("no 'run' export: {}", e))?;
    run.call(&mut store, ()).map_err(|e| format!("trap: {}", e))?;

    // Read tagged value from TEMP_MEM
    let mem = memory.data(&store);
    let raw = i64::from_le_bytes(mem[TEMP_MEM..TEMP_MEM + 8].try_into().unwrap());
    Ok(raw)
}

/// Run a single fuzz test case: compare ClosureVM vs WASM.
#[cfg(not(target_arch = "wasm32"))]
fn fuzz_one(source: &str) -> Result<(), String> {
    let result = fuzz_one_inner(source);
    if let Err(ref e) = result {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/fuzz_errors.log") {
            let _ = writeln!(f, "FAIL {:?}: {}", source, e);
        }
    }
    result
}

fn fuzz_one_inner(source: &str) -> Result<(), String> {
    // 1. Parse
    let exprs = parse_all(source).map_err(|e| format!("parse error: {}", e))?;
    if exprs.is_empty() {
        return Ok(());
    }

    // 2. ClosureVM evaluation — load stdlib
    let stdlib_exprs = match lisp_rlm_wasm::types::get_stdlib_code("core") {
        Some(code) => parse_all(code).map_err(|e| format!("stdlib parse: {}", e))?,
        None => Vec::new(),
    };
    let mut env = Env::new();
    let mut state = EvalState::new();

    // Evaluate stdlib
    for expr in &stdlib_exprs {
        lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state)
            .map_err(|e| format!("stdlib VM error: {}", e))?;
    }

    // Evaluate user expressions
    let mut cl_result = LispVal::Nil;
    for expr in &exprs {
        match lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state) {
            Ok(v) => cl_result = v,
            Err(_) => {
                // ClosureVM errored (e.g., div-by-zero, type error).
                // WASM handles these gracefully (returns 0, coerces types).
                // This is an intentional divergence — not a bug.
                return Ok(());
            }
        }
    }

    // If the source defines (run), call it to get the actual result
    if let Some(LispVal::Lambda { .. }) | Some(LispVal::BuiltinFn(_)) = env.get("run") {
        let run_call = parse_all("(run)").map_err(|e| format!("run parse: {}", e))?;
        if let Some(expr) = run_call.first() {
            match lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state) {
                Ok(v) => cl_result = v,
                Err(_) => {
                    // Same as above: VM error is acceptable divergence.
                    return Ok(());
                }
            }
        }
    }

    // 3. Compile to WASM
    // If the last expression is already (define (run) ...), use source as-is.
    // Otherwise wrap it in (define (run) ...) for the export wrapper.
    let last = &exprs[exprs.len() - 1];
    let is_define_run = matches!(last, LispVal::List(v) if v.len() >= 3
        && matches!(&v[0], LispVal::Sym(s) if s == "define")
        && matches!(&v[1], LispVal::List(n) if n.len() >= 1 && matches!(&n[0], LispVal::Sym(nm) if nm == "run")));

    let full_source = if is_define_run {
        source.to_string()
    } else if exprs.len() > 1 {
        let mut s = String::new();
        for expr in &exprs[..exprs.len() - 1] {
            s.push_str(&expr.to_string());
            s.push('\n');
        }
        s.push_str(&format!("(define (run) {})", last));
        s
    } else {
        format!("(define (run) {})", last)
    };

    let wasm = compile_fuzz(&full_source).map_err(|e| format!("compile error: {}", e))?;

    // 4. Execute WASM
    let wasm_raw = run_wasm_fuzz(&wasm)?;

    // 5. Compare
    let cl_tagged = lispval_to_tagged(&cl_result);
    match cl_tagged {
        Some(expected) => {
            if wasm_raw != expected {
                return Err(format!(
                    "MISMATCH: source={:?}\n  ClosureVM: {:?} → tagged 0x{:016x}\n  WASM:      tagged 0x{:016x} → {:?}",
                    source, cl_result, expected as u64, wasm_raw as u64, decode_tagged(wasm_raw)
                ));
            }
        }
        None => {
            // Can't compare (string, list, lambda, etc.)
            // Just verify the tag is valid
            let tag = wasm_raw as u64 & 0x7;
            if tag > 5 {
                return Err(format!(
                    "INVALID TAG: source={:?}\n  ClosureVM: {:?} (non-comparable)\n  WASM:      tagged 0x{:016x} (tag={})",
                    source, cl_result, wasm_raw as u64, tag
                ));
            }
        }
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    #[test]
    fn fuzz_basic_arithmetic() {
        assert!(fuzz_one("(+ 1 2)").is_ok(), "1+2");
        assert!(fuzz_one("(+ -10 25)").is_ok(), "-10+25");
        assert!(fuzz_one("(+ 0 0)").is_ok(), "0+0");
        assert!(fuzz_one("(+ 9223372036854775807 0)").is_ok(), "max+0");
        assert!(fuzz_one("(+ -9223372036854775808 0)").is_ok(), "min+0");
    }

    #[test]
    fn fuzz_subtraction() {
        let r = fuzz_one("(- 10 3)");
        if r.is_err() { eprintln!("sub: {:?}", r); }
        assert!(r.is_ok());
        let r = fuzz_one("(- 0 0)");
        if r.is_err() { eprintln!("0-0: {:?}", r); }
        assert!(r.is_ok());
        let r = fuzz_one("(- 5)");
        if r.is_err() { eprintln!("neg5: {:?}", r); }
        assert!(r.is_ok());
    }

    #[test]
    fn fuzz_multiplication() {
        assert!(fuzz_one("(* 3 7)").is_ok());
        assert!(fuzz_one("(* 0 999)").is_ok());
        assert!(fuzz_one("(* -1 1)").is_ok());
    }

    #[test]
    fn fuzz_division() {
        assert!(fuzz_one("(/ 10 3)").is_ok());
        assert!(fuzz_one("(/ 100 10)").is_ok());
        assert!(fuzz_one("(/ -7 2)").is_ok());
    }

    #[test]
    fn fuzz_modulo() {
        assert!(fuzz_one("(mod 10 3)").is_ok());
        assert!(fuzz_one("(mod 7 7)").is_ok());
    }

    #[test]
    fn fuzz_comparisons() {
        assert!(fuzz_one("(> 5 3)").is_ok());
        assert!(fuzz_one("(< 1 2)").is_ok());
        assert!(fuzz_one("(>= 5 5)").is_ok());
        assert!(fuzz_one("(<= 3 3)").is_ok());
        assert!(fuzz_one("(= 7 7)").is_ok());
        assert!(fuzz_one("(!= 3 4)").is_ok());
        assert!(fuzz_one("(= 0 0)").is_ok());
        assert!(fuzz_one("(= -1 -1)").is_ok());
    }

    #[test]
    fn fuzz_logic() {
        let cases = [
            ("(and 1 2)", "and-1-2"),
            ("(and 0 1)", "and-0-1"),
            ("(or 0 1)", "or-0-1"),
            ("(or 0 0)", "or-0-0"),
            ("(not 1)", "not-1"),
            ("(not 0)", "not-0"),
        ];
        for (src, label) in &cases {
            if let Err(e) = fuzz_one(src) {
                panic!("{} failed: {}", label, e);
            }
        }
    }

    #[test]
    fn fuzz_if() {
        assert!(fuzz_one("(if 1 42 99)").is_ok());
        assert!(fuzz_one("(if 0 42 99)").is_ok());
        assert!(fuzz_one("(if nil 42 99)").is_ok());
        assert!(fuzz_one("(if false 42 99)").is_ok());
        assert!(fuzz_one("(if true 42)").is_ok());
    }

    #[test]
    fn fuzz_let() {
        assert!(fuzz_one("(let ((x 10) (y 20)) (+ x y))").is_ok());
        assert!(fuzz_one("(let ((a 5)) (* a a))").is_ok());
    }

    #[test]
    fn fuzz_begin() {
        assert!(fuzz_one("(begin 1 2 3)").is_ok());
        assert!(fuzz_one("(begin (+ 1 2) (* 3 4))").is_ok());
    }

    #[test]
    fn fuzz_set() {
        assert!(fuzz_one("(define (run) (let ((x 10)) (set! x 20) x))").is_ok());
    }

    #[test]
    fn fuzz_function_call() {
        assert!(fuzz_one("(define (add a b) (+ a b))\n(add 3 4)").is_ok());
        assert!(fuzz_one("(define (square x) (* x x))\n(square 7)").is_ok());
        assert!(fuzz_one("(define (id x) x)\n(id 42)").is_ok());
    }

    #[test]
    fn fuzz_nested_calls() {
        assert!(fuzz_one("(define (f x) (+ x 1))\n(f (f (f 0)))").is_ok());
    }

    #[test]
    fn fuzz_fibonacci() {
        assert!(fuzz_one("(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))\n(fib 10)").is_ok());
    }

    // ClosureVM doesn't support `while` as a built-in
    #[test]
    #[ignore]
    fn fuzz_while_loop() {
        assert!(fuzz_one(
            "(define (run) (let ((sum 0) (i 0)) (while (< i 10) (set! sum (+ sum i)) (set! i (+ i 1))) sum))"
        ).is_ok());
    }

    // ClosureVM doesn't support `for` loop compilation
    #[test]
    #[ignore]
    fn fuzz_for_loop() {
        assert!(fuzz_one(
            "(define (run) (let ((sum 0)) (for i 1 11 (set! sum (+ sum i))) sum))"
        ).is_ok());
    }

    #[test]
    fn fuzz_abs() {
        assert!(fuzz_one("(abs -5)").is_ok());
        assert!(fuzz_one("(abs 3)").is_ok());
        assert!(fuzz_one("(abs 0)").is_ok());
    }

    #[test]
    fn fuzz_zero_distinguishing() {
        // The whole point of tagging: Num(0), Bool(false), Nil are all different
        let r = fuzz_one("0");
        if r.is_err() { eprintln!("0: {:?}", r); }
        assert!(r.is_ok(), "0");
        let r = fuzz_one("false");
        if r.is_err() { eprintln!("false: {:?}", r); }
        assert!(r.is_ok(), "false");
        let r = fuzz_one("nil");
        if r.is_err() { eprintln!("nil: {:?}", r); }
        assert!(r.is_ok(), "nil");
        let r = fuzz_one("true");
        if r.is_err() { eprintln!("true: {:?}", r); }
        assert!(r.is_ok(), "true");
        let r = fuzz_one("1");
        if r.is_err() { eprintln!("1: {:?}", r); }
        assert!(r.is_ok(), "1");
    }

    #[test]
    fn fuzz_chained_arithmetic() {
        assert!(fuzz_one("(+ 1 (* 2 3) (- 10 5))").is_ok());
        assert!(fuzz_one("(* (+ 1 2) (- 10 3) (/ 100 2))").is_ok());
    }

    #[test]
    fn fuzz_closure() {
        assert!(fuzz_one("(define (make-adder n) (lambda (x) (+ x n)))\n(define (run) ((make-adder 10) 5))").is_ok());
    }

    #[test]
    fn fuzz_conditional_arithmetic() {
        assert!(fuzz_one("(if (> 3 2) (+ 1 2) (* 3 4))").is_ok());
        assert!(fuzz_one("(if (< 1 0) 999 42)").is_ok());
    }

    #[test]
    fn fuzz_deep_nesting() {
        assert!(fuzz_one("(+ (+ (+ 1 2) (+ 3 4)) (+ (+ 5 6) (+ 7 8)))").is_ok());
    }

    #[test]
    fn fuzz_identity_functions() {
        assert!(fuzz_one("(define (run) (let ((x 42)) x))").is_ok());
        assert!(fuzz_one("(define (id x) x) (define (run) (id 42))").is_ok());
    }

    #[test]
    fn fuzz_bool_ops_edge_cases() {
        assert!(fuzz_one("(and true true)").is_ok());
        assert!(fuzz_one("(and true false)").is_ok());
        assert!(fuzz_one("(and false false)").is_ok());
        assert!(fuzz_one("(or true false)").is_ok());
        assert!(fuzz_one("(or false false)").is_ok());
        assert!(fuzz_one("(or true true)").is_ok());
    }
}

// ── Property-based differential fuzz ──
//
// Generates random Lisp programs, runs on ClosureVM and WASM, compares results.
// Catches bugs in emitter instruction lowering that hardcoded tests miss.

#[cfg(not(target_arch = "wasm32"))]
mod prop {
    use super::*;
    use proptest::prelude::*;
    use proptest::prop_compose;

    /// Generate a random safe integer.
    fn safe_int() -> impl Strategy<Value = i64> {
        -1000i64..1000i64
    }

    /// Generate a random arithmetic operator.
    fn arith_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("+"), Just("-"), Just("*"), Just("/"), Just("mod")]
    }

    /// Generate a random comparison operator.
    fn cmp_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just(">"), Just("<"), Just(">="), Just("<="), Just("="), Just("!=")]
    }

    /// Generate a random logic operator.
    fn logic_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("and"), Just("or"), Just("not")]
    }

    /// Generate a simple expression (depth 0 — leaf or binary op on leaves).
    fn leaf_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            safe_int().prop_map(|n| n.to_string()),
            Just("true".into()),
            Just("false".into()),
            Just("nil".into()),
        ]
    }

    /// Generate a numeric leaf (integers only — avoids WASM/ClosureVM type coercion divergence).
    fn num_leaf() -> impl Strategy<Value = String> {
        safe_int().prop_map(|n| n.to_string())
    }

    /// Generate a binary expression at depth 1: (op leaf leaf).
    /// Arithmetic and comparison ops use numeric leaves only to avoid
    /// WASM vs ClosureVM type coercion divergence (WASM coerces bool/nil to numbers).
    fn binary_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            (arith_op(), num_leaf(), num_leaf()).prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
            (cmp_op(), num_leaf(), num_leaf()).prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
            (logic_op(), leaf_expr(), leaf_expr()).prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
        ]
    }

    /// Generate an if expression: (if cond then else).
    fn if_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr(), leaf_expr())
            .prop_map(|(c, t, e)| format!("(if {} {} {})", c, t, e))
    }

    /// Generate a let expression: (let ((x val)) body).
    fn let_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr())
            .prop_map(|(val, body)| format!("(let ((x {})) {})", val, body))
    }

    /// Generate a begin expression: (begin a b).
    fn begin_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr())
            .prop_map(|(a, b)| format!("(begin {} {})", a, b))
    }

    /// Wrap any expression in (define (run) ...).
    fn program(inner: impl Strategy<Value = String>) -> impl Strategy<Value = String> {
        inner.prop_map(|e| format!("(define (run) {})", e))
    }

    proptest! {
        /// Differential fuzz: random leaf expressions.
        #[test]
        fn prop_leaf(expr in program(leaf_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("leaf mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random binary arithmetic/comparison/logic.
        #[test]
        fn prop_binary(expr in program(binary_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("binary mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random if expressions.
        #[test]
        fn prop_if(expr in program(if_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("if mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random let expressions.
        #[test]
        fn prop_let(expr in program(let_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("let mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random begin expressions.
        #[test]
        fn prop_begin(expr in program(begin_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("begin mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: chained arithmetic with 2-6 args.
        #[test]
        fn prop_chained(
            op in arith_op(),
            count in 2usize..=6,
            val in safe_int()
        ) {
            let args: Vec<String> = (0..count).map(|_| format!("{}", val)).collect();
            let expr = format!("(define (run) ({} {}))", op, args.join(" "));
            if let Err(e) = fuzz_one(&expr) {
                panic!("chained mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: nested if (1-4 levels deep).
        #[test]
        fn prop_nested_if(depth in 1usize..=4) {
            let mut s = "42".to_string();
            for _ in 0..depth {
                s = format!("(if 1 {} 0)", s);
            }
            let expr = format!("(define (run) {})", s);
            if let Err(e) = fuzz_one(&expr) {
                panic!("nested-if mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: mixed let with 1-3 bindings.
        #[test]
        fn prop_let_multi(
            n in 1usize..=3,
            body in leaf_expr()
        ) {
            let bindings: Vec<String> = (0..n)
                .map(|i| format!("(x{} {})", i, (i as i64 + 1) * 10))
                .collect();
            let expr = format!("(define (run) (let ({}) {}))", bindings.join(" "), body);
            if let Err(e) = fuzz_one(&expr) {
                panic!("let-multi mismatch: {}\nsource: {}", e, expr);
            }
        }
    }
}
