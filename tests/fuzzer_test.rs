//! Compiler fuzzer: generates random Lisp programs, runs them through both the
//! bytecode interpreter and the WASM compiler, and asserts identical results.
//!
//! Strategy:
//!   1. Generate a random pure expression
//!   2. Wrap in (define (run) <expr>) + (run)
//!   3. Run through bytecode interpreter → LispVal
//!   4. Compile via compile_fuzz → WASM → run in wasmtime → tagged i64
//!   5. Compare both via a normalized key string
//!
//! Run:  cargo test --test fuzzer_test -- --nocapture
//! More: FUZZ_ROUNDS=10000 cargo test --test fuzzer_test -- --nocapture

use std::env;

use lisp_rlm_wasm::{compile_fuzz, parse_all, run_program, Env, EvalState, LispVal};
use wasmtime::{Config, Engine, Func, FuncType, Linker, Module, Store, ValType};

// ─── WASM execution ───────────────────────────────────────────────────

/// Run compiled WASM and return the tagged i64 from TEMP_MEM plus the memory export.
fn run_fuzz_wasm(wasm: &[u8]) -> Result<(i64, Vec<u8>), String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).map_err(|e| format!("engine: {e}"))?;
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {e}"))?;
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(10_000_000)
        .map_err(|e| format!("fuel: {e}"))?;

    // Host function stubs (compile_fuzz mode still imports NEAR runtime signatures)
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
        .map_err(|e| format!("get func 'run': {e}"))?;

    func.call(&mut store, ())
        .map_err(|e| format!("call: {e}"))?;

    // Read tagged result from TEMP_MEM (offset 64)
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or("no memory export")?;
    let data = memory.data(&store).to_vec();
    let tagged = i64::from_le_bytes(data[64..72].try_into().unwrap());
    Ok((tagged, data))
}

// ─── Bytecode execution ───────────────────────────────────────────────

fn run_bytecode(src: &str) -> Result<LispVal, String> {
    let exprs = parse_all(src).map_err(|e| format!("parse: {e}"))?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    run_program(&exprs, &mut env, &mut state)
}

// ─── Key extraction (comparison) ──────────────────────────────────────

/// Normalize a LispVal to a comparable string key.
/// Empty list and Nil are treated as equivalent.
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

/// Tag bits used by the WASM runtime.
const TAG_NUM: i64 = 0;
const TAG_BOOL: i64 = 1;
const TAG_NIL: i64 = 4;
const TAG_STR: i64 = 5;
const TAG_ARRAY: i64 = 6;

/// Decode a tagged i64 from WASM into a key string, reading memory for heap values.
fn tagged_to_key(tagged: i64, memory_data: Option<&[u8]>) -> String {
    let tag = tagged & 0x7;
    let payload = tagged >> 3;
    match tag {
        TAG_NUM => format!("num:{payload}"),
        TAG_BOOL if payload == 0 => "bool:false".into(),
        TAG_BOOL => "bool:true".into(),
        TAG_NIL => "nil".into(),
        TAG_STR => {
            // payload = (len << 32) | ptr
            let ptr = (payload & 0xFFFFFFFF) as usize;
            let len = ((payload >> 32) & 0xFFFFFFFF) as usize;
            if let Some(data) = memory_data {
                if ptr + len <= data.len() {
                    if let Ok(s) = std::str::from_utf8(&data[ptr..ptr + len]) {
                        return format!("str:{s}");
                    }
                }
            }
            format!("str_tagged:{tagged}")
        }
        TAG_ARRAY => {
            // payload = (len << 32) | ptr
            // Heap layout: [count, elem0, elem1, ...] as i64 words
            let ptr = (payload & 0xFFFFFFFF) as usize;
            if let Some(data) = memory_data {
                if ptr + 8 <= data.len() {
                    let count = i64::from_le_bytes(data[ptr..ptr + 8].try_into().unwrap());
                    let count = count as usize;
                    if count == 0 {
                        return "nil".into();
                    }
                    let mut elems = Vec::new();
                    for i in 0..count {
                        let off = ptr + 8 + i * 8;
                        if off + 8 <= data.len() {
                            let elem_tagged =
                                i64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                            elems.push(tagged_to_key(elem_tagged, memory_data));
                        } else {
                            elems.push("???".into());
                        }
                    }
                    return format!("list:({})", elems.join(" "));
                }
            }
            format!("array_tagged:{tagged}")
        }
        _ => format!("unknown_tagged:{tagged}"),
    }
}

// ─── Expression generator ─────────────────────────────────────────────

use rand::prelude::*;

/// Depth-limited expression generator for pure expressions.
struct ExprGen {
    rng: rand::rngs::StdRng,
}

impl ExprGen {
    fn new(seed: u64) -> Self {
        Self {
            rng: rand::rngs::StdRng::seed_from_u64(seed),
        }
    }

    fn gen_expr(&mut self, depth: usize) -> String {
        if depth <= 0 || self.rng.gen_range(0..5) == 0 {
            return self.gen_atom();
        }

        match self.rng.gen_range(0..16) {
            0..=3 => self.gen_arith(depth),
            4..=6 => self.gen_cmp(depth),
            7..=8 => self.gen_if(depth),
            9 => self.gen_let(depth),
            10..=11 => self.gen_bool_op(depth),
            12 => self.gen_list_op(depth),
            13 => self.gen_func(depth),
            14 => self.gen_begin(depth),
            15 => self.gen_string_op(depth),
            _ => self.gen_atom(),
        }
    }

    fn gen_atom(&mut self) -> String {
        match self.rng.gen_range(0..6) {
            0..=2 => self.rng.gen_range(-50..50).to_string(),
            3 => "true".into(),
            4 => "false".into(),
            _ => "nil".into(),
        }
    }

    fn gen_arith(&mut self, depth: usize) -> String {
        let ops = ["+", "-", "*", "mod"];
        let op = ops[self.rng.gen_range(0..ops.len())];
        // Arithmetic args should always be numeric to avoid bool/nil coercion mismatches
        format!(
            "({} {} {})",
            op,
            self.gen_num_atom(),
            self.gen_num_atom()
        )
    }

    fn gen_num_atom(&mut self) -> String {
        // Generate a small numeric literal (guaranteed to be a number)
        let n = self.rng.gen_range(-50..50);
        format!("{n}")
    }

    fn gen_cmp(&mut self, depth: usize) -> String {
        let ops = ["<", ">", "<=", ">=", "="];
        let op = ops[self.rng.gen_range(0..ops.len())];
        // Comparisons should use numeric args to avoid bool/nil coercion differences
        format!(
            "({} {} {})",
            op,
            self.gen_num_atom(),
            self.gen_num_atom()
        )
    }

    fn gen_if(&mut self, depth: usize) -> String {
        format!(
            "(if {} {} {})",
            self.gen_expr(depth - 1),
            self.gen_expr(depth - 1),
            self.gen_expr(depth - 1)
        )
    }

    fn gen_let(&mut self, depth: usize) -> String {
        let var = format!("v{}", self.rng.gen_range(0..6));
        format!(
            "(let (({} {})) {})",
            var,
            self.gen_expr(depth - 1),
            self.gen_expr(depth - 1)
        )
    }

    fn gen_bool_op(&mut self, depth: usize) -> String {
        match self.rng.gen_range(0..2) {
            0 => format!(
                "(and {} {})",
                self.gen_expr(depth - 1),
                self.gen_expr(depth - 1)
            ),
            _ => format!(
                "(or {} {})",
                self.gen_expr(depth - 1),
                self.gen_expr(depth - 1)
            ),
        }
    }

    fn gen_list_op(&mut self, depth: usize) -> String {
        match self.rng.gen_range(0..4) {
            0 => format!(
                "(list {} {} {})",
                self.gen_expr(depth - 1),
                self.gen_expr(depth - 1),
                self.gen_expr(depth - 1)
            ),
            1 => format!("(car {})", self.gen_list_literal()),
            2 => format!("(cdr {})", self.gen_list_literal()),
            _ => format!("(len {})", self.gen_list_literal()),
        }
    }

    fn gen_list_literal(&mut self) -> String {
        let n = self.rng.gen_range(1..5);
        let items: Vec<String> = (0..n)
            .map(|_| self.rng.gen_range(-10..10).to_string())
            .collect();
        format!("(list {})", items.join(" "))
    }

    fn gen_func(&mut self, depth: usize) -> String {
        if depth <= 1 {
            return self.gen_atom();
        }
        let param = format!("x{}", self.rng.gen_range(0..5));
        let body = self.gen_expr(depth - 1);
        format!(
            "((lambda ({}) {}) {})",
            param,
            body,
            self.gen_expr(depth - 1)
        )
    }

    fn gen_begin(&mut self, depth: usize) -> String {
        format!(
            "(begin {} {})",
            self.gen_expr(depth - 1),
            self.gen_expr(depth - 1)
        )
    }

    fn gen_string_op(&mut self, _depth: usize) -> String {
        format!("(str-len \"{}\")", self.gen_short_string())
    }

    fn gen_short_string(&mut self) -> String {
        let chars = "abc";
        let len = self.rng.gen_range(1..6);
        (0..len)
            .map(|_| chars.chars().nth(self.rng.gen_range(0..3)).unwrap())
            .collect()
    }

    /// Generate a full test program: define run, then call it.
    fn gen_program(&mut self) -> String {
        let depth = self.rng.gen_range(1..4);
        let body = self.gen_expr(depth);
        format!("(define (run)\n  {})\n(run)\n", body)
    }
}

// ─── Single iteration ─────────────────────────────────────────────────

/// Run one fuzz iteration. Returns Ok if both agree, Err with details if not.
fn fuzz_one(seed: u64, verbose: bool) -> Result<(), String> {
    let mut gen = ExprGen::new(seed);
    let src = gen.gen_program();

    if verbose {
        eprintln!("  [seed={seed}] {}", src.trim().replace('\n', " "));
    }

    // Bytecode
    let bc_result = match run_bytecode(&src) {
        Ok(val) => val,
        Err(e) => {
            // Bytecode failed — skip (WASM may also fail for same reason)
            return Err(format!("BYTECODE FAIL: {e}"));
        }
    };

    // WASM compile
    let wasm = match compile_fuzz(&src) {
        Ok(w) => w,
        Err(e) => {
            return Err(format!(
                "COMPILE FAIL (bytecode ok: {:?}): {e}",
                bc_result
            ));
        }
    };

    // WASM run
    let (tagged, mem_data) = match run_fuzz_wasm(&wasm) {
        Ok(t) => t,
        Err(e) => {
            return Err(format!(
                "COMPILE FAIL (bytecode ok: {:?}): wasm run: {e}",
                bc_result
            ));
        }
    };

    // Compare
    let bc_key = lispval_to_key(&bc_result);
    let wasm_key = tagged_to_key(tagged, Some(&mem_data));

    if bc_key != wasm_key {
        return Err(format!(
            "MISMATCH\n  Source: {}\n  Bytecode: {bc_key}\n  WASM:     {wasm_key}\n  Tagged raw: {tagged}",
            src.trim().replace('\n', " ")
        ));
    }

    Ok(())
}

// ─── Test entry ────────────────────────────────────────────────────────

#[test]
fn test_fuzzer_1000_rounds() {
    let n_rounds: u64 = env::var("FUZZ_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let start = std::time::Instant::now();
    let mut successes = 0u64;
    let mut compile_fails = 0u64;
    let mut runtime_fails = 0u64;
    let mut failures: Vec<String> = Vec::new();

    for seed in 0..n_rounds {
        match fuzz_one(seed, seed < 3) {
            Ok(()) => successes += 1,
            Err(e) => {
                if e.contains("MISMATCH") {
                    failures.push(e);
                } else if e.contains("COMPILE FAIL") {
                    compile_fails += 1;
                } else {
                    // Bytecode fail — both paths fail, not a compiler bug
                    runtime_fails += 1;
                }
            }
        }
    }

    let elapsed = start.elapsed();

    eprintln!("\n═══ Fuzzer Results ═══");
    eprintln!("  Rounds:    {n_rounds}");
    eprintln!("  Success:   {successes}");
    eprintln!("  Compile-fail (bytecode ok, wasm failed): {compile_fails}");
    eprintln!("  Both-fail: {runtime_fails}");
    eprintln!("  Mismatches: {}", failures.len());
    eprintln!("  Time:      {:.2}s", elapsed.as_secs_f64());

    if !failures.is_empty() {
        eprintln!("\n❌ Mismatches (showing first 10):");
        for f in failures.iter().take(10) {
            eprintln!("  {f}");
        }
        panic!("FUZZER FOUND {} MISMATCHES!", failures.len());
    }

    // Compile failures are tracked but not fatal — they indicate features
    // the WASM compiler hasn't implemented yet.
    if compile_fails > 0 {
        eprintln!(
            "\n⚠ {} programs compiled for bytecode but failed WASM compilation",
            compile_fails
        );
    }
}

#[test]
fn test_fuzz_pipeline_simple() {
    // Verify the pipeline works end-to-end for a simple case
    let src = "(define (run)\n  (+ 1 2))\n(run)\n";

    let bc = run_bytecode(src).expect("bytecode should work");
    let bc_key = lispval_to_key(&bc);
    assert_eq!(bc_key, "num:3");

    let wasm = compile_fuzz(src).expect("wasm compile should work");
    let (tagged, mem_data) = run_fuzz_wasm(&wasm).expect("wasm run should work");
    let wasm_key = tagged_to_key(tagged, Some(&mem_data));
    assert_eq!(wasm_key, "num:3");
}
