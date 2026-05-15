use lisp_rlm_wasm::wasm_emit::compile_near;
fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {}", info);
    }));
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 { eprintln!("usage: compile <input.lisp> [output.wasm] [--target near|outlayer]"); std::process::exit(1); }
    eprintln!("START");
    eprintln!("Reading {}...", args[1]);
    let src = std::fs::read_to_string(&args[1]).unwrap();
    eprintln!("Parsed {} bytes", src.len());
    
    let is_outlayer = args.iter().any(|a| a == "--target" && args.iter().position(|x| x == "--target").map(|i| args.get(i+1).map(|v| v == "outlayer" || v == "outlayer-p2" || v == "wasi-p1").unwrap_or(false)).unwrap_or(false))
        || args.iter().any(|a| a == "outlayer" || a == "wasi-p1");
    let is_p2 = args.iter().any(|a| a == "--target" && args.iter().position(|x| x == "--target").map(|i| args.get(i+1).map(|v| v == "outlayer-p2").unwrap_or(false)).unwrap_or(false))
        || args.iter().any(|a| a == "outlayer-p2");
    let is_wasi_p1 = args.iter().any(|a| a == "wasi-p1");
    
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if is_p2 {
            eprintln!("Target: OutLayer P2 (Component Model)");
            lisp_rlm_wasm::wasi_emit::compile_outlayer_p2(&src)
        } else if is_wasi_p1 {
            eprintln!("Target: WASI P1 (minimal, no outlayer)");
            lisp_rlm_wasm::wasi_emit::compile_wasi_p1(&src)
        } else if is_outlayer {
            eprintln!("Target: OutLayer (WASI P1)");
            lisp_rlm_wasm::wasi_emit::compile_outlayer(&src)
        } else {
            compile_near(&src)
        }
    }));
    
    match result {
        Ok(Ok(wasm)) => {
            let out = args.iter().position(|a| a == "--output").and_then(|i| args.get(i+1).cloned())
                .or_else(|| args.get(2).and_then(|a| if a.ends_with(".wasm") { Some(a.clone()) } else { None }))
                .unwrap_or_else(|| args[1].replace(".lisp", ".wasm"));
            std::fs::write(&out, &wasm).unwrap();
            eprintln!("✅ {} ({} bytes)", out, wasm.len());
        }
        Ok(Err(e)) => { eprintln!("Error: {}", e); std::process::exit(1); }
        Err(p) => { eprintln!("PANIC: {:?}", p); std::process::exit(2); }
    }
}
