//! u128 regression (2026-08-31): nested u128 ops in the SECOND operand
//! position clobbered the enclosing op's saved operand locals
//! (`__u128sa/__u128sb` shared across nesting levels) —
//! (u128/lt (u128/mul A B) (u128/mul C D)) compared C's raw operand
//! instead of A*B's result. Interp was right; wasm lied. Found via the
//! TS lending protocol (JP: "that not using the actual u128 token
//! precision"), root-caused with wasm2wat.

use lisp_rlm_wasm::parse_all;
use lisp_rlm_wasm::typing;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn wasm_eval(src: &str) -> Result<String, String> {
    let exprs = parse_all(src).map_err(|e| e.to_string())?;
    let wasm = lisp_rlm_wasm::wasm_emit::compile_fuzz(src)?;
    let _ = &exprs;
    // execute on wasmtime with zeroed env imports; run writes i64 at addr 64
    use wasmtime::*;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    linker.func_wrap("env", "read_register", |_: Caller<'_, ()>, _: i64, _: i64| {}).map_err(|e| e.to_string())?;
    linker.func_wrap("env", "register_len", |_: i64| -> i64 { 0 }).map_err(|e| e.to_string())?;
    linker.func_wrap("env", "input", |_: Caller<'_, ()>, _: i64| {}).map_err(|e| e.to_string())?;
    linker.func_wrap("env", "value_return", |_: Caller<'_, ()>, _: i64, _: i64| {}).map_err(|e| e.to_string())?;
    let inst = linker.instantiate(&mut store, &module).map_err(|e| e.to_string())?;
    let run = inst.get_typed_func::<(), ()>(&mut store, "run").map_err(|e| e.to_string())?;
    run.call(&mut store, ()).map_err(|e| e.to_string())?;
    let mem = inst.get_memory(&mut store, "memory").unwrap();
    let mut rb = [0u8; 8];
    mem.read(&mut store, 64, &mut rb).map_err(|e| e.to_string())?;
    let v = i64::from_le_bytes(rb);
    let tag = v & 7;
    let payload = ((v as u64) >> 3) as u64;
    if tag == 5 {
        let ptr = (payload & 0xFFFF_FFFF) as usize;
        let len = ((payload as u64) >> 32) as usize;
        let mut buf = vec![0u8; len];
        mem.read(&mut store, ptr, &mut buf).map_err(|e| e.to_string())?;
        Ok(format!("str:{}", String::from_utf8_lossy(&buf)))
    } else {
        Ok(format!("num:{}", (v >> 3) as i64))
    }
}

#[test]
fn nested_second_operand_no_clobber() {
    // 2.625e28 < 2.1e28 → false; pre-fix wasm said 1 (compared "2100..." vs result)
    let r = wasm_eval(
        r#"(define (run) (u128/lt (u128/mul "5250000000000000000000000" "5000") (u128/mul "2100000000000000000000000" "10000")))"#,
    ).expect("must run");
    assert_eq!(r, "num:0");

    // computed right, literal left (the clobber shape that returned 1)
    let r = wasm_eval(
        r#"(define (run) (u128/lt "26250000000000000000000000000" (u128/add "20999999999999999999999999999" "1")))"#,
    ).expect("must run");
    assert_eq!(r, "num:0");

    // small nested both-sides sanity
    let r = wasm_eval(
        r#"(define (run) (u128/lt (u128/mul "30" "40") (u128/mul "25" "50")))"#,
    ).expect("must run");
    assert_eq!(r, "num:1");
}

#[test]
fn ts_bigint_surface() {
    // bigint operators + const folding + BigIntLiteral 'n'-strip
    let ts = r#"
        const SCALE = 10000n;
        const FEE_BP = 500n;
        export function t(amt: bigint): string {
          let add = (amt * (SCALE + FEE_BP) + (SCALE - 1n)) / SCALE;
          return add;
        }
    "#;
    let ir = ts_to_lisp_source(ts).expect("lowering");
    // consts fold to decimal strings; all arithmetic routes through u128/*
    // (no raw + - * / in bigint context — those trap in the type checker)
    assert!(ir.contains(r#"(u128/mul amt (u128/add "10000" "500"))"#),
        "consts must fold to strings and stay in u128 ops: {ir}");
    assert!(ir.contains(r#"(u128/div (u128/add (u128/mul amt"#),
        "fee math must be u128 all the way: {ir}");
    assert!(!ir.contains("(+ ") && !ir.contains("(* ") && !ir.contains("(- "),
        "no raw i64 arithmetic may appear in bigint context: {ir}");
}
