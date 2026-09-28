use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
fn main() {
    let src = std::fs::read_to_string("fixtures/lending.ts").unwrap();
    let ir = ts_to_lisp_source(&src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    std::fs::write("/tmp/lending_nm.wasm", &wasm).unwrap();
    println!("ok {}", wasm.len());
}
