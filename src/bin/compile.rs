fn main() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {}", info);
    }));
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: compile <input.lisp|.ts> [output.wasm] [--output|-o path] [--target near|outlayer]");
        std::process::exit(1);
    }
    if args[1] == "--help" || args[1] == "-h" || args[1] == "help" {
        eprintln!("usage: compile <input.lisp|.ts> [output.wasm] [--output|-o path] [--target near|outlayer]");
        eprintln!("  .ts/.mts inputs lower through the TypeScript frontend;");
        eprintln!("  output defaults to the input path with the extension swapped to .wasm");
        return;
    }
    eprintln!("START");
    eprintln!("Reading {}...", args[1]);
    let src = std::fs::read_to_string(&args[1]).unwrap();
    eprintln!("Parsed {} bytes", src.len());

    let is_outlayer = args.iter().any(|a| {
        a == "--target"
            && args
                .iter()
                .position(|x| x == "--target")
                .map(|i| {
                    args.get(i + 1)
                        .map(|v| v == "outlayer" || v == "outlayer-p2" || v == "wasi-p1")
                        .unwrap_or(false)
                })
                .unwrap_or(false)
    }) || args.iter().any(|a| a == "outlayer" || a == "wasi-p1");
    let is_p2 = args.iter().any(|a| {
        a == "--target"
            && args
                .iter()
                .position(|x| x == "--target")
                .map(|i| args.get(i + 1).map(|v| v == "outlayer-p2").unwrap_or(false))
                .unwrap_or(false)
    }) || args.iter().any(|a| a == "outlayer-p2");
    let is_wasi_p1 = args.iter().any(|a| a == "wasi-p1");

    let sidecar: std::cell::RefCell<Option<serde_json::Value>> = std::cell::RefCell::new(None);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if is_p2 {
            eprintln!("Target: OutLayer P2 (Component Model)");
            lisp_rlm_wasm::wasi::compile_outlayer_p2(&src)
        } else if is_wasi_p1 {
            eprintln!("Target: WASI P1 (minimal, no outlayer)");
            lisp_rlm_wasm::wasi::compile_wasi_p1(&src)
        } else if is_outlayer {
            eprintln!("Target: OutLayer (WASI P1)");
            lisp_rlm_wasm::wasi::compile_outlayer(&src)
        } else if args[1].ends_with(".ts") {
            // TypeScript twin pipeline (same chain as tests/test_ts_lending.rs):
            // ts_frontend lowers TS → lisp source, then parse → typecheck → emit.
            eprintln!("Target: NEAR (TypeScript frontend)");
            let ir = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(&src)?;
            let exprs = lisp_rlm_wasm::parse_all(&ir)?;
            lisp_rlm_wasm::typing::type_check_program(&exprs, true)?;
            let (w, m) = lisp_rlm_wasm::wasm_emit::compile_near_from_exprs_with_map(&exprs)?;
            sidecar.replace(Some(m));
            Ok(w)
        } else {
            let (w, m) = lisp_rlm_wasm::wasm_emit::compile_near_with_map(&src)?;
            sidecar.replace(Some(m));
            Ok(w)
        }
    }));

    match result {
        Ok(Ok(wasm)) => {
            // Output resolution: --output/-o <path> > 2nd positional ending
            // in .wasm > the input path with its extension swapped to .wasm.
            // (2026-09-15 data-loss fix: the old default was
            // `args[1].replace(".lisp", "..wasm")` — a NO-OP for .ts inputs,
            // so out == the SOURCE path: the wasm and then the map sidecar
            // were written straight over the user's source file. `-o` was
            // silently ignored in the same block.)
            let out = args
                .iter()
                .position(|a| a == "--output" || a == "-o")
                .and_then(|i| args.get(i + 1).cloned())
                .or_else(|| {
                    args.get(2).and_then(|a| {
                        if a.ends_with(".wasm") {
                            Some(a.clone())
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_else(|| {
                    let input = &args[1];
                    match input.rfind('.') {
                        // strip the LAST extension (foo.ts → foo.wasm,
                        // foo.mts → foo.wasm, bar.lisp → bar.wasm)
                        Some(dot) if dot > 0 => format!("{}.wasm", &input[..dot]),
                        _ => format!("{}.wasm", input),
                    }
                });
            // DATA-LOSS GUARD: never write any output onto the input source
            // (explicit `--output victim.ts` must refuse, not clobber)
            let same = std::path::Path::new(&out) == std::path::Path::new(&args[1]);
            if same {
                eprintln!("❌ refusing to write output onto the input source: {}", out);
                std::process::exit(1);
            }
            std::fs::write(&out, &wasm).unwrap();
            eprintln!("✅ {} ({} bytes)", out, wasm.len());
            // Symbolication sidecar: fn name → source form (NEAR targets only;
            // the OutLayer/WASI paths don't emit a name section today).
            if let Some(m) = sidecar.borrow_mut().take() {
                let map_path = out.replacen(".wasm", ".wasm.map", 1);
                std::fs::write(&map_path, serde_json::to_vec_pretty(&m).unwrap()).unwrap();
                eprintln!(
                    "🗺️  {} ({} entries)",
                    map_path,
                    m.as_object().map(|o| o.len()).unwrap_or(0)
                );
            }
        }
        Ok(Err(e)) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        Err(p) => {
            eprintln!("PANIC: {:?}", p);
            std::process::exit(2);
        }
    }
}
