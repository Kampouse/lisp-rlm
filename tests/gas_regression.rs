//! Gas regression testing: compile benchmarks, measure WASM sizes + wasmtime fuel.
//!
//! For on-chain gas, deploy and call via:
//!   near contract deploy kampy.testnet use-file /tmp/gas_bench.wasm ...
//!   near contract call-function kampy.testnet run '' --gas 100Tgas ...
//!
//! Run:  cargo test --test gas_regression -- --nocapture
//! More: GAS_SAMPLES=100 cargo test --test gas_regression -- --nocapture

use std::env;

use lisp_rlm_wasm::{compile_fuzz, parse_all, run_program, Env, EvalState, LispVal};
use rand::prelude::*;
use wasmtime::{Config, Engine, Func, FuncType, Linker, Module, Store, ValType};

// ─── Benchmark programs ───────────────────────────────────────────────

const BENCHMARKS: &[(&str, &str)] = &[
    ("arith_simple", "(define (run) (+ 1 2))"),
    ("arith_nested", "(define (run) (+ (* 3 4) (- 10 5)))"),
    ("compare", "(define (run) (if (< 5 10) 1 0))"),
    ("list_ops", "(define (run) (car (list 1 2 3)))"),
    ("list_cdr", "(define (run) (cdr (list 1 2 3)))"),
    ("lambda_call", "(define (run) ((lambda (x) (+ x 1)) 5))"),
    ("bool_and", "(define (run) (and true false))"),
    ("bool_or", "(define (run) (or false 42))"),
    ("let_bind", "(define (run) (let ((x 10)) (+ x 5)))"),
    ("string_len", r#"(define (run) (str-len "hello"))"#),
    ("predicate", "(define (run) (number? 42))"),
    (
        "deep_nest",
        "(define (run) (+ (+ (+ 1 2) (+ 3 4)) (+ (+ 5 6) (+ 7 8))))",
    ),
    ("fib_like", "(define (run) (if (<= 5 1) 1 (+ 2 3)))"),
    ("multi_if", "(define (run) (if false 1 (if true 2 3)))"),
    ("begin_seq", "(define (run) (begin 1 2 3))"),
];

// ─── WASM runner ──────────────────────────────────────────────────────

/// Run WASM locally, return (tagged_result, fuel_consumed).
fn run_wasm(wasm: &[u8]) -> Result<(i64, u64, Vec<u8>), String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).map_err(|e| format!("engine: {e}"))?;
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {e}"))?;
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|e| format!("fuel: {e}"))?;

    let read_reg_fn = Func::wrap(&mut store, |_: i64, _: i64| {});
    let reg_len_fn = Func::wrap(&mut store, |_: i64| -> i64 { 0i64 });
    let input_fn = Func::wrap(&mut store, |_: i64| {});
    let log_fn = Func::new(
        &mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()),
    );
    let value_return_fn = Func::new(
        &mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()),
    );

    let mut linker = Linker::new(&engine);
    linker
        .define(&store, "env", "read_register", read_reg_fn)
        .unwrap();
    linker
        .define(&store, "env", "register_len", reg_len_fn)
        .unwrap();
    linker.define(&store, "env", "input", input_fn).unwrap();
    linker.define(&store, "env", "log_utf8", log_fn).unwrap();
    linker
        .define(&store, "env", "value_return", value_return_fn)
        .unwrap();

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e}"))?;
    let func = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| format!("get func: {e}"))?;

    func.call(&mut store, ())
        .map_err(|e| format!("call: {e}"))?;

    let fuel_left = store.get_fuel().map_err(|e| format!("fuel: {e}"))?;
    let fuel_used = 10_000_000 - fuel_left;

    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or("no memory")?;
    let data = memory.data(&store).to_vec();
    let tagged = i64::from_le_bytes(data[64..72].try_into().unwrap());
    Ok((tagged, fuel_used, data))
}

// ─── Key helpers ───────────────────────────────────────────────────────

const TAG_NUM: i64 = 0;
const TAG_BOOL: i64 = 1;
const TAG_NIL: i64 = 4;

fn lispval_to_key(val: &LispVal) -> String {
    match val {
        LispVal::Num(n) => format!("num:{n}"),
        LispVal::Bool(true) => "bool:true".into(),
        LispVal::Bool(false) => "bool:false".into(),
        LispVal::Nil => "nil".into(),
        LispVal::Str(s) => format!("str:{s}"),
        LispVal::List(elems) => {
            if elems.is_empty() {
                "nil".into()
            } else {
                let inner: Vec<String> = elems.iter().map(lispval_to_key).collect();
                format!("list:({})", inner.join(" "))
            }
        }
        _ => format!("other:{:?}", val),
    }
}

fn tagged_to_key(tagged: i64) -> String {
    let tag = tagged & 0x7;
    let payload = tagged >> 3;
    match tag {
        TAG_NUM => format!("num:{payload}"),
        TAG_BOOL if payload == 0 => "bool:false".into(),
        TAG_BOOL => "bool:true".into(),
        TAG_NIL => "nil".into(),
        _ => format!("tagged:{tagged}"),
    }
}

fn run_bytecode(src: &str) -> Result<LispVal, String> {
    let exprs = parse_all(src).map_err(|e| format!("parse: {e}"))?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    run_program(&exprs, &mut env, &mut state)
}

// ─── Random generator ─────────────────────────────────────────────────

fn gen_random_expr(rng: &mut rand::rngs::StdRng, depth: usize) -> String {
    if depth <= 0 || rng.gen_range(0..5) == 0 {
        return match rng.gen_range(0..4) {
            0..=2 => rng.gen_range(-10..10).to_string(),
            3 => "true".into(),
            _ => "nil".into(),
        };
    }
    match rng.gen_range(0..8) {
        0 => format!(
            "(+ {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        1 => format!(
            "(- {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        2 => format!(
            "(if {} {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        3 => format!(
            "(let ((x {})) {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        4 => format!(
            "(list {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        5 => format!(
            "((lambda (x) {}) {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        6 => format!(
            "(and {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
        _ => format!(
            "(< {} {})",
            gen_random_expr(rng, depth - 1),
            gen_random_expr(rng, depth - 1)
        ),
    }
}

// ─── Test ──────────────────────────────────────────────────────────────

#[test]
fn test_gas_snapshot() {
    let n_samples: usize = env::var("GAS_SAMPLES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let mut rng = rand::rngs::StdRng::seed_from_u64(42);

    eprintln!("\n═══ Gas Regression Snapshot ═══");
    eprintln!("  Compiler: lisp-rlm (local build)");
    eprintln!("  Samples:  {n_samples}");
    eprintln!();

    // Phase 1: Benchmark programs — size + fuel
    eprintln!("── Benchmark Programs ──");
    let mut benchmark_results: Vec<(&str, usize, u64)> = Vec::new();

    for (name, src) in BENCHMARKS {
        let full_src = format!("{src}\n(run)\n");

        let bc = match run_bytecode(&full_src) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("  {name:20} → BYTECODE FAIL: {e}");
                continue;
            }
        };

        let wasm = match compile_fuzz(&full_src) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("  {name:20} → COMPILE FAIL: {e}");
                continue;
            }
        };

        let size = wasm.len();
        let (tagged, fuel, _mem) = match run_wasm(&wasm) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  {name:20} → WASM RUN FAIL: {e}");
                continue;
            }
        };

        let bc_key = lispval_to_key(&bc);
        let wasm_key = tagged_to_key(tagged);
        let correct = bc_key == wasm_key;

        let status = if correct {
            "ok".into()
        } else {
            format!("MISMATCH bc={bc_key} wasm={wasm_key}")
        };
        eprintln!(
            "  {:20} -> {} bytes, {:>6} fuel  {}",
            name, size, fuel, status
        );

        benchmark_results.push((name, size, fuel));
    }

    // Phase 2: Random programs — distribution
    eprintln!("\n── Random Programs (fuzz) ──");
    let mut compile_ok = 0usize;
    let mut compile_fail = 0usize;
    let mut wasm_sizes: Vec<usize> = Vec::new();
    let mut fuel_costs: Vec<u64> = Vec::new();

    for _ in 0..n_samples {
        let depth = rng.gen_range(1..4);
        let body = gen_random_expr(&mut rng, depth);
        let src = format!("(define (run)\n  {})\n(run)\n", body);

        match compile_fuzz(&src) {
            Ok(wasm) => {
                if let Ok((_, fuel, _)) = run_wasm(&wasm) {
                    compile_ok += 1;
                    wasm_sizes.push(wasm.len());
                    fuel_costs.push(fuel);
                } else {
                    compile_fail += 1;
                }
            }
            Err(_) => {
                compile_fail += 1;
            }
        }
    }

    if !wasm_sizes.is_empty() {
        wasm_sizes.sort();
        fuel_costs.sort();

        let median_size = wasm_sizes[wasm_sizes.len() / 2];
        let p90_size = wasm_sizes[(wasm_sizes.len() as f64 * 0.9) as usize];
        let max_size = wasm_sizes.iter().max().unwrap();
        let min_size = wasm_sizes.iter().min().unwrap();

        let median_fuel = fuel_costs[fuel_costs.len() / 2];
        let p90_fuel = fuel_costs[(fuel_costs.len() as f64 * 0.9) as usize];
        let max_fuel = fuel_costs.iter().max().unwrap();
        let min_fuel = fuel_costs.iter().min().unwrap();

        eprintln!("  Compiled: {compile_ok}/{n_samples}");
        eprintln!("  Failed:   {compile_fail}/{n_samples}");
        eprintln!("  WASM size:");
        eprintln!("    min:    {min_size} bytes");
        eprintln!("    median: {median_size} bytes");
        eprintln!("    p90:    {p90_size} bytes");
        eprintln!("    max:    {max_size} bytes");
        eprintln!("  Wasmtime fuel:");
        eprintln!("    min:    {min_fuel}");
        eprintln!("    median: {median_fuel}");
        eprintln!("    p90:    {p90_fuel}");
        eprintln!("    max:    {max_fuel}");
    }

    // Phase 3: Deploy note
    eprintln!("\n── On-Chain Gas ──");
    eprintln!("  Deploy: near contract deploy kampy.testnet use-file <wasm> ...");
    eprintln!("  Call:   near contract call-function kampy.testnet run '' --gas 100Tgas");

    assert!(
        !benchmark_results.is_empty(),
        "No benchmarks compiled successfully"
    );
}
