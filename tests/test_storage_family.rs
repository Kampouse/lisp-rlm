//! String-safe storage family (near/storage_set/get/has/remove) —
//! interp↔wasm equivalence battery.
//!
//! The family maps the raw NEAR host fns (storage_write/read/has_key/remove)
//! honestly: bytes-in-bytes-out. Strings are stored as their UTF-8 bytes, so
//! values survive fresh-memory transactions — unlike the tagged-word
//! near/store|load family whose 8-byte ptr|len payload is heap garbage across
//! transactions (the erc20 hazard).
//!
//! Wasm model (mirrors the on-chain model): every run_near is a FRESH
//! instance (fresh linear memory / heap) sharing only the host storage map —
//! written in run N, read back in run N+1 with zero heap reuse.
//!
//! Pins:
//!  - set/get round-trip, incl. >8-byte values (no tagged-word truncation)
//!  - get miss → "" (both VMs)
//!  - has → 1|0 (both), remove → get "" (both)
//!  - overwrite semantics
//!  - non-Str value / non-Str key → hard error (interp) ≡ trap (wasm)
//!  - cross-family read (near/store-written key via storage_get) → interp
//!    hard-errors loudly (documented seam — never mix families per key)

use lisp_rlm_wasm::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

// ═══════════════════════════════════════════════════════════════════
// INTERP
// ═══════════════════════════════════════════════════════════════════

struct Interp {
    env: Env,
    state: EvalState,
}

impl Interp {
    fn new() -> Self {
        let mut env = Env::new();
        let mut state = EvalState::new();
        let _ = program::run_program(
            &parse_all("(load-file \"runtime/harness.lisp\")").unwrap(),
            &mut env,
            &mut state,
        );
        Interp { env, state }
    }
    fn eval(&mut self, src: &str) -> Result<LispVal, String> {
        program::run_program(&parse_all(src).unwrap(), &mut self.env, &mut self.state)
    }
    fn s(&mut self, src: &str) -> String {
        match self.eval(src) {
            Ok(LispVal::Str(s)) => s,
            other => panic!("expected Str, got {:?}", other.map(|v| format!("{:?}", v))),
        }
    }
    fn n(&mut self, src: &str) -> i64 {
        match self.eval(src) {
            Ok(LispVal::Num(n)) => n,
            other => panic!("expected Num, got {:?}", other.map(|v| format!("{:?}", v))),
        }
    }
    fn err(&mut self, src: &str) -> String {
        match self.eval(src) {
            Err(e) => e,
            Ok(v) => panic!("expected Err, got Ok {:?}", v),
        }
    }
}

#[test]
fn interp_storage_family_lifecycle() {
    let mut it = Interp::new();

    // miss → ""
    assert_eq!(it.s("(near/storage_get \"nope\")"), "");

    // set → Num(0); get → exact bytes; has → 1
    assert_eq!(it.n("(near/storage_set \"k\" \"hello\")"), 0);
    assert_eq!(it.s("(near/storage_get \"k\")"), "hello");
    assert_eq!(it.n("(near/storage_has \"k\")"), 1);
    assert_eq!(it.n("(near/storage_has \"nope\")"), 0);

    // >8 bytes — proves no tagged-word truncation
    let long = "0123456789abcdefghij".repeat(4); // 80 chars
    assert_eq!(it.n(&format!("(near/storage_set \"long\" \"{}\")", long)), 0);
    assert_eq!(it.s("(near/storage_get \"long\")"), long);

    // overwrite
    assert_eq!(it.n("(near/storage_set \"k\" \"second\")"), 0);
    assert_eq!(it.s("(near/storage_get \"k\")"), "second");

    // remove → has 0, get ""
    assert_eq!(it.n("(near/storage_remove \"k\")"), 0);
    assert_eq!(it.n("(near/storage_has \"k\")"), 0);
    assert_eq!(it.s("(near/storage_get \"k\")"), "");
}

#[test]
fn interp_storage_family_hard_errors() {
    let mut it = Interp::new();

    // non-Str value — the erc20 hazard class must be loud, never silent
    let e = it.err("(near/storage_set \"k\" 42)");
    assert!(e.contains("expected string value"), "got: {}", e);

    // non-Str key
    let e = it.err("(near/storage_set 7 \"v\")");
    assert!(e.contains("expected string key"), "got: {}", e);

    // missing args
    assert!(it.err("(near/storage_set \"k\")").contains("missing value"));

    // cross-family read: near/store wrote a Num — storage_get must not
    // return garbage; it hard-errors with the documented seam.
    let _ = it.eval("(near/store \"mixed\" 5)");
    let e = it.err("(near/storage_get \"mixed\")");
    assert!(e.contains("non-string"), "got: {}", e);
}

// ═══════════════════════════════════════════════════════════════════
// WASM — fresh instance per run, shared host storage
// ═══════════════════════════════════════════════════════════════════

struct World {
    storage: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
}

fn mem_read(caller: &mut Caller<'_, ()>, ptr: usize, len: usize) -> Vec<u8> {
    let mem = caller
        .get_export("memory")
        .and_then(|m| m.into_memory())
        .expect("module exports memory");
    let mut buf = vec![0u8; len];
    mem.read(&mut *caller, ptr, &mut buf).expect("mem read");
    buf
}

/// Compile + run `(main)` on a FRESH instance against the shared storage.
/// Returns the near/return payload: raw string bytes for Str results,
/// decoded untagged i64 for Num results.
fn run_near(w: &World, body: &str) -> Result<Val, String> {
    let src = format!(
        "(memory 4)\n{}\n(export \"main\" main)",
        body
    );
    let wasm = compile_near_untyped(&src).map_err(|e| format!("compile: {}", e))?;
    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    let regs: Arc<Mutex<HashMap<i64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let returned: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

    {
        let regs = regs.clone();
        linker
            .func_wrap("env", "read_register", move |mut caller: Caller<'_, ()>, reg: i64, ptr: i64| {
                let bytes = regs.lock().unwrap().get(&reg).cloned().unwrap_or_default();
                let mem = caller.get_export("memory").and_then(|m| m.into_memory()).unwrap();
                mem.write(&mut caller, ptr as usize, &bytes).unwrap();
            })
            .unwrap();
    }
    {
        let regs = regs.clone();
        linker
            .func_wrap("env", "register_len", move |_caller: Caller<'_, ()>, reg: i64| -> i64 {
                regs.lock().unwrap().get(&reg).map(|b| b.len() as i64).unwrap_or(0)
            })
            .unwrap();
    }
    {
        let regs = regs.clone();
        linker
            .func_wrap("env", "input", move |_caller: Caller<'_, ()>, reg: i64| {
                regs.lock().unwrap().insert(reg, Vec::new());
            })
            .unwrap();
    }
    {
        let st = w.storage.clone();
        linker
            .func_wrap("env", "storage_write", move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64, vlen: i64, vptr: i64, _reg: i64| -> i64 {
                let key = mem_read(&mut caller, kptr as usize, klen as usize);
                let val = mem_read(&mut caller, vptr as usize, vlen as usize);
                st.lock().unwrap().insert(key, val);
                0
            })
            .unwrap();
    }
    {
        let st = w.storage.clone();
        let regs = regs.clone();
        linker
            .func_wrap("env", "storage_read", move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64, reg: i64| -> i64 {
                let key = mem_read(&mut caller, kptr as usize, klen as usize);
                match st.lock().unwrap().get(&key) {
                    Some(v) => {
                        regs.lock().unwrap().insert(reg, v.clone());
                        1
                    }
                    None => 0,
                }
            })
            .unwrap();
    }
    {
        let st = w.storage.clone();
        linker
            .func_wrap("env", "storage_remove", move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64, _reg: i64| -> i64 {
                let key = mem_read(&mut caller, kptr as usize, klen as usize);
                st.lock().unwrap().remove(&key).map(|_| 1).unwrap_or(0)
            })
            .unwrap();
    }
    {
        let st = w.storage.clone();
        linker
            .func_wrap("env", "storage_has_key", move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64| -> i64 {
                let key = mem_read(&mut caller, kptr as usize, klen as usize);
                if st.lock().unwrap().contains_key(&key) { 1 } else { 0 }
            })
            .unwrap();
    }
    {
        let returned = returned.clone();
        linker
            .func_wrap("env", "value_return", move |mut caller: Caller<'_, ()>, len: i64, ptr: i64| {
                let bytes = mem_read(&mut caller, ptr as usize, len as usize);
                *returned.lock().unwrap() = Some(bytes);
            })
            .unwrap();
    }
    linker
        .func_wrap("env", "log_utf8", |_caller: Caller<'_, ()>, _l: i64, _p: i64| {})
        .unwrap();

    let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {}", e))?;
    let inst = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {}", e))?;
    let main = inst
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| format!("get main: {}", e))?;
    main.call(&mut store, ())
        .map_err(|e| format!("call main: {}", e))?;

    let bytes = returned.lock().unwrap().clone().unwrap_or_default();
    Ok(Val::Bytes(bytes))
}

enum Val {
    Bytes(Vec<u8>),
}

impl Val {
    fn as_str(&self) -> String {
        match self {
            Val::Bytes(b) => String::from_utf8_lossy(b).to_string(),
        }
    }
    fn as_i64(&self) -> i64 {
        match self {
            // near/return serializes Num as the UNTAGGED 8-byte payload
            Val::Bytes(b) if b.len() == 8 => i64::from_le_bytes(b[..8].try_into().unwrap()),
            Val::Bytes(b) => panic!("expected 8-byte num payload, got {} bytes", b.len()),
        }
    }
}

#[test]
fn wasm_fresh_memory_persistence() {
    // THE erc20-hazard killer: value written in run 1 must read back intact
    // in run 2 — a fresh instance with a fresh heap. A tagged-word store
    // (ptr|len) would return heap garbage here; the string family must not.
    let w = World { storage: Arc::new(Mutex::new(HashMap::new())) };

    let long = "0123456789abcdefghij".repeat(4); // 80 chars
    let src1 = format!(
        "(define (main) (begin (near/storage_set \"k\" \"hello\") (near/storage_set \"long\" \"{}\") (near/return 0)))",
        long
    );
    let v = run_near(&w, &src1).expect("run1 set");
    assert_eq!(v.as_i64(), 0, "set returns Num(0)");

    // run 2: fresh memory, same storage — read back
    let v = run_near(&w, "(define (main) (near/return (near/storage_get \"k\")))")
        .expect("run2 get k");
    assert_eq!(v.as_str(), "hello", "short string survives fresh memory");

    let v = run_near(&w, "(define (main) (near/return (near/storage_get \"long\")))")
        .expect("run3 get long");
    assert_eq!(v.as_str(), long, "80-char string survives fresh memory — no 8B truncation");

    // miss → ""
    let v = run_near(&w, "(define (main) (near/return (near/storage_get \"nope\")))")
        .expect("run4 miss");
    assert_eq!(v.as_str(), "", "missing key → empty string");

    // has → 1 / 0
    let v = run_near(&w, "(define (main) (near/return (near/storage_has \"k\")))").unwrap();
    assert_eq!(v.as_i64(), 1);
    let v = run_near(&w, "(define (main) (near/return (near/storage_has \"nope\")))").unwrap();
    assert_eq!(v.as_i64(), 0);

    // remove → get "" , has 0
    run_near(&w, "(define (main) (near/return (near/storage_remove \"k\")))").unwrap();
    let v = run_near(&w, "(define (main) (near/return (near/storage_get \"k\")))").unwrap();
    assert_eq!(v.as_str(), "");
    let v = run_near(&w, "(define (main) (near/return (near/storage_has \"k\")))").unwrap();
    assert_eq!(v.as_i64(), 0);

    // overwrite
    run_near(&w, "(define (main) (near/storage_set \"long\" \"second\"))").unwrap();
    let v = run_near(&w, "(define (main) (near/return (near/storage_get \"long\")))").unwrap();
    assert_eq!(v.as_str(), "second", "overwrite wins");
}

#[test]
fn wasm_non_string_value_traps() {
    // Num value → TAG_STR assertion trap (unreachable) — wasm twin of the
    // interp "expected string value" hard error. Same event class.
    let w = World { storage: Arc::new(Mutex::new(HashMap::new())) };
    let r = run_near(&w, "(define (main) (near/storage_set \"k\" 42))");
    assert!(r.is_err(), "Num value must trap, got {:?}", r.as_ref().map(|v| v.as_str()));
}

#[test]
fn wasm_storage_bytes_on_chain_shape() {
    // Host-side view: stored bytes must be the exact UTF-8 of the value —
    // this is what a NEAR explorer / contract migration would see on-chain.
    let w = World { storage: Arc::new(Mutex::new(HashMap::new())) };
    run_near(&w, "(define (main) (near/storage_set \"explorer-view\" \"pure bytes\"))").unwrap();
    let st = w.storage.lock().unwrap();
    let got = st
        .get("explorer-view".as_bytes())
        .expect("key stored as raw key bytes");
    assert_eq!(got, b"pure bytes", "on-chain value bytes = exact UTF-8");
}

// ═══════════════════════════════════════════════════════════════════
// Storage-read memo cache (perf/storage-read-cache) — semantics pins.
// The wasm emitter caches storage_get results in linear memory per
// instance; every storage_write op flushes. These pins hold the exact
// read-after-write / read-after-remove / cross-family / same-length-key
// isolation / overflow-fallback behavior the cache must preserve.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn wasm_storage_cache_semantics() {
    let w = World { storage: Arc::new(Mutex::new(HashMap::new())) };

    // tx 1: seed
    run_near(&w, r#"(define (main) (begin (near/storage_set "k" "v1") (near/storage_set "ka" "A") (near/storage_set "kb" "B") (near/return "ok")))"#)
        .unwrap();

    // tx 2: read-hit, then invalidate-by-set, then read the NEW value —
    // the classic read-set-read hazard: first get caches v1, the set must
    // flush, the second get must observe v2 (not the cached v1).
    let r = run_near(
        &w,
        r#"(define (main) (near/return (str-concat (default (near/storage_get "k") "MISS") "|" (begin (near/storage_set "k" "v2") (default (near/storage_get "k") "MISS")))))"#,
    )
    .unwrap();
    assert_eq!(r.as_str(), "v1|v2", "set must invalidate the cached read");

    // tx 3: fresh instance — must see tx 2's write (erc20-hazard class:
    // per-instance cache, never persisted)
    let r = run_near(&w, r#"(define (main) (near/return (default (near/storage_get "k") "MISS")))"#)
        .unwrap();
    assert_eq!(r.as_str(), "v2", "fresh instance reads committed value");

    // tx 4: remove invalidates too — get caches, remove flushes, get misses
    let r = run_near(
        &w,
        r#"(define (main) (near/return (str-concat (default (near/storage_get "k") "MISS") "|" (begin (near/storage_remove "k") (default (near/storage_get "k") "MISS")))))"#,
    )
    .unwrap();
    assert_eq!(r.as_str(), "v2|MISS", "remove must invalidate the cached read");

    // tx 5: same-length keys must NOT alias in the cache (exact byte
    // compare — regression for the (klen>>3)<<3 peephole-eaten tail loop)
    let r = run_near(
        &w,
        r#"(define (main) (near/return (str-concat (default (near/storage_get "ka") "?") (default (near/storage_get "kb") "?") (default (near/storage_get "ka") "?"))))"#,
    )
    .unwrap();
    assert_eq!(r.as_str(), "ABA", "same-length keys are distinct cache entries");

    // tx 6: cross-family write (tagged-word near/store) must flush the
    // string-family cache — the raw 8-byte value reads back as a Str
    let r = run_near(
        &w,
        r#"(define (main) (begin (near/storage_set "cf" "seeded") (near/return (str-concat (default (near/storage_get "cf") "?") "|" (begin (near/store "cf" 4242) (to-string (str-length (default (near/storage_get "cf") "?"))))))))"#,
    )
    .unwrap();
    assert_eq!(r.as_str(), "seeded|8", "near/store must flush the cache (8 raw bytes)");
}

#[test]
fn wasm_storage_cache_overflow_and_long_keys() {
    let w = World { storage: Arc::new(Mutex::new(HashMap::new())) };

    // >64 distinct keys: table fills, further reads run uncached — every
    // value must still be exact (fallback correctness)
    let mut src = String::from("(define (main) (begin ");
    for i in 0..70 {
        src.push_str(&format!("(near/storage_set \"key{:02}\" \"val{:02}x\") ", i, i));
    }
    src.push_str("(near/return \"ok\")))");
    run_near(&w, &src).unwrap();

    // read all 70 back in one tx: reads 65..70 hit the overflow fallback
    let mut src = String::from("(define (main) (near/return (str-concat ");
    for i in 0..70 {
        src.push_str(&format!("(default (near/storage_get \"key{:02}\") \"?\") ", i));
    }
    src.push_str(")))");
    let r = run_near(&w, &src).unwrap();
    let expect: String = (0..70).map(|i| format!("val{:02}x", i)).collect();
    assert_eq!(r.as_str(), expect, "overflow fallback reads stay exact");

    // long key (> 64 bytes → uncached path) still round-trips
    let long_key = "K".repeat(70);
    let r = run_near(
        &w,
        &format!(
            r#"(define (main) (begin (near/storage_set "{}" "longval") (near/return (default (near/storage_get "{}") "MISS"))))"#,
            long_key, long_key
        ),
    )
    .unwrap();
    assert_eq!(r.as_str(), "longval", ">64-byte keys read uncached, exact");
}
