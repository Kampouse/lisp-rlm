fn main() {
    use std::io::Write;
    let mut err = std::io::stderr();
    writeln!(err, "START").ok(); err.flush().ok();
    
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 { writeln!(err, "usage").ok(); std::process::exit(1); }
    writeln!(err, "Reading {}", args[1]).ok(); err.flush().ok();
    
    let src = std::fs::read_to_string(&args[1]).unwrap();
    writeln!(err, "Parsed {} bytes", src.len()).ok(); err.flush().ok();
    
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lisp_rlm_wasm::wasm_emit::compile_near(&src)
    }));
    
    match result {
        Ok(Ok(wasm)) => {
            let out = args.get(2).cloned().unwrap_or_else(|| args[1].replace(".lisp", ".wasm"));
            std::fs::write(&out, &wasm).unwrap();
            writeln!(err, "OK {} bytes", wasm.len()).ok();
        }
        Ok(Err(e)) => { writeln!(err, "Error: {}", e).ok(); std::process::exit(1); }
        Err(p) => { writeln!(err, "PANIC: {:?}", p).ok(); std::process::exit(2); }
    }
}
