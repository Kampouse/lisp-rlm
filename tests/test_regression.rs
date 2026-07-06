//! Comprehensive regression test suite for lisp-rlm WASM emission.
//!
//! Three tiers:
//!   1. COMPILE — lisp → WASM, wasm-tools validate, size bounds
//!   2. RUNTIME — execute via wasmtime with mock host functions, verify output
//!   3. OUTLAYER_E2E — (manual) compile → upload → run on OutLayer mainnet
//!
//! Run all:           cargo test --test test_regression
//! Run tier 1 only:   cargo test --test test_regression compile::
//! Run tier 2 only:   cargo test --test test_regression runtime::
//! Run tier 3:        cargo test --test test_regression outlayer_e2e:: -- --ignored
//!
//! REGRESSION RULE: Any change that breaks a passing test is a regression.
//! To update baselines after intentional changes, update the assertion values.

use std::process::Command;

// ============================================================================
// Shared helpers
// ============================================================================

/// Compile lisp source to outlayer P2 component WASM (via library).
fn compile_p2(src: &str) -> Vec<u8> {
    lisp_rlm_wasm::compile_outlayer_p2(src)
        .unwrap_or_else(|e| panic!("compile_p2 failed: {}", e))
}

/// Compile lisp source to outlayer P1 core WASM (via library).
fn compile_p1(src: &str) -> Vec<u8> {
    lisp_rlm_wasm::compile_outlayer(src)
        .unwrap_or_else(|e| panic!("compile_p1 failed: {}", e))
}

/// Validate WASM with wasm-tools (returns WAT on success).
fn validate(wasm: &[u8], label: &str) -> String {
    let tmp = std::env::temp_dir().join(format!("regress_{}.wasm", label));
    std::fs::write(&tmp, wasm).unwrap_or_else(|e| panic!("write {}: {}", label, e));

    let out = Command::new("wasm-tools")
        .args(["validate", tmp.to_str().unwrap()])
        .output()
        .unwrap_or_else(|e| panic!("wasm-tools spawn: {}", e));

    if !out.status.success() {
        panic!(
            "{}: wasm-tools validate FAILED:\n{}",
            label,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let print = Command::new("wasm-tools")
        .args(["print", tmp.to_str().unwrap()])
        .output()
        .unwrap_or_else(|e| panic!("wasm-tools print spawn: {}", e));

    let _ = std::fs::remove_file(&tmp);
    String::from_utf8_lossy(&print.stdout).to_string()
}

/// Count functions in a WAT string.
fn count_funcs(wat: &str) -> usize {
    wat.lines().filter(|l| l.trim().starts_with("(func ")).count()
}

/// Extract data segment i32.const offsets from WAT.
fn data_offsets(wat: &str) -> Vec<u32> {
    let mut offsets = Vec::new();
    for line in wat.lines() {
        let l = line.trim();
        if l.starts_with("(data ") {
            if let Some(pos) = l.find("i32.const") {
                let rest = &l[pos + 10..];
                if let Some(end) = rest.find(')') {
                    if let Ok(v) = rest[..end].trim().parse::<u32>() {
                        offsets.push(v);
                    }
                }
            }
        }
    }
    offsets
}

// ============================================================================
// TIER 1: Compile + Validate
// ============================================================================

mod compile {
    use super::*;

    /// Macro: compile P2 source, validate with wasm-tools, check size bounds.
    macro_rules! p2_test {
        ($name:ident, $code:expr $(, $size_le:expr)?) => {
            #[test]
            fn $name() {
                let wasm = compile_p2($code);
                assert!(wasm.len() > 100,
                    "{}: WASM too small ({} bytes)", stringify!($name), wasm.len());
                $(
                    assert!(wasm.len() <= $size_le,
                        "{}: WASM bloated ({} > {} bytes)",
                        stringify!($name), wasm.len(), $size_le);
                )?
                let wat = validate(&wasm, stringify!($name));
                let n = count_funcs(&wat);
                assert!(n > 5, "{}: too few functions ({})", stringify!($name), n);
            }
        };
    }

    // --- Minimal programs ---

    p2_test!(p2_const, "(define (main) 42)", 20_000);
    p2_test!(p2_string, r#"(define (main) "hello")"#, 20_000);
    p2_test!(p2_arithmetic, "(define (main) (+ (* 3 4) (- 10 2)))", 20_000);

    // --- let* binding (runtime values, NOT define) ---

    p2_test!(p2_let_star, r#"
(define (main)
  (let* ((x 10) (y (+ x 5)))
    y))
"#, 20_000);

    // --- Conditionals ---

    p2_test!(p2_if, r#"
(define (main)
  (if (> 5 3) 1 0))
"#, 20_000);

    p2_test!(p2_nested_if, r#"
(define (main)
  (if (= 0 0)
    (if (> 1 0) 42 0)
    99))
"#, 20_000);

    // --- Recursion ---

    p2_test!(p2_fibonacci, r#"
(define (fib n)
  (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
"#, 20_000);

    // --- String operations ---

    p2_test!(p2_str_len, r#"(define (main) (str-len "hello"))"#, 20_000);
    p2_test!(p2_str_cat, r#"(define (main) (str-cat "hello" " world"))"#, 20_000);
    p2_test!(p2_str_cat_multi, r#"(define (main) (str-cat "a" "b" "c"))"#, 20_000);

    // --- JSON operations ---

    p2_test!(p2_json_get, r#"(define (main) (json-get "{\"x\":1}" "x"))"#, 25_000);
    p2_test!(p2_json_get_str, r#"(define (main) (json-get-str "{\"name\":\"bob\"}" "name"))"#, 25_000);

    // --- HTTP GET (no network, compilation only) ---

    p2_test!(p2_http_get, r#"
(define (run)
  (http-get "https://api.example.com/price"))
"#, 25_000);

    // --- HTTP POST (RPC call pattern) ---

    p2_test!(p2_http_post, r#"
(define (run)
  (http-post "https://rpc.mainnet.fastnear.com"
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"query\"}"
    "application/json"))
"#, 25_000);

    // --- outlayer/view ---

    p2_test!(p2_outlayer_view, r#"
(define (run)
  (outlayer/view "contract.near" "get_price" "{}"))
"#, 25_000);

    // --- outlayer/transfer ---

    p2_test!(
        p2_outlayer_transfer,
        r#"
(define (run)
  (outlayer/transfer "signer.near" "ed25519:key" "receiver.near" "10000000000000000000"))
"#,
        30_000
    );

    // --- outlayer/call ---

    // outlayer/call is P1 only — tested in P1 section

    // --- outlayer/storage-set + storage-get ---

    p2_test!(
        p2_outlayer_storage_set_get,
        r#"
(define (run)
  (let* ((result (outlayer/storage-set "test_key" "hello_storage"))
         (read (outlayer/storage-get "test_key")))
    read))
"#,
        30_000
    );

    // --- near/log ---

    // near/log is P1 (env.log_utf8) — test in P1 compile section below

    // --- Storage operations ---

    // storage/set, storage/get are P1 outlayer builtins



    // --- Combined pipeline: prices fetch + JSON parse ---

    p2_test!(p2_http_and_let, r#"
(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price")))
    prices))
"#, 30_000);

    p2_test!(
        p2_http_json_pipeline,
        r#"
(define (run)
  (let* ((body (http-get "https://api.example.com/data"))
         (result (str-cat "got: " body)))
    result))
"#,
        30_000
    );

    #[test]
    fn p2_component_version() {
        let wasm = compile_p2("(define (main) 42)");
        assert!(wasm.len() >= 6, "WASM too short for version check");
        // Component version: 0x0D 0x01 (module is 0x01 0x00)
        assert_eq!(&wasm[4..6], &[0x0D, 0x00],
            "P2 component version should be 0x0D 0x00, got {:02x} {:02x}",
            wasm[4], wasm[5]);
    }

    #[test]
    fn p2_data_offset_sanity() {
        let wasm = compile_p2(r#"(define (run) (http-get "https://example.com"))"#);
        let wat = validate(&wasm, "data_offset");
        let offsets = data_offsets(&wat);
        assert!(!offsets.is_empty(), "should have data segments");
        let min_off = *offsets.iter().min().unwrap();
        assert!(min_off >= 48,
            "minimum data offset should be >= 48, got {}", min_off);
    }

    #[test]
    fn p2_http_imports_present() {
        let wasm = compile_p2(r#"(define (run) (http-get "https://example.com"))"#);
        let wat = validate(&wasm, "http_imports");
        assert!(wat.contains("wasi:http"),
            "HTTP component should import wasi:http, got:\n{}", &wat[..wat.len().min(500)]);
    }

    #[test]
    fn p2_no_http_still_valid() {
        let wasm = compile_p2("(define (main) 42)");
        validate(&wasm, "no_http");
    }

    // --- P1 compile tests (core module, not component) ---

    #[test]
    fn p1_const_validates() {
        let wasm = compile_p1("(define (main) 42)");
        validate(&wasm, "p1_const");
        // P1 is a core module, version 0x01 0x00
        assert_eq!(&wasm[4..6], &[0x01, 0x00],
            "P1 should be core module version 0x01 0x00");
    }

    #[test]
    fn p1_http_get_validates() {
        let wasm = compile_p1(r#"(define (run) (http-get "https://example.com"))"#);
        validate(&wasm, "p1_http_get");
    }

    #[test]
    fn p1_outlayer_view_validates() {
        let wasm = compile_p1(r#"(define (run) (outlayer/view "ref.near" "get_price" "{}"))"#);
        validate(&wasm, "p1_view");
    }

    #[test]
    fn p1_combined_pipeline_validates() {
        let wasm = compile_p1(r#"
(define (run)
  (let* ((body (http-get "https://api.example.com/data"))
         (result (str-cat "got: " body)))
    result))
"#);
        validate(&wasm, "p1_combined");
        assert!(wasm.len() > 5000, "combined pipeline should be non-trivial");
    }
}

// ============================================================================
// REGRESSION: WASM size guards — catch unexpected bloat
// ============================================================================
#[cfg(test)]
mod regression {
    use super::*;

    #[test]
    fn p2_minimal_size() {
        // Simplest possible P2 program should stay small
        let wasm = compile_p2("(define (run) 42)");
        assert!(wasm.len() < 80_000,
            "P2 minimal WASM bloated to {} bytes (limit 80KB)", wasm.len());
    }

    #[test]
    fn p2_http_size() {
        // HTTP program with one call — tracks codegen size
        let wasm = compile_p2(r#"(define (run) (http-get "https://example.com"))"#);
        assert!(wasm.len() < 150_000,
            "P2 HTTP WASM bloated to {} bytes (limit 150KB)", wasm.len());
    }

    #[test]
    fn p1_minimal_size() {
        let wasm = compile_p1("(define (main) 42)");
        assert!(wasm.len() < 10_000,
            "P1 minimal WASM bloated to {} bytes (limit 10KB)", wasm.len());
    }

    #[test]
    fn near_minimal_size() {
        let wasm = lisp_rlm_wasm::compile_near("(define (main) 42)").unwrap();
        assert!(wasm.len() < 5_000,
            "NEAR minimal WASM bloated to {} bytes (limit 5KB)", wasm.len());
    }

    #[test]
    fn near_const_return_value() {
        // Verify NEAR target returns correct value
        let wasm = lisp_rlm_wasm::compile_near("(define (main) 42)").unwrap();
        let wat = validate(&wasm, "near_const");
        // TAG_NUM encoding: 42 << 3 = 336
        assert!(wat.contains("i64.const 336"),
            "NEAR const 42 should be tagged as i64.const 336, got:\\n{}{}",& wat[..wat.len().min(200)],"");
    }
}

// ============================================================================
// TIER 2: Runtime via wasmtime mock (P1 core module)
// ============================================================================

mod runtime {
    use super::*;
    use wasmtime::*;

    /// Run a P1 outlayer WASM with full mock host functions.
    /// Returns the i64 at memory offset 65536 (return buffer).
    fn run_p1(src: &str) -> i64 {
        let wasm = compile_p1(src);
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).expect("module load");
        let mut store = Store::new(&engine, ());

        // --- WASI stubs ---
        let fd_read = Func::new(&mut store,
            FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
            |_c, _a, r| { r[0] = Val::I32(0); Ok(()) });
        let fd_write = Func::new(&mut store,
            FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
            |_c, a, r| { r[0] = Val::I32(a[2].unwrap_i32()); Ok(()) });
        let proc_exit = Func::new(&mut store,
            FuncType::new(&engine, vec![ValType::I32], vec![]),
            |_, a, _| Err(Error::msg(format!("proc_exit({})", a[0].unwrap_i32()))));

        // --- NEAR env stubs ---
        let log_fn = Func::new(&mut store,
            FuncType::new(&engine, vec![ValType::I64; 2], vec![]),
            |_, _, _| Ok(()));
        let noop_i64 = Func::wrap(&mut store, |_: i64| {});
        let noop_i64_i64 = Func::wrap(&mut store, |_: i64, _: i64| {});
        let noop_i32_i64 = Func::wrap(&mut store, |_: i32, _: i64| {});
        let noop_i32_i32_to_i32 = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
        let noop_i64_to_i64 = Func::wrap(&mut store, |_: i64| -> i64 { 0 });

        // --- outlayer host stubs ---
        let ol_view = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
        let ol_call = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32,
             _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
        let ol_transfer = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32,
             _: i32, _: i32, _: i32| {});
        let ol_http_get = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
        let ol_http_post = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
        let ol_store = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32| {});
        let ol_load = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32| {});
        let ol_remove = Func::wrap(&mut store, |_: i32, _: i64, _: i32| {});
        let ol_has = Func::wrap(&mut store, |_: i32, _: i64, _: i32| -> i32 { 0 });

        // --- near:rpc/api stubs ---
        let rpc_view = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
        let rpc_call = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32,
             _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
        let rpc_transfer = Func::wrap(&mut store,
            |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32,
             _: i32, _: i32, _: i32| {});

        // --- near:storage/api stubs ---
        let st_set = Func::wrap(&mut store,
            |_: i32, _: i64, _: i32, _: i64, _: i32| {});
        let st_get = Func::wrap(&mut store, |_: i32, _: i64, _: i32| {});
        let st_has = Func::wrap(&mut store, |_: i32, _: i64| -> i32 { 0 });
        let st_del = Func::wrap(&mut store, |_: i32, _: i64| -> i32 { 0 });
        let st_incr = Func::wrap(&mut store, |_: i32, _: i32, _: i64, _: i32| {});
        let st_decr = Func::wrap(&mut store, |_: i32, _: i32, _: i64, _: i32| {});

        // --- Additional stubs for linker (created after main stubs) ---
        let storage_write = Func::wrap(&mut store, |_: u32, _: u32, _: u32, _: u32, _: u32| -> u32 { 0 });
        let promise_create = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32, _: i32| {});
        let promise_and = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
        let promise_then = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32, _: i32| {});
        let promise_result = Func::wrap(&mut store, |_: i32, _: i32, _: i32| {});

        // --- Link everything ---
        let mut linker = Linker::new(&engine);

        // WASI
        linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read).unwrap();
        linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write).unwrap();
        linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit).unwrap();
        linker.define(&store, "wasi_snapshot_preview1", "random_get", noop_i32_i32_to_i32).unwrap();
        linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", noop_i32_i32_to_i32).unwrap();
        linker.define(&store, "wasi_snapshot_preview1", "environ_get", noop_i32_i32_to_i32).unwrap();
        let fd_seek = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });
        linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek).unwrap();

        // NEAR env
        linker.define(&store, "env", "log_utf8", log_fn).unwrap();
        linker.define(&store, "env", "log", noop_i64).unwrap();
        linker.define(&store, "env", "log_s", noop_i64).unwrap();
        linker.define(&store, "env", "read_register", noop_i64_i64).unwrap();
        linker.define(&store, "env", "register_len", noop_i64_to_i64).unwrap();
        linker.define(&store, "env", "account_balance", noop_i64).unwrap();
        linker.define(&store, "env", "attached_deposit", noop_i64).unwrap();
        linker.define(&store, "env", "predecessor_account_id", noop_i32_i64).unwrap();
        linker.define(&store, "env", "current_account_id", noop_i32_i64).unwrap();
        linker.define(&store, "env", "signer_account_id", noop_i32_i64).unwrap();
        linker.define(&store, "env", "block_timestamp", noop_i64).unwrap();
        linker.define(&store, "env", "block_height", noop_i64).unwrap();
        linker.define(&store, "env", "storage_read", noop_i32_i32_to_i32).unwrap();
        linker.define(&store, "env", "storage_write", storage_write).unwrap();
        linker.define(&store, "env", "storage_has_key", noop_i32_i32_to_i32).unwrap();
        linker.define(&store, "env", "promise_create", promise_create).unwrap();
        linker.define(&store, "env", "promise_and", promise_and).unwrap();
        linker.define(&store, "env", "promise_then", promise_then).unwrap();
        linker.define(&store, "env", "promise_result", promise_result).unwrap();
        linker.define(&store, "env", "promise_return", noop_i64).unwrap();
        linker.define(&store, "env", "input_read", noop_i32_i32_to_i32).unwrap();

        // outlayer core
        linker.define(&store, "outlayer", "view", ol_view).unwrap();
        linker.define(&store, "outlayer", "call", ol_call).unwrap();
        linker.define(&store, "outlayer", "transfer", ol_transfer).unwrap();
        linker.define(&store, "outlayer", "http_get", ol_http_get).unwrap();
        linker.define(&store, "outlayer", "http_post", ol_http_post).unwrap();
        linker.define(&store, "outlayer", "store", ol_store).unwrap();
        linker.define(&store, "outlayer", "load", ol_load).unwrap();
        linker.define(&store, "outlayer", "remove", ol_remove).unwrap();
        linker.define(&store, "outlayer", "has", ol_has).unwrap();

        // near:rpc/api
        linker.define(&store, "near:rpc/api@0.1.0", "view", rpc_view).unwrap();
        linker.define(&store, "near:rpc/api@0.1.0", "call", rpc_call).unwrap();
        linker.define(&store, "near:rpc/api@0.1.0", "transfer", rpc_transfer).unwrap();

        // near:storage/api
        linker.define(&store, "near:storage/api@0.1.0", "set", st_set).unwrap();
        linker.define(&store, "near:storage/api@0.1.0", "get", st_get).unwrap();
        linker.define(&store, "near:storage/api@0.1.0", "has", st_has).unwrap();
        linker.define(&store, "near:storage/api@0.1.0", "delete", st_del).unwrap();
        linker.define(&store, "near:storage/api@0.1.0", "increment", st_incr).unwrap();
        linker.define(&store, "near:storage/api@0.1.0", "decrement", st_decr).unwrap();

        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start")
            .expect("_start export");

        match start.call(&mut store, ()) {
            Ok(()) => {}
            Err(trap) => {
                if !trap.to_string().contains("proc_exit") {
                    panic!("_start trap: {}", trap);
                }
            }
        }

        let mem = instance.get_memory(&mut store, "memory").expect("memory");
        let data = mem.data(&store);
        assert!(data.len() >= 65536 + 8, "memory too small: {} bytes", data.len());
        i64::from_le_bytes(data[65536..65536 + 8].try_into().unwrap())
    }

    // --- Arithmetic ---

    #[test]
    #[ignore]
    fn rt_const_42() {
        assert_eq!(run_p1("(define (main) 42)"), 42);
    }

    #[test]
    #[ignore]
    fn rt_arithmetic() {
        assert_eq!(run_p1("(define (main) (+ (* 3 4) (- 10 2)))"), 20);
    }

    // --- let* binding ---

    #[test]
    #[ignore]
    fn rt_let_star() {
        assert_eq!(run_p1(r#"
(define (main)
  (let* ((x 10) (y (+ x 5)))
    y))
"#), 15);
    }

    // --- Conditionals ---

    #[test]
    #[ignore]
    fn rt_if_true() {
        assert_eq!(run_p1("(define (main) (if (> 5 3) 1 0))"), 1);
    }

    #[test]
    #[ignore]
    fn rt_if_false() {
        assert_eq!(run_p1("(define (main) (if (< 5 3) 1 0))"), 0);
    }

    // --- near/log (should not trap) ---

    #[test]
    #[ignore]
    fn rt_near_log() {
        assert_eq!(run_p1(r#"
(define (main)
  (near/log "test message")
  0)
"#), 0);
    }

    // --- outlayer/view (should not trap with mock) ---

    #[test]
    #[ignore]
    fn rt_outlayer_view_no_trap() {
        // Just verify no panic
        let _ = run_p1(r#"(define (run) (outlayer/view "ref.near" "get_price" "{}"))"#);
    }

    // --- Storage (should not trap) ---

    #[test]
    #[ignore]
    fn rt_storage_set_no_trap() {
        assert_eq!(run_p1(r#"
(define (main)
  (storage/set "key" "value")
  0)
"#), 0);
    }
}

// ============================================================================
// TIER 3: OutLayer E2E (manual, requires outlayer CLI + NEAR)
// ============================================================================

mod outlayer_e2e {
    use super::*;

    fn has_outlayer() -> bool {
        Command::new("outlayer").args(["--help"]).output()
            .map(|o| o.status.success()).unwrap_or(false)
    }

    #[test]
    #[ignore]
    fn e2e_http_get_prices() {
        if !has_outlayer() {
            eprintln!("SKIP: outlayer CLI not found");
            return;
        }
        let wasm = compile_p2(r#"(define (run)
  (http-get "https://api.rhea.finance/list-token-price"))"#);
        validate(&wasm, "e2e_http");
        let tmp = std::env::temp_dir().join("e2e_http.wasm");
        std::fs::write(&tmp, &wasm).unwrap();
        // TODO: upload + run + parse receipt
        let _ = tmp;
    }

    #[test]
    #[ignore]
    fn e2e_combined_prices_positions() {
        if !has_outlayer() {
            eprintln!("SKIP: outlayer CLI not found");
            return;
        }
        let wasm = compile_p2(r#"
(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price"))
         (btc (json-get-str prices "btc.bridge.near")))
    (str-cat "BTC: " btc)))
"#);
        validate(&wasm, "e2e_combined");
        let tmp = std::env::temp_dir().join("e2e_combined.wasm");
        std::fs::write(&tmp, &wasm).unwrap();
        // TODO: upload + run + parse receipt
        let _ = tmp;
    }
}

// ============================================================================
// TIER 2b: NEAR-mock execution (pure NEAR target, no WASI)
// Uses the near-mock binary to execute WASM in a NEAR sandbox.
// ============================================================================
    /// Compile combined prices+positions pipeline using outlayer/view (P2 library path)
    #[test]
    fn compile_combined_view_p2() {
        let src = include_str!("../tests_p2/test_combined_view.lisp");
        let wasm = lisp_rlm_wasm::compile_outlayer_p2(src).expect("compile failed");
        std::fs::write("/tmp/test_combined_view.wasm", &wasm).expect("write failed");
        assert!(wasm.len() > 1000, "WASM too small: {} bytes", wasm.len());
    }

    /// Compile outlayer/transfer (P2 library path)
    #[test]
    fn compile_transfer_p2() {
        let src = include_str!("../tests_p2/test_transfer.lisp");
        let wasm = lisp_rlm_wasm::compile_outlayer_p2(src).expect("compile failed");
        std::fs::write("/tmp/test_transfer.wasm", &wasm).expect("write failed");
        assert!(wasm.len() > 500, "WASM too small: {} bytes", wasm.len());
    }

    // Near-mock tier
    mod near_mock {
    use super::*;
    use std::process::Command;

    fn has_near_mock() -> bool {
        Command::new("./target/release/near-mock")
            .args(["--help"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Compile lisp to NEAR target WASM and run via near-mock.
    /// Returns stdout as String.
    fn run_near_mock(src: &str, method: &str, args: &str, deposit: Option<&str>) -> String {
        let wasm = lisp_rlm_wasm::compile_near(src)
            .unwrap_or_else(|e| panic!("compile_near failed: {}", e));
        let tmp = std::env::temp_dir().join("nm_test.wasm");
        std::fs::write(&tmp, &wasm).unwrap();

        let mut cmd = Command::new("./target/release/near-mock");
        cmd.arg(&tmp).arg(method).arg(args);
        if let Some(d) = deposit {
            cmd.arg("--deposit").arg(d);
        }
        let output = cmd.output().expect("near-mock should run");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    /// Extract the return value line from near-mock output (e.g., "📄 42 (raw i64...)")
    fn extract_return(output: &str) -> Option<String> {
        output.lines().find(|l| l.starts_with("📄 ")).map(|l| l.to_string())
    }

    #[test]
    fn nm_const_42() {
        if !has_near_mock() { return; }
        let out = run_near_mock("(define (main) 42)", "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("42"), "expected 42, got: {}", ret);
    }

    #[test]
    fn nm_string_hello() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"(define (main) "hello")"#, "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("hello"), "expected hello, got: {}", ret);
    }

    #[test]
    fn nm_arithmetic() {
        if !has_near_mock() { return; }
        let out = run_near_mock("(define (main) (+ (* 3 4) (- 10 2)))", "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("20"), "expected 20, got: {}", ret);
    }

    #[test]
    fn nm_if_true() {
        if !has_near_mock() { return; }
        let out = run_near_mock("(define (main) (if (> 5 3) 100 0))", "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("100"), "expected 100, got: {}", ret);
    }

    #[test]
    fn nm_if_false() {
        if !has_near_mock() { return; }
        let out = run_near_mock("(define (main) (if (< 5 3) 100 0))", "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("0"), "expected 0, got: {}", ret);
    }

    #[test]
    fn nm_let_star() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"
(define (main)
  (let* ((x 10) (y (+ x 5)))
    y))
"#, "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("15"), "expected 15, got: {}", ret);
    }

    #[test]
    fn nm_str_cat() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"(define (main) (str-cat "hello" " world"))"#, "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("hello"), "expected hello in output, got: {}", ret);
    }

    #[test]
    fn nm_fibonacci_10() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"
(define (fib n)
  (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
(define (main) (fib 10))
"#, "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("55"), "expected 55, got: {}", ret);
    }

    #[test]
    fn nm_near_log() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"
(define (main)
  (near/log "test message")
  0)
"#, "_run", "{}", None);
        // near-mock should not trap — just returns 0
        let ret = extract_return(&out).expect("should have return value");
        assert!(ret.contains("0"), "expected 0, got: {}", ret);
    }

    #[test]
    fn nm_deposit() {
        if !has_near_mock() { return; }
        let out = run_near_mock(r#"
(define (main)
  (let* ((bal (attached-deposit)))
    (to-string bal)))
"#, "_run", "{}", Some("2000000000000000000"));
        let ret = extract_return(&out).expect("should have return value");
        // Should log or return something with the deposit value
        assert!(out.contains("2000000000") || out.contains("2e18") || out.contains("2000000000000000000"),
            "expected deposit value in output, got: {}", out);
    }
}
