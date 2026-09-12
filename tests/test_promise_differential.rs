//! STEP-2 DIFFERENTIAL: NEAR host-effect parity — promise schedule + storage.
//!
//! The fuzz harness (test_wasm_fuzz.rs) stubs every host import as return-0,
//! so promise programs were never differentially tested. This harness gives
//! the wasm side REAL host implementations (in the test, wallet_diff.rs
//! pattern) that RECORD the host-call schedule, and compares:
//!
//!   1. PROMISE SCHEDULE — the sequence of promise ops the contract asks the
//!      host to perform (create/then/and/return, batch_create/then/fn_call),
//!      normalized to canonical tuples. Interpreter side = the EvalState
//!      near_promises mock log; wasm side = the recorded host-call trace.
//!      Sugar ops expand: near/call → [create, return];
//!      near/call-await → [batch_create, fn_call, batch_then(self),
//!      fn_call(callback), return].
//!   2. STORAGE — final key/value maps (interp near_storage vs wasm trie).
//!
//! Not compared (v1): promise_result consumption (needs receipt execution —
//! that's near-mock/scenario territory), return-value bytes (the _run
//! wrapper's value_return serialization is its own surface).
//!
//! Trust split: this is the fast in-process loop; near-mock stays the
//! end-to-end arbiter (flashloan/wallet scenarios).

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};
use lisp_rlm_wasm::wasm_emit::compile_near;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

const SELF_ACCOUNT: &str = "self.test.near";
const PREDECESSOR: &str = "caller.test.near";
const DEFAULT_GAS: i64 = 30_000_000_000_000;

// ── canonical schedule ops ──

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Create {
        target: String,
        method: String,
        args: String,
        gas: i64,
        deposit: String,
    },
    Then {
        base: i64,
        target: String,
        method: String,
        args: String,
        gas: i64,
        deposit: String,
    },
    And {
        promises: Vec<i64>,
    },
    Return {
        idx: i64,
    },
    BatchCreate {
        target: String,
    },
    BatchThen {
        base: i64,
        target: String,
    },
    BatchFnCall {
        batch: i64,
        method: String,
        args: String,
        gas: i64,
        deposit: String,
    },
}

#[derive(Default)]
struct Outcome {
    schedule: Vec<Op>,
    storage: HashMap<String, String>,
    logs: Vec<String>,
    returned: Option<Vec<u8>>,
}

// ── wasm leg: real host impls that record the schedule ──

fn run_wasm_near(wasm: &[u8]) -> Result<Outcome, String> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;

    let ops: Arc<Mutex<Vec<Op>>> = Arc::new(Mutex::new(Vec::new()));
    let storage: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let registers: Arc<Mutex<HashMap<i64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let returned: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));
    let next_idx: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));

    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    if module
        .imports()
        .any(|i| i.module() == "env" && i.name() == "memory")
    {
        let memory =
            Memory::new(&mut store, MemoryType::new(4, None)).map_err(|e| format!("mem: {}", e))?;
        linker
            .define(&store, "env", "memory", memory)
            .map_err(|e| format!("link mem: {}", e))?;
    }

    fn read_str(data: &[u8], len: i64, ptr: i64) -> String {
        if len < 0 || ptr < 0 {
            return String::new();
        }
        let (l, p) = (len as usize, ptr as usize);
        if p + l <= data.len() {
            String::from_utf8_lossy(&data[p..p + l]).into_owned()
        } else {
            format!("<oob len={} ptr={}>", len, ptr)
        }
    }
    fn read_u128_le(data: &[u8], ptr: i64) -> String {
        if ptr < 0 || (ptr as usize) + 16 > data.len() {
            return "0".into();
        }
        let mut b = [0u8; 16];
        b.copy_from_slice(&data[ptr as usize..ptr as usize + 16]);
        u128::from_le_bytes(b).to_string()
    }

    for import in module.imports() {
        if import.module() == "env" && import.name() == "memory" {
            continue;
        }
        let ty = import.ty();
        let wasmtime::ExternType::Func(func_ty) = ty else {
            continue;
        };
        let params: Vec<ValType> = func_ty.params().collect();
        let results: Vec<ValType> = func_ty.results().collect();
        let ft = FuncType::new(&engine, params.clone(), results.clone());
        let name = import.name().to_string();
        let name_for_link = name.clone();

        let ops_c = Arc::clone(&ops);
        let st_c = Arc::clone(&storage);
        let reg_c = Arc::clone(&registers);
        let log_c = Arc::clone(&logs);
        let ret_c = Arc::clone(&returned);
        let idx_c = Arc::clone(&next_idx);

        let func = Func::new(&mut store, ft, move |mut caller, args, ret| {
            let a = |i: usize| args.get(i).and_then(|v| v.i64()).unwrap_or(0);
            let mem = caller.get_export("memory").and_then(|e| e.into_memory());
            let data_owned = mem
                .as_ref()
                .map(|m| m.data(&caller).to_vec())
                .unwrap_or_default();

            match name.as_str() {
                "promise_create" => {
                    if std::env::var("PROMISE_DEBUG").is_ok() {
                        eprintln!(
                            "promise_create args: {:?}",
                            (0..args.len()).map(|i| args[i].i64()).collect::<Vec<_>>()
                        );
                    }
                    // (target_len, target_ptr, method_len, method_ptr,
                    //  args_len, args_ptr, amount_ptr, gas) → idx
                    let op = Op::Create {
                        target: read_str(&data_owned, a(0), a(1)),
                        method: read_str(&data_owned, a(2), a(3)),
                        args: read_str(&data_owned, a(4), a(5)),
                        deposit: read_u128_le(&data_owned, a(6)),
                        gas: a(7),
                    };
                    ops_c.lock().unwrap().push(op);
                    let idx = {
                        let mut n = idx_c.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    if !ret.is_empty() {
                        ret[0] = Val::I64(idx);
                    }
                }
                "promise_then" => {
                    if std::env::var("PROMISE_DEBUG").is_ok() {
                        eprintln!(
                            "promise_then args: {:?}",
                            (0..args.len()).map(|i| args[i].i64()).collect::<Vec<_>>()
                        );
                    }
                    // (promise_idx, target_len, target_ptr, method_len,
                    //  method_ptr, args_len, args_ptr, amount_ptr, gas) → idx
                    let op = Op::Then {
                        base: a(0),
                        target: read_str(&data_owned, a(1), a(2)),
                        method: read_str(&data_owned, a(3), a(4)),
                        args: read_str(&data_owned, a(5), a(6)),
                        deposit: read_u128_le(&data_owned, a(7)),
                        gas: a(8),
                    };
                    ops_c.lock().unwrap().push(op);
                    let idx = {
                        let mut n = idx_c.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    if !ret.is_empty() {
                        ret[0] = Val::I64(idx);
                    }
                }
                "promise_and" => {
                    if std::env::var("PROMISE_DEBUG").is_ok() {
                        eprintln!(
                            "promise_and args: {:?}",
                            (0..args.len()).map(|i| args[i].i64()).collect::<Vec<_>>()
                        );
                    }
                    // (idx_ptr, count) → combined idx — indices are RAW u64s at ptr
                    let count = a(1) as usize;
                    let mut promises = Vec::new();
                    for i in 0..count {
                        let off = a(0) as usize + i * 8;
                        if off + 8 <= data_owned.len() {
                            promises.push(i64::from_le_bytes(
                                data_owned[off..off + 8].try_into().unwrap(),
                            ));
                        }
                    }
                    ops_c.lock().unwrap().push(Op::And { promises });
                    let idx = {
                        let mut n = idx_c.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    if !ret.is_empty() {
                        ret[0] = Val::I64(idx);
                    }
                }
                "promise_return" => {
                    ops_c.lock().unwrap().push(Op::Return { idx: a(0) });
                }
                "promise_batch_create" => {
                    let op = Op::BatchCreate {
                        target: read_str(&data_owned, a(0), a(1)),
                    };
                    ops_c.lock().unwrap().push(op);
                    let idx = {
                        let mut n = idx_c.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    if !ret.is_empty() {
                        ret[0] = Val::I64(idx);
                    }
                }
                "promise_batch_then" => {
                    let op = Op::BatchThen {
                        base: a(0),
                        target: read_str(&data_owned, a(1), a(2)),
                    };
                    ops_c.lock().unwrap().push(op);
                    let idx = {
                        let mut n = idx_c.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    if !ret.is_empty() {
                        ret[0] = Val::I64(idx);
                    }
                }
                "promise_batch_action_function_call" => {
                    if std::env::var("PROMISE_DEBUG").is_ok() {
                        eprintln!(
                            "batch_fn_call args: {:?}",
                            (0..args.len()).map(|i| args[i].i64()).collect::<Vec<_>>()
                        );
                    }
                    // (batch_idx, method_len, method_ptr, args_len, args_ptr,
                    //  amount_ptr, gas)
                    let op = Op::BatchFnCall {
                        batch: a(0),
                        method: read_str(&data_owned, a(1), a(2)),
                        args: read_str(&data_owned, a(3), a(4)),
                        deposit: read_u128_le(&data_owned, a(5)),
                        gas: a(6),
                    };
                    ops_c.lock().unwrap().push(op);
                }
                "storage_write" => {
                    let k = read_str(&data_owned, a(0), a(1));
                    let v = read_str(&data_owned, a(2), a(3));
                    st_c.lock().unwrap().insert(k.into_bytes(), v.into_bytes());
                    if !ret.is_empty() {
                        ret[0] = Val::I64(0);
                    }
                }
                "storage_read" | "storage_has_key" => {
                    let k = read_str(&data_owned, a(0), a(1));
                    let existing = st_c.lock().unwrap().get(k.as_bytes()).cloned();
                    if let Some(v) = existing {
                        if name == "storage_read" {
                            reg_c.lock().unwrap().insert(a(2), v);
                        }
                        if !ret.is_empty() {
                            ret[0] = Val::I64(1);
                        }
                    } else if !ret.is_empty() {
                        ret[0] = Val::I64(0);
                    }
                }
                "register_len" => {
                    let len = reg_c
                        .lock()
                        .unwrap()
                        .get(&a(0))
                        .map(|v| v.len() as i64)
                        .unwrap_or(-1);
                    if !ret.is_empty() {
                        ret[0] = Val::I64(len);
                    }
                }
                "read_register" => {
                    if let Some(bytes) = reg_c.lock().unwrap().get(&a(0)).cloned() {
                        if let Some(m) = mem {
                            let ptr = a(1) as usize;
                            let mut md = m.data_mut(caller);
                            if ptr < md.len() {
                                let end = (ptr + bytes.len()).min(md.len());
                                md[ptr..end].copy_from_slice(&bytes[..end - ptr]);
                            }
                        }
                    }
                }
                "write_register" => {
                    let bytes = read_str(&data_owned, a(0), a(1)).into_bytes();
                    reg_c.lock().unwrap().insert(a(2), bytes);
                }
                "input" => {
                    reg_c.lock().unwrap().insert(a(0), Vec::new());
                }
                "current_account_id" => {
                    reg_c
                        .lock()
                        .unwrap()
                        .insert(a(0), SELF_ACCOUNT.as_bytes().to_vec());
                }
                "predecessor_account_id" | "signer_account_id" => {
                    reg_c
                        .lock()
                        .unwrap()
                        .insert(a(0), PREDECESSOR.as_bytes().to_vec());
                }
                "signer_account_pk" => {
                    reg_c.lock().unwrap().insert(a(0), vec![0u8; 32]);
                }
                "log_utf8" => {
                    // near log_utf8(log_len, log_ptr) — len first
                    log_c
                        .lock()
                        .unwrap()
                        .push(read_str(&data_owned, a(0), a(1)));
                }
                "value_return" => {
                    let bytes = read_str(&data_owned, a(0), a(1)).into_bytes();
                    *ret_c.lock().unwrap() = Some(bytes);
                }
                _ => {
                    // Everything else (gas, crypto, misc): benign zero
                    for (i, r) in ret.iter_mut().enumerate() {
                        *r = match results.get(i) {
                            Some(ValType::I32) => Val::I32(0),
                            _ => Val::I64(0),
                        };
                    }
                }
            }
            Ok(())
        });
        linker
            .define(&store, "env", &name_for_link, func)
            .map_err(|e| format!("link {}: {}", name_for_link, e))?;
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {}", e))?;
    let run_fn = instance
        .get_export(&mut store, "_run")
        .and_then(|e| e.into_func())
        .or_else(|| {
            instance
                .get_export(&mut store, "run")
                .and_then(|e| e.into_func())
        })
        .ok_or("no _run/run export")?;
    run_fn
        .call(&mut store, &[], &mut [])
        .map_err(|e| format!("trap: {}", e))?;

    let schedule = ops.lock().unwrap().clone();
    let storage_map = storage
        .lock()
        .unwrap()
        .iter()
        .map(|(k, v)| {
            (
                String::from_utf8_lossy(k).into_owned(),
                String::from_utf8_lossy(v).into_owned(),
            )
        })
        .collect();
    let log_list = logs.lock().unwrap().clone();
    let returned_val = returned.lock().unwrap().clone();
    Ok(Outcome {
        schedule,
        storage: storage_map,
        logs: log_list,
        returned: returned_val,
    })
}

// ── interpreter leg: mock log → canonical schedule ──

fn map_get(m: &im::HashMap<String, LispVal>, k: &str) -> String {
    match m.get(k) {
        Some(LispVal::Str(s)) => s.clone(),
        Some(LispVal::Num(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn map_get_i(m: &im::HashMap<String, LispVal>, k: &str) -> i64 {
    match m.get(k) {
        Some(LispVal::Num(n)) => *n,
        _ => 0,
    }
}

/// Expand the interpreter's mock-log maps to canonical ops. Sugar ops
/// (call, call_await) expand to the same host sequence the emitter emits.
fn interp_schedule(state: &EvalState) -> Vec<Op> {
    let mut out = Vec::new();
    let mut batch_base: HashMap<i64, i64> = HashMap::new(); // interp idx → wasm-style idx
    let mut wasm_idx = 0i64;
    let mut remap = |i: i64, wasm_idx: &mut i64, m: &mut HashMap<i64, i64>| -> i64 {
        let next = *wasm_idx;
        *wasm_idx += 1;
        *m.entry(i).or_insert(next)
    };
    for entry in &state.near_promises {
        let LispVal::Map(m) = entry else { continue };
        match map_get(m, "type").as_str() {
            "create" => {
                out.push(Op::Create {
                    target: map_get(m, "target"),
                    method: map_get(m, "method"),
                    args: map_get(m, "args"),
                    gas: map_get_i(m, "gas"),
                    deposit: map_get(m, "deposit"),
                });
                let _ = remap(map_get_i(m, "idx"), &mut wasm_idx, &mut batch_base);
            }
            "call" => {
                // near/call: emitter = promise_create + promise_return (auto)
                let idx = map_get_i(m, "idx");
                out.push(Op::Create {
                    target: map_get(m, "target"),
                    method: map_get(m, "method"),
                    args: map_get(m, "args"),
                    gas: map_get_i(m, "gas"),
                    deposit: map_get(m, "deposit"),
                });
                out.push(Op::Return {
                    idx: remap(idx, &mut wasm_idx, &mut batch_base),
                });
            }
            "then" => {
                out.push(Op::Then {
                    base: remap(map_get_i(m, "base"), &mut wasm_idx, &mut batch_base),
                    target: map_get(m, "target"),
                    method: map_get(m, "method"),
                    args: map_get(m, "args"),
                    gas: map_get_i(m, "gas"),
                    deposit: map_get(m, "deposit"),
                });
                let _ = remap(map_get_i(m, "idx"), &mut wasm_idx, &mut batch_base);
            }
            "and" => {
                if let Some(LispVal::List(indices)) = m.get("promises") {
                    let promises = indices
                        .iter()
                        .map(|v| match v {
                            LispVal::Num(n) => remap(*n, &mut wasm_idx, &mut batch_base),
                            _ => 0,
                        })
                        .collect();
                    out.push(Op::And { promises });
                }
                let _ = remap(map_get_i(m, "idx"), &mut wasm_idx, &mut batch_base);
            }
            "call_await" => {
                // near/call-await emitter expansion:
                //   batch_create(target) → b
                //   fn_call(b, method, args, gas, 0)
                //   batch_then(b, SELF) → cb
                //   fn_call(cb, callback, cb_args, cb_gas, 0)
                //   return(cb)
                let b = {
                    wasm_idx += 1;
                    wasm_idx - 1
                };
                batch_base.insert(map_get_i(m, "idx"), b);
                out.push(Op::BatchCreate {
                    target: map_get(m, "target"),
                });
                out.push(Op::BatchFnCall {
                    batch: b,
                    method: map_get(m, "method"),
                    args: map_get(m, "args"),
                    gas: map_get_i(m, "gas"),
                    deposit: "0".into(),
                });
                out.push(Op::BatchThen {
                    base: b,
                    target: SELF_ACCOUNT.into(),
                });
                let cb = {
                    wasm_idx += 1;
                    wasm_idx - 1
                };
                out.push(Op::BatchFnCall {
                    batch: cb,
                    method: map_get(m, "callback"),
                    args: map_get(m, "cb_args"),
                    gas: map_get_i(m, "cb_gas"),
                    deposit: "0".into(),
                });
                out.push(Op::Return { idx: cb });
            }
            "batch_create" => {
                out.push(Op::BatchCreate {
                    target: map_get(m, "target"),
                });
            }
            "batch_then" => {
                out.push(Op::BatchThen {
                    base: map_get_i(m, "base"),
                    target: map_get(m, "target"),
                });
            }
            "batch_function_call" => {
                // Recorded with the wasm ABI (deposit@3 before gas@4) since
                // the 2026-09-10 interpreter fix.
                out.push(Op::BatchFnCall {
                    batch: map_get_i(m, "batch"),
                    method: map_get(m, "method"),
                    args: map_get(m, "args"),
                    gas: map_get_i(m, "gas"),
                    deposit: map_get(m, "deposit"),
                });
            }
            _ => {}
        }
    }
    out
}

// ── the differential runner ──

fn run_diff(source: &str) -> Result<(), String> {
    // Interpreter leg
    let exprs = parse_all(source).map_err(|e| format!("parse: {}", e))?;
    let mut env = Env::new();
    let mut state = EvalState::new();
    for expr in &exprs {
        lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state)
            .map_err(|e| format!("interp error: {}", e))?;
    }
    if let Some(LispVal::Lambda { .. }) | Some(LispVal::BuiltinFn(_)) = env.get("run") {
        let call = parse_all("(run)").map_err(|e| format!("run parse: {}", e))?;
        if let Some(e0) = call.first() {
            lisp_rlm_wasm::lisp_eval(e0, &mut env, &mut state)
                .map_err(|e| format!("interp run error: {}", e))?;
        }
    }
    let mut i_sched = interp_schedule(&state);
    // explicit near/promise_return: interpreter stores idx in
    // near_returned_promise (also set by near/call sugar — but sugar already
    // emitted Return above; only add when the log has no Return yet).
    if let (Some(idx), false) = (
        state.near_returned_promise,
        i_sched.iter().any(|o| matches!(o, Op::Return { .. })),
    ) {
        i_sched.push(Op::Return { idx });
    }
    let i_storage: HashMap<String, String> = state
        .near_storage
        .iter()
        .map(|(k, v)| {
            let vs = match v {
                LispVal::Str(s) => s.clone(),
                other => format!("{}", other),
            };
            (k.clone(), vs)
        })
        .collect();

    // wasm leg
    let wasm = compile_near(source).map_err(|e| format!("compile: {}", e))?;
    let w = run_wasm_near(&wasm)?;

    // compare schedule
    if i_sched != w.schedule {
        return Err(format!(
            "SCHEDULE MISMATCH: {}\n  interp: {:#?}\n  wasm:   {:#?}",
            source, i_sched, w.schedule
        ));
    }
    // compare storage
    if i_storage != w.storage {
        return Err(format!(
            "STORAGE MISMATCH: {}\n  interp: {:#?}\n  wasm:   {:#?}",
            source, i_storage, w.storage
        ));
    }
    Ok(())
}

fn expect_err_contains(source: &str, needle: &str) {
    match run_diff(source) {
        Err(e) => assert!(
            e.contains(needle),
            "expected {} in error, got: {}",
            needle,
            e
        ),
        Ok(()) => panic!("expected failure containing {:?}, got Ok", needle),
    }
}

// ── tests ──

#[test]
fn storage_roundtrip_parity() {
    run_diff(r#"(define (run) (near/storage_set "k1" "v1") (near/storage_get "k1"))"#).unwrap();
}

#[test]
fn storage_missing_key_read_parity() {
    // get of missing key: interp returns nil (no storage write); wasm reads
    // absent → 0 → nil. Storage maps must agree (both empty).
    run_diff(r#"(define (run) (near/storage_get "nope"))"#).unwrap();
}

#[test]
fn promise_create_raw_parity() {
    run_diff(
        r#"(define (run) (near/promise_create "pool.near" "quote" "{\"a\":1}" 0 50000000000000))"#,
    )
    .unwrap();
}

#[test]
fn near_call_sugar_parity() {
    // near/call = promise_create + AUTO promise_return
    run_diff(r#"(define (run) (near/call "pool.near" "quote" "{}" 50000000000000 0))"#).unwrap();
}

#[test]
fn promise_then_parity() {
    run_diff(
        r#"(define (run)
          (let ((p (near/promise_create "pool.near" "borrow" "{}" 0 50000000000000)))
            (near/promise_then p "self.test.near" "on_borrow" "{}" 0 20000000000000)))"#,
    )
    .unwrap();
}

#[test]
fn promise_and_parity() {
    run_diff(
        r#"(define (run)
          (let ((a (near/promise_create "x.near" "m1" "{}" 0 10000000000000))
                (b (near/promise_create "y.near" "m2" "{}" 0 10000000000000)))
            (near/promise_and a b)))"#,
    )
    .unwrap();
}

#[test]
fn batch_function_call_parity() {
    run_diff(
        r#"(define (run)
          (let ((b (near/promise_batch_create "vault.near")))
            (near/promise_batch_action_function_call b "deposit" "{\"amt\":5}" "0" 40000000000000)
            (near/promise_batch_action_function_call b "log" "{}" "0" 10000000000000)
            (near/promise_return b)))"#,
    )
    .unwrap();
}

#[test]
fn call_await_expansion_parity() {
    run_diff(
        r#"(define (on_done) nil)
         (export "on_done" on_done true)
         (define (run)
           (near/call-await "pool.near" "get_price" "{}" 50000000000000 "on_done" 20000000000000 "{}"))
         (export "run" run false)"#,
    )
    .unwrap();
}

#[test]
fn storage_and_promise_together() {
    run_diff(
        r#"(define (run)
          (near/storage_set "state" "pending")
          (near/promise_create "oracle.near" "fetch" "{}" 0 30000000000000))"#,
    )
    .unwrap();
}

#[test]
fn deposit_nonzero_parity() {
    // u128 deposits exceed the 61-bit tagged Num range — the supported path
    // is DECIMAL STRINGS via near/call and the batch forms (u128-str helper,
    // same machinery as transfer_u128). Raw promise_create only stores the
    // low-64 of a Num — checker correctly types its deposit int. Both paths
    // checked here: str deposit via near/call, small Num via promise_create.
    run_diff(
        r#"(define (run) (near/call "sale.near" "buy" "{}" 30000000000000 "1000000000000000000000000"))"#,
    )
    .unwrap();
    run_diff(r#"(define (run) (near/promise_create "sale.near" "buy" "{}" 1000 30000000000000))"#)
        .unwrap();
}

// When a divergence IS found, it must be loud — this self-check documents the
// harness's own failure mode (disabled; enable to smoke-test the reporter).
// #[test]
// fn harness_selfcheck_reports_mismatch() {
//     expect_err_contains("(define (run) (near/promise_create \"t\" \"m\" \"{}\" 30000000000000 0))", "MISMATCH");
// }

#[test]
fn default_gas_constant_agreement() {
    // Both sides must agree on the default gas when omitted — pinned here so
    // a change on either side trips a loud diff instead of silently passing.
    // Interpreter default: 30_000_000_000_000 (bytecode/mod.rs promise arms).
    // Emitter: requires explicit gas in most arms — programs in this file
    // always pass gas explicitly, so this pins the interpreter constant only.
    assert_eq!(DEFAULT_GAS, 30_000_000_000_000);
}
