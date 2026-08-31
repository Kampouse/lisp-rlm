//! lisp-rlm unified CLI — the developer loop for lisp-rlm contracts.
//!
//! Build on the foundation of the Kampouse-line `lisp_rl.rs` skeleton;
//! v2 (2026-08-30) makes it real:
//!   - `init` scaffolds TS-first (src/main.ts, near.json, scenario tests)
//!   - `build` compiles via library calls (.ts → ts_to_lisp → compile_near)
//!     and reports REAL wasm exports (parsed from the export section)
//!   - `simulate` runs methods on the near-vm-run sandbox VM (auto-builds)
//!   - `test` runs tests/scenarios/*.json — {method, args, view, expect}
//!     through the sandbox VM and diffs results (exit code = red/green)
//!   - deploy/call/view/create/bench delegate to near-compile (full
//!     key/faucet machinery lives there; not duplicated)
//! Every subcommand supports --json for machine-parseable output.

use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
#[command(
    name = "lisp-rlm",
    version,
    about = "Lisp-RLM: lisp & TS contracts for NEAR — build, simulate, test, deploy"
)]
struct Cli {
    /// Machine-parseable JSON output
    #[arg(global = true, long)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Scaffold a new contract project (TS by default; --lisp for lisp)
    Init {
        name: String,
        /// Scaffold a lisp project instead of TS
        #[arg(long)]
        lisp: bool,
    },
    /// Compile the project (near.json → wasm) and list exports
    Build {
        dir: Option<String>,
        #[arg(long)]
        target: Option<String>,
    },
    /// Run a method on the sandbox VM (auto-builds if wasm is missing)
    Simulate {
        /// Project dir (or direct path to a .wasm)
        path: String,
        method: String,
        /// JSON args (default {})
        #[arg(long, default_value = "{}")]
        args: String,
        #[arg(long)]
        view: bool,
        /// Prepaid gas in Tgas (default 200)
        #[arg(long, default_value = "200")]
        prepaid: f64,
    },
    /// Run tests/scenarios/*.json through the sandbox VM
    Test { dir: Option<String> },
    /// Build + deploy to NEAR (delegates to near-compile)
    Deploy { dir: Option<String>, #[command(flatten)] near: NearAuth },
    /// Call a contract method (delegates to near-compile)
    Call {
        contract: String,
        method: String,
        args: Option<String>,
        #[command(flatten)]
        near: NearAuth,
        #[arg(long)]
        deposit: Option<String>,
        #[arg(long)]
        gas: Option<String>,
    },
    /// Read-only contract call (delegates to near-compile)
    View {
        contract: String,
        method: String,
        args: Option<String>,
        #[arg(long, default_value = "testnet")]
        network: String,
    },
    /// Create + fund a new account (delegates to near-compile)
    Create {
        account_id: String,
        funder: Option<String>,
        #[arg(long, default_value = "testnet")]
        network: String,
        #[arg(long)]
        key_path: Option<String>,
        #[arg(long)]
        fund: bool,
    },
    /// Fuel-meter a compiled contract (delegates to near-compile bench)
    Bench { file: String },
    /// Solidity → lisp → wasm
    Sol {
        #[command(subcommand)]
        cmd: SolCmd,
    },
}

#[derive(Subcommand)]
enum SolCmd {
    Compile { input: String, #[arg(short, long)] output: String },
}

#[derive(clap::Args, Clone)]
struct NearAuth {
    #[arg(long)]
    account: Option<String>,
    #[arg(long, default_value = "testnet")]
    network: String,
    #[arg(long)]
    key_path: Option<String>,
    #[arg(long)]
    seed_phrase: bool,
}

fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    if let Err(e) = run(cli) {
        if json {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({"ok": false, "error": e})).unwrap()
            );
        } else {
            eprintln!("error: {}", e);
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Init { name, lisp } => cmd_init(&name, lisp, cli.json),
        Commands::Build { dir, target } => cmd_build(dir.as_deref(), target.as_deref(), cli.json),
        Commands::Simulate { path, method, args, view, prepaid } => {
            cmd_simulate(&path, &method, &args, view, prepaid, cli.json)
        }
        Commands::Test { dir } => cmd_test(dir.as_deref(), cli.json),
        Commands::Deploy { dir, near } => {
            let mut a: Vec<String> = vec!["deploy".into()];
            if let Some(d) = &dir {
                a.push(d.clone());
            }
            append_auth(&mut a, &near);
            delegate("near-compile", &a)
        }
        Commands::Call { contract, method, args, near, deposit, gas } => {
            let mut a = vec!["call".into(), contract, method];
            if let Some(x) = &args {
                a.push(x.clone());
            }
            append_auth(&mut a, &near);
            if let Some(d) = &deposit {
                a.extend(["--deposit".into(), d.clone()]);
            }
            if let Some(g) = &gas {
                a.extend(["--gas".into(), g.clone()]);
            }
            delegate("near-compile", &a)
        }
        Commands::View { contract, method, args, network } => {
            let mut a = vec!["view".into(), contract, method];
            if let Some(x) = &args {
                a.push(x.clone());
            }
            if network != "testnet" {
                a.extend(["--network".into(), network]);
            }
            delegate("near-compile", &a)
        }
        Commands::Create { account_id, funder, network, key_path, fund } => {
            let mut a = vec!["create".into(), account_id];
            if let Some(f) = &funder {
                a.push(f.clone());
            }
            a.extend(["--network".into(), network]);
            if let Some(kp) = &key_path {
                a.extend(["--key-path".into(), kp.clone()]);
            }
            if fund {
                a.push("--fund".into());
            }
            delegate("near-compile", &a)
        }
        Commands::Bench { file } => delegate("near-compile", &["bench".into(), file]),
        Commands::Sol { cmd } => match cmd {
            SolCmd::Compile { input, output } => cmd_sol(&input, &output, cli.json),
        },
    }
}

// ── init ──────────────────────────────────────────────────────────────────

fn cmd_init(name: &str, lisp: bool, json: bool) -> Result<(), String> {
    let slug = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string());
    let base = Path::new(name);
    fs::create_dir_all(base.join("src")).map_err(|e| format!("{}", e))?;
    fs::create_dir_all(base.join("tests/scenarios")).map_err(|e| format!("{}", e))?;
    let _ = fs::create_dir_all(base.join("target"));

    let (src_file, src_body) = if lisp {
        (
            "src/main.lisp",
            format!(
                r#"(memory 4)
(define (hello) (near/return_str "Hello from {n}!"))
(export "hello" hello true)
"#,
                n = slug
            ),
        )
    } else {
        (
            "src/main.ts",
            format!(
                r#"// {n} — lisp-rlm TS contract
// Build:    lisp-rlm build
// Simulate: lisp-rlm simulate . new   (new_ exports as "new")
// Test:     lisp-rlm test

// NB: `new` is reserved in TS — the dialect spells NEAR's constructor `new_`
export function new_(): void {{
  near.storageSet("count", "0");
  near.log("initialized");
}}

export function increment(): void {{
  const c = strToNum(near.storageGet("count") ?? "0") + 1;
  near.storageSet("count", toStr(c));
  console.log("count:", c);
}}

export function get_count(): string {{
  return near.storageGet("count") ?? "0";
}}
"#,
                n = slug
            ),
        )
    };

    let src_key = if lisp { "src/main.lisp" } else { "src/main.ts" };
    let config = format!(
        r#"{{"name":"{n}","src":"{s}","account":"","network":"testnet","output":"target/{n}.wasm","tests":"tests/scenarios"}}
"#,
        n = slug,
        s = src_key
    );
    fs::write(base.join("near.json"), config).map_err(|e| format!("{}", e))?;
    fs::write(base.join(src_file), src_body).map_err(|e| format!("{}", e))?;
    let scenario = r#"{
  "name": "counter round-trip",
  "steps": [
    { "method": "new", "args": {} },
    { "method": "increment", "args": {} },
    { "method": "increment", "args": {} },
    { "method": "get_count", "args": {}, "view": true, "expect": "2" }
  ]
}
"#;
    fs::write(base.join("tests/scenarios/counter.json"), scenario)
        .map_err(|e| format!("{}", e))?;

    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"ok": true, "name": name, "src": src_key}))
                .unwrap()
        );
    } else {
        println!("Created project '{}' ({}):", name, if lisp { "lisp" } else { "ts" });
        println!("  cd {} && lisp-rlm build && lisp-rlm test", name);
    }
    Ok(())
}

// ── build ─────────────────────────────────────────────────────────────────

fn cmd_build(dir: Option<&str>, target: Option<&str>, json: bool) -> Result<(), String> {
    let project_dir = dir.unwrap_or(".");
    let (src, output, default_target) = load_near_json(project_dir)?;
    let target = target.unwrap_or(&default_target);
    let src_path = Path::new(project_dir).join(&src);
    let source = fs::read_to_string(&src_path).map_err(|e| format!("read {}: {}", src, e))?;

    let wasm = compile_source(&source, &src, target)?;
    let out = Path::new(project_dir).join(&output);
    if let Some(p) = out.parent() {
        let _ = fs::create_dir_all(p);
    }
    fs::write(&out, &wasm).map_err(|e| format!("write: {}", e))?;
    let exports = wasm_func_exports(&wasm);

    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({"ok": true, "bytes": wasm.len(), "exports": exports, "output": output})
            )
            .unwrap()
        );
    } else {
        println!("{} ({} bytes) [target={}]", output, wasm.len(), target);
        println!("  exports: {}", exports.join(", "));
    }
    Ok(())
}

fn compile_source(source: &str, src: &str, target: &str) -> Result<Vec<u8>, String> {
    let effective = if src.ends_with(".sol") {
        let vals = lisp_rlm_wasm::solidity::translate_solidity(source)?;
        vals.iter().map(|v| format!("{}\n", v)).collect()
    } else if src.ends_with(".ts") {
        lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(source)?
    } else {
        source.to_string()
    };
    match target {
        "near" => lisp_rlm_wasm::wasm_emit::compile_near(&effective).map_err(|e| format!("{}", e)),
        "outlayer" | "wasi" | "wasi-p1" => lisp_rlm_wasm::wasi::compile_outlayer(&effective)
            .map_err(|e| format!("{}", e)),
        "outlayer-p2" | "wasi-p2" | "component" => {
            lisp_rlm_wasm::wasi::compile_outlayer_p2(&effective).map_err(|e| format!("{}", e))
        }
        _ => Err(format!("unknown target '{}'", target)),
    }
}

// ── simulate ──────────────────────────────────────────────────────────────

fn cmd_simulate(
    path: &str,
    method: &str,
    args: &str,
    view: bool,
    prepaid: f64,
    json: bool,
) -> Result<(), String> {
    let wasm_path = resolve_wasm(path, json)?;
    let mut a: Vec<String> = vec![wasm_path, method.into(), args.into()];
    if view {
        a.push("--view".into());
    }
    a.extend(["--prepaid".into(), prepaid.to_string()]);
    if json {
        let out = capture("near-vm-run", &a)?;
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"ok": true, "output": out})).unwrap()
        );
        Ok(())
    } else {
        delegate("near-vm-run", &a)
    }
}

/// dir → target/<name>.wasm (auto-build when missing). A direct .wasm
/// path passes through untouched.
fn resolve_wasm(path: &str, json: bool) -> Result<String, String> {
    if path.ends_with(".wasm") {
        return Ok(path.into());
    }
    let (src, output, _t) = load_near_json(path)?;
    let out = Path::new(path).join(&output);
    if !out.exists() {
        // auto-build (keep stdout quiet in json mode)
        cmd_build(Some(path), None, json)?;
    }
    let _ = src;
    Ok(out.to_string_lossy().to_string())
}

// ── test ──────────────────────────────────────────────────────────────────

/// Scenario format (tests/scenarios/*.json):
///   { "name": "...", "steps": [
///       { "method": "new", "args": {} },
///       { "method": "get_x", "view": true, "expect": "42" } ] }
/// `expect` compares against the runner's 📄 result line (string contains).
fn cmd_test(dir: Option<&str>, json: bool) -> Result<(), String> {
    let project_dir = dir.unwrap_or(".");
    let wasm = resolve_wasm(project_dir, json)?;
    let tests_dir = near_json_tests_dir(project_dir)
        .unwrap_or_else(|| Path::new(project_dir).join("tests/scenarios"));

    let entries = fs::read_dir(&tests_dir)
        .map_err(|e| format!("read {}: {} (no scenarios?)", tests_dir.display(), e))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err(format!("no scenario files in {}", tests_dir.display()));
    }

    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut failures: Vec<serde_json::Value> = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let body = fs::read_to_string(&path).map_err(|e| format!("{}", e))?;
        let scen: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("{}: {}", path.display(), e))?;
        let scen_name = scen["name"].as_str().unwrap_or("scenario").to_string();
        // state reset per scenario file
        let _ = capture("near-vm-run", &[wasm.clone(), "reset".into()])?;
        for (i, step) in scen["steps"]
            .as_array()
            .ok_or_else(|| format!("{}: missing steps", path.display()))?
            .iter()
            .enumerate()
        {
            let method = step["method"].as_str().ok_or("step missing method")?.to_string();
            let args = step["args"].to_string();
            let view = step["view"].as_bool().unwrap_or(false);
            let mut a: Vec<String> = vec![wasm.clone(), method.clone(), args];
            if view {
                a.push("--view".into());
            }
            a.extend(["--prepaid".into(), "200".into()]);
            let out = capture("near-vm-run", &a)?;
            let result_line = out.lines().rev().find(|l| l.contains('📄')).unwrap_or("").to_string();
            if let Some(expect) = step["expect"].as_str() {
                let got = result_line.trim().trim_start_matches('📄').trim();
                let ok = got.contains(expect);
                if ok {
                    passed += 1;
                } else {
                    failed += 1;
                    failures.push(serde_json::json!({
                        "scenario": scen_name, "step": i, "method": method,
                        "expect": expect, "got": got
                    }));
                    if !json {
                        eprintln!("FAIL {} step {} ({}): expect '{}' got '{}'", scen_name, i, method, expect, got);
                    }
                }
            } else {
                // no expectation: run counts as passed if the runner succeeded
                if out.contains("❌") {
                    failed += 1;
                    failures.push(serde_json::json!({
                        "scenario": scen_name, "step": i, "method": method,
                        "error": out.trim()
                    }));
                } else {
                    passed += 1;
                }
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({"ok": failed == 0, "passed": passed, "failed": failed, "failures": failures})
            )
            .unwrap()
        );
    } else {
        println!(
            "{}: {} passed, {} failed",
            if failed == 0 { "PASS" } else { "FAIL" },
            passed,
            failed
        );
    }
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn near_json_tests_dir(dir: &str) -> Option<PathBuf> {
    let c = fs::read_to_string(Path::new(dir).join("near.json")).ok()?;
    let j: serde_json::Value = serde_json::from_str(&c).ok()?;
    j["tests"].as_str().map(|t| Path::new(dir).join(t))
}

// ── sol ───────────────────────────────────────────────────────────────────

fn cmd_sol(input: &str, output: &str, json: bool) -> Result<(), String> {
    let sol = fs::read_to_string(input).map_err(|e| format!("read {}: {}", input, e))?;
    let vals = lisp_rlm_wasm::solidity::translate_solidity(&sol)?;
    let lisp: String = vals.iter().map(|v| format!("{}\n", v)).collect();
    let wasm = lisp_rlm_wasm::wasm_emit::compile_near_untyped(&lisp).map_err(|e| format!("{}", e))?;
    fs::write(output, &wasm).map_err(|e| format!("write: {}", e))?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({"ok": true, "bytes": wasm.len(), "output": output}))
                .unwrap()
        );
    } else {
        println!("{} ({} bytes)", output, wasm.len());
    }
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────

fn delegate(bin: &str, args: &[String]) -> Result<(), String> {
    let bin = find_bin(bin).ok_or_else(|| format!("{} not found (build it first)", bin))?;
    let status = Command::new(&bin)
        .args(args)
        .status()
        .map_err(|e| format!("{}: {}", bin.display(), e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited {}", bin.display(), status))
    }
}

fn capture(bin: &str, args: &[String]) -> Result<String, String> {
    let bin = find_bin(bin).ok_or_else(|| format!("{} not found (build it first)", bin))?;
    let out = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {}", bin.display(), e))?;
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("{} exited {}: {}", bin.display(), out.status, s));
    }
    Ok(s)
}

fn append_auth(a: &mut Vec<String>, auth: &NearAuth) {
    if let Some(acc) = &auth.account {
        a.extend(["--account".into(), acc.clone()]);
    }
    if auth.network != "testnet" {
        a.extend(["--network".into(), auth.network.clone()]);
    }
    if let Some(kp) = &auth.key_path {
        a.extend(["--key-path".into(), kp.clone()]);
    }
    if auth.seed_phrase {
        a.push("--seed-phrase".into());
    }
}

/// Locate a sibling binary: exe dir, ../near-vm-run/target/release (repo
/// layout), ./near-vm-run/target/release (cwd = repo), then $PATH.
fn find_bin(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent().map(|p| p.to_path_buf()) {
            candidates.push(d.join(name));
            // exe in <repo>/target/release → runner project sits beside repo
            candidates.push(
                d.parent()
                    .and_then(|rd| rd.parent())
                    .map(|repo| repo.parent().map(|p| p.join("near-vm-run/target/release").join(name)))
                    .flatten()
                    .unwrap_or_default(),
            );
            // exe in <repo>/target/debug
            candidates.push(
                d.parent()
                    .and_then(|rd| rd.parent())
                    .map(|repo| repo.join("near-vm-run/target/release").join(name))
                    .unwrap_or_default(),
            );
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("near-vm-run/target/release").join(name));
        candidates.push(cwd.join("target/release").join(name));
    }
    for c in candidates.iter().filter(|c| !c.as_os_str().is_empty()) {
        if c.exists() {
            return Some(c.clone());
        }
    }
    let o = Command::new("which").arg(name).output().ok()?.stdout;
    let p = String::from_utf8_lossy(&o).trim().to_string();
    if p.is_empty() {
        None
    } else {
        Some(PathBuf::from(p))
    }
}

fn load_near_json(dir: &str) -> Result<(String, String, String), String> {
    let path = Path::new(dir).join("near.json");
    let c = fs::read_to_string(&path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    let j: serde_json::Value = serde_json::from_str(&c).map_err(|e| format!("{}", e))?;
    let name = j["name"].as_str().unwrap_or("contract").to_string();
    let src = j["src"].as_str().unwrap_or("src/main.lisp").to_string();
    let output = j["output"]
        .as_str()
        .unwrap_or(&format!("target/{}.wasm", name))
        .to_string();
    let target = j["target"].as_str().unwrap_or("near").to_string();
    Ok((src, output, target))
}

// ── wasm export-section reader ────────────────────────────────────────────

fn wasm_func_exports(wasm: &[u8]) -> Vec<String> {
    let mut exports = Vec::new();
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return exports;
    }
    let mut p = 8usize;
    while p < wasm.len() {
        let Some(id) = wasm.get(p).copied() else { break };
        p += 1;
        let Some(size) = read_uleb(wasm, &mut p) else { break };
        let sect_end = p + size as usize;
        if id == 7 {
            let Some(count) = read_uleb(wasm, &mut p) else { break };
            for _ in 0..count {
                let Some(nlen) = read_uleb(wasm, &mut p) else { break };
                let nlen = nlen as usize;
                let Some(name_bytes) = wasm.get(p..p + nlen) else { break };
                let name = String::from_utf8_lossy(name_bytes).to_string();
                p += nlen;
                let Some(kind) = wasm.get(p).copied() else { break };
                p += 1;
                let Some(_idx) = read_uleb(wasm, &mut p) else { break };
                if kind == 0 {
                    exports.push(name);
                }
            }
        }
        p = sect_end;
    }
    exports
}

fn read_uleb(wasm: &[u8], p: &mut usize) -> Option<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let b = *wasm.get(*p)?;
        *p += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    Some(result)
}
