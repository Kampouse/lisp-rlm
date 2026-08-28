use lisp_rlm_wasm::wasm_emit::compile_fuzz;

fn main() {
    let src = r#"
(define (check)
  (let ((a 100) (b 200))
    (u128/store a 42 0)
    (u128/fit_i64 a)
    (u128/new 7 8 b)
    (u128/load b)
    (u128/load_high b)))
(export "check" check)
"#;
    let wasm = compile_fuzz(src).expect("compile");
    let wat = wasmprinter::print_bytes(&wasm).expect("wat");
    let lines: Vec<&str> = wat.lines().collect();
    // print only the function containing the u128 ops
    let mut on = false;
    for l in lines {
        if l.contains("(func $check") || l.contains("func (;4") || l.contains("func (;5") {
            on = true;
        }
        if on {
            println!("{}", l);
            if l.trim() == ")" && !l.contains("module") {
                on = false;
            }
        }
    }
    std::fs::write("/tmp/probe3.wat", wat).ok();
    println!("full wat: /tmp/probe3.wat ({} bytes)", wasm.len());
}
