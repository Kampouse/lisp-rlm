fn main() {
    let src = std::fs::read_to_string("/tmp/src.txt").unwrap();
    match lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src) {
        Ok(l) => println!("OK:\n{}", l),
        Err(e) => println!("ERR: {}", e),
    }
}
