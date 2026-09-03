fn main() {
    let src = std::fs::read_to_string("fixtures/surface_tour2_exotic.ts").unwrap();
    let ir = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src).unwrap();
    println!("{}", ir);
}
