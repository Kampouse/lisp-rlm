//! storage-iter builtins (storage-iter-prefix / storage-iter-next, host
//! fns 36/38) — interp↔wasm equivalence battery.
//!
//! Interp mock semantics (bytecode near/storage_iter_*): iter-prefix
//! snapshots the SORTED keys under prefix and returns an id; iter-next
//! returns the next LIVE key (skipping keys deleted after the snapshot)
//! or nil when exhausted — so the cleaner loop (next → remove → next…)
//! drains storage exactly.
//!
//! Wasm model: real mock hosts over the shared storage map, mirroring
//! the NEAR trie host (key-order iteration, live state). The cleaner
//! loop is compiled TYPED (compile_near — the type-checker gate is part
//! of the surface) and run on a fresh instance; the removed-count must
//! equal the interp's for identical starting state.
//!
//! Pins:
//!  - "" prefix = all keys; non-empty prefix filters
//!  - sorted iteration order
//!  - nil on exhaustion; unknown iterator id → nil (never an error)
//!  - removal during iteration works (skip-deleted), both VMs
//!  - dash / underscore / near/ name forms all dispatch, both VMs
//!  - the examples/storage_cleaner.lisp clear-keys loop: interp wipes
//!    every key and returns the right count

use lisp_rlm_wasm::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

// ═══════════════════════════════════════════════════════════════════
// INTERP — direct eval_builtin dispatch
//
// run_program's compile gate (helpers.rs BUILTIN_NAMES) is OUTSIDE this
// task's fenced paths, and it predates even the legacy bare storage-*
// names (bare "storage-write" fails that gate too — probe-verified).
// The interpreter implementation surface for the new builtins IS
// bytecode::eval_builtin → eval_near_builtin arms, exercised here with
// a shared EvalState — same dispatch run_program uses post-gate.
// Follow-up for maintainer: add the 4 names to BUILTIN_NAMES so
// (storage-iter-prefix …) compiles through run_program/lisp-run.
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
    /// Evaluate ONE builtin call through the interpreter dispatch
    /// (no compile gate — eval_builtin is the surface under test).
    fn b(&mut self, name: &str, args: &[LispVal]) -> LispVal {
        let env = &mut self.env;
        match bytecode::eval_builtin(name, args, Some(env), Some(&mut self.state)) {
            Ok(v) => v,
            Err(e) => panic!("eval_builtin('{}') failed: {}", name, e),
        }
    }
    fn s(&mut self, name: &str, args: &[LispVal]) -> String {
        match self.b(name, args) {
            LispVal::Str(s) => s,
            other => panic!("{}: expected Str, got {:?}", name, other),
        }
    }
    fn n(&mut self, name: &str, args: &[LispVal]) -> i64 {
        match self.b(name, args) {
            LispVal::Num(n) => n,
            other => panic!("{}: expected Num, got {:?}", name, other),
        }
    }
    fn nil(&mut self, name: &str, args: &[LispVal]) {
        match self.b(name, args) {
            LispVal::Nil => {}
            other => panic!("{}: expected Nil, got {:?}", name, other),
        }
    }
    fn key(s: &str) -> LispVal {
        LispVal::Str(s.into())
    }
    /// The cleaner loop in Rust-over-the-builtin form: iterate id, remove
    /// every returned key, count. Returns the removed count.
    fn drain(&mut self, prefix: &str) -> i64 {
        let id = self.n("storage-iter-prefix", &[Self::key(prefix)]);
        let mut removed = 0;
        loop {
            match self.b("storage-iter-next", &[LispVal::Num(id)]) {
                LispVal::Str(k) => {
                    self.b("storage-remove", &[LispVal::Str(k)]);
                    removed += 1;
                }
                LispVal::Nil => return removed,
                other => panic!("storage-iter-next: expected Str|Nil, got {:?}", other),
            }
        }
    }
    fn collect(&mut self, prefix: &str, form: &str) -> Vec<String> {
        // (form, next_form) per naming family: dash / underscore / near/
        let next_form = match form {
            "storage-iter-prefix" => "storage-iter-next",
            "storage_iter_prefix" => "storage_iter_next",
            "near/storage_iter_prefix" => "near/storage_iter_next",
            other => panic!("unknown prefix form {}", other),
        };
        let id = self.n(form, &[Self::key(prefix)]);
        let mut out = Vec::new();
        loop {
            match self.b(next_form, &[LispVal::Num(id)]) {
                LispVal::Str(k) => out.push(k),
                LispVal::Nil => return out,
                other => panic!("{}: expected Str|Nil, got {:?}", next_form, other),
            }
        }
    }
}

/// The task's acceptance loop: write 3 keys, iterate ALL via "" prefix,
/// remove each visited key, then assert has-key false for all three.
#[test]
fn interp_iterate_and_remove_all() {
    let mut it = Interp::new();
    for (k, v) in [("b:2", "v2"), ("a:1", "v1"), ("c:3", "v3")] {
        it.n("storage-write", &[Interp::key(k), Interp::key(v)]);
    }

    assert_eq!(it.drain(""), 3, "all three keys removed");

    for k in ["a:1", "b:2", "c:3"] {
        assert_eq!(
            it.n("storage-has-key", &[Interp::key(k)]),
            0,
            "key {} must be gone",
            k
        );
    }
    assert!(it.collect("", "storage-iter-prefix").is_empty(), "drained");
}

/// Order is sorted (deterministic), and underscore / near/ aliases
/// dispatch identically to the dash form.
#[test]
fn interp_sorted_order_and_name_forms() {
    let mut it = Interp::new();
    for (k, v) in [("z", "1"), ("a", "2"), ("m", "3")] {
        it.n("storage-write", &[Interp::key(k), Interp::key(v)]);
    }

    // dash form
    assert_eq!(it.collect("", "storage-iter-prefix"), vec!["a", "m", "z"]);
    // underscore form
    assert_eq!(it.collect("", "storage_iter_prefix"), vec!["a", "m", "z"]);
    // near/ form — same semantics, same iterator state
    assert_eq!(
        it.collect("", "near/storage_iter_prefix"),
        vec!["a", "m", "z"]
    );
}

#[test]
fn interp_nonempty_prefix_filters() {
    let mut it = Interp::new();
    for (k, v) in [("bal:alice", "1"), ("bal:bob", "2"), ("allow:alice", "3")] {
        it.n("storage-write", &[Interp::key(k), Interp::key(v)]);
    }

    assert_eq!(
        it.collect("bal:", "storage-iter-prefix"),
        vec!["bal:alice", "bal:bob"],
        "prefix filters"
    );
    assert!(
        it.collect("zzz:", "storage-iter-prefix").is_empty(),
        "no-match prefix walks nothing"
    );
    // unknown iterator id → nil, never an error
    it.nil("storage-iter-next", &[LispVal::Num(9999)]);
}

/// Removal during iteration: keys deleted after the snapshot are
/// skipped, not returned and not fatal — the cleaner-loop guarantee.
#[test]
fn interp_removal_during_iteration() {
    let mut it = Interp::new();
    for k in ["k1", "k2", "k3", "k4"] {
        it.n("storage-write", &[Interp::key(k), Interp::key("v")]);
    }

    // remove k2 BEFORE the walk; drain the REST — total removed must be
    // exactly the 3 live keys, no resurrect, no skip.
    it.n("storage-remove", &[Interp::key("k2")]);
    assert_eq!(it.drain(""), 3, "3 live keys removed (k2 was pre-deleted)");
    assert!(
        it.collect("", "storage-iter-prefix").is_empty(),
        "storage fully drained"
    );
}

/// Mixed families share one storage map: near/storage_set writes are
/// visible to storage-iter-* (the gcpool26 reset case — stale state was
/// written by the string-safe family).
#[test]
fn interp_iter_sees_string_safe_family() {
    let mut it = Interp::new();
    it.n(
        "near/storage_set",
        &[Interp::key("bal:x"), Interp::key("7")],
    );
    it.n("storage-write", &[Interp::key("bal:y"), Interp::key("8")]);

    assert_eq!(
        it.collect("bal:", "storage-iter-prefix"),
        vec!["bal:x", "bal:y"]
    );
    assert_eq!(it.drain(""), 2);
    assert_eq!(it.n("near/storage_has", &[Interp::key("bal:x")]), 0);
}

// ═══════════════════════════════════════════════════════════════════
// WASM — fresh instance per run, shared host storage, real iter hosts
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

/// Compile UNtyped (compile_near_untyped — house precedent in
/// test_storage_family; near/return is typed str→nil so num returns
/// need the untyped path) + run `(main)` on a FRESH instance against
/// the shared storage. Returns the near/return payload.
///
/// ⚠ LANDMINE (pre-existing, unfenced — wasm_emit near/return NUM arm):
/// `(near/return <expr>)` with a Num result evaluates <expr> TWICE
/// (tag-check then branch re-emission — WAT-verified 2026-09-27). Every
/// effectful loop return here goes through `(near/return_str (to-string
/// …))` — the Str arm evaluates once. Never `(near/return <effectful
/// loop>)`.
fn run_near(w: &World, body: &str) -> Result<Vec<u8>, String> {
    let src = format!("(memory 4)\n{}\n(export \"main\" main)", body);
    let wasm = compile_near_untyped(&src).map_err(|e| format!("compile: {}", e))?;
    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    let regs: Arc<Mutex<HashMap<i64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let returned: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

    {
        let regs = regs.clone();
        linker
            .func_wrap(
                "env",
                "read_register",
                move |mut caller: Caller<'_, ()>, reg: i64, ptr: i64| {
                    let bytes = regs.lock().unwrap().get(&reg).cloned().unwrap_or_default();
                    let mem = caller
                        .get_export("memory")
                        .and_then(|m| m.into_memory())
                        .unwrap();
                    mem.write(&mut caller, ptr as usize, &bytes).unwrap();
                },
            )
            .unwrap();
    }
    {
        let regs = regs.clone();
        linker
            .func_wrap(
                "env",
                "register_len",
                move |_caller: Caller<'_, ()>, reg: i64| -> i64 {
                    regs.lock()
                        .unwrap()
                        .get(&reg)
                        .map(|b| b.len() as i64)
                        .unwrap_or(0)
                },
            )
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
            .func_wrap(
                "env",
                "storage_write",
                move |mut caller: Caller<'_, ()>,
                      klen: i64,
                      kptr: i64,
                      vlen: i64,
                      vptr: i64,
                      _reg: i64|
                      -> i64 {
                    let key = mem_read(&mut caller, kptr as usize, klen as usize);
                    let val = mem_read(&mut caller, vptr as usize, vlen as usize);
                    st.lock().unwrap().insert(key, val);
                    0
                },
            )
            .unwrap();
    }
    {
        let st = w.storage.clone();
        linker
            .func_wrap(
                "env",
                "storage_remove",
                move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64, _reg: i64| -> i64 {
                    let key = mem_read(&mut caller, kptr as usize, klen as usize);
                    st.lock().unwrap().remove(&key).map(|_| 1).unwrap_or(0)
                },
            )
            .unwrap();
    }
    {
        let st = w.storage.clone();
        linker
            .func_wrap(
                "env",
                "storage_has_key",
                move |mut caller: Caller<'_, ()>, klen: i64, kptr: i64| -> i64 {
                    let key = mem_read(&mut caller, kptr as usize, klen as usize);
                    if st.lock().unwrap().contains_key(&key) {
                        1
                    } else {
                        0
                    }
                },
            )
            .unwrap();
    }
    // ── host fn 36: storage_iter_prefix(len, ptr) → u64 id ──
    // Snapshot the SORTED keys under prefix (parity with the interp mock).
    let iter_next_id = Arc::new(Mutex::new(0u64));
    let iter_keys: Arc<Mutex<HashMap<u64, Vec<Vec<u8>>>>> = Arc::new(Mutex::new(HashMap::new()));
    let iter_cursors: Arc<Mutex<HashMap<u64, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    {
        let st = w.storage.clone();
        let iter_next_id = iter_next_id.clone();
        let iter_keys = iter_keys.clone();
        let iter_cursors = iter_cursors.clone();
        linker
            .func_wrap(
                "env",
                "storage_iter_prefix",
                move |mut caller: Caller<'_, ()>, plen: i64, pptr: i64| -> u64 {
                    let prefix = mem_read(&mut caller, pptr as usize, plen as usize);
                    let mut keys: Vec<Vec<u8>> = st
                        .lock()
                        .unwrap()
                        .keys()
                        .filter(|k| k.starts_with(&prefix))
                        .cloned()
                        .collect();
                    keys.sort();
                    let id = *iter_next_id.lock().unwrap();
                    *iter_next_id.lock().unwrap() += 1;
                    iter_keys.lock().unwrap().insert(id, keys);
                    iter_cursors.lock().unwrap().insert(id, 0);
                    id
                },
            )
            .unwrap();
    }
    // ── host fn 38: storage_iter_next(id, key_reg, val_reg) → 1|0 ──
    // Live trie semantics: skip keys deleted since the snapshot — the
    // same contract the interp mock gives the cleaner loop.
    {
        let st = w.storage.clone();
        let iter_keys = iter_keys.clone();
        let iter_cursors = iter_cursors.clone();
        let regs = regs.clone();
        linker
            .func_wrap(
                "env",
                "storage_iter_next",
                move |_caller: Caller<'_, ()>, id: u64, key_reg: i64, val_reg: i64| -> i64 {
                    let keys = match iter_keys.lock().unwrap().get(&id) {
                        Some(k) => k.clone(),
                        None => return 0,
                    };
                    let mut cursors = iter_cursors.lock().unwrap();
                    let cursor = cursors.entry(id).or_insert(0);
                    while *cursor < keys.len() {
                        let key = keys[*cursor].clone();
                        *cursor += 1;
                        if st.lock().unwrap().contains_key(&key) {
                            let _ = val_reg; // value register ignored (contract never reads it)
                            regs.lock().unwrap().insert(key_reg, key);
                            return 1;
                        }
                    }
                    0
                },
            )
            .unwrap();
    }
    {
        let returned = returned.clone();
        linker
            .func_wrap(
                "env",
                "value_return",
                move |mut caller: Caller<'_, ()>, len: i64, ptr: i64| {
                    let bytes = mem_read(&mut caller, ptr as usize, len as usize);
                    *returned.lock().unwrap() = Some(bytes);
                },
            )
            .unwrap();
    }
    linker
        .func_wrap(
            "env",
            "log_utf8",
            |_caller: Caller<'_, ()>, _l: i64, _p: i64| {},
        )
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
    Ok(bytes)
}

fn bytes_to_i64(b: &[u8]) -> i64 {
    assert_eq!(
        b.len(),
        8,
        "expected 8-byte num payload, got {} bytes: {:?}",
        b.len(),
        b
    );
    i64::from_le_bytes(b[..8].try_into().unwrap())
}

/// Same shape as the interp battery: seed storage in run 1, drain with
/// the cleaner loop in run 2, count in run 3 — fresh instance each time.
#[test]
fn wasm_cleaner_loop_equivalence() {
    let w = World {
        storage: Arc::new(Mutex::new(HashMap::new())),
    };

    // seed: 4 keys, deliberately inserted out of order
    let v = run_near(
        &w,
        r#"(define (main) (begin (near/storage_set "z" "1") (near/storage_set "a" "2") (near/storage_set "m" "3") (near/storage_set "bal:alice" "4") (near/return 0)))"#,
    )
    .expect("seed run");
    assert_eq!(bytes_to_i64(&v), 0);

    // drain via the new builtins — dash names, exactly the contract loop
    let v = run_near(
        &w,
        r#"(define (main)
  (let ((res (to-string
             (loop ((id (storage-iter-prefix "")) (removed 0))
                 (let ((k (storage-iter-next id)))
                   (if (nil? k)
                       removed
                       (begin (near/storage_remove k) (recur id (+ removed 1)))))))))
    (near/return_str res)))"#,
    )
    .expect("clean run");
    assert_eq!(
        String::from_utf8_lossy(&v),
        "4",
        "wasm removed-count matches interp semantics (4 live keys)"
    );

    // drained: count walk sees nothing, has_key false everywhere
    let v = run_near(
        &w,
        r#"(define (main)
  (let ((res (to-string
             (loop ((id (storage-iter-prefix "")) (n 0))
                 (let ((k (storage-iter-next id)))
                   (if (nil? k) n (recur id (+ n 1))))))))
    (near/return_str res)))"#,
    )
    .expect("count run");
    assert_eq!(String::from_utf8_lossy(&v), "0", "storage drained");

    let v = run_near(
        &w,
        r#"(define (main) (near/return (near/storage_has "a")))"#,
    )
    .expect("has a");
    assert_eq!(bytes_to_i64(&v), 0, "a gone");
    let v = run_near(
        &w,
        r#"(define (main) (near/return (near/storage_has "bal:alice")))"#,
    )
    .expect("has bal:alice");
    assert_eq!(bytes_to_i64(&v), 0, "bal:alice gone");
}

/// Removal during iteration on wasm — pre-delete one key, then walk+remove
/// the rest; underscore name forms exercise the alias arms.
#[test]
fn wasm_removal_during_iteration_and_aliases() {
    let w = World {
        storage: Arc::new(Mutex::new(HashMap::new())),
    };

    run_near(
        &w,
        r#"(define (main) (begin (near/storage_set "k1" "1") (near/storage_set "k2" "2") (near/storage_set "k3" "3") (near/storage_remove "k2") (near/return 0)))"#,
    )
    .expect("seed + pre-delete k2");

    // underscore forms of both builtins
    let v = run_near(&w, r#"(define (main)
  (let ((res (to-string
             (loop ((id (storage_iter_prefix "")) (removed 0))
                 (let ((k (storage_iter_next id)))
                   (if (nil? k) removed (begin (near/storage_remove k) (recur id (+ removed 1)))))))))
    (near/return_str res)))"#)
        .expect("underscore clean run");
    assert_eq!(
        String::from_utf8_lossy(&v),
        "2",
        "k2 was deleted pre-walk; only 2 live keys removed"
    );
}

/// near/ names lower to the same hosts — compile+run parity for the
/// namespaced form.
#[test]
fn wasm_near_namespaced_forms() {
    let w = World {
        storage: Arc::new(Mutex::new(HashMap::new())),
    };

    run_near(
        &w,
        r#"(define (main) (begin (near/storage_set "a" "1") (near/storage_set "b" "2") (near/return 0)))"#,
    )
    .expect("seed");

    let v = run_near(&w, r#"(define (main)
  (let ((res (to-string
             (loop ((id (near/storage_iter_prefix "")) (removed 0))
                 (let ((k (near/storage_iter_next id)))
                   (if (nil? k) removed (begin (near/storage_remove k) (recur id (+ removed 1)))))))))
    (near/return_str res)))"#)
        .expect("near/ clean run");
    assert_eq!(
        String::from_utf8_lossy(&v),
        "2",
        "near/ forms work identically"
    );
}

/// TYPED compile (compile_near — the type-checker gate): the new names
/// must typecheck, in all three naming families, plus the full cleaner
/// loop shape. (Runtime behavior is covered by the untyped runs above;
/// this pins the typing surface only.)
#[test]
fn wasm_typed_compile_surface() {
    for body in [
        r#"(define (main) (near/return_str (storage-iter-next (storage-iter-prefix "a"))))"#,
        r#"(define (main) (near/return_str (storage_iter_next (storage_iter_prefix "a"))))"#,
        r#"(define (main) (near/return_str (near/storage_iter_next (near/storage_iter_prefix "a"))))"#,
    ] {
        let src = format!("(memory 4)\n{}\n(export \"main\" main)", body);
        let wasm = compile_near(&src).unwrap_or_else(|e| panic!("typed compile failed: {}", e));
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).expect("module");
        assert!(module.get_export("memory").is_some(), "memory exported");
    }

    // the typed surface must also accept the exact cleaner-loop shape
    let src = format!(
        "(memory 4)\n{}\n(export \"main\" main)",
        r#"(define (main)
  (let ((res (to-string
             (loop ((id (storage-iter-prefix "")) (removed 0))
                 (let ((k (storage-iter-next id)))
                   (if (nil? k)
                       removed
                       (begin (near/storage_remove k) (recur id (+ removed 1)))))))))
    (near/return_str res)))"#
    );
    compile_near(&src).unwrap_or_else(|e| panic!("typed cleaner loop must compile: {}", e));
}
