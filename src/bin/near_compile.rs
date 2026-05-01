use std::fs;
use std::io::{self, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse flags
    let mut mode = "compile"; // default
    let mut input_file: Option<String> = None;
    let mut output_file: Option<String>;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--repl" | "-r" => mode = "repl",
            "--wasm" | "-w" => mode = "compile", // explicit
            _ if input_file.is_none() => input_file = Some(arg.clone()),
            _ => {} // second positional = output (handled below)
        }
    }

    match mode {
        "repl" => run_repl(),
        _ => run_compile(&args),
    }
}

// ── COMPILE MODE ──

fn run_compile(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: near-compile [--repl] <input.lisp> [output.wasm]");
        eprintln!();
        eprintln!("  near-compile file.lisp           Compile to WASM (validated)");
        eprintln!("  near-compile --repl              Interactive REPL (WASM + wasmtime)");
        eprintln!("  near-compile --repl              (reads from stdin, instant feedback)");
        std::process::exit(1);
    }

    // Parse args, skip --repl if present
    let positional: Vec<&str> = args.iter().skip(1).filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let src_path = positional.get(0).expect("need input file");
    let src = fs::read_to_string(src_path).expect("read input");

    let (wasm_bytes, func_names) = lisp_rlm_wasm::wasm_emit::compile_near_named(&src).unwrap_or_else(|e| {
        eprintln!("❌ Compile error: {}", e);
        std::process::exit(1);
    });

    if let Err(e) = validate_wasm(&wasm_bytes, &func_names) {
        let out = positional.get(1).map(|s| s.to_string()).unwrap_or_else(|| src_path.replace(".lisp", ".wasm"));
        let _ = fs::write(&out, &wasm_bytes);
        std::process::exit(1);
    }

    let out = positional.get(1).map(|s| s.to_string()).unwrap_or_else(|| src_path.replace(".lisp", ".wasm"));
    fs::write(&out, &wasm_bytes).expect("write WASM");
    println!("✅ {} ({} bytes) — validated", out, wasm_bytes.len());
}

fn validate_wasm(wasm: &[u8], func_names: &[String]) -> Result<(), String> {
    let mut validator = wasmparser::Validator::new();
    match validator.validate_all(wasm) {
        Ok(_) => Ok(()),
        Err(e) => {
            let err_str = e.to_string();
            let offset = extract_offset(&err_str);
            let func_name = offset.and_then(|off| find_function_at_offset(wasm, off, func_names));
            match func_name {
                Some(name) => eprintln!("❌ WASM error in function `{}`: {}", name, err_str),
                None => eprintln!("❌ WASM validation error: {}", err_str),
            }
            Err(err_str)
        }
    }
}

fn extract_offset(err: &str) -> Option<usize> {
    for part in err.rsplit("offset ") {
        let s = part.trim();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            if let Ok(n) = usize::from_str_radix(hex.trim_end_matches(')'), 16) { return Some(n); }
        }
        if let Ok(n) = s.trim_end_matches(')').parse::<usize>() { return Some(n); }
    }
    None
}

fn find_function_at_offset(wasm: &[u8], target_offset: usize, func_names: &[String]) -> Option<String> {
    let mut pos = 8;
    while pos < wasm.len() {
        let section_id = *wasm.get(pos)? as usize;
        pos += 1;
        let (section_size, leb_bytes) = read_leb128(&wasm[pos..])?;
        pos += leb_bytes;
        let section_start = pos;
        let section_end = pos + section_size;
        if section_id == 10 {
            let mut body_pos = section_start;
            let (_count, leb) = read_leb128(&wasm[body_pos..])?;
            body_pos += leb;
            let mut func_idx = 0;
            while body_pos < section_end && func_idx < func_names.len() {
                let (body_size, leb) = read_leb128(&wasm[body_pos..])?;
                let body_start = body_pos;
                let body_end = body_pos + leb + body_size;
                if target_offset >= body_start && target_offset < body_end {
                    return Some(func_names[func_idx].clone());
                }
                body_pos = body_end;
                func_idx += 1;
            }
            return None;
        }
        pos = section_end;
    }
    None
}

fn read_leb128(data: &[u8]) -> Option<(usize, usize)> {
    let mut result = 0usize;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7F) as usize) << shift;
        shift += 7;
        if byte & 0x80 == 0 { return Some((result, i + 1)); }
        if shift > 63 { return None; }
    }
    None
}

// ── REPL MODE ──

fn run_repl() {
    // Check wasmtime available
    if !is_wasmtime_available() {
        eprintln!("❌ wasmtime crate needed for REPL. Recompile with wasmtime feature.");
        std::process::exit(1);
    }

    println!("⚡ NEAR Lisp REPL (WASM + wasmtime, mock NEAR runtime)");
    println!("   :help for commands, :quit to exit");
    println!();

    let mut history: Vec<String> = Vec::new();

    loop {
        print!("> ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() { break; }
        let input = input.trim().to_string();
        if input.is_empty() { continue; }

        match input.as_str() {
            ":quit" | ":q" => break,
            ":help" | ":h" => {
                println!("Commands:");
                println!("  :help      This help");
                println!("  :quit      Exit");
                println!("  :defs      Show definitions");
                println!("  :reset     Clear all definitions");
                println!("  :wat       Show compiled WASM (WAT format)");
                println!("  :size      Show WASM byte size");
                println!("  :near      Deploy last compiled WASM to testnet");
                println!();
                println!("WASM emitter: hof/map hof/filter hof/reduce");
                println!("  near/log \"msg\"  near/log \"x=\" 42  near/log_num 99");
                continue;
            }
            ":defs" => {
                if history.is_empty() { println!("  (none)"); }
                else { for h in &history { println!("  {}", h); } }
                continue;
            }
            ":reset" => { history.clear(); println!("✓ reset"); continue; }
            ":wat" => {
                let src = repl_source(&history, "(near/return 0)");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => match wasmprinter::print_bytes(&wasm) {
                        Ok(wat) => println!("{}", wat),
                        Err(e) => println!("Error: {}", e),
                    },
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            ":size" => {
                let src = repl_source(&history, "(near/return 0)");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => println!("{} bytes", wasm.len()),
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            ":near" => {
                let src = repl_source(&history, "(near/return 0)");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => match deploy(&wasm) {
                        Ok(msg) => println!("{}", msg),
                        Err(e) => println!("Error: {}", e),
                    },
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            _ => {}
        }

        if input.starts_with("(define ") {
            let src = repl_source_with_def(&history, &input);
            match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                Ok(wasm) => {
                    let mut v = wasmparser::Validator::new();
                    match v.validate_all(&wasm) {
                        Ok(_) => { history.push(input); println!("✓ ({} bytes)", wasm.len()); }
                        Err(e) => println!("❌ Validation: {}", e),
                    }
                }
                Err(e) => println!("Compile error: {}", e),
            }
            continue;
        }

        let src = repl_source(&history, &input);
        match eval_wasm(&src) {
            Ok((result, logs)) => {
                for l in &logs { println!("  LOG: {}", l); }
                println!("{}", result);
            }
            Err(e) => println!("Error: {}", e),
        }
    }
}

fn repl_source(history: &[String], expr: &str) -> String {
    let mut src = String::from("(memory 4)\n\n");
    for h in history { src.push_str(h); src.push('\n'); }
    src.push_str("\n(define (__repl_main) ");
    src.push_str(expr);
    src.push_str(")\n(export \"__repl_main\" __repl_main true)\n");
    src
}

fn repl_source_with_def(history: &[String], new_def: &str) -> String {
    let mut src = String::from("(memory 4)\n\n");
    for h in history { src.push_str(h); src.push('\n'); }
    src.push_str(new_def);
    src.push_str("\n(define (__repl_main) 0)\n(export \"__repl_main\" __repl_main true)\n");
    src
}

fn is_wasmtime_available() -> bool {
    // wasmtime is always available since we depend on it
    true
}

fn eval_wasm(src: &str) -> Result<(String, Vec<String>), String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near(src)?;
    let mut v = wasmparser::Validator::new();
    v.validate_all(&wasm).map_err(|e| format!("WASM validation: {}", e))?;
    run_wasmtime(&wasm)
}

fn run_wasmtime(wasm: &[u8]) -> Result<(String, Vec<String>), String> {
    use std::sync::{Arc, Mutex};
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;

    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut store = Store::new(&engine, ());

    let memory = Memory::new(&mut store, MemoryType::new(4, None)).map_err(|e| format!("memory: {}", e))?;

    let logs_c = logs.clone();
    let log_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        move |mut caller, args, _| {
            let len = args[0].unwrap_i64() as usize;
            let ptr = args[1].unwrap_i64() as usize;
            if len > 0 && len < 4096 {
                if let Some(data) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let mem = data.data(&caller);
                    if ptr + len <= mem.len() {
                        if let Ok(s) = std::str::from_utf8(&mem[ptr..ptr+len]) {
                            logs_c.lock().unwrap().push(s.to_string());
                        }
                    }
                }
            }
            Ok(())
        });

    let value_return_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));

    let noop1 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64], []),
        |_, _, _| Ok(()));

    let noop2 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));

    let reg_len_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let panic_fn = Func::new(&mut store,
        FuncType::new(&engine, [], []),
        |_, _, _| Err(wasmtime::Error::msg("NEAR panic")));

    let mut linker = Linker::new(&engine);
    linker.define(&store, "env", "log_utf8", log_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "value_return", value_return_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "input", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "read_register", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "register_len", reg_len_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic_utf8", panic_fn.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic", panic_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "memory", memory).map_err(|e| format!("link: {}", e))?;

    let instance = linker.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;
    let main_fn = instance.get_func(&mut store, "__repl_main").ok_or("__repl_main not found")?;
    main_fn.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

    let mut result_val = String::from("()");
    if let Some(mem) = instance.get_memory(&mut store, "memory") {
        let data = mem.data(&store);
        let off: usize = 64; // TEMP_MEM
        if off + 8 <= data.len() {
            let val = i64::from_le_bytes(data[off..off+8].try_into().unwrap_or([0;8]));
            result_val = val.to_string();
        }
    }

    let captured_logs = logs.lock().unwrap().drain(..).collect();
    Ok((result_val, captured_logs))
}

fn deploy(wasm: &[u8]) -> Result<String, String> {
    let tmp = format!("/tmp/near_repl_deploy_{}.wasm", std::process::id());
    std::fs::write(&tmp, wasm).map_err(|e| format!("write: {}", e))?;
    let home = std::env::var("HOME").unwrap_or_default();
    let key = format!("{}/.near-credentials/testnet/kampy.testnet.json", home);
    let output = std::process::Command::new("near")
        .args(["contract", "deploy", "kampy.testnet", "use-file", &tmp,
               "without-init-call", "network-config", "testnet",
               "sign-with-access-key-file", &key, "send"])
        .output().map_err(|e| format!("near CLI: {}", e))?;
    let _ = std::fs::remove_file(&tmp);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("successfully") || stdout.contains("Contract code") {
        for line in stdout.lines() {
            if let Some(id) = line.split("Transaction ID:").nth(1) {
                return Ok(format!("✅ https://explorer.testnet.near.org/transactions/{}", id.trim()));
            }
        }
    }
    Err(format!("deploy failed: {}", stdout))
}
