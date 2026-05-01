use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "--help" {
        eprintln!("Usage: near-repl");
        eprintln!("       echo '(+ 1 2)' | near-repl");
        eprintln!("Interactive REPL. Compiles to WASM via near-compile path, runs via wasmtime with mock NEAR host.");
        return;
    }

    println!("⚡ NEAR Lisp REPL (WASM + wasmtime, mock NEAR runtime)");
    println!("   :help for commands, :quit to exit");
    println!("   Compiles through the same WASM emitter as near-compile");
    println!("   Catches type mismatches, i32/i64 issues, NEAR host function bugs");
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
                println!("WASM emitter ops: hof/map hof/filter hof/reduce");
                println!("  near/log \"msg\"  near/log \"x=\" 42  near/log_num 99");
                println!("  + - * / mod abs < > <= >= = != and or not");
                println!("  if let set! begin while for reduce map-into");
                continue;
            }
            ":defs" => {
                if history.is_empty() { println!("  (none)"); }
                else { for h in &history { println!("  {}", h); } }
                continue;
            }
            ":reset" => {
                history.clear();
                println!("✓ reset");
                continue;
            }
            ":wat" => {
                let src = build_source(&history, "(near/return 0)");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => {
                        match wasmprinter::print_bytes(&wasm) {
                            Ok(wat) => println!("{}", wat),
                            Err(e) => println!("Error printing WAT: {}", e),
                        }
                    }
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            ":size" => {
                let src = build_source(&history, "(near/return 0)");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => println!("{} bytes", wasm.len()),
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            ":near" => {
                println!("Compiling with all defs...");
                let src = build_source(&history, "(near/return 0)");
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

        // Define → add to history, validate by compiling
        if input.starts_with("(define ") {
            let src = build_source_with_def(&history, &input);
            match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                Ok(wasm) => {
                    // Also validate
                    let mut v = wasmparser::Validator::new();
                    match v.validate_all(&wasm) {
                        Ok(_) => {
                            history.push(input);
                            println!("✓ ({} bytes)", wasm.len());
                        }
                        Err(e) => println!("❌ Validation: {}", e),
                    }
                }
                Err(e) => println!("Compile error: {}", e),
            }
            continue;
        }

        // Evaluate expression
        let src = build_source(&history, &input);
        match eval_wasm(&src) {
            Ok((result, logs)) => {
                if !logs.is_empty() {
                    for l in &logs { println!("  LOG: {}", l); }
                }
                println!("{}", result);
            }
            Err(e) => println!("Error: {}", e),
        }
    }
}

fn build_source(history: &[String], expr: &str) -> String {
    let mut src = String::from("(memory 4)\n\n");
    for h in history {
        src.push_str(h);
        src.push('\n');
    }
    src.push_str("\n(define (__repl_main) ");
    src.push_str(expr);
    src.push_str(")\n(export \"__repl_main\" __repl_main true)\n");
    src
}

fn build_source_with_def(history: &[String], new_def: &str) -> String {
    let mut src = String::from("(memory 4)\n\n");
    for h in history {
        src.push_str(h);
        src.push('\n');
    }
    src.push_str(new_def);
    src.push('\n');
    // Add a dummy main so it compiles
    src.push_str("(define (__repl_main) 0)\n(export \"__repl_main\" __repl_main true)\n");
    src
}

fn eval_wasm(src: &str) -> Result<(String, Vec<String>), String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near(src)?;

    // Validate first
    let mut v = wasmparser::Validator::new();
    v.validate_all(&wasm).map_err(|e| format!("WASM validation: {}", e))?;

    // Run with wasmtime + mock NEAR host
    run_with_wasmtime(&wasm)
}

fn run_with_wasmtime(wasm: &[u8]) -> Result<(String, Vec<String>), String> {
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;

    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let return_value: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));

    let mut store = Store::new(&engine, ());

    // Create mock NEAR host functions
    let log_utf8_type = FuncType::new(&engine, [ValType::I64, ValType::I64], []);
    let value_return_type = FuncType::new(&engine, [ValType::I64, ValType::I64], []);
    let input_type = FuncType::new(&engine, [ValType::I64], []);
    let read_register_type = FuncType::new(&engine, [ValType::I64, ValType::I64], []);
    let register_len_type = FuncType::new(&engine, [ValType::I64], [ValType::I64]);
    let panic_type = FuncType::new(&engine, [], []);

    // Shared memory — 4 pages (256KB)
    let memory = Memory::new(&mut store, MemoryType::new(4, None)).map_err(|e| format!("memory: {}", e))?;

    let logs_clone = logs.clone();
    let rv_clone = return_value.clone();

    let log_fn = Func::new(&mut store, log_utf8_type, move |mut caller, args, _results| {
        // log_utf8(len: u64, ptr: u64) — read string from memory and log it
        let len = args[0].unwrap_i64() as usize;
        let ptr = args[1].unwrap_i64() as usize;
        if len > 0 && len < 4096 {
            if let Some(data) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let mem = data.data(&caller);
                if ptr + len <= mem.len() {
                    if let Ok(s) = std::str::from_utf8(&mem[ptr..ptr+len]) {
                        logs_clone.lock().unwrap().push(s.to_string());
                    }
                }
            }
        }
        Ok(())
    });

    let rv_clone2 = return_value.clone();
    let value_return_fn = Func::new(&mut store, value_return_type, move |_caller, args, _results| {
        // value_return(len, ptr) — store the i64 value at ptr
        let len = args[0].unwrap_i64() as usize;
        let ptr = args[1].unwrap_i64() as usize;
        if len == 8 && ptr < 256 {
            rv_clone2.lock().unwrap().replace(ptr as i64); // store ptr as marker
        }
        Ok(())
    });

    let input_fn = Func::new(&mut store, input_type, move |_caller, _args, _results| {
        // input(register_id) — no-op for REPL
        Ok(())
    });

    let read_register_fn = Func::new(&mut store, read_register_type, move |_caller, _args, _results| {
        // read_register(register_id, ptr) — no-op
        Ok(())
    });

    let register_len_fn = Func::new(&mut store, register_len_type, move |_caller, _args, results| {
        // register_len(register_id) -> u64
        results[0] = Val::I64(0);
        Ok(())
    });

    let panic_fn = Func::new(&mut store, panic_type, move |_caller, _args, _results| {
        Err(wasmtime::Error::msg("NEAR panic called"))
    });

    // We need to handle the fact that the WASM imports memory from "env"
    // but we created it internally. Create a linker with all imports.
    let mut linker = Linker::new(&engine);

    // Define mock host functions
    linker.define(&store, "env", "log_utf8", log_fn).map_err(|e| format!("link log: {}", e))?;
    linker.define(&store, "env", "value_return", value_return_fn).map_err(|e| format!("link return: {}", e))?;
    linker.define(&store, "env", "input", input_fn).map_err(|e| format!("link input: {}", e))?;
    linker.define(&store, "env", "read_register", read_register_fn).map_err(|e| format!("link read_reg: {}", e))?;
    linker.define(&store, "env", "register_len", register_len_fn).map_err(|e| format!("link reg_len: {}", e))?;
    linker.define(&store, "env", "panic_utf8", panic_fn.clone()).map_err(|e| format!("link panic: {}", e))?;
    linker.define(&store, "env", "panic", panic_fn).map_err(|e| format!("link panic: {}", e))?;
    linker.define(&store, "env", "memory", memory).map_err(|e| format!("link memory: {}", e))?;

    // Instantiate
    let instance = linker.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;

    // Call __repl_main (exported wrapper is () -> ())
    let main_fn = instance.get_func(&mut store, "__repl_main")
        .ok_or("function __repl_main not found")?;

    main_fn.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

    // The mock value_return captured nothing useful since the wrapper overwrites it.
    // Read from memory instead (see below).

    // Actually — simpler approach: the export wrapper stores result at TEMP_MEM (offset 64)
    // then calls value_return(8, 64). Read the value from memory at offset 64.
    let mut result_val = String::from("()");
    if let Some(mem) = instance.get_memory(&mut store, "memory") {
        let data = mem.data(&store);
        let temp_mem: usize = 64;
        if temp_mem + 8 <= data.len() {
            let val = i64::from_le_bytes(data[temp_mem..temp_mem+8].try_into().unwrap_or([0;8]));
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
        .output()
        .map_err(|e| format!("near CLI: {}", e))?;

    let _ = std::fs::remove_file(&tmp);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stdout.contains("successfully") || stdout.contains("Contract code") {
        for line in stdout.lines() {
            if let Some(id) = line.split("Transaction ID:").nth(1) {
                return Ok(format!("✅ https://explorer.testnet.near.org/transactions/{}", id.trim()));
            }
        }
        Ok("✅ Deployed!".to_string())
    } else {
        Err(format!("deploy failed: {}", stdout))
    }
}
