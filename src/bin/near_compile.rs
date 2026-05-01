use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// ── PROJECT CONFIG ──

#[derive(Debug, Clone)]
struct ProjectConfig {
    name: String,
    src: String,
    account: String,
    network: String,
    key_path: String,
    output: String,
    tests: String,
}

fn load_project_config(dir: &str) -> Result<ProjectConfig, String> {
    let config_path = Path::new(dir).join("near.json");
    if !config_path.exists() {
        return Err("no near.json found".into());
    }
    let content = fs::read_to_string(&config_path)
        .map_err(|e| format!("read near.json: {}", e))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("parse near.json: {}", e))?;
    let name = json["name"].as_str().unwrap_or("contract").to_string();
    let src = json["src"].as_str().unwrap_or("src/main.lisp").to_string();
    let account = json["account"].as_str().unwrap_or("").to_string();
    let network = json["network"].as_str().unwrap_or("testnet").to_string();
    let key_path = json["key_path"].as_str().unwrap_or("").to_string();
    let output = json["output"].as_str().unwrap_or(&format!("target/{}.wasm", name)).to_string();
    let tests = json["tests"].as_str().unwrap_or("tests/").to_string();
    Ok(ProjectConfig { name, src, account, network, key_path, output, tests })
}

// ── MAIN ──

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let cmd = &args[1];

    match cmd.as_str() {
        "init" => {
            let name = args.get(2).map(|s| s.as_str()).unwrap_or_else(|| {
                eprintln!("Usage: near-compile init <name>");
                std::process::exit(1);
            });
            run_init(name);
        }
        "build" => run_build(args.get(2).map(|s| s.as_str())),
        "deploy" => run_deploy(args.get(2).map(|s| s.as_str())),
        "test" => run_project_test(args.get(2).map(|s| s.as_str())),
        "--repl" | "-r" => run_repl(),
        _ => {
            // Legacy: treat as file.lisp compile or test
            if cmd == "test" {
                run_test(&args);
            } else if args.iter().any(|a| a == "--repl" || a == "-r") {
                run_repl();
            } else {
                run_compile(&args);
            }
        }
    }
}

fn print_usage() {
    eprintln!("NEAR Lisp Compiler");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  near-compile init <name>              Scaffold a new project");
    eprintln!("  near-compile build [dir]              Build project from near.json");
    eprintln!("  near-compile deploy [dir]             Build and deploy to NEAR");
    eprintln!("  near-compile test [dir]               Build and run tests");
    eprintln!("  near-compile --repl                   Interactive REPL");
    eprintln!("  near-compile <file.lisp> [out.wasm]   Compile single file (legacy)");
    eprintln!("  near-compile test <file.lisp>         Run inline tests (legacy)");
}

// ── INIT ──

fn run_init(name: &str) {
    let base = Path::new(name);

    // Create dirs
    fs::create_dir_all(base.join("src")).expect("create src/");
    fs::create_dir_all(base.join("tests")).expect("create tests/");
    fs::create_dir_all(base.join("target")).expect("create target/");

    // near.json
    let config = format!(r#"{{
  "name": "{name}",
  "src": "src/main.lisp",
  "account": "",
  "network": "testnet",
  "output": "target/{name}.wasm",
  "tests": "tests/"
}}
"#);
    fs::write(base.join("near.json"), config).expect("write near.json");

    // src/main.lisp
    let main_lisp = format!(r#"(memory 4)

(define (hello)
  (near/log "Hello from {name}!")
  (near/return 0))

(export "hello" hello true)
"#);
    fs::write(base.join("src/main.lisp"), main_lisp).expect("write src/main.lisp");

    // tests/main_test.lisp
    fs::write(base.join("tests/main_test.lisp"), ";; Add your tests here\n;; (test \"name\" expr expected)\n").expect("write tests/main_test.lisp");

    println!("✅ Created project '{}' with:", name);
    println!("   {}/near.json", name);
    println!("   {}/src/main.lisp", name);
    println!("   {}/tests/main_test.lisp", name);
    println!();
    println!("   cd {} && near-compile build", name);
}

// ── BUILD ──

fn do_build(project_dir: &str) -> Result<(ProjectConfig, Vec<u8>, Vec<String>), String> {
    let config = load_project_config(project_dir)?;

    let src_path = Path::new(project_dir).join(&config.src);
    let source = fs::read_to_string(&src_path)
        .map_err(|e| format!("read {}: {}", config.src, e))?;

    let base_dir = src_path.parent().unwrap_or(Path::new("."));
    let resolved = lisp_rlm_wasm::wasm_emit::resolve_modules(&source, base_dir)?;

    // Compile and validate
    let wasm_bytes = lisp_rlm_wasm::wasm_emit::compile_near(&resolved)?;
    let func_names: Vec<String> = extract_func_names(&resolved).unwrap_or_default();

    // Validate
    let mut validator = wasmparser::Validator::new();
    if let Err(e) = validator.validate_all(&wasm_bytes) {
        let err_str = e.to_string();
        let offset = extract_offset(&err_str);
        let func_name = offset.and_then(|off| find_function_at_offset(&wasm_bytes, off, &func_names));
        match func_name {
            Some(name) => return Err(format!("WASM error in `{}`: {}", name, err_str)),
            None => return Err(format!("WASM validation error: {}", err_str)),
        }
    }

    // Write output
    let out_path = Path::new(project_dir).join(&config.output);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create output dir: {}", e))?;
    }
    fs::write(&out_path, &wasm_bytes).map_err(|e| format!("write {}: {}", config.output, e))?;

    Ok((config, wasm_bytes, func_names))
}

fn run_build(dir: Option<&str>) {
    let project_dir = dir.unwrap_or(".");
    match do_build(project_dir) {
        Ok((config, wasm, _)) => {
            println!("✅ {} ({} bytes) — validated", config.output, wasm.len());
        }
        Err(e) => {
            eprintln!("❌ Build failed: {}", e);
            std::process::exit(1);
        }
    }
}

// ── DEPLOY ──

fn run_deploy(dir: Option<&str>) {
    let project_dir = dir.unwrap_or(".");
    let (config, _wasm, _) = match do_build(project_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("❌ Build failed: {}", e);
            std::process::exit(1);
        }
    };

    if config.account.is_empty() {
        eprintln!("❌ No account configured in near.json");
        std::process::exit(1);
    }

    let wasm_path = Path::new(project_dir).join(&config.output);
    let network = &config.network;
    let account = &config.account;

    // Resolve key path
    let key_path = if config.key_path.is_empty() {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/.near-credentials/{}/{}.json", home, network, account)
    } else {
        shellexpand::tilde(&config.key_path).to_string()
    };

    println!("🚀 Deploying to {} ({})...", account, network);

    let output = std::process::Command::new("near")
        .args(["contract", "deploy", account, "use-file",
               &wasm_path.to_string_lossy(),
               "without-init-call", "network-config", network,
               "sign-with-access-key-file", &key_path, "send"])
        .output()
        .expect("near CLI not found");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    if combined.contains("successfully") || combined.contains("Contract code") {
        for line in combined.lines() {
            if let Some(id) = line.split("Transaction ID:").nth(1) {
                let explorer = if network == "mainnet" { "https://explorer.near.org" } else { "https://explorer.testnet.near.org" };
                println!("✅ {}/transactions/{}", explorer, id.trim());
                return;
            }
        }
        println!("✅ Deployed successfully");
    } else {
        eprintln!("❌ Deploy failed: {}", combined.trim());
        std::process::exit(1);
    }
}

// ── PROJECT TEST ──

fn run_project_test(dir: Option<&str>) {
    let project_dir = dir.unwrap_or(".");

    // First try project-based tests
    if let Ok(config) = load_project_config(project_dir) {
        // Build first
        let (_, _wasm_bytes, _) = match do_build(project_dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("❌ Build failed: {}", e);
                std::process::exit(1);
            }
        };

        // Find test files
        let tests_dir = Path::new(project_dir).join(&config.tests);
        if !tests_dir.exists() {
            println!("No tests directory found at {}", config.tests);
            return;
        }

        let mut test_files: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = fs::read_dir(&tests_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "lisp").unwrap_or(false) {
                    test_files.push(path);
                }
            }
        }
        test_files.sort();

        if test_files.is_empty() {
            println!("No test files found in {}", config.tests);
            return;
        }

        // Read source for non-test definitions
        let src_path = Path::new(project_dir).join(&config.src);
        let source = fs::read_to_string(&src_path).expect("read source");
        let base_dir = src_path.parent().unwrap_or(Path::new("."));
        let resolved_source = lisp_rlm_wasm::wasm_emit::resolve_modules(&source, base_dir)
            .expect("resolve modules");
        let clean_source = strip_test_forms(&resolved_source);

        let mut total_passed = 0;
        let mut total_failed = 0;

        for test_file in &test_files {
            let test_src = fs::read_to_string(test_file).expect("read test file");
            println!("📋 {}:", test_file.display());

            // Parse tests from file
            let exprs = match lisp_rlm_wasm::parser::parse_all(&test_src) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("  ❌ Parse error: {}", e);
                    continue;
                }
            };

            let tests = extract_tests(&exprs);
            if tests.is_empty() {
                println!("  (no test cases)");
                continue;
            }

            let (passed, failed) = run_tests(&clean_source, &tests);
            total_passed += passed;
            total_failed += failed;
        }

        println!("\n{} passed, {} failed", total_passed, total_failed);
        if total_failed > 0 {
            std::process::exit(1);
        }
    } else {
        // Legacy: single file test
        run_test_legacy(dir);
    }
}

fn run_test_legacy(_dir: Option<&str>) {
    let args: Vec<String> = std::env::args().collect();
    let positional: Vec<&str> = args.iter().skip(2).filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let src_path = match positional.get(0) {
        Some(p) => p,
        None => {
            eprintln!("Usage: near-compile test <file.lisp>");
            std::process::exit(1);
        }
    };
    let src = fs::read_to_string(src_path).expect("read input");
    run_test_from_source(&src);
}

fn run_test_from_source(src: &str) {
    let exprs = match lisp_rlm_wasm::parser::parse_all(src) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("❌ Parse error: {}", e);
            std::process::exit(1);
        }
    };

    let mut non_test_forms = Vec::new();
    for e in &exprs {
        if let lisp_rlm_wasm::types::LispVal::List(items) = e {
            if items.len() >= 2 {
                if let lisp_rlm_wasm::types::LispVal::Sym(s) = &items[0] {
                    if s == "test" && items.len() >= 4 { continue; }
                }
            }
        }
        non_test_forms.push(e.clone());
    }

    let clean_src: String = non_test_forms.iter()
        .map(|f| format!("{}\n", lisp_val_to_string(f)))
        .collect();

    let tests = extract_tests(&exprs);
    if tests.is_empty() {
        println!("No test cases found.");
        return;
    }

    println!("Running {} test(s)...\n", tests.len());
    let (passed, failed) = run_tests(&clean_src, &tests);
    println!("\n{} passed, {} failed", passed, failed);
    if failed > 0 {
        std::process::exit(1);
    }
}

struct TestCase {
    name: String,
    expr: lisp_rlm_wasm::types::LispVal,
    expected: lisp_rlm_wasm::types::LispVal,
    expr_src: String,
    expected_src: String,
}

fn extract_tests(exprs: &[lisp_rlm_wasm::types::LispVal]) -> Vec<TestCase> {
    let mut tests = Vec::new();
    for e in exprs {
        if let lisp_rlm_wasm::types::LispVal::List(items) = e {
            if items.len() >= 4 {
                if let lisp_rlm_wasm::types::LispVal::Sym(s) = &items[0] {
                    if s == "test" {
                        let name = match &items[1] {
                            lisp_rlm_wasm::types::LispVal::Str(n) => n.clone(),
                            lisp_rlm_wasm::types::LispVal::Num(n) => n.to_string(),
                            other => format!("{:?}", other),
                        };
                        tests.push(TestCase {
                            name,
                            expr: items[2].clone(),
                            expected: items[3].clone(),
                            expr_src: lisp_val_to_string(&items[2]),
                            expected_src: lisp_val_to_string(&items[3]),
                        });
                    }
                }
            }
        }
    }
    tests
}

fn run_tests(base_src: &str, tests: &[TestCase]) -> (usize, usize) {
    let mut passed = 0;
    let mut failed = 0;

    for (i, tc) in tests.iter().enumerate() {
        let mut test_src = base_src.to_string();
        test_src.push_str(&format!(
            "\n(define (__test_{}_expr) {})\n(export \"__test_{}_expr\" __test_{}_expr true)\n",
            i, tc.expr_src, i, i
        ));
        test_src.push_str(&format!(
            "(define (__test_{}_expected) {})\n(export \"__test_{}_expected\" __test_{}_expected true)\n",
            i, tc.expected_src, i, i
        ));

        let wasm = match lisp_rlm_wasm::wasm_emit::compile_near(&test_src) {
            Ok(w) => w,
            Err(e) => {
                println!("  ❌ {}: compile error: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        let mut validator = wasmparser::Validator::new();
        if let Err(e) = validator.validate_all(&wasm) {
            println!("  ❌ {}: WASM validation: {}", tc.name, e);
            failed += 1;
            continue;
        }

        let expr_val = match run_test_fn(&wasm, &format!("__test_{}_expr", i)) {
            Ok(v) => v,
            Err(e) => {
                println!("  ❌ {}: runtime error: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        let expected_val = match run_test_fn(&wasm, &format!("__test_{}_expected", i)) {
            Ok(v) => v,
            Err(e) => {
                println!("  ❌ {}: expected runtime error: {}", tc.name, e);
                failed += 1;
                continue;
            }
        };

        if expr_val == expected_val {
            println!("  ✅ {}: {} = {}", tc.name, tc.expr_src, tc.expected_src);
            passed += 1;
        } else {
            println!("  ❌ {}: {} = {}, expected {}", tc.name, tc.expr_src, expr_val, expected_val);
            failed += 1;
        }
    }

    (passed, failed)
}

// ── LEGACY COMPILE ──

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
    run_test_from_source(&src);
}

fn run_compile(args: &[String]) {
    if args.len() < 2 {
        eprintln!("Usage: near-compile [--repl] <input.lisp> [output.wasm]");
        eprintln!();
        eprintln!("  near-compile file.lisp           Compile to WASM (validated)");
        eprintln!("  near-compile --repl              Interactive REPL (WASM + wasmtime)");
        eprintln!("  near-compile test file.lisp       Run inline tests");
        std::process::exit(1);
    }

    let positional: Vec<&str> = args.iter().skip(1).filter(|a| !a.starts_with('-')).map(String::as_str).collect();
    let src_path = positional.get(0).expect("need input file");
    let src = fs::read_to_string(src_path).expect("read input");

    let src = strip_test_forms(&src);

    let wasm_bytes = match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
        Ok(w) => w,
        Err(e) => { eprintln!("❌ Compile error: {}", e); std::process::exit(1); }
    };
    let func_names: Vec<String> = extract_func_names(&src).unwrap_or_default();

    if let Err(_e) = validate_wasm(&wasm_bytes, &func_names) {
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
    src.lines()
        .filter(|line| !line.trim().starts_with("(test "))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── TEST RUNNER HELPERS ──

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
    let noop0r = Func::new(&mut store,
        FuncType::new(&engine, [], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });
    let panic_fn = Func::new(&mut store,
        FuncType::new(&engine, [], []),
        |_, _, _| Err(wasmtime::Error::msg("NEAR panic")));

    // All NEAR host function stubs needed for linking
    let noop_5i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });
    let noop_3i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));
    let noop_3i64_ret = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });
    let noop_6i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], []),
        |_, _, _| Ok(()));
    let noop_4i64_0 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });
    let noop_8i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });
    let noop_9i64 = Func::new(&mut store,
        FuncType::new(&engine, [ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
        |_, _, r| { r[0] = Val::I64(0); Ok(()) });

    let mut linker = Linker::new(&engine);
    linker.define(&store, "env", "log_utf8", log_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "value_return", value_return_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "input", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "read_register", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "register_len", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic_utf8", panic_fn.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "panic", panic_fn).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "memory", memory).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "write_register", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "current_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "signer_account_pk", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "predecessor_account_id", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_index", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "block_timestamp", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "epoch_height", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_usage", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "account_balance", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "account_locked_balance", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "attached_deposit", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "prepaid_gas", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "used_gas", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_write", noop_5i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_read", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_remove", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_has_key", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "sha256", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "keccak256", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "random_seed", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "ed25519_verify", noop_6i64_0).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "log_utf16", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_create", noop_8i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_then", noop_9i64).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_and", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_results_count", noop0r.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_result", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_return", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_prefix", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_range", noop_4i64_0).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "storage_iter_next", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_create", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_then", noop_3i64_ret.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_create_account", noop1.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_deploy_contract", noop2.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_function_call", noop_6i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_transfer", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_stake", noop_4i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_add_key_with_full_access", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_add_key_with_function_call", noop_6i64_0).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_delete_key", noop_3i64_0.clone()).map_err(|e| format!("link: {}", e))?;
    linker.define(&store, "env", "promise_batch_action_delete_account", noop_3i64_0).map_err(|e| format!("link: {}", e))?;

    let instance = linker.instantiate(&mut store, &module).map_err(|e| format!("instantiate: {}", e))?;
    let func = instance.get_func(&mut store, fn_name).ok_or_else(|| format!("{} not found", fn_name))?;
    func.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

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

fn extract_func_names(source: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut depth = 0isize;
    let mut current = String::new();
    for ch in source.chars() {
        if ch == '(' { depth += 1; current.push(ch); }
        else if ch == ')' {
            depth -= 1; current.push(ch);
            if depth == 0 {
                let trimmed = current.trim();
                if trimmed.starts_with("(define (") {
                    // Extract function name from (define (name params...) body)
                    let inner = &trimmed[9..];
                    if let Some(end) = inner.find(|c: char| c == ' ' || c == ')') {
                        names.push(inner[..end].to_string());
                    }
                }
                current.clear();
            }
        } else if depth > 0 { current.push(ch); }
    }
    Ok(names)
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
    if !is_wasmtime_available() {
        eprintln!("❌ wasmtime crate needed for REPL. Recompile with wasmtime feature.");
        std::process::exit(1);
    }

    println!("⚡ NEAR Lisp REPL (WASM + wasmtime, mock NEAR runtime)");
    println!("   :help for commands, :quit to exit");

    let repl_storage: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>> = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let repl_memory: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = std::sync::Arc::new(std::sync::Mutex::new(vec![0u8; 262144]));
    let repl_input: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    println!();

    let mut history: Vec<String> = Vec::new();

    // Auto-load project defines if near.json exists
    if let Ok(config) = load_project_config(".") {
        let src_path = Path::new(&config.src);
        if let Ok(source) = fs::read_to_string(src_path) {
            let base_dir = src_path.parent().unwrap_or(Path::new("."));
            if let Ok(resolved) = lisp_rlm_wasm::wasm_emit::resolve_modules(&source, base_dir) {
                // Extract top-level forms by counting parens
                let mut defines: Vec<String> = Vec::new();
                let mut memory_decl = String::from("(memory 4)");
                let mut depth = 0isize;
                let mut current = String::new();
                for ch in resolved.chars() {
                    if ch == '(' { depth += 1; current.push(ch); }
                    else if ch == ')' {
                        depth -= 1;
                        current.push(ch);
                        if depth == 0 {
                            let trimmed = current.trim();
                            if trimmed.starts_with("(define ") {
                                defines.push(trimmed.to_string());
                            } else if trimmed.starts_with("(memory ") {
                                memory_decl = trimmed.to_string();
                            }
                            current.clear();
                        }
                    } else if depth > 0 {
                        current.push(ch);
                    }
                }
                if !defines.is_empty() {
                    let test_src = format!("{}\n{}\n(define (__repl_main) 0)\n(export \"__repl_main\" __repl_main true)\n",
                        memory_decl, defines.join("\n"));
                    match lisp_rlm_wasm::wasm_emit::compile_near(&test_src) {
                        Ok(_wasm) => {
                            println!("📦 Loaded {} definitions from {}", defines.len(), config.src);
                            history = defines;
                        }
                        Err(e) => println!("⚠️  Failed to load {}: {}", config.src, e),
                    }
                }
            }
        }
    }

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
                println!("  :push      Deploy all definitions to NEAR testnet");
                println!("  :call fn   Call a view function on the deployed contract");
                println!("  :call! fn  Call a mutable function (costs gas)");
                println!("  :input     Set mock input JSON (e.g. :input '{{\"amount\": 42}}')");
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
            s if s.starts_with(":input") => {
                let json_str = s.trim_start_matches(":input").trim();
                if json_str.is_empty() { println!("Usage: :input {{\"key\": value}}"); continue; }
                *repl_input.lock().unwrap() = json_str.as_bytes().to_vec();
                println!("✓ input set ({} bytes)", json_str.len());
                continue;
            }
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
            s if s == ":push" => {
                if history.is_empty() { println!("No definitions to push"); continue; }
                let src = repl_source_with_def(&history, "");
                match lisp_rlm_wasm::wasm_emit::compile_near(&src) {
                    Ok(wasm) => {
                        match deploy(&wasm) {
                            Ok(msg) => println!("{}", msg),
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    Err(e) => println!("Compile error: {}", e),
                }
                continue;
            }
            s if s.starts_with(":call!") => {
                let method = s.trim_start_matches(":call!").trim().trim_matches('"').trim_matches('\'');
                if method.is_empty() { println!("Usage: :call! <method_name>"); continue; }
                match call_testnet_mutable(method) {
                    Ok(output) => println!("{}", output),
                    Err(e) => println!("Error: {}", e),
                }
                continue;
            }
            s if s.starts_with(":call") && !s.starts_with(":call!") => {
                let method = s.trim_start_matches(":call").trim().trim_matches('"').trim_matches('\'');
                if method.is_empty() { println!("Usage: :call <method_name>"); continue; }
                match call_testnet_view(method) {
                    Ok(output) => println!("{}", output),
                    Err(e) => println!("Error: {}", e),
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
        match eval_wasm(&src, Some(&repl_storage), Some(&repl_memory), Some(&repl_input)) {
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

fn eval_wasm(src: &str, shared_storage: Option<&std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>>>, shared_memory: Option<&std::sync::Arc<std::sync::Mutex<Vec<u8>>>>, shared_input: Option<&std::sync::Arc<std::sync::Mutex<Vec<u8>>>>) -> Result<(String, Vec<String>), String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near(src)?;
    let mut v = wasmparser::Validator::new();
    v.validate_all(&wasm).map_err(|e| format!("WASM validation: {}", e))?;
    run_wasmtime(&wasm, shared_storage, shared_memory, shared_input)
}

fn run_wasmtime(wasm: &[u8], shared_storage: Option<&std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Vec<u8>, Vec<u8>>>>>, shared_memory: Option<&std::sync::Arc<std::sync::Mutex<Vec<u8>>>>, shared_input: Option<&std::sync::Arc<std::sync::Mutex<Vec<u8>>>>) -> Result<(String, Vec<String>), String> {
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

    let mock_input_data = shared_input.cloned().unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
    let rc_input = registers.clone();
    let input_fn = Func::new(&mut store, FuncType::new(&engine, [ValType::I64], []),
        move |_, args, _| {
            let rid = args[0].unwrap_i64() as u64;
            let data = mock_input_data.lock().unwrap().clone();
            if !data.is_empty() {
                rc_input.lock().unwrap().insert(rid, data);
            }
            Ok(())
        });

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
    linker.define(&store, "env", "input", input_fn).map_err(|e| format!("link: {}", e))?;
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

    if let Some(sm) = shared_memory {
        let mem_data = sm.lock().unwrap();
        let dst = memory.data_mut(&mut store);
        let len = std::cmp::min(mem_data.len(), dst.len());
        dst[..len].copy_from_slice(&mem_data[..len]);
    }
    let main_fn = instance.get_func(&mut store, "__repl_main").ok_or("__repl_main not found")?;
    main_fn.call(&mut store, &[], &mut []).map_err(|e| format!("call: {}", e))?;

    if let Some(sm) = shared_memory {
        let src = memory.data(&store);
        sm.lock().unwrap()[..src.len()].copy_from_slice(src);
    }
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

fn call_testnet_view(method: &str) -> Result<String, String> {
    let args_base64 = base64::encode("{}");
    let rpc_payload = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"query","params":{{"request_type":"call_function","finality":"optimistic","account_id":"kampy.testnet","method_name":"{}","args_base64":"{}"}}}}"#,
        method, args_base64
    );
    let output = std::process::Command::new("curl")
        .args(["-s", "-X", "POST", "https://rpc.testnet.near.org",
               "-H", "Content-Type: application/json",
               "-d", &rpc_payload])
        .output().map_err(|e| format!("curl: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_rpc_result(method, &stdout)
}

fn call_testnet_mutable(method: &str) -> Result<String, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let key = format!("{}/.near-credentials/testnet/kampy.testnet.json", home);
    let output = std::process::Command::new("near")
        .args(["contract", "call-function", "as-transaction", "kampy.testnet", method,
               "json-args", "{}", "prepaid-gas", "100 Tgas", "attached-deposit", "0 NEAR",
               "sign-as", "kampy.testnet", "network-config", "testnet",
               "sign-with-access-key-file", &key, "send"])
        .output().map_err(|e| format!("near CLI: {}", e))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);

    let mut result = format!("🔄 {} (mutable)\n", method);

    if let Some(val) = parse_near_cli_return(&stdout) {
        result.push_str(&format!("  return: {}\n", val));
    }

    for line in combined.lines() {
        if let Some(id) = line.split("Transaction ID:").nth(1) {
            result.push_str(&format!("  ✅ https://explorer.testnet.near.org/transactions/{}", id.trim()));
            return Ok(result);
        }
    }

    if combined.contains("no matching key") {
        return Err("No matching access key found. Account may need funding.".to_string());
    }

    result.push_str(&format!("  output: {}", combined.trim()));
    Ok(result)
}

fn parse_rpc_result(method: &str, raw: &str) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| format!("JSON parse: {}\nRaw: {}", e, raw))?;

    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown");
        return Err(format!("RPC error: {}", msg));
    }

    let mut result = format!("📋 {} → ", method);

    if let Some(res) = v.pointer("/result/result").and_then(|r| r.as_array()) {
        let bytes: Vec<u8> = res.iter().filter_map(|b| b.as_u64().map(|v| v as u8)).collect();
        if bytes.len() == 8 {
            let val = i64::from_le_bytes(bytes.as_slice().try_into().unwrap_or([0;8]));
            result.push_str(&format!("{}", val));
        } else if bytes.is_empty() {
            result.push_str("(void)");
        } else {
            match std::str::from_utf8(&bytes) {
                Ok(s) => result.push_str(&format!("\"{}\"", s)),
                Err(_) => result.push_str(&format!("{:?}", bytes)),
            }
        }
    }

    if let Some(logs) = v.pointer("/result/logs").and_then(|l| l.as_array()) {
        for log in logs.iter().filter_map(|l| l.as_str()) {
            result.push_str(&format!("\n  LOG: {}", log));
        }
    }

    Ok(result)
}

fn parse_near_cli_return(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.parse::<i64>().is_ok() {
            return Some(trimmed.to_string());
        }
    }
    None
}
