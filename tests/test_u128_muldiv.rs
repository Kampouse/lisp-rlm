//! u128/muldiv — the CLMM core intrinsic (q = a·b/d over the full 256-bit
//! product). Raydium-CLMM-grade swap math needs it: pool formulas like
//! `dy = L·(sqrtP_next − sqrtP_cur) >> 64` are muldivs, and a plain
//! u128/mul hard-errors on the 256-bit intermediate.
//!
//! Vectors from exact 256-bit Python arithmetic (overflow rows assert the
//! ERR — same contract as Raydium's FullMath::mulDiv). Both runtimes are
//! asserted: the interp VM (bytecode) and the WASM emitter output
//! (string-ABI arm + limb-local fast path + try/catch checked variant).

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::run_program;

// (name, a, b, d, expected_q, expect_err)
const VECTORS: &[(&str, &str, &str, &str, &str, bool)] = &[
    (
        "q64-passthrough",
        "1000000000000000000",
        "18446744073709551616",
        "18446744073709551616",
        "1000000000000000000",
        false,
    ),
    (
        "dy-from-delta",
        "1000000000000000000000",
        "27670116110564327424",
        "18446744073709551616",
        "1500000000000000000000",
        false,
    ),
    (
        "yocto",
        "1000000000000000000000000",
        "1000000000000000000000000",
        "1000000000000000000",
        "1000000000000000000000000000000",
        false,
    ),
    (
        "u64-amount",
        "18446744073709551616",
        "18446744073709551615",
        "18446744073709551616",
        "18446744073709551615",
        false,
    ),
    (
        "rem-nontrivial",
        "1152921504606846976",
        "1152921504606846977",
        "7",
        "189889713683559410579532652126741650",
        false,
    ),
    (
        "max128-safe",
        "340282366920938463463374607431768211455",
        "340282366920938463463374607431768211455",
        "340282366920938463463374607431768211455",
        "340282366920938463463374607431768211455",
        false,
    ),
    (
        "overflow-q",
        "340282366920938463463374607431768211456",
        "18446744073709551616",
        "1",
        "",
        true,
    ),
    (
        "overflow-tiny-d",
        "170141183460469231731687303715884105728",
        "170141183460469231731687303715884105728",
        "1267650600228229401496703205376",
        "",
        true,
    ),
];

/// Interp VM: eval `(u128/muldiv a b d)` directly.
fn interp_muldiv(a: &str, b: &str, d: &str) -> Result<String, String> {
    let src = format!(r#"(define (run) (u128/muldiv "{}" "{}" "{}"))"#, a, b, d);
    let exprs = parse_all(&src).map_err(|e| e.to_string())?;
    let mut env = lisp_rlm_wasm::Env::new();
    let mut state = lisp_rlm_wasm::EvalState::new();
    lisp_rlm_wasm::run_program(&exprs, &mut env, &mut state).map_err(|e| e.to_string())?;
    let call = parse_all("(run)").map_err(|e| e.to_string())?;
    match lisp_rlm_wasm::run_program(&call, &mut env, &mut state).map_err(|e| e.to_string())? {
        lisp_rlm_wasm::LispVal::Str(s) => Ok(s),
        other => Ok(format!("{:?}", other)),
    }
}

/// Wasm: same source through the NEAR-target emitter, executed on wasmtime,
/// result decoded from TEMP_MEM (the harness in test_u128_nested.rs).
fn wasm_eval(src: &str) -> Result<String, String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_fuzz(src).map_err(|e| e.to_string())?;
    use wasmtime::*;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    linker
        .func_wrap(
            "env",
            "read_register",
            |_: Caller<'_, ()>, _: i64, _: i64| {},
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "register_len", |_: Caller<'_, ()>, _: i64| -> i64 {
            0
        })
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "input", |_: Caller<'_, ()>, _: i64| {})
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            "env",
            "value_return",
            |_: Caller<'_, ()>, _: i64, _: i64| {},
        )
        .map_err(|e| e.to_string())?;
    let inst = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;
    let run = inst
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| e.to_string())?;
    // compile_fuzz emits `run` writing the tagged result at addr 64? The
    // nested test used `run` + read addr 64 — mirror exactly.
    run.call(&mut store, ()).map_err(|e| e.to_string())?;
    let mem = inst.get_memory(&mut store, "memory").unwrap();
    let mut rb = [0u8; 8];
    mem.read(&mut store, 64, &mut rb)
        .map_err(|e| e.to_string())?;
    let v = i64::from_le_bytes(rb);
    let tag = v & 7;
    let payload = ((v as u64) >> 3) as u64;
    if tag == 5 {
        let ptr = (payload & 0xFFFF_FFFF) as usize;
        let len = ((payload as u64) >> 32) as usize;
        let mut buf = vec![0u8; len];
        mem.read(&mut store, ptr, &mut buf)
            .map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    } else {
        Ok(format!("num:{}", (v >> 3) as i64))
    }
}

#[test]
fn muldiv_vectors_both_runtimes() {
    for (name, a, b, d, expect, expect_err) in VECTORS {
        // interpreter
        let i = interp_muldiv(a, b, d);
        // wasm (string-ABI arm)
        let src = if *expect_err {
            format!(
                r#"(define (run) (try (u128/muldiv "{}" "{}" "{}") (catch e "ERR")))"#,
                a, b, d
            )
        } else {
            format!(r#"(define (run) (u128/muldiv "{}" "{}" "{}"))"#, a, b, d)
        };
        let w = wasm_eval(&src);
        if *expect_err {
            assert!(i.is_err(), "interp must error on {name}: {:?}", i);
            assert!(
                w.is_err() || w.as_deref() == Ok("ERR"),
                "wasm must error on {name}: {:?}",
                w
            );
        } else {
            let iw = i.unwrap_or_else(|e| panic!("interp failed on {name}: {e}"));
            // interp wraps result in tagged string form — strip a leading
            // "str:" if the harness adds one
            let iw = iw.strip_prefix("str:").unwrap_or(&iw).to_string();
            assert_eq!(iw, *expect, "interp vector {name}");
            let ww = w.unwrap_or_else(|e| panic!("wasm failed on {name}: {e}"));
            let ww = ww.strip_prefix("str:").unwrap_or(&ww).to_string();
            assert_eq!(ww, *expect, "wasm vector {name}");
        }
    }
}

#[test]
fn muldiv_limb_fastpath_agrees() {
    // The limb-local Level-1 path (nested/limb operands) must produce the
    // same values as the string arm — feed NESTED muldivs.
    let src = r#"(define (run) (u128/lt (u128/muldiv "1000000000000000000000000" "1000000000000000000000000" "1000000000000000000") "1000000000000000000000000000000"))"#;
    let w = wasm_eval(src).expect("nested muldiv must run");
    // 1e42/1e33 = 1e27; 1e27 < 1e27 false → num:0
    assert_eq!(w, "num:0");

    let src = r#"(define (run) (u128/lt (u128/muldiv "1000" "2000" "7") "3000000"))"#;
    let w = wasm_eval(src).expect("must run");
    // 2000000/7 = 285714 (floor) and 285714 < 3000000 → true
    assert_eq!(w, "num:1");
}

#[test]
fn muldiv_checked_variant_returns_false() {
    // Under an active try: overflow must resolve to TAGGED_FALSE → "(catch …)"
    // path (the _ck sentinel), not a hard trap.
    let src = r#"(define (run) (try (u128/muldiv "340282366920938463463374607431768211456" "18446744073709551616" "1") (catch e "ERR-caught")))"#;
    let w = wasm_eval(src).expect("checked muldiv must not hard-trap");
    assert!(w.contains("ERR-caught") || w.contains("ERR"), "got {w}");
}
