use std::fs;
use std::io::{self, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Parse mode
    let mode = if args.len() >= 2 && args[1] == "test" {
        "test"
    } else if args.iter().any(|a| a == "--repl" || a == "-r") {
        "repl"
    } else {
        "compile"
    };

    match mode {
        "repl" => run_repl(),
        "test" => run_test(&args),
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
        eprintln!("  near-compile test file.lisp       Run inline tests");
        std::process::exit(1);
    }

    // Parse args, skip --repl if present
    let positional: Vec<&str> = args.iter().skip(1).filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let src_path = positional.get(0).expect("need input file");
    let src = fs::read_to_string(src_path).expect("read input");

    // Strip test forms for normal compilation
    let src = strip_test_forms(&src);

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

/// Remove (test ...) forms from source so they don't interfere with compilation
fn strip_test_forms(src: &str) -> String {
    // Simple approach: remove lines starting with (test
    src.lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("(test ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── TEST MODE ──

struct TestCase {
    name: String,
    expr: lisp_rlm_wasm::types::LispVal,
    expected: lisp_rlm_wasm::types::LispVal,
    expr_src: String,
    expected_src: String,
}

fn run_test(args: &[String]) {
    let positional: Vec<&str> = args.iter().skip(2).filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let src_path = match positional.get(0) {
        Some(p) => p,
        None => {
            eprintln!("Usage: near-compile test <file.lisp>");
            std::process::exit(1);
        }
    };
    let src = fs::read_to_string(src_path).expect("read input");

    // Parse all forms
    let exprs = match lisp_rlm_wasm::parser::parse_all(&src) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("❌ Parse error: {}", e);
            std::process::exit(1);
        }
    };

    // Extract test cases and non-test forms
    let mut tests = Vec::new();
    let mut non_test_forms = Vec::new();

    for e in &exprs {
        if let lisp_rlm_wasm::types::LispVal::List(items) = e {
            if items.len() >= 2 {
                if let lisp_rlm_wasm::types::LispVal::Sym(s) = &items[0] {
                    if s == "test" && items.len() >= 4 {
                        let name = match &items[1] {
                            lisp_rlm_wasm::types::LispVal::Str(n) => n.clone(),
                            lisp_rlm_wasm::types::LispVal::Num(n) => n.to_string(),
                            other => format!("{:?}", other),
                        };
                        let expr_src = lisp_val_to_string(&items[2]);
                        let expected_src = lisp_val_to_string(&items[3]);
                        tests.push(TestCase {
                            name,
                            expr: items[2].clone(),
                            expected: items[3].clone(),
                            expr_src,
                            expected_src,
                        });
                        continue;
                    }
                }
            }
        }
        non_test_forms.push(e.clone());
    }

    if tests.is_empty() {
        println!("No test cases found.");
        std::process::exit(0);
    }

    println!("Running {} test(s)...\n", tests.len());

    let mut passed = 0;
    let mut failed = 0;

    for (i, tc) in tests.iter().enumerate() {
        // Build source: non-test definitions + a __test_N function that returns the expression
        let mut test_src = String::new();

        // Rebuild source from non-test forms (we need to re-serialize them)
        for form in &non_test_forms {
            test_src.push_str(&lisp_val_to_string(form));
            test_src.push('\n');
        }

        // Add test function for expr
        let test_fn = format!(
            "\n(define (__test_{}_expr) {})\n(export \"__test_{}_expr\" __test_{}_expr true)\n",
            i, tc.expr_src, i, i
        );
        // Add test function for expected
        let expected_fn = format!(
            "(define (__test_{}_expected) {})\n(export \"__test_{}_expected\" __test_{}_expected true)\n",
            i, tc.expected_src, i, i
        );
        test_src.push_str(&test_fn);
        test_src.push_str(&expected_fn);

        // Compile
        let wasm = match lisp_rlm_wasm::wasm_emit::compile_near(&test_src) {
            Ok(w) => w,
            Err(e) => {
                println!("❌ {}: compile error: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        // Validate
        let mut validator = wasmparser::Validator::new();
        if let Err(e) = validator.validate_all(&wasm) {
            println!("❌ {}: WASM validation error: {}", tc.name, e);
            failed += 1;
            continue;
        }

        // Run both functions and compare
        let expr_val = match run_test_fn(&wasm, &format!("__test_{}_expr", i)) {
            Ok(v) => v,
            Err(e) => {
                println!("❌ {}: runtime error evaluating expression: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        let expected_val = match run_test_fn(&wasm, &format!("__test_{}_expected", i)) {
            Ok(v) => v,
            Err(e) => {
                println!("❌ {}: runtime error evaluating expected: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        if expr_val == expected_val {
            println!("✅ {}: {} = {}", tc.name, tc.expr_src, tc.expected_src);
            passed += 1;
        } else {
            println!("❌ {}: {} = {}, expected {}", tc.name, tc.expr_src, expr_val, expected_val);
            failed += 1;
        }
    }

    println!("\n{} passed, {} failed", passed, failed);
    if failed > 0 {
        std::process::exit(1);
    }
}

fn run_test_fn(wasm: &[u8], fn_name: &str) -> Result<i64, String> {
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

    let noop0 = Func::new(&mut store,
        FuncType::new(&engine, [], []),
        |_, _, _| Ok(()));

    let noop_ret_i64 = Func::new(&mut store,
        FuncType::new(&engine, [], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_5i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_2i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_3i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));

    let noop_8i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_9i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_4i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let noop_6i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));

    let noop_3i64_ret = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, results| { results[0] = Val::I64(0); Ok(()) });

    let mut linker = Linker::new(&engine);
    linker.define(&store, "env", "log_utf8", log_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "value_return", value_return_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "input", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "read_register", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "register_len", reg_len_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic_utf8", panic_fn.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic", panic_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "memory", memory).map_err(|e| format!("link: {}", e))?;
    // Register all NEAR host function stubs
    linker.define(&store, "env", "write_register", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "current_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_pk", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "predecessor_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_index", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_timestamp", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "epoch_height", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_usage", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "account_balance", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "account_locked_balance", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "attached_deposit", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "prepaid_gas", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "used_gas", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_write", noop_5i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_read", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_remove", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_has_key", noop_2i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "sha256", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "keccak256", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "random_seed", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "ed25519_verify", noop_6i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "log_utf16", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_create", noop_8i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_then", noop_9i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_and", noop_2i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_results_count", noop_ret_i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_result", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_return", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_prefix", noop_2i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_range", noop_4i64_0).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_next", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_create", noop_2i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_then", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_create_account", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_deploy_contract", noop_2i64.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_function_call", noop_6i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_transfer", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_stake", noop_4i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_add_key_with_full_access", noop_4i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_add_key_with_function_call", noop_6i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_delete_key", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_delete_account", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;

    let instance = linker.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;
    let func = instance.get_func(&mut store, fn_name).ok_or_else(|| format!("{} not found", fn_name))?;
    func.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

    // Read return value from TEMP_MEM (offset 64)
    let mut result: i64 = 0;
    if let Some(mem) = instance.get_memory(&mut store, "memory") {
        let data = mem.data(&store);
        let off: usize = 64;
        if off + 8 <= data.len() {
            result = i64::from_le_bytes(data[off..off+8].try_into().unwrap_or([0;8]));
        }
    }

    Ok(result)
}

/// Simple LispVal to string (for reconstructing source)
fn lisp_val_to_string(v: &lisp_rlm_wasm::types::LispVal) -> String {
    match v {
        lisp_rlm_wasm::types::LispVal::Num(n) => n.to_string(),
        lisp_rlm_wasm::types::LispVal::Bool(b) => b.to_string(),
        lisp_rlm_wasm::types::LispVal::Str(s) => format!("\"{}\"", s),
        lisp_rlm_wasm::types::LispVal::Sym(s) => s.clone(),
        lisp_rlm_wasm::types::LispVal::Nil => "nil".to_string(),
        lisp_rlm_wasm::types::LispVal::List(items) => {
            let inner: Vec<String> = items.iter().map(lisp_val_to_string).collect();
            format!("({})", inner.join(" "))
        }
        _ => format!("{:?}", v),
    }
}

// ── VALIDATION ──

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

    // Persistent mock storage across REPL calls
    let repl_storage: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>> = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
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
        match eval_wasm(&src, Some(&repl_storage)) {
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
    true
}

fn eval_wasm(src: &str, shared_storage: Option<&std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>>>) -> Result<(String, Vec<String>), String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near(src)?;
    let mut v = wasmparser::Validator::new();
    v.validate_all(&wasm).map_err(|e| format!("WASM validation: {}", e))?;
    run_wasmtime(&wasm, shared_storage)
}

fn run_wasmtime(wasm: &[u8], shared_storage: Option<&std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>>>) -> Result<(String, Vec<String>), String> {
    use std::sync::{Arc, Mutex};
    use std::collections::HashMap;
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;

    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let registers: Arc<Mutex<HashMap<u64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    let storage: Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>> = shared_storage.cloned().unwrap_or_else(|| Arc::new(Mutex::new(HashMap::new())));

    let mut store = Store::new(&engine, ());
    let memory = Memory::new(&mut store, MemoryType::new(4, None)).map_err(|e| format!("memory: {}", e))?;

    // ── Log ──
    let logs_c = logs.clone();
    let log_fn = Func::new(&mut store, FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        move |mut caller, args, _| {
            let len = args[0].unwrap_i64() as usize;
            let ptr = args[1].unwrap_i64() as usize;
            if len > 0 && len < 8192 {
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

    let value_return_fn = Func::new(&mut store, FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));

    // ── Registers ──
    let rc1 = registers.clone();
    let read_register_fn = Func::new(&mut store, FuncType::new(&engine, [ValType::I64, ValType::I64], []),
        move |mut caller, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            let ptr = args[1].unwrap_i64() as usize;
            if let Some(data) = rc1.lock().unwrap().get(&rid).cloned() {
                if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let md = mem.data_mut(&mut caller);
                    if ptr + data.len() <= md.len() { md[ptr..ptr+data.len()].copy_from_slice(&data); }
                }
            }
            Ok(())
        });

    let rc2 = registers.clone();
    let register_len_fn = Func::new(&mut store, FuncType::new(&engine, [ValType::I64], [ValType::I64]),
        move |_, args, results| {
            let rid = args[0].unwrap_i64() as u64;
            let len = rc2.lock().unwrap().get(&rid).map(|d| d.len() as i64).unwrap_or(0);
            results[0] = Val::I64(len); Ok(())
        });

    // ── Storage ──
    let sc1 = storage.clone(); let rc3 = registers.clone();
    let storage_write_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        move |mut caller, args, results| {
            let (kl,kp,vl,vp,rid) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize,
                args[2].unwrap_i64() as usize, args[3].unwrap_i64() as usize, args[4].unwrap_i64() as u64);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp+kl <= md.len() && vp+vl <= md.len() {
                    let key = md[kp..kp+kl].to_vec(); let val = md[vp..vp+vl].to_vec();
                    let old = sc1.lock().unwrap().insert(key, val);
                    if rid != u64::MAX { if let Some(old) = old { rc3.lock().unwrap().insert(rid, old); } }
                }
            }
            results[0] = Val::I64(0); Ok(())
        });

    let sc2 = storage.clone(); let rc4 = registers.clone();
    let storage_read_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        move |mut caller, args, results| {
            let (kl,kp,rid) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize, args[2].unwrap_i64() as u64);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp+kl <= md.len() {
                    if let Some(val) = sc2.lock().unwrap().get(&md[kp..kp+kl]).cloned() {
                        rc4.lock().unwrap().insert(rid, val); results[0] = Val::I64(1); return Ok(());
                    }
                }
            }
            results[0] = Val::I64(0); Ok(())
        });

    let sc3 = storage.clone(); let rc5 = registers.clone();
    let storage_remove_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        move |mut caller, args, results| {
            let (kl,kp,rid) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize, args[2].unwrap_i64() as u64);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp+kl <= md.len() {
                    if let Some(val) = sc3.lock().unwrap().remove(&md[kp..kp+kl].to_vec()) {
                        if rid != u64::MAX { rc5.lock().unwrap().insert(rid, val); }
                        results[0] = Val::I64(1); return Ok(());
                    }
                }
            }
            results[0] = Val::I64(0); Ok(())
        });

    let sc4 = storage.clone();
    let storage_has_key_fn = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64], [ValType::I64]),
        move |mut caller, args, results| {
            let (kl,kp) = (args[0].unwrap_i64() as usize, args[1].unwrap_i64() as usize);
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let md = mem.data(&caller);
                if kp+kl <= md.len() {
                    results[0] = Val::I64(if sc4.lock().unwrap().contains_key(&md[kp..kp+kl]) { 1 } else { 0 });
                    return Ok(());
                }
            }
            results[0] = Val::I64(0); Ok(())
        });

    // ── Stubs ──
    let panic_fn = Func::new(&mut store, FuncType::new(&engine, [], []),
        |_,_,_| Err(wasmtime::Error::msg("NEAR panic")));
    let noop0 = Func::new(&mut store, FuncType::new(&engine, [], []), |_,_,_| Ok(()));
    let noop1 = Func::new(&mut store, FuncType::new(&engine, [ValType::I64], []), |_,_,_| Ok(()));
    let noop1r = Func::new(&mut store, FuncType::new(&engine, [ValType::I64], [ValType::I64]),
        |_,_,r| { r[0] = Val::I64(0); Ok(()) });
    let noop0r = Func::new(&mut store, FuncType::new(&engine, [], [ValType::I64]),
        |_,_,r| { r[0] = Val::I64(0); Ok(()) });

    let mut linker = Linker::new(&engine);
    linker.define(&store, "env", "log_utf8", log_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "value_return", value_return_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "read_register", read_register_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "register_len", register_len_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "input", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_write", storage_write_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_read", storage_read_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_remove", storage_remove_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_has_key", storage_has_key_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic_utf8", panic_fn.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic", panic_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "memory", memory).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "current_account_id", noop1r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_id", noop1r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_pk", noop1r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "predecessor_account_id", noop1r).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_index", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_timestamp", noop0r).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "account_balance", noop0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "attached_deposit", noop0).map_err(|e| format!("link: {}", e))?;

    let instance = linker.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;
    let main_fn = instance.get_func(&mut store, "__repl_main").ok_or("__repl_main not found")?;
    main_fn.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

    let mut result_val = String::from("()");
    if let Some(mem) = instance.get_memory(&mut store, "memory") {
        let data = mem.data(&store);
        let off: usize = 64;
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
