//! Money-safety equivalence: overflow behavior interp vs wasm.
//! The interp TRAPS on i64 overflow (hard-error policy). The wasm emitter
//! emits checked arithmetic (emit_checked_add/sub → Unreachable on overflow).
//! If the emitter ever regresses to raw adds, an overflowing balance
//! diverges: interp errors, on-chain it wraps. That's a money-safety bug.

use lisp_rlm_wasm::*;
use wasmtime::*;

fn eval_interp(src: &str) -> Result<LispVal, String> {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let _ = lisp_rlm_wasm::program::run_program(
        &parse_all("(load-file \"runtime/harness.lisp\")")?,
        &mut env,
        &mut state,
    );
    let r = lisp_rlm_wasm::program::run_program(&parse_all(src)?, &mut env, &mut state)?;
    // If the program defines main, the P1 contract equivalent is running it.
    if src.contains("(define (main") {
        return lisp_rlm_wasm::program::run_program(&parse_all("(main)")?, &mut env, &mut state);
    }
    Ok(r)
}

/// Run a P1 outlayer WASM with host stubs. A wasm trap (overflow →
/// Unreachable, or proc_exit error) maps to Err. Normal exit returns the
/// i64 at the return buffer.
fn eval_wasm(expr_src: &str) -> Result<i64, String> {
    let src = if expr_src.contains("(define") {
        expr_src.to_string() // already a full program (top-level defines)
    } else {
        format!("(define (main) {})", expr_src)
    };
    let wasm = match lisp_rlm_wasm::compile_outlayer(&src) {
        Ok(w) => w,
        // Compile-time refusal (e.g. const-fold overflow hard-error) is a
        // SAFE outcome: the overflowing contract never deploys. Map to Err.
        Err(e) => return Err(format!("compile refused: {}", e)),
    };
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).expect("module load");
    let mut store = Store::new(&engine, Vec::new()); // data = captured stdout

    // --- WASI stubs ---
    let fd_read = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |_c, _a, r| { r[0] = Val::I32(0); Ok(()) });
    let fd_write = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |mut c, a, r| {
            let iov_ptr = a[1].unwrap_i32() as u32;
            let iov_len = a[2].unwrap_i32() as u32;
            let mut chunks: Vec<Vec<u8>> = Vec::new();
            {
                let mem = c.get_export("memory").and_then(|e| e.into_memory()).unwrap();
                let data = mem.data(&c);
                for i in 0..iov_len as usize {
                    let p = u32::from_le_bytes(data[(iov_ptr as usize + i*8)..][..4].try_into().unwrap()) as usize;
                    let l = u32::from_le_bytes(data[(iov_ptr as usize + i*8 + 4)..][..4].try_into().unwrap()) as usize;
                    chunks.push(data[p..p+l].to_vec());
                }
            }
            for chunk in chunks {
                c.data_mut().extend_from_slice(&chunk);
            }
            r[0] = Val::I32(a[2].unwrap_i32());
            Ok(())
        });
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
    let st_set = Func::wrap(&mut store, |_: i32, _: i32, _: i64, _: i32| {});
    let st_get = Func::wrap(&mut store, |_: i32, _: i64, _: i32| {});
    let st_has = Func::wrap(&mut store, |_: i32, _: i64| -> i32 { 0 });
    let st_del = Func::wrap(&mut store, |_: i32, _: i64| -> i32 { 0 });
    let st_incr = Func::wrap(&mut store, |_: i32, _: i32, _: i64, _: i32| {});
    let st_decr = Func::wrap(&mut store, |_: i32, _: i32, _: i64, _: i32| {});

    let storage_write = Func::wrap(&mut store, |_: u32, _: u32, _: u32, _: u32, _: u32| -> u32 { 0 });
    let promise_create = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32, _: i32| {});
    let promise_and = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let promise_then = Func::wrap(&mut store, |_: i32, _: i64, _: i64, _: i32, _: i32| {});
    let promise_result = Func::wrap(&mut store, |_: i32, _: i32, _: i32| {});

    // --- Link everything ---
    let mut linker = Linker::new(&engine);

    linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "random_get", noop_i32_i32_to_i32).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", noop_i32_i32_to_i32).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_get", noop_i32_i32_to_i32).unwrap();
    let fd_seek = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });
    linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek).unwrap();

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

    linker.define(&store, "outlayer", "view", ol_view).unwrap();
    linker.define(&store, "outlayer", "call", ol_call).unwrap();
    linker.define(&store, "outlayer", "transfer", ol_transfer).unwrap();
    linker.define(&store, "outlayer", "http_get", ol_http_get).unwrap();
    linker.define(&store, "outlayer", "http_post", ol_http_post).unwrap();
    linker.define(&store, "outlayer", "store", ol_store).unwrap();
    linker.define(&store, "outlayer", "load", ol_load).unwrap();
    linker.define(&store, "outlayer", "remove", ol_remove).unwrap();
    linker.define(&store, "outlayer", "has", ol_has).unwrap();

    linker.define(&store, "near:rpc/api@0.1.0", "view", rpc_view).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "call", rpc_call).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "transfer", rpc_transfer).unwrap();

    linker.define(&store, "near:storage/api@0.1.0", "set", st_set).unwrap();
    linker.define(&store, "near:storage/api@0.1.0", "get", st_get).unwrap();
    linker.define(&store, "near:storage/api@0.1.0", "has", st_has).unwrap();
    linker.define(&store, "near:storage/api@0.1.0", "delete", st_del).unwrap();
    linker.define(&store, "near:storage/api@0.1.0", "increment", st_incr).unwrap();
    linker.define(&store, "near:storage/api@0.1.0", "decrement", st_decr).unwrap();

    let instance = linker.instantiate(&mut store, &module).expect("instantiate");
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start")
        .expect("_start export");

    let ret = start.call(&mut store, ());

    // proc_exit(0) is the NORMAL exit path (stub raises it as a host error).
    // Any other trap (unreachable = checked-arith overflow, OOB, etc.) is Err.
    // NB: use {:?} — wasmtime's Display hides the host-error cause chain.
    match ret {
        Ok(()) => {}
        Err(trap) => {
            let dbg = format!("{:?}", trap);
            if !dbg.contains("proc_exit") {
                return Err(format!("{:?}", trap));
            }
        }
    }

    // P1 result is printed to stdout (ASCII number or string); parse it back.
    let out = store.data().clone();
    let s = String::from_utf8_lossy(&out).trim().to_string();
    if s.is_empty() {
        return Err(format!("empty stdout: {:?}", out));
    }
    s.parse::<i64>().map_err(|e| format!("stdout not a number ({:?}): {}", s, e))
}

/// Money-safety invariant: interp and wasm must agree on trap-vs-value.
/// If interp traps and wasm returns a value → SILENT BALANCE WRAP on-chain.
fn assert_money_safe(src: &str, ctx: &str) {
    let interp = eval_interp(src);
    let wasm = eval_wasm(src);
    match (interp, wasm) {
        (Err(_), Ok(v)) => panic!(
            "DIVERGENCE ({}): interp traps but wasm returned {} — \
             balances silently wrap on-chain. Emitter needs checked ops.",
            ctx, v
        ),
        (Err(_), Err(_)) => {}   // aligned: both trap
        (Ok(a), Ok(b)) => {
            let a_num = match a {
                LispVal::Num(n) => n,
                other => panic!("interp returned non-num {:?} for ({})", other, ctx),
            };
            assert_eq!(a_num, b, "both returned but different values ({})", ctx);
        }
        (Ok(a), Err(e)) => panic!("interp ok {:?} but wasm errored ({}): {}", a, ctx, e),
    }
}

#[test]
fn overflow_add_i64_max() {
    assert_money_safe("(+ 9223372036854775807 1)", "add-max");
}

#[test]
fn overflow_sub_i64_min() {
    assert_money_safe("(- -9223372036854775808 1)", "sub-min");
}

#[test]
fn overflow_mul() {
    assert_money_safe("(* 4611686018427387905 4)", "mul-2^62ish");
}

#[test]
fn no_overflow_sanity() {
    assert_money_safe("(+ 123456789 987654321)", "add-normal");
}

#[test]
fn balance_like_ops_sanity() {
    // typical token math shape: balance + amount, both in range
    assert_money_safe("(+ 1000000000000000000 250000000000000000)", "deposit-add");
}

#[test]
fn interp_actually_traps_on_overflow() {
    let r = eval_interp("(+ 9223372036854775807 1)");
    assert!(
        r.as_ref().is_err_and(|e| e.contains("overflow")),
        "interp must trap on i64 overflow, and it does not: {:?}",
        r
    );
}

/// The dangerous case: overflow from RUNTIME values (storage reads, call
/// results) can't be caught by const-fold. The emitter must emit checked
/// arithmetic there — raw i64.add would silently wrap balances on-chain.
#[test]
fn runtime_overflow_add_traps_in_wasm() {
    // identity fn defeats const-fold; overflow happens at run time
    // 2^59 + 2^59 = 2^60: fits i64 (interp used to allow it) but leaves the
    // 61-bit tagged payload range → wasm traps; interp must trap too.
    assert_money_safe(
        "(define (id x) x) (define (main) (+ (id 576460752303423488) (id 576460752303423488)))",
        "runtime-add-2^60",
    );
}

#[test]
fn runtime_overflow_sub_traps_in_wasm() {
    // -2^60 - 1: leaves the payload range at the negative edge
    assert_money_safe(
        "(define (id x) x) (define (main) (- (id -1152921504606846976) (id 1)))",
        "runtime-sub--2^60-1",
    );
}

#[test]
fn runtime_no_overflow_agrees() {
    assert_money_safe(
        "(define (id x) x) (define (main) (+ (id 123456789) (id 987654321)))",
        "runtime-add-normal",
    );
}

// ═══ shl retag — the only widening bitop ═══
// band/bor/bnot/shr outputs stay within input range; shl·s can leave
// [-2^60, 2^60). Before this fix the emitter re-tagged with a bare shl —
// silent wrap. Now emit_tag_num_checked traps.

/// wasm-only: shl isn't in the interp surface (drift class, GAPS).
/// In-range shifts must work exactly as before.
#[test]
fn shl_in_range_returns_value() {
    // 2^57 << 2 = 2^59 — inside payload range
    let v = eval_wasm(
        "(define (id x) x) (define (main) (shl (id 144115188075855872) (id 2)))",
    ).expect("in-range shl must succeed");
    assert_eq!(v, 576460752303423488, "2^57<<2 == 2^59");
}

/// wasm-only: shifted-out-of-payload-range must TRAP, never wrap.
#[test]
fn shl_out_of_range_traps() {
    // 2^59 << 1 = 2^60: fits i64 but leaves [-2^60, 2^60) → trap
    let r = eval_wasm(
        "(define (id x) x) (define (main) (shl (id 576460752303423488) (id 1)))",
    );
    assert!(r.is_err(), "shl to 2^60 must trap, got {:?}", r);

    // 2^55 << 10 = 2^65: overflows i64 entirely → trap
    let r = eval_wasm(
        "(define (id x) x) (define (main) (shl (id 36028797018963968) (id 10)))",
    );
    assert!(r.is_err(), "shl to 2^65 must trap, got {:?}", r);
}

/// wasm-only: negative shift / huge shift masks like raw wasm (s & 63).
#[test]
fn shl_shift_masking_survives() {
    // shift 64 ≡ 0 (wasm masks k&63) → identity, must NOT trap
    let v = eval_wasm(
        "(define (id x) x) (define (main) (shl (id 123) (id 64)))",
    ).expect("shl by 64 masks to 0");
    assert_eq!(v, 123);
}
