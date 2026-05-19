use lisp_rlm_wasm::wasm_emit::compile_pure_to_wat;

#[test]
fn test_pure_wat() {
    let wat = compile_pure_to_wat(
        "(borsh-schema (Counter (count i64)))\n(define (t1) (borsh-serialize \"Counter\" 42))\n(export \"run\" t1 true)"
    ).unwrap();
    println!("{}", wat);
}
