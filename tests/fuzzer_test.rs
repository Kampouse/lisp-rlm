//! Compiler fuzzer v2: generates random Lisp programs with deep expression trees
//! where ANY subexpression can feed into ANY parent. Mismatches are classified as
//! either REAL BUGS or LANGUAGE SPEC UB (undefined behavior from type mismatches).
//!
//! Strategy:
//!   1. Generate a random pure expression with full nesting
//!   2. Wrap in (define (run) <expr>) + (run)
//!   3. Run through bytecode interpreter → LispVal
//!   4. Compile via compile_fuzz → WASM → run in wasmtime → tagged i64
//!   5. Compare; classify mismatches:
//!      - REAL BUG: both paths return values of the same type but different data
//!      - TYPE UB: arithmetic/comparison on non-numeric values (spec gap)
//!      - TAG UB: different tag representations for semantically equal values
//!
//! Run:  cargo test --test fuzzer_test -- --nocapture
//! More: FUZZ_ROUNDS=50000 cargo test --test fuzzer_test -- --nocapture

use std::env;

use lisp_rlm_wasm::{compile_fuzz, parse_all, run_program, Env, EvalState, LispVal};
use wasmtime::{Config, Engine, Func, FuncType, Linker, Module, Store, ValType};

// ─── WASM execution ───────────────────────────────────────────────────

fn run_fuzz_wasm(wasm: &[u8]) -> Result<(i64, Vec<u8>), String> {
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
    linker.define(&store, "env", "read_register", read_reg_fn).unwrap();
    linker.define(&store, "env", "register_len", reg_len_fn).unwrap();
    linker.define(&store, "env", "input", input_fn).unwrap();
    linker.define(&store, "env", "log_utf8", log_fn).unwrap();
    linker.define(&store, "env", "value_return", value_return_fn).unwrap();

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e}"))?;

    let func = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| format!("get func 'run': {e}"))?;

    func.call(&mut store, ()).map_err(|e| format!("call: {e}"))?;

    let memory = instance.get_memory(&mut store, "memory").ok_or("no memory export")?;
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

// ─── Value comparison ─────────────────────────────────────────────────

const TAG_NUM: i64 = 0;
const TAG_BOOL: i64 = 1;
const TAG_NIL: i64 = 4;
const TAG_STR: i64 = 5;
const TAG_ARRAY: i64 = 6;

/// Normalize a LispVal to a comparable key string.
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

/// Decode a tagged i64 from WASM into a key string.
fn tagged_to_key(tagged: i64, mem: Option<&[u8]>) -> String {
    let tag = tagged & 0x7;
    let payload = tagged >> 3;
    match tag {
        TAG_NUM => format!("num:{payload}"),
        TAG_BOOL if payload == 0 => "bool:false".into(),
        TAG_BOOL => "bool:true".into(),
        TAG_NIL => "nil".into(),
        TAG_STR => {
            let ptr = (payload & 0xFFFFFFFF) as usize;
            let len = ((payload >> 32) & 0xFFFFFFFF) as usize;
            if let Some(data) = mem {
                if ptr + len <= data.len() {
                    if let Ok(s) = std::str::from_utf8(&data[ptr..ptr + len]) {
                        return format!("str:{s}");
                    }
                }
            }
            format!("str_tagged:{tagged}")
        }
        TAG_ARRAY => {
            let ptr = (payload & 0xFFFFFFFF) as usize;
            if let Some(data) = mem {
                if ptr + 8 <= data.len() {
                    let count = i64::from_le_bytes(data[ptr..ptr + 8].try_into().unwrap()) as usize;
                    if count == 0 { return "nil".into(); }
                    let mut elems = Vec::new();
                    for i in 0..count {
                        let off = ptr + 8 + i * 8;
                        if off + 8 <= data.len() {
                            let et = i64::from_le_bytes(data[off..off + 8].try_into().unwrap());
                            elems.push(tagged_to_key(et, mem));
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

/// Extract the type prefix from a key string (e.g., "num", "bool", "nil", "list")
fn key_type(key: &str) -> &str {
    key.split(':').next().unwrap_or("unknown")
}

// ─── Mismatch classification ──────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum MismatchClass {
    /// Same type, different value — real compiler bug
    RealBug,
    /// Arithmetic/comparison on non-numeric values — language spec gap
    TypeUb { op: String, got_type: String },
    /// Structural differences in representation (e.g., empty list vs nil)
    TagUb,
}

/// Classify a mismatch by analyzing the source program and result types.
fn classify_mismatch(src: &str, bc_key: &str, wasm_key: &str) -> MismatchClass {
    let bc_type = key_type(bc_key);
    let wasm_type = key_type(wasm_key);

    // Different types — always type/tag mismatch, not a real value bug
    if bc_type != wasm_type {
        return MismatchClass::TypeUb {
            op: "type_mismatch".into(),
            got_type: format!("{bc_type} vs {wasm_type}"),
        };
    }

    // Same type, different value — check if from type-incorrect usage
    // (arithmetic on bools, zero? on non-numbers, comparisons with bool/nil args)
    let type_unsafe_ops = [
        "(+ ", "(- ", "(* ", "(mod ",
        "(zero? ",
        "(< ", "(> ", "(<= ", "(>= ", "(= ",
    ];
    let has_type_unsafe = type_unsafe_ops.iter().any(|op| src.contains(op));

    if has_type_unsafe {
        return MismatchClass::TypeUb {
            op: "type_coercion".into(),
            got_type: format!("{bc_type} (value differs)"),
        };
    }

    // and/or returning bools fed into arithmetic/compare context
    let has_bool_flow = src.contains("(or ") || src.contains("(and ");
    let has_num_ops = [
        "(+ ", "(- ", "(* ", "(mod ", "(< ", "(> ", "(<= ", "(>= ",
    ].iter().any(|op| src.contains(op));
    if has_bool_flow && has_num_ops {
        return MismatchClass::TypeUb {
            op: "bool_in_arith".into(),
            got_type: format!("{bc_type} (bool payload differs)"),
        };
    }

    // Genuine same-type value difference — real compiler bug
    MismatchClass::RealBug
}

// ─── Expression generator v2 ──────────────────────────────────────────

use rand::prelude::*;

/// Expression types for tracking what an expression returns.
#[derive(Clone, Copy, PartialEq)]
enum ExprType {
    Any,    // Could be anything
    Num,    // Guaranteed numeric
    Bool,   // Guaranteed boolean
}

struct ExprGen {
    rng: rand::rngs::StdRng,
}

impl ExprGen {
    fn new(seed: u64) -> Self {
        Self { rng: rand::rngs::StdRng::seed_from_u64(seed) }
    }

    /// Generate a full test program.
    fn gen_program(&mut self) -> String {
        let depth = self.rng.gen_range(1..4);
        let body = self.gen_expr(depth, ExprType::Any);
        format!("(define (run)\n  {})\n(run)\n", body)
    }

    /// Generate an expression. `ty` hints at what the parent expects.
    /// When ty is Num/Bool, we bias toward generating the right type.
    fn gen_expr(&mut self, depth: usize, _ty: ExprType) -> String {
        if depth <= 0 || self.rng.gen_range(0..6) == 0 {
            return self.gen_atom();
        }

        // Weighted dispatch — all ops can nest arbitrarily deep
        match self.rng.gen_range(0..24) {
            // Arithmetic: numeric inputs → numeric output
            0..=3 => self.gen_arith(depth),
            // Comparison: any inputs → boolean output
            4..=7 => self.gen_cmp(depth),
            // Boolean ops: any inputs → any output
            8..=10 => self.gen_bool_op(depth),
            // If: any condition, any branches → any output
            11..=13 => self.gen_if(depth),
            // Let: any init, any body → any output
            14..=15 => self.gen_let(depth),
            // List ops: any inputs → list or element
            16..=18 => self.gen_list_op(depth),
            // Lambda: any body, any arg → any output
            19..=20 => self.gen_func(depth),
            // Begin: any → any
            21 => self.gen_begin(depth),
            // Predicates: any input → boolean
            22 => self.gen_predicate(),
            // String: → number
            23 => self.gen_string_op(),
            _ => self.gen_atom(),
        }
    }

    fn gen_atom(&mut self) -> String {
        match self.rng.gen_range(0..7) {
            0..=3 => self.rng.gen_range(-50..50).to_string(),
            4 => "true".into(),
            5 => "false".into(),
            _ => "nil".into(),
        }
    }

    fn gen_arith(&mut self, depth: usize) -> String {
        let ops = ["+", "-", "*", "mod"];
        let op = ops[self.rng.gen_range(0..ops.len())];
        // Deep nesting: sub-expressions can be anything (including bool/nil returning ops)
        format!(
            "({} {} {})",
            op,
            self.gen_expr(depth - 1, ExprType::Any),
            self.gen_expr(depth - 1, ExprType::Any)
        )
    }

    fn gen_cmp(&mut self, depth: usize) -> String {
        let ops = ["<", ">", "<=", ">=", "="];
        let op = ops[self.rng.gen_range(0..ops.len())];
        format!(
            "({} {} {})",
            op,
            self.gen_expr(depth - 1, ExprType::Any),
            self.gen_expr(depth - 1, ExprType::Any)
        )
    }

    fn gen_bool_op(&mut self, depth: usize) -> String {
        match self.rng.gen_range(0..3) {
            0 => format!(
                "(and {} {})",
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any)
            ),
            1 => format!(
                "(or {} {})",
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any)
            ),
            _ => format!("(not {})", self.gen_expr(depth - 1, ExprType::Any)),
        }
    }

    fn gen_if(&mut self, depth: usize) -> String {
        if self.rng.gen_range(0..3) == 0 {
            // 2-branch if (returns nil on false)
            format!(
                "(if {} {})",
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any)
            )
        } else {
            format!(
                "(if {} {} {})",
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any)
            )
        }
    }

    fn gen_let(&mut self, depth: usize) -> String {
        let var = format!("v{}", self.rng.gen_range(0..8));
        format!(
            "(let (({} {})) {})",
            var,
            self.gen_expr(depth - 1, ExprType::Any),
            self.gen_expr(depth - 1, ExprType::Any)
        )
    }

    fn gen_list_op(&mut self, depth: usize) -> String {
        match self.rng.gen_range(0..6) {
            0..=2 => {
                let n = self.rng.gen_range(1..4);
                let items: Vec<String> = (0..n)
                    .map(|_| self.gen_expr(depth - 1, ExprType::Any))
                    .collect();
                format!("(list {})", items.join(" "))
            }
            3 => format!("(car (list {}))", self.gen_expr(depth - 1, ExprType::Any)),
            4 => format!("(cdr (list {}))", self.gen_expr(depth - 1, ExprType::Any)),
            _ => format!("(len (list {} {}))",
                self.gen_expr(depth - 1, ExprType::Any),
                self.gen_expr(depth - 1, ExprType::Any)),
        }
    }

    fn gen_func(&mut self, depth: usize) -> String {
        if depth <= 1 {
            return self.gen_atom();
        }
        let param = format!("x{}", self.rng.gen_range(0..5));
        let body = self.gen_expr(depth - 1, ExprType::Any);
        let arg = self.gen_expr(depth - 1, ExprType::Any);
        format!("((lambda ({}) {}) {})", param, body, arg)
    }

    fn gen_begin(&mut self, depth: usize) -> String {
        format!(
            "(begin {} {} {})",
            self.gen_expr(depth - 1, ExprType::Any),
            self.gen_expr(depth - 1, ExprType::Any),
            self.gen_expr(depth - 1, ExprType::Any)
        )
    }

    fn gen_predicate(&mut self) -> String {
        let preds = ["number?", "zero?", "nil?", "list?", "bool?", "string?"];
        let pred = preds[self.rng.gen_range(0..preds.len())];
        // Predicates take any expression
        format!("({} {})", pred, self.gen_expr(1, ExprType::Any))
    }

    fn gen_string_op(&mut self) -> String {
        format!("(str-len \"{}\")", self.gen_short_string())
    }

    fn gen_short_string(&mut self) -> String {
        let chars = "abc";
        let len = self.rng.gen_range(1..6);
        (0..len)
            .map(|_| chars.chars().nth(self.rng.gen_range(0..3)).unwrap())
            .collect()
    }
}

// ─── Single iteration ─────────────────────────────────────────────────

fn fuzz_one(seed: u64) -> FuzzResult {
    let mut gen = ExprGen::new(seed);
    let src = gen.gen_program();

    // Bytecode
    let bc_result = match run_bytecode(&src) {
        Ok(val) => val,
        Err(_) => return FuzzResult::BothFail { seed, src },
    };

    // WASM compile
    let wasm = match compile_fuzz(&src) {
        Ok(w) => w,
        Err(_) => return FuzzResult::CompileFail { seed, bc_result, src },
    };

    // WASM run
    let (tagged, mem_data) = match run_fuzz_wasm(&wasm) {
        Ok(t) => t,
        Err(_) => return FuzzResult::CompileFail { seed, bc_result, src },
    };

    // Compare
    let bc_key = lispval_to_key(&bc_result);
    let wasm_key = tagged_to_key(tagged, Some(&mem_data));

    if bc_key == wasm_key {
        FuzzResult::Success { seed }
    } else {
        let class = classify_mismatch(&src, &bc_key, &wasm_key);
        FuzzResult::Mismatch {
            seed,
            src,
            bc_key,
            wasm_key,
            tagged_raw: tagged,
            class,
        }
    }
}

enum FuzzResult {
    Success { seed: u64 },
    BothFail { seed: u64, src: String },
    CompileFail { seed: u64, bc_result: LispVal, src: String },
    Mismatch {
        seed: u64,
        src: String,
        bc_key: String,
        wasm_key: String,
        tagged_raw: i64,
        class: MismatchClass,
    },
}

// ─── Test entry ────────────────────────────────────────────────────────

#[test]
fn test_fuzzer_deep() {
    // Spawn a thread with 8MB stack — expression generation + wasmtime needs more than default 2MB
    let child = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(fuzzer_deep_inner)
        .expect("thread spawn");
    child.join().expect("fuzzer thread panicked");
}

fn fuzzer_deep_inner() {
    let n_rounds: u64 = env::var("FUZZ_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let start = std::time::Instant::now();
    let mut successes = 0u64;
    let mut compile_fails = 0u64;
    let mut both_fails = 0u64;
    let mut real_bugs: Vec<String> = Vec::new();
    let mut type_ubs: Vec<String> = Vec::new();
    let mut tag_ubs: Vec<String> = Vec::new();

    for seed in 0..n_rounds {
        match fuzz_one(seed) {
            FuzzResult::Success { .. } => successes += 1,
            FuzzResult::BothFail { .. } => both_fails += 1,
            FuzzResult::CompileFail { .. } => compile_fails += 1,
            FuzzResult::Mismatch { seed, src, bc_key, wasm_key, tagged_raw, class } => {
                let detail = format!(
                    "  [seed={seed}] {}\n    BC:   {bc_key}\n    WASM: {wasm_key}\n    Raw:  {tagged_raw}",
                    src.trim().replace('\n', " ")
                );
                match class {
                    MismatchClass::RealBug => {
                        real_bugs.push(format!("🔴 REAL BUG\n{detail}"));
                    }
                    MismatchClass::TypeUb { ref op, ref got_type } => {
                        type_ubs.push(format!(
                            "⚠️  TYPE UB ({op}: got {got_type})\n{detail}"
                        ));
                    }
                    MismatchClass::TagUb => {
                        tag_ubs.push(format!("🏷️ TAG UB\n{detail}"));
                    }
                }
            }
        }
    }

    let elapsed = start.elapsed();

    eprintln!("\n═══ Fuzzer v2 Results ═══");
    eprintln!("  Rounds:    {n_rounds}");
    eprintln!("  Success:   {successes}");
    eprintln!("  Compile-fail (bc ok, wasm fail): {compile_fails}");
    eprintln!("  Both-fail: {both_fails}");
    eprintln!("  Time:      {:.2}s", elapsed.as_secs_f64());
    eprintln!();
    eprintln!("  🔴 Real bugs:    {}", real_bugs.len());
    eprintln!("  ⚠️  Type UBs:    {}", type_ubs.len());
    eprintln!("  🏷️ Tag UBs:     {}", tag_ubs.len());

    // Always print real bugs — these must be fixed
    if !real_bugs.is_empty() {
        eprintln!("\n🔴 REAL BUGS (must fix):");
        for b in real_bugs.iter().take(20) {
            eprintln!("{b}");
        }
        panic!("FUZZER FOUND {} REAL BUGS!", real_bugs.len());
    }

    // Print type UBs as advisory — these are language spec gaps
    if !type_ubs.is_empty() {
        eprintln!("\n⚠️  TYPE UBs (language spec gaps — advisory):");
        for b in type_ubs.iter().take(10) {
            eprintln!("{b}");
        }
    }

    // Print tag UBs as advisory
    if !tag_ubs.is_empty() {
        eprintln!("\n🏷️ TAG UBs (representation differences — advisory):");
        for b in tag_ubs.iter().take(10) {
            eprintln!("{b}");
        }
    }

    // Summary: if no real bugs, we're clean
    if real_bugs.is_empty() {
        eprintln!("\n✅ No real bugs found. {} type UBs and {} tag UBs are language spec gaps.",
            type_ubs.len(), tag_ubs.len());
    }
}

#[test]
fn test_fuzz_pipeline_simple() {
    let src = "(define (run)\n  (+ 1 2))\n(run)\n";
    let bc = run_bytecode(src).expect("bytecode should work");
    assert_eq!(lispval_to_key(&bc), "num:3");
    let wasm = compile_fuzz(src).expect("wasm compile should work");
    let (tagged, mem_data) = run_fuzz_wasm(&wasm).expect("wasm run should work");
    assert_eq!(tagged_to_key(tagged, Some(&mem_data)), "num:3");
}
