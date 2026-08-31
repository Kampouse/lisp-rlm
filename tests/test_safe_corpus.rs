//! corpus/safe.lisp — 2-of-3 multisig battery.
//!
//! Interp: full 3-signer lifecycle via the near-config signer seam
//! (storage is the EvalState map — stateful by nature).
//!
//! Wasm: chained single-signer runs over SHARED host storage — each run is a
//! fresh instance (fresh FP/memory, like fresh on-chain transactions) but
//! storage, the signer cell, and the promise log are Arc<Mutex<..>> shared
//! across instances. Pins: quorum refusal at 1 slot, idempotent re-approval,
//! exact 61-bit-safe amount split/recombine, refusal-state preservation, and
//! the near/transfer_u128 promise payload (exact u128 yocto string).

use lisp_rlm_wasm::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

const AMT: &str = "1500000000000000000000000"; // 1.5 NEAR in yocto

fn corpus() -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/safe.lisp");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read corpus/safe.lisp: {}", e))
}

// ═══════════════════════════════════════════════════════════════════
// INTERP — full lifecycle, all signers, all refusal paths
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
        let _ = program::run_program(&parse_all(&corpus()).unwrap(), &mut env, &mut state);
        Interp { env, state }
    }
    fn eval(&mut self, src: &str) -> Result<LispVal, String> {
        program::run_program(&parse_all(src).unwrap(), &mut self.env, &mut self.state)
    }
    fn ok(&mut self, src: &str) -> LispVal {
        self.eval(src).unwrap_or_else(|e| panic!("interp failed: {}\n{}", e, src))
    }
    fn s(&mut self, src: &str) -> String {
        match self.ok(src) {
            LispVal::Str(s) => s,
            other => panic!("expected Str from {}, got {:?}", src, other),
        }
    }
    fn n(&mut self, src: &str) -> i64 {
        match self.ok(src) {
            LispVal::Num(n) => n,
            other => panic!("expected Num from {}, got {:?}", src, other),
        }
    }
    fn signer(&mut self, who: &str) {
        // near-config is a tree-walker-only seam (bytecode compiler does not
        // compile it — drift class). Set the context field directly instead.
        self.state
            .near_context
            .insert("signer_account_id".into(), LispVal::Str(who.into()));
    }
}

#[test]
fn interp_lifecycle_and_refusals() {
    let mut it = Interp::new();

    // init + idempotent re-init
    assert_eq!(it.s("(init)"), "1");
    assert_eq!(it.s("(init)"), "");

    // non-owner cannot propose or approve
    it.signer("dave.near");
    assert_eq!(it.s("(propose \"t1\" \"zoe.near\" \"1500000000000000000000000\")"), "");
    assert_eq!(it.s("(approve \"t1\" \"zoe.near\")"), "");
    assert_eq!(it.n("(approvals \"t1\")"), 0);

    // alice proposes — echoes amount; her slot is implicit
    it.signer("alice.near");
    assert_eq!(it.s("(propose \"t1\" \"zoe.near\" \"1500000000000000000000000\")"), AMT);
    assert_eq!(it.n("(approvals \"t1\")"), 1);
    // exact split/recombine round-trip (61-bit words, u128 string math)
    assert_eq!(it.s("(tx-amount \"t1\")"), AMT);

    // zero / over-cap / non-numeric amounts refused
    assert_eq!(it.s("(propose \"t0\" \"zoe.near\" \"0\")"), "");
    assert_eq!(it.s("(propose \"tbig\" \"zoe.near\" \"1000000000000000000000000000000000000\")"), "");
    assert!(it.eval("(propose \"tbad\" \"zoe.near\" \"12abc\")").is_err());

    // idempotent re-approval by same owner
    assert_eq!(it.s("(approve \"t1\" \"zoe.near\")"), "1");
    assert_eq!(it.n("(approvals \"t1\")"), 1);

    // execute at 1 slot → refused, state intact
    assert_eq!(it.s("(execute \"t1\" \"zoe.near\")"), "");
    assert_eq!(it.n("(approvals \"t1\")"), 1);
    assert_eq!(it.s("(tx-amount \"t1\")"), AMT);

    // bob approves → quorum
    it.signer("bob.near");
    assert_eq!(it.s("(approve \"t1\" \"zoe.near\")"), "1");
    assert_eq!(it.n("(approvals \"t1\")"), 2);

    // wrong recipient → refused (key sealed to zoe)
    assert_eq!(it.s("(execute \"t1\" \"attacker.near\")"), "");
    assert_eq!(it.n("(approvals \"t1\")"), 2);

    // execute fires: exactly one promise, then cleanup
    let before = it.state.near_promises.len();
    assert_eq!(it.s("(execute \"t1\" \"zoe.near\")"), "1");
    assert_eq!(it.state.near_promises.len(), before + 1);
    assert_eq!(it.n("(approvals \"t1\")"), 0);
    assert_eq!(it.s("(tx-amount \"t1\")"), "0"); // cleaned → words missing → 0

    // re-execution after cleanup → refused (no tx)
    assert_eq!(it.s("(execute \"t1\" \"zoe.near\")"), "");

    // cancel: 3/3 required
    it.signer("bob.near");
    assert_eq!(it.s("(propose \"t2\" \"zoe.near\" \"2000000000000000000000000\")"),
               "2000000000000000000000000");
    it.signer("carol.near");
    assert_eq!(it.s("(approve \"t2\" \"zoe.near\")"), "1");
    assert_eq!(it.n("(approvals \"t2\")"), 2);
    assert_eq!(it.s("(cancel \"t2\" \"zoe.near\")"), ""); // 2/3 not enough
    it.signer("alice.near");
    assert_eq!(it.s("(approve \"t2\" \"zoe.near\")"), "1");
    assert_eq!(it.n("(approvals \"t2\")"), 3);
    assert_eq!(it.s("(cancel \"t2\" \"zoe.near\")"), "1");
    assert_eq!(it.n("(approvals \"t2\")"), 0);

    // non-owner execute refused
    it.signer("bob.near");
    assert_eq!(it.s("(propose \"t3\" \"zoe.near\" \"3000000000000000000000000\")"),
               "3000000000000000000000000");
    it.signer("carol.near");
    assert_eq!(it.s("(approve \"t3\" \"zoe.near\")"), "1");
    it.signer("dave.near");
    assert_eq!(it.s("(execute \"t3\" \"zoe.near\")"), "");
    assert_eq!(it.n("(approvals \"t3\")"), 2);
}

// ═══════════════════════════════════════════════════════════════════
// WASM — shared-state chained runs (fresh instance per run, shared
// storage/signer/promises — the on-chain model)
// ═══════════════════════════════════════════════════════════════════

struct World {
    storage: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>,
    signer: Arc<Mutex<String>>,
    promises: Arc<Mutex<Vec<(String, String)>>>, // (target, amount-string)
}

impl World {
    fn new() -> Self {
        World {
            storage: Arc::new(Mutex::new(HashMap::new())),
            signer: Arc::new(Mutex::new("alice.near".into())),
            promises: Arc::new(Mutex::new(Vec::new())),
        }
    }
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

fn mem_write(caller: &mut Caller<'_, ()>, ptr: usize, bytes: &[u8]) {
    let mem = caller
        .get_export("memory")
        .and_then(|m| m.into_memory())
        .expect("module exports memory");
    mem.write(&mut *caller, ptr, bytes).expect("mem write");
}

/// Compile corpus+driver, run `(main)` against the shared World, return the
/// value_return payload decoded as (i64-raw, str-decode).
fn run_near(w: &World, driver: &str) -> Result<(i64, Option<String>), String> {
    let src = format!(
        "{}\n(memory 4)\n{}\n(export \"main\" main)",
        corpus(),
        driver
    );
    let wasm = compile_near_untyped(&src).map_err(|e| format!("compile: {}", e))?;
    let engine = Engine::default();
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    let regs: Arc<Mutex<HashMap<i64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let returned: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

    // registers
    {
        let regs = regs.clone();
        linker
            .func_wrap("env", "read_register", move |mut caller: Caller<'_, ()>, reg: i64, ptr: i64| {
                let bytes = regs.lock().unwrap().get(&reg).cloned().unwrap_or_default();
                mem_write(&mut caller, ptr as usize, &bytes);
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
            .func_wrap("env", "input", move |mut caller: Caller<'_, ()>, reg: i64| {
                regs.lock().unwrap().insert(reg, Vec::new());
                let _ = caller;
            })
            .unwrap();
    }
    // context
    {
        let regs = regs.clone();
        let signer = w.signer.clone();
        linker
            .func_wrap("env", "signer_account_id", move |_caller: Caller<'_, ()>, reg: i64| {
                regs.lock().unwrap().insert(reg, signer.lock().unwrap().as_bytes().to_vec());
            })
            .unwrap();
    }
    // storage
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
    // value_return
    {
        let returned = returned.clone();
        linker
            .func_wrap("env", "value_return", move |mut caller: Caller<'_, ()>, len: i64, ptr: i64| {
                let bytes = mem_read(&mut caller, ptr as usize, len as usize);
                *returned.lock().unwrap() = Some(bytes);
            })
            .unwrap();
    }
    // promises
    {
        let promises = w.promises.clone();
        linker
            .func_wrap("env", "promise_batch_create", move |mut caller: Caller<'_, ()>, alen: i64, aptr: i64| -> i64 {
                let target = String::from_utf8_lossy(&mem_read(&mut caller, aptr as usize, alen as usize)).to_string();
                promises.lock().unwrap().push((target, String::new())); // amount filled by action
                0
            })
            .unwrap();
    }
    {
        let promises = w.promises.clone();
        linker
            .func_wrap("env", "promise_batch_action_transfer", move |mut caller: Caller<'_, ()>, _idx: i64, amt_ptr: i64| {
                let bytes = mem_read(&mut caller, amt_ptr as usize, 16);
                let amount = u128::from_le_bytes(bytes.try_into().unwrap());
                let mut p = promises.lock().unwrap();
                if let Some(last) = p.last_mut() {
                    last.1 = amount.to_string();
                }
            })
            .unwrap();
    }
    linker
        .func_wrap("env", "log_utf8", |_caller: Caller<'_, ()>, _l: i64, _p: i64| {})
        .unwrap();
    linker
        .func_wrap("env", "panic_utf8", |_caller: Caller<'_, ()>, _l: i64, _p: i64| -> Result<()> {
            Err(wasmtime::Error::msg("panic_utf8"))
        })
        .unwrap();

    let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {}", e))?;
    let inst = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {}", e))?
        ;
    let main = inst
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| format!("get main: {}", e))?;
    main.call(&mut store, ())
        .map_err(|e| format!("call main: {}", e))?;

    let bytes = returned.lock().unwrap().clone().unwrap_or_default();
    // Decode by payload shape (53bf10f raw-str return arm):
    //  - 8 bytes  → untagged i64 (Num results)
    //  - any other length → the raw UTF-8 string bytes (Str results —
    //    near/return value_returns the view's bytes, not the tagged i64)
    let raw = if bytes.len() == 8 {
        i64::from_le_bytes(bytes[..8].try_into().unwrap())
    } else {
        0
    };
    let s = if bytes.len() == 8 {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).to_string())
    };
    Ok((raw, s))
}

#[test]
fn wasm_shared_storage_lifecycle() {
    let w = World::new();

    // run 1 (alice): init + propose → approvals == 1
    let (v, _) = run_near(
        &w,
        &format!(
            "(define (main) (begin (init) (propose \"t1\" \"zoe.near\" \"{}\") (approvals \"t1\")))",
            AMT
        ),
    )
    .expect("run1");
    assert_eq!(v, 1, "proposer's implicit slot → 1 approval");

    // run 2 (alice): tx-amount returns Str — since 53bf10f the raw-str
    // return arm value_returns the view's BYTES (the old tagged-i64
    // payload surfaced as tag-bit garbage). Exact content now decodable:
    // the u128 yocto string round-trips intact.
    let (_raw, s) = run_near(&w, "(define (main) (tx-amount \"t1\"))").expect("run2");
    assert_eq!(s.as_deref(), Some(AMT), "tx-amount → exact 25-digit yocto string");

    // run 3 (alice): re-approve idempotent
    let (v, _) = run_near(
        &w,
        "(define (main) (begin (approve \"t1\" \"zoe.near\") (approvals \"t1\")))",
    )
    .expect("run3");
    assert_eq!(v, 1, "re-approval must not double-count");

    // run 4 (alice): execute at 1 slot refused, state intact
    let (v, _) = run_near(
        &w,
        "(define (main) (begin (execute \"t1\" \"zoe.near\") (approvals \"t1\")))",
    )
    .expect("run4");
    assert_eq!(v, 1, "execute refused at 1 slot; approvals still 1");
    assert_eq!(w.promises.lock().unwrap().len(), 0, "no promise fired");

    // run 5: bob approves → 2 slots → executes and cleans up
    *w.signer.lock().unwrap() = "bob.near".into();
    let (v, _) = run_near(
        &w,
        "(define (main) (begin (approve \"t1\" \"zoe.near\") (execute \"t1\" \"zoe.near\") (approvals \"t1\")))",
    )
    .expect("run5");
    assert_eq!(v, 0, "post-execute cleanup → 0 approvals");
    let promises = w.promises.lock().unwrap();
    assert_eq!(promises.len(), 1, "exactly one transfer promise");
    assert_eq!(promises[0].0, "zoe.near", "promise target");
    assert_eq!(promises[0].1, AMT, "promise u128 amount exact");
}

#[test]
fn wasm_transfer_u128_pin() {
    let w = World::new();
    let (v, _) = run_near(
        &w,
        &format!(
            "(define (main) (begin (near/transfer_u128 \"zoe.near\" \"{}\") 7))",
            AMT
        ),
    )
    .expect("transfer pin");
    assert_eq!(v, 7);
    let promises = w.promises.lock().unwrap();
    assert_eq!(promises.len(), 1);
    assert_eq!(promises[0].1, AMT, "u128 transfer amount byte-exact");
}
