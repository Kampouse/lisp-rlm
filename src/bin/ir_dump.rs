fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&path).unwrap();
    match lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src) {
        Ok(ir) => print!("{}", ir),
        Err(e) => eprintln!("lower error: {}", e),
    }
}
