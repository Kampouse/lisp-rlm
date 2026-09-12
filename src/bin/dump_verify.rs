use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;

fn main() {
    let src = std::fs::read_to_string("zk/groth16_verify.ts").expect("read zk/groth16_verify.ts");
    match ts_to_lisp_source(&src) {
        Ok(ir) => {
            println!("OK {} bytes", ir.len());
            // Print just the init and verify functions (first ~3000 chars)
            let end = ir.len().min(5000);
            println!("{}", &ir[..end]);
        }
        Err(e) => {
            eprintln!("COMPILE ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
