//! u128 TC parse-cache staleness regression (2026-10-06).
//!
//! Bug: the tail-call loop-back (`tc()` self-call arm, `self_sets`,
//! `tc_let`) rebinds param/let locals in the SAME wasm activation and
//! branches back — but the u128 runtime parse-cache flags (L1.5) for
//! those names were never invalidated. A param parsed as u128 on the
//! first loop iteration kept serving the FIRST value's limbs forever.
//! Found by accumulator-passing tail recursion returning a single byte
//! (acc parsed once as "0", every iteration mul'd the stale "0").
//! Fixed by emit_parse_cache_invalidate_names at every same-activation
//! loop-back (force-allocates the memo entry so eager flag=0 lands on
//! the same local a later textual use picks up).

use lisp_rlm_wasm::parse_all;

fn wasm_eval(src: &str) -> Result<String, String> {
    let exprs = parse_all(src).map_err(|e| e.to_string())?;
    let _ = &exprs;
    let wasm = lisp_rlm_wasm::wasm_emit::compile_fuzz(src)?;
    use wasmtime::*;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    linker
        .func_wrap("env", "read_register", |_: Caller<'_, ()>, _: i64, _: i64| {})
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "register_len", |_: i64| -> i64 { 0 })
        .map_err(|e| e.to_string())?;
    linker.func_wrap("env", "input", |_: Caller<'_, ()>, _: i64| {}).map_err(|e| e.to_string())?;
    linker
        .func_wrap("env", "value_return", |_: Caller<'_, ()>, _: i64, _: i64| {})
        .map_err(|e| e.to_string())?;
    let inst = linker.instantiate(&mut store, &module).map_err(|e| e.to_string())?;
    let run = inst
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| e.to_string())?;
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
        Ok(String::from_utf8_lossy(&buf).to_string())
    } else {
        Ok(format!("num:{}", (v >> 3) as i64))
    }
}

/// The exact shape that failed: accumulator-passing tail recursion whose
/// accumulator is a u128 string param. Before the fix this returned 100
/// (one byte — every iteration saw the stale "0" limbs from iteration 0).
#[test]
fn tailrec_accumulator_parse_cache() {
    // acc over bytes [100..107]: acc = acc*256 + b  (BE fold, 8 bytes)
    // = sum b_i * 256^(7-i)
    let src = r#"(define (f i acc)
  (if (>= i 8) acc
      (f (+ i 1) (u128/add (u128/mul acc "256") (itoa (byte-at s i))))))
(define s "hijklmnop")
(define (run) (f 0 "0"))"#;
    let r = wasm_eval(src).unwrap();
    let bs: Vec<i128> = b"hijklmnop"[..8].iter().map(|&b| b as i128).collect();
    let mut exp: i128 = 0;
    for &b in &bs {
        exp = exp * 256 + b;
    }
    assert_eq!(r, format!("{}", exp), "accumulator folded wrong — stale parse cache");
}

/// Same staleness via tc_let: a let binding rebound each loop iteration.
#[test]
fn tc_let_rebinding_parse_cache() {
    let src = r#"(define (count i total)
  (if (>= i 6) total
      (let ((next (u128/mul total "10")))
        (count (+ i 1) (u128/add next "7")))))
(define (run) (count 0 "0"))"#;
    let r = wasm_eval(src).unwrap();
    // 6 iterations of total = total*10 + 7 → 777777
    assert_eq!(r, "777777", "tc_let rebound binding served stale limbs");
}

/// self_sets path: self-call in BOTH if branches (param rebind per branch).
#[test]
fn if_branch_self_sets_parse_cache() {
    let src = r#"(define (grow i acc)
  (if (>= i 5) acc
      (if (= (mod i 2) 0)
          (grow (+ i 1) (u128/add (u128/mul acc "3") "1"))
          (grow (+ i 1) (u128/add (u128/mul acc "2") "5")))))
(define (run) (grow 0 "1"))"#;
    let r = wasm_eval(src).unwrap();
    // i=0: 1*3+1=4; i=1: 4*2+5=13; i=2: 13*3+1=40; i=3: 40*2+5=85; i=4: 85*3+1=256
    assert_eq!(r, "256", "if-branch self_sets served stale limbs");
}

/// Non-tail recursion (fresh activations) must stay correct — parse cache
/// is per-activation there and never needed invalidation.
#[test]
fn nontail_recursion_still_correct() {
    let src = r#"(define (f i)
  (if (>= i 8) "0"
      (u128/add (u128/mul (f (+ i 1)) "256") (itoa (byte-at s i)))))
(define s "hijklmnop")
(define (run) (f 0))"#;
    let r = wasm_eval(src).unwrap();
    let bs: Vec<i128> = b"hijklmnop".iter().map(|&b| b as i128).collect();
    // f(i) = f(i+1)*256 + b_i → f(0) = b7*256^7 + b6*256^6 + ... + b0
    let mut e: i128 = 0;
    for i in (0..8).rev() {
        e = e * 256 + bs[i];
    }
    assert_eq!(r, format!("{}", e));
}
