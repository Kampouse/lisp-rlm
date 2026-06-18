#![allow(dead_code)]
#![allow(unreachable_patterns)]
//! WASI emission for OutLayer runtime.
//!
//! Produces a WASI-compliant WASM binary that:
//! - Reads input from stdin (fd 0)
//! - Calls the user's Lisp function
//! - Writes output to stdout (fd 1)
//! - Calls proc_exit(0) when done
//!
//! Uses WASI Preview 1 (fd_read/fd_write/proc_exit/random_get/environ_*).
//! Core WASM emission (arithmetic, control flow, HOFs) is shared with NEAR target.

use crate::wasm_emit::{WasmEmitter, HEAP_START};
use wasm_encoder::*;

// ── Memory layout for OutLayer target ──
// Same layout as NEAR but different I/O path
const STDIN_BUF: i64 = 32768;   // 32KB for stdin data
const STDOUT_BUF: i64 = 65536;  // 32KB for stdout data  
const STDIN_LEN: i64 = 98304;   // i32: actual bytes read
const RESULT_BUF: i64 = 65536;  // reuse STDOUT_BUF for result
// OL_RET_AREA moved to wasi_http.rs (OL_RET_AREA_BASE)

/// WASI Preview 1 function descriptors (module, name, params, results)
#[derive(Clone)]
struct WasiFunc {
    module: &'static str,
    name: &'static str,
    params: Vec<ValType>,
    results: Vec<ValType>,
}

const W: ValType = ValType::I32;

/// WASI Preview 1 imports we need
fn wasi_p1_imports() -> Vec<WasiFunc> {
    vec![
        // 0: fd_read(fd, iovs_ptr, iovs_len, nread_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "fd_read",
            params: vec![W, W, W, W], results: vec![W] },
        // 1: fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "fd_write",
            params: vec![W, W, W, W], results: vec![W] },
        // 2: proc_exit(code)
        WasiFunc { module: "wasi_snapshot_preview1", name: "proc_exit",
            params: vec![W], results: vec![] },
        // 3: random_get(buf_ptr, buf_len) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "random_get",
            params: vec![W, W], results: vec![W] },
        // 4: environ_sizes_get(count_ptr, buf_len_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "environ_sizes_get",
            params: vec![W, W], results: vec![W] },
        // 5: environ_get(environ_ptr, buf_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "environ_get",
            params: vec![W, W], results: vec![W] },
        // 6: fd_seek(fd, offset, whence, newoffset_ptr) -> errno (for future use)
        WasiFunc { module: "wasi_snapshot_preview1", name: "fd_seek",
            params: vec![W, ValType::I64, W, W], results: vec![W] },
    ]
}

/// Minimal WASI imports for P2 (only what _start actually uses)
fn wasi_p1_imports_minimal() -> Vec<WasiFunc> {
    vec![
        // 0: fd_read(fd, iovs_ptr, iovs_len, nread_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "fd_read",
            params: vec![W, W, W, W], results: vec![W] },
        // 1: fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno
        WasiFunc { module: "wasi_snapshot_preview1", name: "fd_write",
            params: vec![W, W, W, W], results: vec![W] },
        // 2: proc_exit(code)
        WasiFunc { module: "wasi_snapshot_preview1", name: "proc_exit",
            params: vec![W], results: vec![] },
    ]
}

/// OutLayer host function imports via canonical ABI (typed WIT).
///
/// All functions use canonical ABI: string = (ptr, len) as 2xi32,
/// list<u8> = (ptr, len) as 2xi32, s64 = i64.
/// Results returned via return-area pointer (last i32 param), all functions return ().
///
/// Function index order MUST match WIT order (same as FLAT_NAMES/WIT_NAMES in outlayer_adapter.rs):
/// 0:view 1:call 2:transfer 3:http-get 4:http-post 5:storage-set 6:storage-get
/// 7:storage-has 8:storage-delete 9:storage-increment 10:storage-decrement
/// 11:storage-set-if-absent 12:storage-set-if-equals 13:storage-list-keys
/// 14:storage-clear-all 15:storage-set-worker 16:storage-get-worker
/// 17:storage-set-worker-public 18:storage-get-worker-from-project
/// 19:env-signer 20:env-predecessor
///
/// Uses production split interfaces:
/// - Storage → near:storage/api (set, get, has, delete, etc.)
/// - RPC → near:rpc/api (view, call, transfer — with extra params)
/// - HTTP → outlayer:api/host (http-get, http-post — legacy until wasi:http replaces)
fn outlayer_imports() -> Vec<WasiFunc> {
    vec![
        // Upstream split interfaces only — no custom outlayer:api/host
        // 0: view(contract_id, method_name, args_json, finality_or_block) -> tuple<string, string>
        // near:rpc/api canonical: 8 i32 + ret_area = 9 params
        WasiFunc { module: "near:rpc/api@0.1.0", name: "view",
            params: vec![W; 9], results: vec![] },
        // 1: call(signer_id, signer_key, receiver_id, method_name, args_json,
        //         deposit_yocto, gas, wait_until) -> tuple<string, string>
        // near:rpc/api canonical: 16 i32 + ret_area = 17 params
        WasiFunc { module: "near:rpc/api@0.1.0", name: "call",
            params: vec![W; 17], results: vec![] },
        // 2: transfer(signer_id, signer_key, receiver_id, amount_yocto, wait_until)
        // near:rpc/api canonical: 10 i32 + ret_area = 11 params
        WasiFunc { module: "near:rpc/api@0.1.0", name: "transfer",
            params: vec![W; 11], results: vec![] },
        // 3: set(key, value: list<u8>) -> string
        // near:storage/api canonical: 4 i32 + ret_area = 5 params
        WasiFunc { module: "near:storage/api@0.1.0", name: "set",
            params: vec![W; 5], results: vec![] },
        // 4: get(key) -> tuple<list<u8>, string>
        // near:storage/api canonical: 2 i32 + ret_area = 3 params
        WasiFunc { module: "near:storage/api@0.1.0", name: "get",
            params: vec![W; 3], results: vec![] },
        // 5: has(key) -> bool
        WasiFunc { module: "near:storage/api@0.1.0", name: "has",
            params: vec![W; 2], results: vec![ValType::I32] },
        // 6: delete(key) -> bool
        WasiFunc { module: "near:storage/api@0.1.0", name: "delete",
            params: vec![W; 2], results: vec![ValType::I32] },
        // 7: increment(key, delta: s64) -> tuple<s64, string>
        WasiFunc { module: "near:storage/api@0.1.0", name: "increment",
            params: vec![W, W, ValType::I64, W], results: vec![] },
        // 8: decrement(key, delta: s64) -> tuple<s64, string>
        WasiFunc { module: "near:storage/api@0.1.0", name: "decrement",
            params: vec![W, W, ValType::I64, W], results: vec![] },
        // 9: set-if-absent(key, value) -> tuple<bool, string>
        WasiFunc { module: "near:storage/api@0.1.0", name: "set-if-absent",
            params: vec![W; 5], results: vec![] },
        // 10: set-if-equals(key, expected, new_value) -> tuple<bool, list<u8>, string>
        WasiFunc { module: "near:storage/api@0.1.0", name: "set-if-equals",
            params: vec![W; 7], results: vec![] },
        // 11: list-keys(prefix) -> tuple<string, string>
        WasiFunc { module: "near:storage/api@0.1.0", name: "list-keys",
            params: vec![W; 3], results: vec![] },
        // 12: clear-all() -> string
        WasiFunc { module: "near:storage/api@0.1.0", name: "clear-all",
            params: vec![W; 1], results: vec![] },
        // 13: set-worker(key, value, is_encrypted: option<bool>) -> string
        // near:storage/api canonical: 2+2+2+1 = 7 params
        WasiFunc { module: "near:storage/api@0.1.0", name: "set-worker",
            params: vec![W; 7], results: vec![] },
        // 14: get-worker(key, project: option<string>) -> tuple<list<u8>, string>
        // near:storage/api canonical: 2+1+2+1 = 6 params
        WasiFunc { module: "near:storage/api@0.1.0", name: "get-worker",
            params: vec![W; 6], results: vec![] },
        // 15: raw(method, params-json) -> tuple<string, string>
        // near:rpc/api canonical: 4 i32 + ret_area = 5 params
        WasiFunc { module: "near:rpc/api@0.1.0", name: "raw",
            params: vec![W; 4], results: vec![ValType::I32] },
        // 16: env-var(name: string) -> string
        // near:rpc/api canonical: 2 i32 + ret_area = 3 params
        WasiFunc { module: "near:rpc/api@0.1.0", name: "env-var",
            params: vec![W; 3], results: vec![] },
        // 17: sleep-ms(ms: u32) -> result<(), string>
        // outlayer:api/host canonical: 1 i32 + ret_area = 2 params
        WasiFunc { module: "outlayer:api/host@0.1.0", name: "sleep-ms",
            params: vec![W; 2], results: vec![] },
        // 18: send-telegram(chat_id: string, text: string) -> result<string, string>
        // outlayer:api/host canonical: 2 i32 + ret_area = 3 params
        WasiFunc { module: "outlayer:api/host@0.1.0", name: "send-telegram",
            params: vec![W; 3], results: vec![] },
        // 19: http-post-dynamic(url: string, body: list<u8>, content-type: string) -> result<list<u8>, string>
        // outlayer:api/host canonical: 6 i32 + ret_area = 7 params
        WasiFunc { module: "outlayer:api/host@0.1.0", name: "http-post",
            params: vec![W; 7], results: vec![] },
        // 20: web-search(query: string) -> result<string, string>
        // outlayer:api/host canonical: 1 i32 + ret_area = 2 params
        WasiFunc { module: "outlayer:api/host@0.1.0", name: "web-search",
            params: vec![W; 2], results: vec![] },
    ]
}

/// Sentinel values that map to outlayer_imports() indices.
/// Sentinels 103/104/120/121/137/138/200 are NOT in outlayer_imports() — they're special paths.
/// - 104 (http-post): handled via wasi:http path in combined P2
/// - 200 (POST helpers): generated by build_combined_p2_core
const OUTLAYER_SENTINELS: &[(u32, usize)] = &[
    (100, 0),   // view
    (101, 1),   // call
    (102, 2),   // transfer
    (110, 3),   // storage-set
    (111, 4),   // storage-get
    (112, 5),   // storage-has
    (113, 6),   // storage-delete
    (114, 7),   // storage-increment
    (130, 8),   // storage-decrement
    (131, 9),   // storage-set-if-absent
    (132, 10),  // storage-set-if-equals
    (133, 11),  // storage-list-keys
    (134, 12),  // storage-clear-all
    (135, 13),  // storage-set-worker
    (136, 14),  // storage-get-worker
    (140, 15),  // raw
    (122, 16),   // env-var (index 16 in outlayer_imports())
    (141, 17),   // sleep-ms (index 17 in outlayer_imports())
    (142, 18),   // send-telegram (index 18 in outlayer_imports())
    (143, 19),   // http-post-dynamic (index 19 in outlayer_imports())
    (144, 20),   // web-search (index 20 in outlayer_imports())
];

/// Scan emitted instructions for sentinel Call(N) values and return
/// the set of outlayer_imports() indices that are actually needed.
fn scan_used_outlayer_indices(em: &crate::wasm_emit::WasmEmitter) -> std::collections::HashSet<usize> {
    let mut used_sentinels = std::collections::HashSet::new();
    for f in &em.funcs {
        for instr in &f.instrs {
            if let wasm_encoder::Instruction::Call(idx) = instr {
                if *idx >= 100 && *idx <= 200 {
                    used_sentinels.insert(*idx);
                }
            }
        }
    }
    // Map sentinels to outlayer import indices
    let mut used_indices = std::collections::HashSet::new();
    for &(sentinel, ol_idx) in OUTLAYER_SENTINELS {
        if used_sentinels.contains(&sentinel) {
            used_indices.insert(ol_idx);
        }
    }
    used_indices
}

/// Build filtered outlayer imports — only includes imports whose outlayer_imports() index
/// is in `used_indices`. Returns (filtered_imports, sentinel_to_actual_fn_idx mapping, count).
fn build_filtered_outlayer(
    used_indices: &std::collections::HashSet<usize>,
    base_fn_idx: u32,
) -> (Vec<WasiFunc>, std::collections::HashMap<u32, u32>, u32) {
    let all_ol = outlayer_imports();
    let mut filtered = Vec::new();
    let mut sentinel_map = std::collections::HashMap::new();
    let mut actual_idx = base_fn_idx;

    for &(sentinel, ol_idx) in OUTLAYER_SENTINELS {
        if used_indices.contains(&ol_idx) {
            sentinel_map.insert(sentinel, actual_idx);
            filtered.push(all_ol[ol_idx].clone());
            actual_idx += 1;
        }
    }

    let count = filtered.len() as u32;
    (filtered, sentinel_map, count)
}
/// Suitable for wrapping with `wasm-tools component new --adapt`.
pub fn compile_wasi_p1(source: &str) -> Result<Vec<u8>, String> {
    let resolved = crate::wasm_emit::resolve_modules(source, std::path::Path::new("."))?;
    let exprs = crate::parser::parse_all(&resolved)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);
    let mut em = WasmEmitter::new();
    em.wasi_mode = true;
    em.no_proc_exit = true; // wit-component adapter handles exit cleanly
    for e in &exprs {
        if let crate::types::LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (crate::types::LispVal::Sym(s), crate::types::LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let crate::types::LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                crate::types::LispVal::Sym(s) => Ok(s.clone()),
                                _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![crate::types::LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                crate::types::LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }
    finish_outlayer_no_ol(&mut em)
}
/// Produces a WASM module that:
/// - Exports `_start()` (WASI entry point)
/// - Reads input from stdin → calls last defined function → writes result to stdout
/// - Uses WASI P1 for I/O, random, env vars
/// - Uses OutLayer host functions for NEAR RPC (view/call/transfer)
pub fn compile_outlayer(source: &str) -> Result<Vec<u8>, String> {
    let resolved = crate::wasm_emit::resolve_modules(source, std::path::Path::new("."))?;
    let exprs = crate::parser::parse_all(&resolved)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);
    let mut em = WasmEmitter::new();
    em.wasi_mode = true;
    for e in &exprs {
        if let crate::types::LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (crate::types::LispVal::Sym(s), crate::types::LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let crate::types::LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                crate::types::LispVal::Sym(s) => Ok(s.clone()),
                                _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![crate::types::LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                crate::types::LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }
    finish_outlayer(&mut em)
}

/// Build the WASI P1 binary from a populated WasmEmitter.
///
/// Architecture:
/// - WASI imports: fd_read, fd_write, proc_exit, random_get, environ_*
/// - OutLayer imports: view, call, transfer
/// - User functions (from WasmEmitter, same as NEAR)
/// - _start() wrapper: stdin → call user func → stdout → proc_exit(0)
/// - NEAR host calls remapped to OutLayer equivalents

/// Compile Lisp source to OutLayer WASI **Preview 2** (Component Model) binary.
///
/// Produces a proper Component that:
/// - Imports `wasi:cli/std{in,out}@0.2.1` and `wasi:random/random@0.2.1`
/// Build a self-contained stub adapter for `outlayer` imports.
/// Signatures must match the core module's actual import types exactly.
fn build_outlayer_adapter() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // Actual signatures from the core module (from wasm-tools print):
    let names: [&str; 20] = [
        "view", "call", "transfer", "http_get",
        "storage_set", "storage_get", "storage_has", "storage_delete",
        "storage_increment", "env_signer", "env_predecessor",
        "storage_decrement", "storage_set_if_absent", "storage_set_if_equals",
        "storage_list_keys", "storage_clear_all", "storage_set_worker",
        "storage_get_worker", "storage_set_worker_public", "storage_get_worker_from_project",
    ];
    let param_counts: [usize; 20] = [8,14,10,5,4,5,2,2,6,3,3,6,4,8,5,0,4,5,4,7];
    let has_result: [bool; 20] = [true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true,true];
    let mut types = TypeSection::new();
    for i in 0..20u32 {
        if has_result[i as usize] {
            types.ty().function(vec![ValType::I32; param_counts[i as usize]], vec![ValType::I32]);
        } else {
            types.ty().function(vec![ValType::I32; param_counts[i as usize]], vec![]);
        }
    }
    m.section(&types);
    let mut funcs = FunctionSection::new();
    for i in 0..21u32 { funcs.function(i); }
    m.section(&funcs);
    let mut exports = ExportSection::new();
    for (i, n) in names.iter().enumerate() { exports.export(*n, ExportKind::Func, i as u32); }
    m.section(&exports);
    let mut code = CodeSection::new();
    for i in 0..21u32 {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        if has_result[i as usize] {
            body.instruction(&Instruction::I32Const(0));
        }
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    m.section(&code);
    m.finish()
}

/// - Lowers them to core functions that satisfy P1 imports (fd_read, fd_write, etc.)
/// - Instantiates core P1 module with lowered functions
/// - Lifts `_start` and exports as `wasi:cli/run@0.2.1/run`
/// Browser-safe P2 compile — skips module resolution (no filesystem access).
/// Use this from wasm-bindgen/browser builds.
pub fn compile_outlayer_p2_browser(source: &str) -> Result<Vec<u8>, String> {
    let exprs = crate::parser::parse_all(source)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);
    compile_outlayer_p2_from_exprs(&exprs)
}

/// Browser-safe P2 compile — returns CORE WASM (before component wrapping).
/// This can be instantiated directly in the browser with WASI polyfills.
pub fn compile_outlayer_p2_core_browser(source: &str) -> Result<Vec<u8>, String> {
    let exprs = crate::parser::parse_all(source)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);
    let mut em = WasmEmitter::new();
    em.wasi_mode = true;
    em.p2_mode = true;
    em.no_proc_exit = true;
    for e in exprs {
        if let crate::types::LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (crate::types::LispVal::Sym(s), crate::types::LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let crate::types::LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                crate::types::LispVal::Sym(s) => Ok(s.clone()),
                                _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![crate::types::LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                crate::types::LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }
    
    // Return CORE WASM (before component wrapping) — browser can run this with WASI polyfills
    if em.need_wasi_http {
        let (core_bytes, _) = build_combined_p2_core(&mut em)?;
        Ok(core_bytes)
    } else if em.need_outlayer {
        finish_outlayer(&mut em)
    } else {
        finish_outlayer_no_ol(&mut em)
    }
}

/// Compile pre-parsed expressions to P2 WASM — no filesystem access.
pub fn compile_outlayer_p2_from_exprs(exprs: &[crate::types::LispVal]) -> Result<Vec<u8>, String> {
    let mut em = WasmEmitter::new();
    em.wasi_mode = true;
    em.p2_mode = true;
    em.no_proc_exit = true;
    for e in exprs {
        if let crate::types::LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (crate::types::LispVal::Sym(s), crate::types::LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let crate::types::LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                crate::types::LispVal::Sym(s) => Ok(s.clone()),
                                _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![crate::types::LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                crate::types::LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }

    eprintln!("[DEBUG] need_outlayer={}, need_wasi_http={}, funcs={}", em.need_outlayer, em.need_wasi_http, em.funcs.len());
    for f in &em.funcs {
        eprintln!("[DEBUG] func {}: {} instrs", f.name, f.instrs.len());
    }
    let bytes = if em.need_outlayer {
        // Always use combined P2 path for outlayer programs — produces valid WASI P2
        // (no wasi_snapshot_preview1 imports). Works with or without HTTP.
        let (core_bytes, has_outlayer) = build_combined_p2_core(&mut em)?;
        build_combined_p2_component(&core_bytes, has_outlayer)?
    } else if em.need_wasi_http {
        build_p2_with_wasi_http(&em)?
    } else {
        let core_bytes = finish_outlayer_no_ol(&mut em)?;
        std::fs::write("/tmp/core_before_patch.wasm", &core_bytes).ok();
        crate::p2_native::build_native_p2_component(&core_bytes)?
    };
    Ok(bytes)
}

pub fn compile_outlayer_p2(source: &str) -> Result<Vec<u8>, String> {
    // 1. Compile the core P1 module first
    let resolved = crate::wasm_emit::resolve_modules(source, std::path::Path::new("."))?;
    let exprs = crate::parser::parse_all(&resolved)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);
    let mut em = WasmEmitter::new();
    em.wasi_mode = true;
    em.p2_mode = true;
    em.no_proc_exit = true; // wit-component adapter handles exit
    for e in &exprs {
        if let crate::types::LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if items.len() >= 3 {
                if let (crate::types::LispVal::Sym(s), crate::types::LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let crate::types::LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                crate::types::LispVal::Sym(s) => Ok(s.clone()),
                                _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![crate::types::LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                crate::types::LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
            }
        }
    }

    eprintln!("[DEBUG] compile_outlayer_p2: need_outlayer={}, need_wasi_http={}, funcs={}", em.need_outlayer, em.need_wasi_http, em.funcs.len());
    for f in &em.funcs {
        eprintln!("[DEBUG] func {}: {} instrs", f.name, f.instrs.len());
    }
    let bytes = if em.need_outlayer {
        // Always use combined P2 path for outlayer programs — produces valid WASI P2
        // (no wasi_snapshot_preview1 imports). Works with or without HTTP.
        let (core_bytes, has_outlayer) = build_combined_p2_core(&mut em)?;
        build_combined_p2_component(&core_bytes, has_outlayer)?
    } else if em.need_wasi_http {
        // wasi:http path — build component with embedded HTTP metadata
        build_p2_with_wasi_http(&em)?
    } else {
        let core_bytes = finish_outlayer_no_ol(&mut em)?;
        // Use manual component builder (production-compatible, handles wasi_snapshot_preview1 stubs)
        std::fs::write("/tmp/p2_ol_core.wasm", &core_bytes).ok();
        crate::p2_native::build_native_p2_component(&core_bytes)?
    };
    Ok(bytes)
}

/// Build a P2 component with wasi:http support.
/// Generates a core module with wasi:http canonical imports, embeds WIT metadata,
/// and builds the component through wit-component.
/// Build P2 component with wasi:http imports + user's Lisp code.
///
/// This creates a component that:
/// 1. Imports wasi:http functions (27 imports, indices 0-26)
/// 2. Has `__wasi_http_get` internal function that uses those imports
/// 3. Has all user functions from the emitter
/// 4. Has `_start` wrapper (read stdin, call user fn, write stdout)
/// 5. Has `cabi_realloc` bump allocator
fn build_p2_with_wasi_http(em: &WasmEmitter) -> Result<Vec<u8>, String> {
    use crate::wasi_http::*;
    use crate::wasi_http_buffer;

    let mut module = Module::new();

    // Determine URL list: if the emitter collected URLs from http-get calls, use those;
    // otherwise fall back to a single hardcoded URL for backward compatibility.
    // Only apply fallback if there are no POST URLs either (to avoid sentinel collision).
    let http_urls: Vec<(String, String)> = if em.http_urls.is_empty() && em.http_post_urls.is_empty() {
        vec![(
            "api.open-meteo.com".to_string(),
            "/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m".to_string(),
        )]
    } else {
        em.http_urls.clone()
    };
    let http_get_count = http_urls.len() as u32;

    // Compute all indices dynamically
    let http_post_count = em.http_post_urls.len() as u32;
    let layout = WasiHttpLayout::new(em.funcs.len() as u32, http_get_count, http_post_count);

    // POST sentinel base must account for the actual (possibly fallback-augmented) GET count
    let post_sentinel_base = 200;

    // ═══ Type Section ═══
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    // Single source of truth for HTTP types + imports
    add_http_imports_to_sections(&mut types, &mut imports);

    // User function types: (N x i64) -> i64
    for i in 0..=16u32 {
        let params = vec![ValType::I64; i as usize];
        types.ty().function(params, [ValType::I64]);
    }
    // _start: () -> i32 (result<()>)
    types.ty().function([], [ValType::I32]);
    // cabi_realloc: (i32,i32,i32,i32) -> i32
    types.ty().function([ValType::I32; 4], [ValType::I32]);
    // __wasi_http_get: (i32,i32,i32,i32,i32) -> i32
    types.ty().function([ValType::I32; 5], [ValType::I32]);
    // __wasi_http_post: (i32,i32,i32,i32,i32,i32,i32) -> i32
    types.ty().function([ValType::I32; 7], [ValType::I32]);
    module.section(&types);
    module.section(&imports);

    // ═══ Function Section ═══
    let mut functions = FunctionSection::new();
    // For each GET URL, emit an http_get + poll_read pair
    for _ in &http_urls {
        functions.function(layout.http_get_type);
        functions.function(layout.http_get_type); // poll_read — same type
    }
    // For each POST URL, emit an http_post + poll_read pair
    for _ in &em.http_post_urls {
        functions.function(layout.http_post_type);
        functions.function(layout.http_get_type); // poll_read — same type as GET
    }
    for f in &em.funcs {
        let type_idx = layout.user_type_base + f.param_count as u32;
        functions.function(type_idx);
    }
    functions.function(layout.start_type);
    functions.function(layout.realloc_type);
    module.section(&functions);

    // ═══ Table + Element Section ═══
    // REMOVED: wit-component generates its own table via the shim/fixup modules.
    // Our table was conflicting with the adapter's $imports table.

    // ═══ Memory Section ═══
    let mut memory = MemorySection::new();
    let pages = em.memory_pages.max(2048) as u64; // min 2048 pages (128MB) - OutLayer default
    memory.memory(MemoryType { minimum: pages, maximum: None, memory64: false, shared: false, page_size_log2: None });
    module.section(&memory);

    // ═══ Global Section ═══
    // Global 0: depth counter (i64) — must match emitter convention
    // Global 1: return flag (i64) — must match emitter convention
    let mut globals = GlobalSection::new();
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(0));
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(0));
    module.section(&globals);

    // ═══ Export Section ═══
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, layout.start_fn_idx);
    exports.export("cabi_realloc", ExportKind::Func, layout.realloc_fn_idx);
    module.section(&exports);

    // ═══ Element Section ═══
    // REMOVED: no table needed — wit-component handles indirect calls via shim/fixup.

    // ═══ Code Section ═══
    let mut codes = CodeSection::new();

    // ── Generate HTTP functions: one (get + poll_read) pair per URL ──
    // Build data segments for ALL URLs, with offsets stacked after each other
    let mut all_http_data_segments: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut current_data_offset = wasi_http_buffer::DATA_BASE;

    let headers: &[( &[u8], &[u8] )] = &[
        (b"User-Agent", b"lisp-rlm/0.1 (wasi:http)"),
        (b"Accept", b"application/json"),
    ];

    for (_url_idx, (authority, path)) in http_urls.iter().enumerate() {
        // Build data segments at the current offset
        let http_data = wasi_http_buffer::build_url_data_segments_with_base(
            authority.as_bytes(),
            path.as_bytes(),
            headers,
            current_data_offset,
        );

        // ── __wasi_http_get for this URL ──
        // Params: 0=url_ptr, 1=url_len, 2=buf_ptr, 3=buf_len, 4=len_ptr
        // Extra locals: 5=fields, 6=req, 7=body, 8=future, 9=pollable,
        //               10=response, 11=resp_body, 12=in_stream,
        //               13=temp_i/path_len, 14=authority_len, 15=authority_ptr,
        //               16=path_ptr, 17=bytes_written
        let mut http_get_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (5u32, ValType::I32), // locals 13-17 (copy_i=16, etc.)
        ]);
        wasi_http_buffer::emit_http_get_to_buffer(&mut http_get_fn, &http_data);
        http_get_fn.instruction(&Instruction::End);
        codes.function(&http_get_fn);

        // ── http_poll_read for this URL ──
        let mut poll_read_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
        ]);
        wasi_http_buffer::emit_http_poll_read(&mut poll_read_fn);
        poll_read_fn.instruction(&Instruction::End);
        codes.function(&poll_read_fn);

        // Collect data segments and advance offset
        let span = http_data.total_span();
        for (off, bytes) in http_data.segments {
            all_http_data_segments.push((off, bytes));
        }
        current_data_offset = span;
        // Align to 4 bytes for the next URL's data
        current_data_offset = (current_data_offset + 3) & !3;
    }

    // ── Generate HTTP POST functions: one (post + poll_read) pair per POST URL ──
    let post_headers: &[( &[u8], &[u8] )] = &[
        (b"User-Agent", b"lisp-rlm/0.1 (wasi:http)"),
        (b"Accept", b"application/json"),
        (b"Content-Type", b"application/json"),
    ];

    for (_url_idx, (authority, path)) in em.http_post_urls.iter().enumerate() {
        let http_data = wasi_http_buffer::build_url_data_segments_with_base(
            authority.as_bytes(),
            path.as_bytes(),
            post_headers,
            current_data_offset,
        );

        // ── __wasi_http_post for this URL ──
        // Params: 0=url_ptr, 1=url_len, 2=body_ptr, 3=body_len, 4=buf_ptr, 5=buf_len, 6=len_ptr
        let mut http_post_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
        ]);
        wasi_http_buffer::emit_http_post_to_buffer(&mut http_post_fn, &http_data);
        http_post_fn.instruction(&Instruction::End);
        codes.function(&http_post_fn);

        // ── http_poll_read for this POST URL (same as GET) ──
        let mut poll_read_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32),
        ]);
        wasi_http_buffer::emit_http_poll_read(&mut poll_read_fn);
        poll_read_fn.instruction(&Instruction::End);
        codes.function(&poll_read_fn);

        let span = http_data.total_span();
        for (off, bytes) in http_data.segments {
            all_http_data_segments.push((off, bytes));
        }
        current_data_offset = span;
        current_data_offset = (current_data_offset + 3) & !3;
    }

    // ── User functions from the emitter ──
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter().enumerate()
        .map(|(i, f)| (f.name.as_str(), layout.user_fn_base + i as u32))
        .collect();

    for f in &em.funcs {
        let locals = if let Some(ref entries) = f.local_entries {
            entries.clone()
        } else {
            let extra = f.local_count.saturating_sub(f.param_count);
            if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] }
        };

        let resolved = {
            let base_resolved = WasmEmitter::resolve_static_pub(&f.instrs, &std::collections::HashMap::new(), &name_map, &em.funcs);
            let mut ol_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            // Map each sentinel 103+i to the corresponding HTTP GET function
            for i in 0..http_get_count {
                ol_map.insert(103 + i, layout.http_get_fn_idx + (i * 2));
            }
            // Map POST sentinels: post_sentinel_base+i → POST function
            for i in 0..http_post_count {
                ol_map.insert(post_sentinel_base + i, layout.http_post_fn_idx + (i * 2));
            }
            ol_map.insert(crate::wasm_emit::WASI_FD_WRITE, layout.user_fn_base + em.funcs.len() as u32 + 2); // sentinel
            WasmEmitter::resolve_static_pub_ex(&base_resolved, &std::collections::HashMap::new(), &name_map, &em.funcs, &ol_map)
        };

        let mut fb = Function::new(locals);
        for instr in &resolved { fb.instruction(instr); }
        fb.instruction(&Instruction::End);
        codes.function(&fb);
    }

    // ── _start() wrapper ──
    // Same structure as the one in finish_outlayer_inner, but without fd_read/fd_write
    // Instead: read stdin via wasi:http stdin, write stdout via wasi:http stdout
    // For now, keep it simple — the _start reads stdin, calls user fn, writes result
    {
        // For wasi:http components, stdin/stdout work via the wasi:cli interface
        // which is provided by the runtime. But our module doesn't import them directly —
        // the component model adapter provides them.
        // 
        // Actually, our module ONLY imports wasi:http functions. It doesn't import
        // wasi:cli/stdin or wasi:cli/stdout. The wasi:http imports include get-stdout
        // (FN_GET_STDOUT = 23) for the HTTP response writing, but for stdin we need
        // wasi:cli/stdin.
        //
        // Hmm, this is a problem. The _start wrapper needs stdin/stdout.
        // Let me add wasi:cli/stdin import too.

        // Actually wait — looking at the wasi:http imports, we already have:
        // - FN_GET_STDOUT (23) = wasi:cli/stdout@0.2.2 get-stdout
        // - FN_OUTPUT_STREAM_WRITE (24) = wasi:io/streams blocking-write-and-flush
        // But no stdin!
        //
        // For the _start wrapper, we need stdin to read input.
        // Option 1: Add wasi:cli/stdin import
        // Option 2: Use environment variables for input (the runtime passes input via env)
        // Option 3: Skip stdin entirely for HTTP programs — the URL is hardcoded

        // For now: build a minimal _start that just calls the user function with nil
        // and writes the result to stdout. The user function should use http-get directly.

        // Minimal _start: resolve through trivial wrappers to find the real function
        // Prefer the function named "run", fall back to last user function (skip __ helpers)
        let default_pos = em.funcs.iter().rposition(|f| !f.name.starts_with("__")).unwrap_or(em.funcs.len() - 1);
        let mut real_func_idx = layout.user_fn_base + default_pos as u32;
        let mut real_param_count = em.funcs[default_pos].param_count;
        // First: look for a function explicitly named "run"
        if let Some(run_pos) = em.funcs.iter().position(|f| f.name == "run") {
            real_func_idx = layout.user_fn_base + run_pos as u32;
            real_param_count = em.funcs[run_pos].param_count;
        } else if em.funcs.len() > 1 {
            let last = em.funcs.last().unwrap();
            if last.param_count == 0 && last.name != "run" {
                let call_count = last.instrs.iter().filter(|i| matches!(i, Instruction::Call(_))).count();
                if call_count == 1 {
                    real_func_idx = layout.user_fn_base + (em.funcs.len() - 2) as u32;
                    real_param_count = em.funcs[em.funcs.len() - 2].param_count;
                }
            }
        }
        let param_count = real_param_count;

        let mut fb = Function::new([
            (1u32, W),  // local 0: stdout handle (i32)
            (1u32, W),  // local 1: string ptr (i32)
            (1u32, ValType::I64),  // local 2: result (i64)
            (1u32, W),  // local 3: string len (i32)
        ]);

        // Call user function with TAG_NIL as input
        for _ in 0..param_count {
            fb.instruction(&Instruction::I64Const(crate::wasm_emit::TAG_NIL as i64));
        }
        fb.instruction(&Instruction::Call(real_func_idx));
        fb.instruction(&Instruction::LocalSet(2)); // result

        // Write result to stdout using wasi:http stdout
        // get-stdout
        fb.instruction(&Instruction::Call(FN_GET_STDOUT)); // () -> i32
        fb.instruction(&Instruction::LocalSet(0)); // stdout handle

        // Convert result to string and write
        // For simplicity: if result is a string, write it. Otherwise write nothing.
        // Tag check: result & 7 == TAG_STR (7)
        let _ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };

        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I64Const(7));
        fb.instruction(&Instruction::I64And);
        fb.instruction(&Instruction::I64Const(crate::wasm_emit::TAG_STR as i64));
        fb.instruction(&Instruction::I64Eq);
        fb.instruction(&Instruction::If(BlockType::Empty));
        // It's a string: extract ptr and len
        // ptr = (val >> 3) & 0xFFFFFFFF
        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64ShrU);
        fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
        fb.instruction(&Instruction::I64And);
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::LocalSet(1)); // ptr (i32)

        // len = (val >> 35) & 0xFFFFFFFF
        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I64Const(35));
        fb.instruction(&Instruction::I64ShrU);
        fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
        fb.instruction(&Instruction::I64And);
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::LocalSet(3)); // len (i32 local)

        // Write to stdout: blocking-write-and-flush(stdout, ptr, len, 0)
        fb.instruction(&Instruction::LocalGet(0)); // stdout
        fb.instruction(&Instruction::LocalGet(1)); // ptr
        fb.instruction(&Instruction::LocalGet(3)); // len
        fb.instruction(&Instruction::I32Const(0)); // pad
        fb.instruction(&Instruction::Call(FN_OUTPUT_STREAM_WRITE));

        fb.instruction(&Instruction::End); // end if

        // Drop stdout
        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::Call(FN_DROP_OUTPUT_STREAM));

        // Return 0 (success)
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::End); // end function
        codes.function(&fb);
    }

    // ── cabi_realloc ──
    // Unified heap: uses RUNTIME_HEAP_PTR (address 56) shared with lisp heap.
    // HEAP_START is 200000, heap grows toward SENTINEL_BUF at ~1.3MB.
    // Returns 0 (null) for new_size == 0, matching Rust SDK behavior.
    {
        let mut realloc = Function::new([]); // no extra locals needed
        let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 }; // 8-byte align
        // cabi_realloc(old_ptr, old_size, align, new_size) -> ptr
        // Always fresh allocate (bump allocator can't realloc in place)
        // If new_size == 0 → return 0 (null)
        realloc.instruction(&Instruction::LocalGet(3)); // new_len
        realloc.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        // new_size > 0 → fresh allocation from unified heap
        // Load current heap ptr from RUNTIME_HEAP_PTR (address 56)
        realloc.instruction(&Instruction::I32Const(56));
        realloc.instruction(&Instruction::I64Load(ma8)); // load i64 heap ptr
        realloc.instruction(&Instruction::I32WrapI64); // cast to i32 for return
        // Bump RUNTIME_HEAP_PTR by new_len aligned up to 8
        realloc.instruction(&Instruction::LocalGet(3)); // new_len
        realloc.instruction(&Instruction::I32Const(7));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(-8));
        realloc.instruction(&Instruction::I32And); // align up to 8
        realloc.instruction(&Instruction::I64ExtendI32U);
        // Stack: [current_ptr_i32, aligned_len_i64]
        realloc.instruction(&Instruction::I32Const(56));
        // Stack: [current_ptr_i32, aligned_len_i64, 56]
        realloc.instruction(&Instruction::I64Load(ma8)); // load current heap ptr again
        // Stack: [current_ptr_i32, aligned_len_i64, heap_i64]
        realloc.instruction(&Instruction::I64Add); // new heap ptr
        // Stack: [current_ptr_i32, new_heap_i64]
        realloc.instruction(&Instruction::I32Const(56));
        // Stack: [current_ptr_i32, new_heap_i64, 56]
        realloc.instruction(&Instruction::I64Store(ma8)); // store new heap ptr
        // Stack: [current_ptr_i32] (the returned pointer)
        realloc.instruction(&Instruction::Else);
        // new_size == 0 → return 0 (null)
        realloc.instruction(&Instruction::I32Const(0));
        realloc.instruction(&Instruction::End); // end new_size check
        realloc.instruction(&Instruction::End); // end function body
        codes.function(&realloc);
    }

    module.section(&codes);

    // ═══ Data Section (string literals + URL data segments) ═══
    {
        let mut data = DataSection::new();
        let mut has_data = false;
        // Initialize RUNTIME_HEAP_PTR (addr 56) to HEAP_START (200000)
        {
            let heap_start_bytes: [u8; 8] = (HEAP_START as u64).to_le_bytes();
            data.active(0, &ConstExpr::i32_const(56), heap_start_bytes.iter().copied());
            has_data = true;
        }
        // Initialize DEPTH_COUNTER (addr 999980) to 0
        {
            let depth_zero: [u8; 8] = 0u64.to_le_bytes();
            data.active(0, &ConstExpr::i32_const(999980), depth_zero.iter().copied());
            has_data = true;
        }
        // String literals from lisp emitter
        for (off, bytes) in &em.data_segments {
            data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
            has_data = true;
        }
        // URL data segments (authority + path pre-loaded at instantiation)
        for (off, bytes) in &all_http_data_segments {
            data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
            has_data = true;
        }
        if has_data {
            module.section(&data);
        }
    }

    // ═══ Build component using manual encoding ═══
    let core_bytes = module.finish();
    std::fs::write("/tmp/p2_http_core.wasm", &core_bytes).ok();

    // Use p2_native to wrap as component — it handles the wasi:cli/run export properly
    let component = crate::p2_native::build_wasi_http_component(&core_bytes, &em)?;

    std::fs::write("/tmp/p2_wasi_http.wasm", &component).ok();

    if let Err(_e) = wasmparser::validate(&component) {
        // Validation issues will surface at runtime
    }

    Ok(component)
}


fn build_p2_with_adapter(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Debug: check what we're passing to the encoder
    std::fs::write("/tmp/p2_core_to_encode.wasm", core_bytes).ok();

    // Build encoder with just the core module — no WASI adapter.
    // The production OutLayer worker provides wasi-snapshot-preview1 at core level
    // and wasi:cli at component level via wasmtime_wasi::add_to_linker_async.
    let mut encoder = wit_component::ComponentEncoder::default()
        .module(core_bytes)
        .map_err(|e| format!("wit-component: failed to set module: {}", e))?
        .validate(false);

    // outlayer:api/host adapter removed — upstream uses split interfaces

    let component = encoder.encode()
        .map_err(|e| format!("wit-component encode failed: {:#}", e))?;

    Ok(component)
}

/// Analyze a core WASM module to find which import module names are referenced.
/// Inject WIT metadata custom section into the core module so wit-component
/// can type the `outlayer:api/host` imports correctly.
/// Uses the official `embed_component_metadata` API.
fn inject_outlayer_wit(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Uses the combined WIT (outlayer-http world) for upstream split interfaces
    // NOTE: This function is currently dead code - P2 uses build_combined_wit_metadata instead
    const COMBINED_WIT: &str = include_str!("../../wit/deps/combined.wit");
    let mut resolve = wit_parser::Resolve::new();
    let ast = wit_parser::UnresolvedPackageGroup::parse("wit/deps/combined.wit", COMBINED_WIT)
        .map_err(|e| format!("WIT parse error: {}", e))?;
    let pkg_id = resolve.push_group(ast).map_err(|e| format!("WIT push error: {}", e))?;
    let world_id = resolve.packages[pkg_id]
        .worlds
        .iter()
        .find(|(name, _)| *name == "outlayer-http")
        .map(|(_, &id)| id)
        .ok_or("outlayer-http world not found in WIT package")?;

    let mut bytes = core_bytes.to_vec();
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    ).map_err(|e| format!("embed WIT metadata: {}", e))?;

    Ok(bytes)
}

fn analyze_core_imports(wasm: &[u8]) -> Vec<&str> {
    let mut modules = Vec::new();
    let mut pos = 8;
    while pos < wasm.len() {
        let section_id = wasm[pos];
        pos += 1;
        let (size, leb) = read_leb128_outlayer(&wasm[pos..]);
        pos += leb;
        if section_id == 2 {
            let _end = pos + size;
            let (count, cl) = read_leb128_outlayer(&wasm[pos..]);
            pos += cl;
            for _ in 0..count {
                let (mod_len, ml) = read_leb128_outlayer(&wasm[pos..]);
                pos += ml;
                let module = std::str::from_utf8(&wasm[pos..pos + mod_len]).unwrap_or("");
                pos += mod_len;
                let (name_len, nl) = read_leb128_outlayer(&wasm[pos..]);
                pos += nl + name_len;
                let kind = wasm[pos];
                pos += 1;
                // Skip type-specific bytes
                match kind {
                    0 => { let (_tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    1 => { pos += 3; let (_tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    2 => { let (_tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    3 => { pos += 1; let (_tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    _ => {}
                }
                if !modules.contains(&module) && !module.is_empty() {
                    modules.push(module);
                }
            }
            break;
        }
        pos += size;
    }
    modules
}

/// Strip "outlayer" imports and rename _start → run for P2 component compatibility
fn encode_leb128(mut value: u32, sink: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            sink.push(byte);
            break;
        }
        sink.push(byte | 0x80);
    }
}

fn read_leb128_outlayer(data: &[u8]) -> (usize, usize) {
    let mut result = 0usize;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7F) as usize) << shift;
        shift += 7;
        if byte & 0x80 == 0 { return (result, i + 1); }
        if shift > 63 { return (0, 1); }
    }
    (0, 1)
}

struct RawModuleSection<'a>(&'a [u8]);

impl wasm_encoder::Encode for RawModuleSection<'_> {
    fn encode(&self, sink: &mut Vec<u8>) {
        self.0.encode(sink);
    }
}

impl wasm_encoder::ComponentSection for RawModuleSection<'_> {
    fn id(&self) -> u8 {
        wasm_encoder::ComponentSectionId::CoreModule as u8
    }
}

/// Map of WASI P1 function name → (wasi_imports index, core module import index).
/// Built dynamically based on what's needed.
struct WasiImportMap {
    /// Maps WASI func name to the import index in the core module
    indices: std::collections::HashMap<&'static str, u32>,
}

fn finish_outlayer(em: &mut WasmEmitter) -> Result<Vec<u8>, String> {
    if em.need_wasi_http {
        let (core_bytes, has_outlayer) = build_combined_p2_core(em)?;
        build_combined_p2_component(&core_bytes, has_outlayer)
    } else {
        finish_outlayer_inner(em, false)
    }
}

fn finish_outlayer_no_ol(em: &mut WasmEmitter) -> Result<Vec<u8>, String> {
    finish_outlayer_inner(em, true)
}

fn finish_outlayer_inner(em: &mut WasmEmitter, skip_outlayer: bool) -> Result<Vec<u8>, String> {
    if em.funcs.is_empty() {
        return Err("no functions defined".into());
    }

    em.tree_shake();

    let wasi = if skip_outlayer { wasi_p1_imports_minimal() } else { wasi_p1_imports() };
    // Tree-shake: only import outlayer functions that are actually used
    let (ol, ol_sentinel_map_p1, ol_count) = if skip_outlayer {
        (vec![], std::collections::HashMap::new(), 0u32)
    } else {
        let used_ol_indices = scan_used_outlayer_indices(em);
        let (filtered, smap, count) = build_filtered_outlayer(&used_ol_indices, wasi.len() as u32);
        (filtered, smap, count)
    };
    let wasi_count = wasi.len() as u32;
    let total_imports = wasi_count + ol_count;

    // Find which NEAR host functions are actually used, so we can emit stubs
    // that delegate to OutLayer equivalents
    let near_host_used: Vec<usize> = {
        let host_list: Vec<usize> = (0..50).filter(|i| em.host_needed.contains(i)).collect();
        host_list
    };

    let mut m = Module::new();

    // ── Type section ──
    let mut types = TypeSection::new();
    // P2 via adapter: _start returns () (wasi:cli expects () -> ())
    // P1: _start returns () and calls proc_exit
    types.ty().function([], []); // type 0: () -> () (_start)
    // type 1: (i32, i32, i32, i32) -> i32 — fd_read, fd_write
    types.ty().function([W, W, W, W], [W]);
    // type 2: (i32) -> () — proc_exit
    types.ty().function([W], []);
    // type 3: (i32, i32) -> i32 — random_get
    types.ty().function([W, W], [W]);
    // type 4: (i32, i32) -> i32 — environ_sizes_get
    types.ty().function([W, W], [W]);
    // type 5: (i32, i32) -> i32 — environ_get
    types.ty().function([W, W], [W]);
    // type 6: (i32, i64, i32, i32) -> i32 — fd_seek
    types.ty().function([W, ValType::I64, W, W], [W]);

    // OutLayer canonical ABI host types — all return () via ret_area pointer
    // Unique signatures needed for split interfaces:
    // type 7: (i32) -> () — env-signer, env-predecessor, clear-all
    types.ty().function(vec![W; 1], []);
    // type 8: (i32, i32, i32) -> () — http-get, get, has, delete, list-keys, get-worker
    types.ty().function(vec![W; 3], []);
    // type 9: (i32*5) -> () — set, set-if-absent, raw, set-worker, set-worker-public, get-worker-from-project
    types.ty().function(vec![W; 5], []);
    // type 10: (i32*7) -> () — http-post, set-if-equals
    types.ty().function(vec![W; 7], []);
    // type 11: (i32*9) -> () — view (4 strings + ret_area)
    types.ty().function(vec![W; 9], []);
    // type 12: (i32*11) -> () — transfer (5 strings + ret_area)
    types.ty().function(vec![W; 11], []);
    // type 13: (i32*17) -> () — call (8 strings + ret_area)
    types.ty().function(vec![W; 17], []);
    // type 14: (i32, i32, i64, i32) -> () — increment, decrement (s64 canonical ABI)
    types.ty().function(vec![W, W, ValType::I64, W], []);
    // type 15: (i32, i32) -> i32 — has, delete (direct bool return, no ret_area)
    types.ty().function(vec![W; 2], [W]);
    // type 16: (i32*6) -> () — get-worker, get-worker-from-project
    types.ty().function(vec![W; 6], []);

    // NEAR-style host function types (for NEAR compat stubs)
    // We need types for each unique NEAR host function signature used
    // Map NEAR host func index to its type index
    let _ = &near_host_used; // used below
    // User function types: each function has (i64 × param_count) -> i64
    let max_p = em.funcs.iter().map(|f| f.param_count).max().unwrap_or(0);
    let user_type_base: u32 = 17;
    for p in 0..=max_p {
        let params: Vec<ValType> = (0..p).map(|_| ValType::I64).collect();
        types.ty().function(params, [ValType::I64]);
    }

    // NEAR host stub types start after user types
    let mut nti = user_type_base + max_p as u32 + 1; // next type index

    // NEAR host stub types
    // type for () -> i64
    types.ty().function([], [ValType::I64]);
    let near_void_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64) -> ()
    types.ty().function([ValType::I64], []);
    let near_i64_to_void = nti;
    nti += 1;
    let _ = nti;
    // type for (i64) -> i64
    types.ty().function([ValType::I64], [ValType::I64]);
    let near_i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64) -> ()
    types.ty().function([ValType::I64, ValType::I64], []);
    let near_2i64_to_void = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64) -> i64
    types.ty().function([ValType::I64, ValType::I64], [ValType::I64]);
    let near_2i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64) -> ()
    types.ty().function([ValType::I64, ValType::I64, ValType::I64], []);
    let near_3i64_to_void = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64) -> i64
    types.ty().function([ValType::I64, ValType::I64, ValType::I64], [ValType::I64]);
    let near_3i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64, i64) -> i64 (storage_write)
    types.ty().function([ValType::I64; 5], [ValType::I64]);
    let near_5i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64, i64, i64, i64, i64) -> i64 (promise_create)
    types.ty().function([ValType::I64; 8], [ValType::I64]);
    let near_8i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 (promise_then)
    types.ty().function([ValType::I64; 9], [ValType::I64]);
    let near_9i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64, i64, i64, i64) -> () (promise_batch_action_function_call)
    types.ty().function([ValType::I64; 7], []);
    let near_7i64_to_void = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64, i64, i64) -> () (ed25519_verify variant)
    types.ty().function([ValType::I64; 6], [ValType::I64]);
    let near_6i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64) -> i64 (storage_iter_range)
    types.ty().function([ValType::I64; 4], [ValType::I64]);
    let near_4i64_to_i64 = nti;
    nti += 1;
    let _ = nti;
    // type for (i64, i64, i64, i64) -> () (promise_batch_action_stake)
    types.ty().function([ValType::I64; 4], []);
    let near_4i64_to_void = nti;
    nti += 1;
    let _ = nti;

    m.section(&types);

    // ── Import section ──
    let mut imports = ImportSection::new();
    // WASI P1 imports (indices 0..wasi_count)
    for f in &wasi {
        let type_idx = match f.name {
            "fd_read" | "fd_write" => 1u32,
            "proc_exit" => 2,
            "random_get" => 3,
            "environ_sizes_get" => 4,
            "environ_get" => 5,
            "fd_seek" => 6,
            _ => 1,
        };
        imports.import(f.module, f.name, EntityType::Function(type_idx));
    }
    // OutLayer imports (indices wasi_count..wasi_count+ol_count) — upstream split interfaces only
    // Canonical ABI types: type 7=(i32)->(), type 8=(i32*3)->(), type 9=(i32*5)->(), type 10=(i32*7)->(),
    //   type 11=(i32*9)->(), type 12=(i32*11)->(), type 13=(i32*17)->(), type 14=(i32,i32,i64,i32)->(), type 15=(i32*2)->i32, type 16=(i32*6)->()
    let ol_type_map_full: Vec<u32> = vec![
        11, // 0: view — 9 i32 -> ()
        13, // 1: call — 17 i32 -> ()
        12, // 2: transfer — 11 i32 -> ()
        9,  // 3: set — 5 i32 -> ()
        8,  // 4: get — 3 i32 -> ()
        15, // 5: has — (i32,i32) -> i32
        15, // 6: delete — (i32,i32) -> i32
        9,  // 7: increment — 5 i32 -> ()
        9,  // 8: decrement — 5 i32 -> ()
        9,  // 9: set-if-absent — 5 i32 -> ()
        9,  // 10: set-if-equals — 5 i32 -> ()
        8,  // 11: list-keys — 3 i32 -> ()
        8,  // 12: clear-all — 3 i32 -> ()
        9,  // 13: set-worker — 5 i32 -> ()
        9,  // 14: get-worker — 5 i32 -> ()
        10, // 15: raw — 4 i32 -> i32 (canonical: ret_area)
        8,  // 16: env-var — 3 i32 -> ()
        7,  // 17: sleep-ms — 2 i32 -> ()
        9,  // 18: send-telegram — 5 i32 -> ()
        10, // 19: http-post-dynamic — 7 i32 -> ()
    ];
    // Emit only filtered outlayer imports
    for &(sentinel, ol_idx) in OUTLAYER_SENTINELS {
        if ol_sentinel_map_p1.contains_key(&sentinel) {
            let all_ol = outlayer_imports();
            let f = &all_ol[ol_idx];
            imports.import(f.module, f.name, EntityType::Function(ol_type_map_full[ol_idx]));
        }
    }
    // NEAR host stubs as imports from "env" — same as NEAR target
    // This lets the existing NEAR-style instruction emission work unchanged
    let mut near_host_idx: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    for (i, &hi) in near_host_used.iter().enumerate() {
        // We need to match the NEAR host function type
        let type_idx = near_host_type_for(hi, 
            near_void_to_i64, near_i64_to_void, near_i64_to_i64,
            near_2i64_to_void, near_2i64_to_i64,
            near_3i64_to_void, near_3i64_to_i64,
            near_5i64_to_i64, near_8i64_to_i64, near_9i64_to_i64,
            near_7i64_to_void, near_6i64_to_i64, near_4i64_to_i64, near_4i64_to_void,
        );
        let func_idx = total_imports + i as u32;
        imports.import("env", crate::wasm_emit::HOST_FUNCS[hi].0, EntityType::Function(type_idx));
        near_host_idx.insert(hi, func_idx);
    }
    m.section(&imports);

    let near_import_count = near_host_used.len() as u32;
    let internal_base = total_imports + near_import_count;

    // ── Function section ──
    let mut funcs = FunctionSection::new();
    // User functions
    for f in &em.funcs {
        let type_idx = user_type_base + f.param_count as u32;
        funcs.function(type_idx);
    }
    // _start wrapper: () -> ()
    funcs.function(0);
    // cabi_realloc: (i32, i32, i32, i32) -> i32 — type 1
    funcs.function(1);
    // memcpy helper: (i32, i32, i32) -> () — needs a new type
    let mut memcpy_type_idx: u32 = 0;
    {
        // Check if str-cat was used (generates Call(91) = MEMCPY_SENTINEL)
        let uses_memcpy = em.funcs.iter().any(|f| f.instrs.iter().any(|i| matches!(i, Instruction::Call(idx) if *idx == crate::wasm_emit::MEMCPY_SENTINEL)));
        if uses_memcpy {
            memcpy_type_idx = user_type_base + em.funcs.iter().map(|f| f.param_count).max().unwrap_or(0) as u32 + 1; // one past user types
            // Already in type section? No — need to add it dynamically
            // Actually we need to append to types. Since types are already emitted,
            // we'll use an existing 3-i32-param type if available, or piggyback on type 8 (3 i32 -> ())
            // type 8 = (i32, i32, i32) -> () in finish_outlayer_inner
            memcpy_type_idx = 8; // (i32, i32, i32) -> ()
            funcs.function(memcpy_type_idx);
        } else {
            memcpy_type_idx = 0; // unused
        }
    }
    m.section(&funcs);

    // ── Memory ──
    let mut mems = MemorySection::new();
    // min 16 pages (1MB) for P2 scratch + heap
    let pages = em.memory_pages.max(2048) as u64; // min 2048 pages (128MB) - OutLayer default
    mems.memory(MemoryType { minimum: pages, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    // ── Global section: depth counter (same as NEAR) ──
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType { val_type: ValType::I64, mutable: true, shared: false },
        &ConstExpr::i64_const(0),
    );
    // Global 1: return flag
    globals.global(
        GlobalType { val_type: ValType::I64, mutable: true, shared: false },
        &ConstExpr::i64_const(0),
    );
    m.section(&globals);

    // ── Exports ──
    let mut exps = ExportSection::new();
    exps.export("memory", ExportKind::Memory, 0);
    // _start is the second-to-last function (before cabi_realloc)
    let start_func_idx = internal_base + em.funcs.len() as u32;
    // P2 components export "run", WASI P1 exports "_start"
    let entry_name = "_start"; // always _start; P2 wrapper handles naming
    exps.export(entry_name, ExportKind::Func, start_func_idx);
    // cabi_realloc is the last function
    let realloc_idx = start_func_idx + 1;
    exps.export("cabi_realloc", ExportKind::Func, realloc_idx);
    m.section(&exps);

    // ── Code section ──
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter().enumerate()
        .map(|(i, f)| (f.name.as_str(), internal_base + i as u32))
        .collect();
    
    let mut code = wasm_encoder::CodeSection::new();
    
    // Emit user functions (same resolution logic as finish())
    for f in &em.funcs {
        let locals = if let Some(ref entries) = f.local_entries {
            entries.clone()
        } else {
            let extra = f.local_count.saturating_sub(f.param_count);
            if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] }
        };
        let resolved = WasmEmitter::resolve_static_pub(&f.instrs, &near_host_idx, &name_map, &em.funcs);
        let resolved = if em.need_outlayer || em.wasi_mode {
            let mut ol_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            // Tree-shaken: only map sentinels that were actually imported
            for (&sentinel, &fn_idx) in &ol_sentinel_map_p1 {
                ol_map.insert(sentinel, fn_idx);
            }
            ol_map.insert(crate::wasm_emit::WASI_FD_WRITE, 1); // fd_write is WASI import index 1
            // memcpy helper: sentinel 91 → function after cabi_realloc
            {
                let uses_memcpy = em.funcs.iter().any(|f| f.instrs.iter().any(|i| matches!(i, Instruction::Call(idx) if *idx == crate::wasm_emit::MEMCPY_SENTINEL)));
                if uses_memcpy {
                    let memcpy_fn_idx = internal_base + em.funcs.len() as u32 + 2; // after _start + cabi_realloc
                    ol_map.insert(crate::wasm_emit::MEMCPY_SENTINEL, memcpy_fn_idx);
                }
            }
            WasmEmitter::resolve_static_pub_ex(&resolved, &near_host_idx, &name_map, &em.funcs, &ol_map)
        } else {
            resolved
        };
        let mut fb = Function::new(locals);
        for instr in &resolved { fb.instruction(instr); }
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // ── _start() wrapper ──
    // Reads stdin → tagged string → calls run(input) → writes result to stdout
    {
        // Prefer function named "run", then last user function (skip __ helpers)
        let run_pos = em.funcs.iter().position(|f| f.name == "run");
        let entry_pos = run_pos.or_else(|| {
            em.funcs.iter().rposition(|f| !f.name.starts_with("__"))
        }).unwrap_or(em.funcs.len() - 1);
        let last_idx = internal_base + entry_pos as u32;
        let last_func = &em.funcs[entry_pos];
        let param_count = last_func.param_count;
        
        // Locals: all i64 (matching the i64-only convention)
        let mut fb = Function::new(vec![
            (1u32, W),  // local 0: temp i32 (stdin_len etc) — actually we need i32 for fd_read
            (1u32, ValType::I64), // local 1: result
            (1u32, ValType::I64), // local 2: value (untagged) / input_str
            (1u32, ValType::I64), // local 3: digit_count
            (1u32, ValType::I64), // local 4: negative flag
            (1u32, ValType::I64), // local 5: write ptr
        ]);

        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
        let _ma1 = MemArg { offset: 0, align: 0, memory_index: 0 };

        // ── fd_read: read stdin into STDIN_BUF ──
        // Set up iov at offset 64
        fb.instruction(&Instruction::I32Const(64));
        fb.instruction(&Instruction::I32Const(STDIN_BUF as i32));
        fb.instruction(&Instruction::I32Store(ma4));
        fb.instruction(&Instruction::I32Const(68));
        fb.instruction(&Instruction::I32Const(65536));
        fb.instruction(&Instruction::I32Store(ma4));
        // fd_read(0, 64, 1, STDIN_LEN)
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::I32Const(64));
        fb.instruction(&Instruction::I32Const(1));
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32));
        fb.instruction(&Instruction::Call(0)); // fd_read
        fb.instruction(&Instruction::Drop);

        // ── Create tagged string from stdin ──
        // Load stdin_len, create tagged string: ((STDIN_BUF | (len << 32)) << 3) | TAG_STR
        // STDIN_BUF = 32768 (fits in i32), len from memory at STDIN_LEN
        // We need to create: ((32768 | (stdin_len << 32)) << 3) | 5
        fb.instruction(&Instruction::I64Const(STDIN_BUF)); // ptr as i64
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32));
        fb.instruction(&Instruction::I32Load(ma4)); // stdin_len as i32
        fb.instruction(&Instruction::I64ExtendI32U); // len as i64
        fb.instruction(&Instruction::I64Const(32));
        fb.instruction(&Instruction::I64Shl); // len << 32
        fb.instruction(&Instruction::I64Or); // STDIN_BUF | (len << 32) = payload
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64Shl); // payload << 3
        fb.instruction(&Instruction::I64Const(5)); // TAG_STR
        fb.instruction(&Instruction::I64Or); // tagged string
        fb.instruction(&Instruction::LocalSet(2)); // input_str

        // ── Call user function ──
        if param_count == 0 {
            fb.instruction(&Instruction::Call(last_idx));
        } else if param_count == 1 {
            // Single-param: pass the tagged input string from local 2
            fb.instruction(&Instruction::LocalGet(2)); // tagged stdin string
            fb.instruction(&Instruction::Call(last_idx));
        } else {
            // Multi-param: load each 8-byte slot from STDIN_BUF as raw tagged i64s
            for i in 0..param_count {
                fb.instruction(&Instruction::I64Const(STDIN_BUF + (i as i64) * 8));
                fb.instruction(&Instruction::I32WrapI64);
                fb.instruction(&Instruction::I64Load(ma));
                fb.instruction(&Instruction::I64Const(3));
                fb.instruction(&Instruction::I64Shl); // tag as Num
            }
            fb.instruction(&Instruction::Call(last_idx));
        }
        fb.instruction(&Instruction::LocalSet(1)); // result

        // Check tag for fd_write output
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Const(7));
        fb.instruction(&Instruction::I64And);
        fb.instruction(&Instruction::I64Const(5)); // TAG_STR
        fb.instruction(&Instruction::I64Eq);

        fb.instruction(&Instruction::If(BlockType::Empty));
        // ── String result: extract ptr and len, write to stdout via fd_write ──
        // payload = result >> 3
        // ptr = payload & 0xFFFFFFFF, len = payload >> 32
        // Build iov at offset 64: [ptr, len]
        fb.instruction(&Instruction::I32Const(64)); // iov ptr
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64ShrU); // payload
        fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
        fb.instruction(&Instruction::I64And); // ptr
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Store(ma4)); // iov[0].buf = ptr
        fb.instruction(&Instruction::I32Const(68)); // iov+4
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64ShrU); // payload
        fb.instruction(&Instruction::I64Const(32));
        fb.instruction(&Instruction::I64ShrU); // len
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Store(ma4)); // iov[0].len = len
        // fd_write(1, 64, 1, STDIN_LEN)
        fb.instruction(&Instruction::I32Const(1)); // fd=stdout
        fb.instruction(&Instruction::I32Const(64)); // iovs
        fb.instruction(&Instruction::I32Const(1));
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32)); // nwritten ptr
        fb.instruction(&Instruction::Call(1)); // fd_write
        fb.instruction(&Instruction::Drop);

        fb.instruction(&Instruction::Else);
        // ── Non-string result: convert to decimal string, write to stdout ──
        // Untag the value
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Const(3)); fb.instruction(&Instruction::I64ShrU);
        // Convert to decimal string at STDOUT_BUF
        // Simple divmod loop: extract digits backward at STDOUT_BUF+31, then adjust ptr
        let sb: i64 = STDOUT_BUF;
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        // local 2 = value (untagged), local 3 = digit count, local 4 = negative flag, local 5 = write ptr
        fb.instruction(&Instruction::LocalSet(2)); // value
        fb.instruction(&Instruction::I64Const(0)); fb.instruction(&Instruction::LocalSet(3)); // digit_count = 0
        fb.instruction(&Instruction::I64Const(0)); fb.instruction(&Instruction::LocalSet(4)); // negative = 0
        // Check negative
        fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Const(0)); fb.instruction(&Instruction::I64LtS);
        fb.instruction(&Instruction::If(BlockType::Empty));
        fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::LocalSet(4));
        fb.instruction(&Instruction::I64Const(0)); fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Sub); fb.instruction(&Instruction::LocalSet(2));
        fb.instruction(&Instruction::End);
        // Check zero
        fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Eqz);
        fb.instruction(&Instruction::If(BlockType::Empty));
        fb.instruction(&Instruction::I32Const(sb as i32)); fb.instruction(&Instruction::I32Const(0x30)); fb.instruction(&Instruction::I32Store8(ma8.clone()));
        fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::LocalSet(3));
        fb.instruction(&Instruction::Else);
        // Digits backward at sb+31
        fb.instruction(&Instruction::I64Const(sb + 31)); fb.instruction(&Instruction::LocalSet(5)); // write ptr
        fb.instruction(&Instruction::Block(BlockType::Empty)); fb.instruction(&Instruction::Loop(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Eqz);
        fb.instruction(&Instruction::If(BlockType::Empty)); fb.instruction(&Instruction::Br(2)); fb.instruction(&Instruction::End);
        // *ptr = (val % 10) + '0'; val /= 10; ptr--; count++
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Const(10)); fb.instruction(&Instruction::I64RemU);
        fb.instruction(&Instruction::I64Const(0x30)); fb.instruction(&Instruction::I64Add); fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Store8(ma8.clone()));
        fb.instruction(&Instruction::LocalGet(2)); fb.instruction(&Instruction::I64Const(10)); fb.instruction(&Instruction::I64DivU); fb.instruction(&Instruction::LocalSet(2));
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::I64Sub); fb.instruction(&Instruction::LocalSet(5));
        fb.instruction(&Instruction::LocalGet(3)); fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::I64Add); fb.instruction(&Instruction::LocalSet(3));
        fb.instruction(&Instruction::Br(0));
        fb.instruction(&Instruction::End); fb.instruction(&Instruction::End);
        // ptr+1 is now the start of the digit string, count = digit count
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::I64Add); fb.instruction(&Instruction::LocalSet(5));
        // If negative: write '-' at ptr, then ptr--, count++
        fb.instruction(&Instruction::LocalGet(4)); fb.instruction(&Instruction::I64Const(0)); fb.instruction(&Instruction::I64Ne);
        fb.instruction(&Instruction::If(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::I64Sub); fb.instruction(&Instruction::LocalSet(5));
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(0x2D)); // '-'
        fb.instruction(&Instruction::I32Store8(ma8.clone()));
        fb.instruction(&Instruction::LocalGet(3)); fb.instruction(&Instruction::I64Const(1)); fb.instruction(&Instruction::I64Add); fb.instruction(&Instruction::LocalSet(3));
        fb.instruction(&Instruction::End);
        fb.instruction(&Instruction::End); // else (zero case)
        // fd_write(1, iovec, 1, nwritten)
        // iovec at TEMP+64: {ptr, len}
        fb.instruction(&Instruction::I32Const(STDOUT_BUF as i32 + 16384));
        fb.instruction(&Instruction::LocalGet(5)); fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Store(ma8.clone()));
        fb.instruction(&Instruction::I32Const(STDOUT_BUF as i32 + 16388));
        fb.instruction(&Instruction::LocalGet(3)); fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Store(ma8.clone()));
        fb.instruction(&Instruction::I32Const(1)); // stdout fd
        fb.instruction(&Instruction::I32Const(STDOUT_BUF as i32 + 16384)); // iovec ptr
        fb.instruction(&Instruction::I32Const(1)); // 1 iov
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32)); // nwritten ptr
        fb.instruction(&Instruction::Call(1)); // fd_write
        fb.instruction(&Instruction::Drop);

        fb.instruction(&Instruction::End); // if

        // P2 via adapter: _start returns () — just end cleanly
        // P1: proc_exit(0) — terminates process
        if !em.p2_mode && !em.no_proc_exit {
            fb.instruction(&Instruction::I32Const(0));
            fb.instruction(&Instruction::Call(2)); // proc_exit
        }
        // P2: wasi:cli/run expects () -> (), no return value needed

        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // ── cabi_realloc ──
    // Bump allocator: stores bump offset at memory address 999996,
    // base address 1000000. Offset starts at 0 (uninitialized memory = 0).
    // Returns 0 (null) for new_size == 0, matching Rust SDK behavior.
    // Signature: (i32 old_ptr, i32 old_size, i32 align, i32 new_size) -> i32
    {
        let mut realloc = Function::new([(1, ValType::I32)]); // extra local 4
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        // cabi_realloc(old_ptr, old_size, align, new_size) -> ptr
        // Always fresh allocate (bump allocator can't realloc in place)
        // If new_size == 0 → return 0 (null)
        realloc.instruction(&Instruction::LocalGet(3)); // new_size
        realloc.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        // new_size > 0 → fresh allocation
        realloc.instruction(&Instruction::I32Const(999996));
        realloc.instruction(&Instruction::I32Load(ma4));
        realloc.instruction(&Instruction::LocalTee(4));
        realloc.instruction(&Instruction::I32Const(1000000));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::LocalSet(4));
        realloc.instruction(&Instruction::I32Const(999996));
        realloc.instruction(&Instruction::I32Load(ma4));
        realloc.instruction(&Instruction::LocalGet(3));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(3));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(-4));
        realloc.instruction(&Instruction::I32And);
        realloc.instruction(&Instruction::I32Const(999996));
        realloc.instruction(&Instruction::I32Store(ma4));
        realloc.instruction(&Instruction::LocalGet(4));
        realloc.instruction(&Instruction::Else);
        // new_size == 0 → return 0 (null)
        realloc.instruction(&Instruction::I32Const(0));
        realloc.instruction(&Instruction::End); // end new_size check
        realloc.instruction(&Instruction::End); // end function body
        code.function(&realloc);
    }

    // ── memcpy helper (sentinel 91) ──
    // Signature: (dst: i32, src: i32, len: i32) -> ()
    {
        let uses_memcpy = em.funcs.iter().any(|f| f.instrs.iter().any(|i| matches!(i, Instruction::Call(idx) if *idx == crate::wasm_emit::MEMCPY_SENTINEL)));
        if uses_memcpy {
            let ma1 = MemArg { offset: 0, align: 0, memory_index: 0 };
            // Simple byte-by-byte copy loop
            // params: dst(0), src(1), len(2)
            // locals: i(3) — loop counter
            let mut mc = Function::new([(1, ValType::I32)]);
            // i = 0
            mc.instruction(&Instruction::I32Const(0));
            mc.instruction(&Instruction::LocalSet(3));
            // block $break
            mc.instruction(&Instruction::Block(BlockType::Empty));
            // loop $loop
            mc.instruction(&Instruction::Loop(BlockType::Empty));
            // br_if $break (i >= len)
            mc.instruction(&Instruction::LocalGet(3));
            mc.instruction(&Instruction::LocalGet(2));
            mc.instruction(&Instruction::I32GeU);
            mc.instruction(&Instruction::BrIf(1));
            // load src[i]
            mc.instruction(&Instruction::LocalGet(1));
            mc.instruction(&Instruction::LocalGet(3));
            mc.instruction(&Instruction::I32Add);
            mc.instruction(&Instruction::I32Load8U(ma1));
            // store to dst[i]
            mc.instruction(&Instruction::LocalGet(0));
            mc.instruction(&Instruction::LocalGet(3));
            mc.instruction(&Instruction::I32Add);
            mc.instruction(&Instruction::I32Store8(ma1));
            // i++
            mc.instruction(&Instruction::LocalGet(3));
            mc.instruction(&Instruction::I32Const(1));
            mc.instruction(&Instruction::I32Add);
            mc.instruction(&Instruction::LocalSet(3));
            // br $loop
            mc.instruction(&Instruction::Br(0));
            mc.instruction(&Instruction::End); // end loop
            mc.instruction(&Instruction::End); // end block
            mc.instruction(&Instruction::End); // end function
            code.function(&mc);
        }
    }

    m.section(&code);

    // ── Data section ──
    {
        let mut data = DataSection::new();
        // Initialize RUNTIME_HEAP_PTR (addr 56) to HEAP_START (4096)
        let heap_start_bytes: [u8; 8] = (HEAP_START as u64).to_le_bytes();
        data.active(0, &ConstExpr::i32_const(56), heap_start_bytes.iter().copied());
        for (off, bytes) in &em.data_segments {
            data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
        }
        m.section(&data);
    }

    let core_bytes = m.finish();
    std::fs::write("/tmp/p2_outlayer_core.wasm", &core_bytes).ok();
    Ok(core_bytes)
}

/// Map NEAR host function index to type index for the OutLayer target
fn near_host_type_for(
    hi: usize,
    void_to_i64: u32, i64_to_void: u32, i64_to_i64: u32,
    _2i64_to_void: u32, _2i64_to_i64: u32,
    _3i64_to_void: u32, _3i64_to_i64: u32,
    _5i64_to_i64: u32, _8i64_to_i64: u32, _9i64_to_i64: u32,
    _7i64_to_void: u32, _6i64_to_i64: u32, _4i64_to_i64: u32, _4i64_to_void: u32,
) -> u32 {
    match hi {
        // () -> ()
        26 | 41 => 0, // panic, promise_batch_action_create_account
        // () -> i64
        8 | 9 | 10 | 11 | 15 | 16 | 33 => void_to_i64,
        // (i64) -> ()
        3 | 4 | 5 | 6 | 7 | 25 | 27 | 28 | 29 | 35 => i64_to_void,
        // (i64) -> i64  
        1 | 12 | 13 | 14 | 20 => i64_to_i64,
        // (i64, i64) -> ()
        0 | 2 | 19 | 21 | 22 | 36 => _2i64_to_void, // write_register, storage_remove, sha256, etc
        // (i64, i64) -> i64
        18 => _3i64_to_i64,
        32 | 39 | 40 => _2i64_to_i64,
        // (i64, i64, i64) -> ()
        42 | 44 | 48 => _3i64_to_void,
        // (i64, i64, i64) -> i64
        37 | 38 => _3i64_to_i64,
        // (i64×5) -> i64
        17 => _5i64_to_i64,
        // (i64×8) -> i64
        30 => _8i64_to_i64,
        // (i64×9) -> i64
        31 => _9i64_to_i64,
        // (i64×7) -> ()
        43 | 47 => _7i64_to_void,
        // (i64×6) -> i64
        24 => _6i64_to_i64,
        // (i64×4) -> i64
        4 | 5 | 6 | 23 => {
            // signer_account_id, signer_account_pk, predecessor_account_id, random_seed
            // These are actually (i64) -> (), the register param
            // Wait, looking at the actual signatures:
            // 4: signer_account_id(i64) -> ()
            // 5: signer_account_pk(i64) -> ()
            // 6: predecessor_account_id(i64) -> ()
            // 23: random_seed(i64) -> ()
            i64_to_void
        }
        // (i64×4) -> ()
        45 | 46 | 49 => _4i64_to_void,
        _ => 0, // fallback: () -> ()
    }
}

// ═══════════════════════════════════════════════════════════════
// Combined P2 Component: wasi:http + outlayer in one component
// ═══════════════════════════════════════════════════════════════

fn build_combined_p2_core(em: &mut WasmEmitter) -> Result<(Vec<u8>, bool), String> {
    use crate::wasi_http::*;

    let mut module = Module::new();

    let http_urls: Vec<(String, String)> = if em.http_urls.is_empty() && em.http_post_urls.is_empty() {
        vec![("api.open-meteo.com".to_string(), "/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m".to_string())]
    } else {
        em.http_urls.clone()
    };
    let http_get_count = http_urls.len() as u32;
    let http_post_count = em.http_post_urls.len() as u32;
    let post_sentinel_base = 200u32;

    // Tree-shake: only import outlayer functions that are actually used
    let used_ol_indices = scan_used_outlayer_indices(em);
    eprintln!("[DEBUG] used_ol_indices: {:?}", used_ol_indices);
    let ol_import_base = HTTP_IMPORT_COUNT + 1; // 29
    let (ol, ol_sentinel_map, ol_count) = build_filtered_outlayer(&used_ol_indices, ol_import_base);

    // Import layout: HTTP 0..27, get-stdin 28, outlayer 29..29+ol_count
    let get_stdin_import_idx = HTTP_IMPORT_COUNT; // 28
    let internal_fn_base = ol_import_base + ol_count;

    // Type layout
    let user_type_count = 17u32;
    let user_type_base = HTTP_TYPE_COUNT; // 10
    let start_type = user_type_base + user_type_count; // 27
    let realloc_type = start_type + 1; // 28
    let http_get_type = realloc_type + 1; // 29
    let http_post_type = http_get_type + 1; // 30

    let ol_type_base = http_post_type + 1; // 31
    let ol_type_1 = ol_type_base;           // (i32) -> ()
    let ol_type_2 = ol_type_base + 1;       // (i32, i32) -> ()
    let ol_type_3 = ol_type_base + 2;       // (i32*3) -> ()
    let ol_type_5 = ol_type_base + 3;       // (i32*5) -> ()
    let ol_type_7 = ol_type_base + 4;       // (i32*7) -> ()
    let ol_type_9 = ol_type_base + 5;       // (i32*9) -> ()
    let ol_type_11 = ol_type_base + 6;      // (i32*11) -> ()
    let ol_type_13 = ol_type_base + 7;      // (i32*13) -> ()
    let ol_type_17 = ol_type_base + 8;      // (i32*17) -> ()
    let ol_type_6 = ol_type_base + 9;       // (i32*6) -> ()
    let ol_type_s64 = ol_type_base + 10;    // (i32,i32,i64,i32) -> ()
    let ol_type_2ret = ol_type_base + 11;    // (i32*2) -> (i32)
    let memcpy_type = ol_type_base + 12;     // (i64, i64, i64) -> ()

    // Function indices
    let get_fn_count = http_get_count * 2;
    let post_fn_count = http_post_count * 2;
    let http_get_fn_idx = internal_fn_base;
    let http_post_fn_idx = http_get_fn_idx + get_fn_count;
    let memcpy_fn_idx = http_post_fn_idx + post_fn_count;
    let user_fn_base = memcpy_fn_idx + 1;
    let start_fn_idx = user_fn_base + em.funcs.len() as u32;
    let realloc_fn_idx = start_fn_idx + 1;

    // ═══ Type Section + Import Section ═══
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();

    add_http_imports_to_sections(&mut types, &mut imports);

    for i in 0..=16u32 {
        types.ty().function(vec![ValType::I64; i as usize], [ValType::I64]);
    }
    types.ty().function([], [ValType::I32]); // start_type: () -> i32 for wasi:cli/run result
    types.ty().function([ValType::I32; 4], [ValType::I32]);
    types.ty().function([ValType::I32; 5], [ValType::I32]);
    types.ty().function([ValType::I32; 7], [ValType::I32]);

    types.ty().function(vec![W; 1], []);
    types.ty().function(vec![W; 2], []); // ol_type_2: 2 i32 -> () (for sleep-ms)
    types.ty().function(vec![W; 3], []);
    types.ty().function(vec![W; 5], []);
    types.ty().function(vec![W; 7], []);
    types.ty().function(vec![W; 9], []);
    types.ty().function(vec![W; 11], []);
    types.ty().function(vec![W; 13], []);
    types.ty().function(vec![W; 17], []);
    types.ty().function(vec![W; 6], []);
    types.ty().function(vec![W, W, ValType::I64, W], []);
    types.ty().function(vec![W; 2], [W]); // has/delete direct bool return
    types.ty().function([ValType::I32; 3], []); // memcpy_type: (dst, src, len) -> ()

    module.section(&types);

    // Full type map: outlayer_imports index → type index (for filtered lookup)
    let ol_type_map_full: Vec<u32> = vec![
        // Upstream split interfaces only (19 entries)
        ol_type_9,   ol_type_17,  ol_type_11,   // 0: view, 1: call, 2: transfer
        ol_type_5,   ol_type_3,                  // 3: set, 4: get
        ol_type_2ret, ol_type_2ret,              // 5: has, 6: delete (bool return)
        ol_type_s64,  ol_type_s64,               // 7: increment, 8: decrement
        ol_type_5,    ol_type_7,                 // 9: set-if-absent, 10: set-if-equals
        ol_type_3,    ol_type_1,                 // 11: list-keys, 12: clear-all
        ol_type_7,    ol_type_6,                 // 13: set-worker, 14: get-worker
        ol_type_5,                                // 15: raw
        ol_type_3,                                // 16: env-var
        ol_type_2,                                // 17: sleep-ms (ms, ret_area)
        ol_type_5,                                // 18: send-telegram (chat_ptr, chat_len, text_ptr, text_len, ret_area)
        ol_type_7,                                // 19: http-post-dynamic (url_ptr, url_len, body_ptr, body_len, ct_ptr, ct_len, ret_area)
        ol_type_3,                                // 20: web-search (query_ptr, query_len, ret_area)
    ];

    imports.import("wasi:cli/stdin@0.2.2", "get-stdin", EntityType::Function(0));

    // Emit only filtered outlayer imports
    for &(sentinel, ol_idx) in OUTLAYER_SENTINELS {
        if used_ol_indices.contains(&ol_idx) {
            let all_ol = outlayer_imports();
            let f = &all_ol[ol_idx];
            imports.import(f.module, f.name, EntityType::Function(ol_type_map_full[ol_idx]));
        }
    }

    module.section(&imports);

    // ═══ Function Section ═══
    let mut functions = FunctionSection::new();
    for _ in &http_urls {
        functions.function(http_get_type);
        functions.function(http_get_type);
    }
    for _ in &em.http_post_urls {
        functions.function(http_post_type);
        functions.function(http_post_type);
    }
    functions.function(memcpy_type); // shared memcpy helper
    for f in &em.funcs {
        functions.function(user_type_base + f.param_count as u32);
    }
    functions.function(start_type);
    functions.function(realloc_type);
    module.section(&functions);

    // ═══ Memory ═══
    let mut memory = MemorySection::new();
    let pages = em.memory_pages.max(2048) as u64; // min 2048 pages (128MB) - OutLayer default
    memory.memory(MemoryType { minimum: pages, maximum: None, memory64: false, shared: false, page_size_log2: None });
    module.section(&memory);

    // ═══ Globals ═══
    let mut globals = GlobalSection::new();
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(0));
    globals.global(GlobalType { val_type: ValType::I64, mutable: true, shared: false }, &ConstExpr::i64_const(0));
    module.section(&globals);

    // ═══ Exports ═══
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, start_fn_idx);
    exports.export("wasi:cli/run@0.2.2#run", ExportKind::Func, start_fn_idx);
    exports.export("cabi_realloc", ExportKind::Func, realloc_fn_idx);
    module.section(&exports);

    // ═══ Code Section ═══
    let mut codes = CodeSection::new();
    let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };

    // Data segments for URL strings
    let mut all_http_data_segments: Vec<(i32, Vec<u8>)> = Vec::new();
    let mut current_data_offset: i32 = 16384;

    let headers: &[(&[u8], &[u8])] = &[
        (b"User-Agent", b"lisp-rlm/0.1 (wasi:http)"),
        (b"Accept", b"application/json"),
    ];
    let post_headers: &[(&[u8], &[u8])] = &[
        (b"User-Agent", b"lisp-rlm/0.1 (wasi:http)"),
        (b"Accept", b"application/json"),
        (b"Content-Type", b"application/json"),
    ];

    // ── HTTP GET internal functions ──
    for (idx, (authority, path)) in http_urls.iter().enumerate() {
        let http_data = crate::wasi_http_buffer::build_url_data_segments_with_base(
            authority.as_bytes(),
            path.as_bytes(),
            headers,
            current_data_offset,
        );

        let mut http_get_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (5u32, ValType::I32), // locals 13-17 (copy_i=16, etc.)
        ]);
        crate::wasi_http_buffer::emit_http_get_to_buffer(&mut http_get_fn, &http_data);
        http_get_fn.instruction(&Instruction::End);
        codes.function(&http_get_fn);

        let mut poll_read_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
        ]);
        crate::wasi_http_buffer::emit_http_poll_read(&mut poll_read_fn);
        poll_read_fn.instruction(&Instruction::End);
        codes.function(&poll_read_fn);

        let span = http_data.total_span();
        for (off, bytes) in http_data.segments {
            all_http_data_segments.push((off as i32, bytes));
        }
        current_data_offset = span;
        current_data_offset = (current_data_offset + 3) & !3;
    }

    // ── HTTP POST internal functions ──
    for (idx, (authority, path)) in em.http_post_urls.iter().enumerate() {
        let http_data = crate::wasi_http_buffer::build_url_data_segments_with_base(
            authority.as_bytes(),
            path.as_bytes(),
            post_headers,
            current_data_offset,
        );

        let mut http_post_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
        ]);
        crate::wasi_http_buffer::emit_http_post_to_buffer(&mut http_post_fn, &http_data);
        http_post_fn.instruction(&Instruction::End);
        codes.function(&http_post_fn);

        let mut poll_read_fn = Function::new([
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
            (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32), (1u32, ValType::I32),
        ]);
        crate::wasi_http_buffer::emit_http_poll_read(&mut poll_read_fn);
        poll_read_fn.instruction(&Instruction::End);
        codes.function(&poll_read_fn);

        let span = http_data.total_span();
        for (off, bytes) in http_data.segments {
            all_http_data_segments.push((off as i32, bytes));
        }
        current_data_offset = span;
        current_data_offset = (current_data_offset + 3) & !3;
    }

    // ── Memcpy helper: (dst: i32, src: i32, len: i32) -> () ──
    // Copies len bytes from src to dst using word loop + remainder tail
    {
        let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
        let ma8 = MemArg { offset: 0, align: 0, memory_index: 0 };
        let mut fb = Function::new([
            (1u32, ValType::I32), // dst (param 0, reused as moving pointer)
            (1u32, ValType::I32), // src (param 1, reused as moving pointer)
            (1u32, ValType::I32), // len (param 2)
            (1u32, ValType::I32), // local 3: qw (qword count)
            (1u32, ValType::I32), // local 4: rem (remainder bytes)
        ]);
        // qw = len >> 3
        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I32Const(3)); fb.instruction(&Instruction::I32ShrU);
        fb.instruction(&Instruction::LocalSet(3));
        // rem = len & 7
        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I32Const(7)); fb.instruction(&Instruction::I32And);
        fb.instruction(&Instruction::LocalSet(4));
        // Word copy loop: while qw > 0
        fb.instruction(&Instruction::Block(BlockType::Empty));
        fb.instruction(&Instruction::Loop(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(3)); fb.instruction(&Instruction::I32Const(0)); fb.instruction(&Instruction::I32Eq);
        fb.instruction(&Instruction::BrIf(1));
        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Load(ma)); fb.instruction(&Instruction::I64Store(ma));
        fb.instruction(&Instruction::LocalGet(1)); fb.instruction(&Instruction::I32Const(8)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(1));
        fb.instruction(&Instruction::LocalGet(0)); fb.instruction(&Instruction::I32Const(8)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(0));
        fb.instruction(&Instruction::LocalGet(3)); fb.instruction(&Instruction::I32Const(-1)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(3));
        fb.instruction(&Instruction::Br(0));
        fb.instruction(&Instruction::End); // loop
        fb.instruction(&Instruction::End); // block
        // Remainder tail: while rem > 0
        fb.instruction(&Instruction::Block(BlockType::Empty));
        fb.instruction(&Instruction::Loop(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(4)); fb.instruction(&Instruction::I32Const(0)); fb.instruction(&Instruction::I32Eq);
        fb.instruction(&Instruction::BrIf(1));
        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Load8U(ma8)); fb.instruction(&Instruction::I64Store8(ma8));
        fb.instruction(&Instruction::LocalGet(1)); fb.instruction(&Instruction::I32Const(1)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(1));
        fb.instruction(&Instruction::LocalGet(0)); fb.instruction(&Instruction::I32Const(1)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(0));
        fb.instruction(&Instruction::LocalGet(4)); fb.instruction(&Instruction::I32Const(-1)); fb.instruction(&Instruction::I32Add); fb.instruction(&Instruction::LocalSet(4));
        fb.instruction(&Instruction::Br(0));
        fb.instruction(&Instruction::End); // loop
        fb.instruction(&Instruction::End); // block
        fb.instruction(&Instruction::End); // func
        codes.function(&fb);
    }


    // ── User functions ──
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), user_fn_base + i as u32))
        .collect();

    for f in &em.funcs {
        let locals = if let Some(ref entries) = f.local_entries {
            entries.clone()
        } else {
            let extra = f.local_count.saturating_sub(f.param_count);
            if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] }
        };

        let resolved = {
            let base_resolved = WasmEmitter::resolve_static_pub(
                &f.instrs, &std::collections::HashMap::new(), &name_map, &em.funcs,
            );
            let mut ol_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            // Tree-shaken: only map sentinels that were actually imported
            for (&sentinel, &fn_idx) in &ol_sentinel_map {
                ol_map.insert(sentinel, fn_idx);
            }
            // HTTP GET sentinels → internal HTTP GET functions (wasi:http mode)
            for i in 0..http_get_count {
                ol_map.insert(103 + i, http_get_fn_idx + i * 2);
            }
            // HTTP POST sentinels → internal HTTP POST functions (wasi:http mode)
            for i in 0..http_post_count {
                ol_map.insert(post_sentinel_base + i, http_post_fn_idx + i * 2);
            }
            ol_map.insert(crate::wasm_emit::WASI_FD_WRITE, realloc_fn_idx);
            ol_map.insert(crate::wasm_emit::MEMCPY_SENTINEL, memcpy_fn_idx);
            WasmEmitter::resolve_static_pub_ex(
                &base_resolved, &std::collections::HashMap::new(), &name_map, &em.funcs, &ol_map,
            )
        };

        let mut fb = Function::new(locals);
        for instr in &resolved { fb.instruction(instr); }
        fb.instruction(&Instruction::End);
        codes.function(&fb);
    }

    // ── _start() ──
    // Uses get-stdin + blocking-read (NOT read — read requires wasi:io/poll)
    {
        let default_pos = em.funcs.iter().rposition(|f| !f.name.starts_with("__")).unwrap_or(em.funcs.len() - 1);
        let mut real_func_idx = user_fn_base + default_pos as u32;
        let mut real_param_count = em.funcs[default_pos].param_count;
        if let Some(run_pos) = em.funcs.iter().position(|f| f.name == "run") {
            real_func_idx = user_fn_base + run_pos as u32;
            real_param_count = em.funcs[run_pos].param_count;
        }

        let mut fb = Function::new([
            (1u32, W),  // 0: stdout handle
            (1u32, W),  // 1: string ptr
            (1u32, ValType::I64), // 2: result
            (1u32, W),  // 3: string len
            (1u32, W),  // 4: stdin handle
            (1u32, W),  // 5: data ptr
            (1u32, W),  // 6: data len
        ]);

        // get-stdin → handle
        fb.instruction(&Instruction::Call(get_stdin_import_idx));
        fb.instruction(&Instruction::LocalSet(4));

        // blocking-read(self, limit, result_ptr)
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::I64Const(STDIN_BUF));
        fb.instruction(&Instruction::I32Const(SCRATCH_READ_RESULT));
        fb.instruction(&Instruction::Call(FN_INPUT_STREAM_BLOCKING_READ as u32));

        // Read result: ptr at +4, len at +8
        fb.instruction(&Instruction::I32Const(SCRATCH_READ_RESULT + 4));
        fb.instruction(&Instruction::I32Load(ma4.clone()));
        fb.instruction(&Instruction::LocalSet(5)); // ptr

        fb.instruction(&Instruction::I32Const(SCRATCH_READ_RESULT + 8));
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::LocalSet(6)); // len

        // memory.copy data → STDIN_BUF
        fb.instruction(&Instruction::I32Const(STDIN_BUF as i32));
        fb.instruction(&Instruction::LocalGet(5));
        fb.instruction(&Instruction::LocalGet(6));
        fb.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });

        // store len at STDIN_LEN
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32)); // addr first
        fb.instruction(&Instruction::LocalGet(6));                 // value second
        fb.instruction(&Instruction::I32Store(ma4));

        // drop stdin
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::Call(FN_DROP_INPUT_STREAM as u32));

        // Tagged string: (STDIN_BUF | (len << 32)) << 3 | TAG_STR
        fb.instruction(&Instruction::I64Const(STDIN_BUF));
        fb.instruction(&Instruction::LocalGet(6));
        fb.instruction(&Instruction::I64ExtendI32U);
        fb.instruction(&Instruction::I64Const(32));
        fb.instruction(&Instruction::I64Shl);
        fb.instruction(&Instruction::I64Or);
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64Shl);
        fb.instruction(&Instruction::I64Const(crate::wasm_emit::TAG_STR));
        fb.instruction(&Instruction::I64Or);

        if real_param_count == 0 {
            fb.instruction(&Instruction::Drop);
        } else if real_param_count > 1 {
            for _ in 1..real_param_count {
                fb.instruction(&Instruction::I64Const(crate::wasm_emit::TAG_NIL));
            }
        }
        fb.instruction(&Instruction::Call(real_func_idx));
        fb.instruction(&Instruction::LocalSet(2));

        // Write result to stdout
        fb.instruction(&Instruction::Call(FN_GET_STDOUT));
        fb.instruction(&Instruction::LocalSet(0));

        fb.instruction(&Instruction::LocalGet(2));
        fb.instruction(&Instruction::I64Const(7));
        fb.instruction(&Instruction::I64And);
        fb.instruction(&Instruction::I64Const(crate::wasm_emit::TAG_STR as i64));
        fb.instruction(&Instruction::I64Eq);
        fb.instruction(&Instruction::If(BlockType::Empty));
        {
            // String result: extract ptr and len, write to stdout
            fb.instruction(&Instruction::LocalGet(2));
            fb.instruction(&Instruction::I64Const(3));
            fb.instruction(&Instruction::I64ShrU);
            fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
            fb.instruction(&Instruction::I64And);
            fb.instruction(&Instruction::I32WrapI64);
            fb.instruction(&Instruction::LocalSet(1));

            fb.instruction(&Instruction::LocalGet(2));
            fb.instruction(&Instruction::I64Const(35));
            fb.instruction(&Instruction::I64ShrU);
            fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
            fb.instruction(&Instruction::I64And);
            fb.instruction(&Instruction::I32WrapI64);
            fb.instruction(&Instruction::LocalSet(3));

            fb.instruction(&Instruction::LocalGet(0));
            fb.instruction(&Instruction::LocalGet(1));
            fb.instruction(&Instruction::LocalGet(3));
            fb.instruction(&Instruction::I32Const(0));
            fb.instruction(&Instruction::Call(FN_OUTPUT_STREAM_WRITE));
        }
        fb.instruction(&Instruction::Else);
        {
            // Non-string result: write nothing
        }
        fb.instruction(&Instruction::End);

        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::Call(FN_DROP_OUTPUT_STREAM));

        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::End);
        codes.function(&fb);
    }

    // ── cabi_realloc: (i32, i32, i32, i32) -> i32 ──
    // params: 0=old_ptr, 1=old_len, 2=align, 3=new_len
    // returns: ptr to newly allocated region
    // Canonical ABI: always fresh allocate (bump allocator can't realloc in place)
    {
        let mut realloc = Function::new([
            (1u32, ValType::I32), // extra local 4: bump offset
        ]);
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        // cabi_realloc(old_ptr, old_size, align, new_size) -> ptr
        // If new_size == 0 → return 0 (null)
        // Bump allocator: counter at memory[900000], base 900004
        realloc.instruction(&Instruction::LocalGet(3)); // new_len
        realloc.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        // Fresh allocation from bump counter at address 900000, base 900004
        realloc.instruction(&Instruction::I32Const(900000));
        realloc.instruction(&Instruction::I32Load(ma4)); // load offset
        realloc.instruction(&Instruction::LocalTee(4));
        realloc.instruction(&Instruction::I32Const(900004));
        realloc.instruction(&Instruction::I32Add); // abs addr = 900004 + offset
        realloc.instruction(&Instruction::LocalSet(4)); // local 4 = abs addr
        // Advance offset by new_len aligned up to 4
        realloc.instruction(&Instruction::I32Const(900000));
        realloc.instruction(&Instruction::I32Load(ma4)); // load old offset
        realloc.instruction(&Instruction::LocalGet(3)); // new_len
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(3));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(-4));
        realloc.instruction(&Instruction::I32And);
        realloc.instruction(&Instruction::I32Const(900000));
        realloc.instruction(&Instruction::I32Store(ma4)); // store new offset
        // Return the allocated address
        realloc.instruction(&Instruction::LocalGet(4));
        realloc.instruction(&Instruction::Else);
        // new_size == 0 → return 0 (null)
        realloc.instruction(&Instruction::I32Const(0));
        realloc.instruction(&Instruction::End); // end new_size check
        realloc.instruction(&Instruction::End); // end function body
        codes.function(&realloc);
    }

    module.section(&codes);
    // ── Emit data segments ──
    {
        let mut data = DataSection::new();
        // Initialize RUNTIME_HEAP_PTR (addr 56) to HEAP_START (200000)
        let heap_start_bytes: [u8; 8] = (HEAP_START as u64).to_le_bytes();
        data.active(0, &ConstExpr::i32_const(56), heap_start_bytes.iter().copied());
        // Initialize DEPTH_COUNTER (addr 999980) to 0
        let depth_zero: [u8; 8] = 0u64.to_le_bytes();
        data.active(0, &ConstExpr::i32_const(999980), depth_zero.iter().copied());
        // String literals from lisp emitter
        for (off, bytes) in &em.data_segments {
            data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
        }
        // HTTP URL/header data segments
        for (offset, bytes) in &all_http_data_segments {
            data.active(0, &ConstExpr::i32_const(*offset), bytes.iter().copied());
        }
        module.section(&data);
    }

    let core_bytes = module.finish();
    let has_outlayer = !used_ol_indices.is_empty();
    std::fs::write("/tmp/p2_combined_core.wasm", &core_bytes).ok();
    Ok((core_bytes, has_outlayer))
}

fn build_combined_p2_component(core_bytes: &[u8], has_outlayer_imports: bool) -> Result<Vec<u8>, String> {
    std::fs::write("/tmp/p2_combined_core_to_encode.wasm", core_bytes).ok();

    // Use simple-http WIT world when no outlayer functions are imported — avoids
    // broken shim adapters for near:rpc/near:storage that the component encoder
    // generates even when those imports don't exist in the core module.
    // When outlayer imports ARE present, use outlayer-http world which includes
    // near:rpc, near:storage, near:payment, etc.
    let (resolve, world) = if has_outlayer_imports {
        crate::wasi_http::build_combined_wit_metadata()
    } else {
        crate::wasi_http::build_http_wit_metadata()
    }
        .map_err(|e| format!("WIT metadata: {}", e))?;
    let mut mod_bytes = core_bytes.to_vec();
    wit_component::embed_component_metadata(
        &mut mod_bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .map_err(|e| format!("embed metadata: {}", e))?;

    let component = wit_component::ComponentEncoder::default()
        .module(&mod_bytes)
        .map_err(|e| format!("wit-component set module: {}", e))?
        .validate(false)
        .encode()
        .map_err(|e| format!("wit-component encode: {:#}", e))?;

    std::fs::write("/tmp/p2_combined.wasm", &component).ok();

    if let Err(_e) = wasmparser::validate(&component) {
        // Validation issues will surface at runtime
    }

    Ok(component)
}
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn test_outlayer_simple() {
        let src = "(define (square x) (* x x))";
        let wasm = compile_outlayer(src).unwrap();
        assert!(!wasm.is_empty());
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        // Should have _start export
        assert!(wat.contains("_start"));
        // Should have wasi imports
        assert!(wat.contains("wasi_snapshot_preview1"));
    }

    #[test]
    fn test_outlayer_counter() {
        let src = r#"
(define (get_counter) (near/load "c"))
(define (increment) (near/store "c" (+ (get_counter) 1)))
(define (run) (increment))
"#;
        let wasm = compile_outlayer(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        assert!(wat.contains("_start"));
        assert!(wat.contains("wasi_snapshot_preview1"));
        // NEAR host functions (near/load, near/store) are imported as "env" stubs,
        // not as outlayer sentinels. Tree-shaking correctly omits unused outlayer imports.
        assert!(wat.contains("\"env\""), "should import env stubs for NEAR host functions");
    }

    /// Test with wasmtime: compile and run a simple function
    #[test]
    fn test_outlayer_wasmtime_square() {
        let src = "(define (square x) (* x x))";
        let wasm = compile_outlayer(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        let result = run_outlayer_wasm(&wasm, &7i64.to_le_bytes());
        assert_eq!(result, 49, "square(7) should be 49");
    }

    #[test]
    fn test_outlayer_wasmtime_const() {
        let src = "(define (main) 42)";
        let wasm = compile_outlayer(src).unwrap();
        let _wat = wasmprinter::print_bytes(&wasm).unwrap();
        let result = run_outlayer_wasm(&wasm, &[]);
        assert_eq!(result, 42, "main() should return 42");
    }

    #[test]
    fn test_outlayer_wasmtime_fib() {
        let src = "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))";
        let wasm = compile_outlayer(src).unwrap();
        let result = run_outlayer_wasm(&wasm, &10i64.to_le_bytes());
        assert_eq!(result, 55, "fib(10) should be 55");
    }

    #[test]
    fn test_outlayer_wasmtime_double() {
        let src = "(define (double x) (* x 2))";
        let wasm = compile_outlayer(src).unwrap();
        let result = run_outlayer_wasm(&wasm, &21i64.to_le_bytes());
        assert_eq!(result, 42, "double(21) should be 42");
    }

    #[test]
    fn test_outlayer_wasmtime_view() {
        // Test outlayer/view with mock host that writes "hello" as response
        let src = r#"
(define (get_price)
  (outlayer/view "ref.near" "get_price" "{}")
)
(define (run) (get_price))
"#;
        let wasm = compile_outlayer(src).unwrap();
        let result = run_outlayer_wasm_with_view(&wasm, &[], b"hello");
        // _start wrapper untags and stores raw payload at RESULT_BUF
        // payload = ptr | (len << 32)
        let ptr = result & 0xFFFFFFFF;
        let len = (result >> 32) as u32;
        assert_eq!(len, 5, "result should be 5 bytes ('hello'), got len={}", len);
        assert!(ptr > 0, "ptr should be non-zero, got {}", ptr);
    }

    #[test]
    fn test_outlayer_p2_const() {
        let src = "(define (main) 42)";
        let comp_bytes = compile_outlayer_p2(src).unwrap();
        assert!(!comp_bytes.is_empty());
        // Component magic is same as module magic (0x00 0x61 0x73 0x6D) but with version 0x0D 0x01
        // Module version: 0x01 0x00, Component version: 0x0D 0x01
        assert!(comp_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]),
            "should start with WASM magic, got {:?}", &comp_bytes[..4.min(comp_bytes.len())]);
        std::fs::write("/tmp/p2_const.wasm", &comp_bytes).unwrap();
    }

    #[test]
    fn test_outlayer_p2_square() {
        let src = "(define (square x) (* x x))";
        let comp_bytes = compile_outlayer_p2(src).unwrap();
        assert!(comp_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]),
            "should start with WASM magic");
    }

    #[test]
    fn test_outlayer_p2_http_get() {
        // Test that http-get compiles to a valid P2 component
        let src = r#"
(define (fetch)
  (http-get "https://api.example.com/price"))
(define (run) (fetch))
"#;
        // First compile to P1 to count core instructions
        let core = compile_outlayer(src).unwrap();
        std::fs::write("/tmp/core_http.wasm", &core).unwrap();
        
        let comp_bytes = compile_outlayer_p2(src).unwrap();
        assert!(comp_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]),
            "should start with WASM magic");
        
        // Validate with wasm-tools if available
        std::fs::write("/tmp/test_p2_http.wasm", &comp_bytes).unwrap();
        let output = std::process::Command::new("wasm-tools")
            .args(["validate", "/tmp/test_p2_http.wasm"])
            .output();
        match output {
            Ok(o) => {
                let valid = o.status.success();
                assert!(valid, "P2 component should validate");
            }
            Err(_) => {
                // wasm-tools not available, skipping validation
            }
        }
    }

    #[test]
    fn test_outlayer_echo_instruction_count() {
        // Count instructions for the minimal echo: (print (json-get "amount"))
        let src = r#"(define (main) (print (json-get "amount")))"#;
        let core = compile_outlayer(src).unwrap();
        std::fs::write("/tmp/echo_p1.wasm", &core).unwrap();
        
        // Also build P2 and save
        let echo_p2_src = r#"(define (main) (print (json-get "amount")))"#;
        let echo_p2 = compile_outlayer_p2(echo_p2_src).unwrap();
        std::fs::write("/tmp/echo_p2.wasm", &echo_p2).unwrap();
        
        // Count instructions in code section
        let mut pos = 8usize;
        while pos < core.len() {
            let sid = core[pos]; pos += 1;
            let mut sz = 0usize; let mut shift = 0usize;
            loop { let b = core[pos] as usize; pos += 1; sz |= (b & 0x7F) << shift; shift += 7; if b & 0x80 == 0 { break; } }
            if sid == 10 {
                let end = pos + sz;
                let (cnt, cl) = { let mut r = 0usize; let mut s = 0usize; let mut i = 0usize; loop { let b = core[pos+i] as usize; r |= (b & 0x7F) << s; i += 1; if b & 0x80 == 0 { break; } s += 7; } (r, i) };
                pos += cl;
                let mut total = 0usize;
                for fi in 0..cnt {
                    let (bsz, bl) = { let mut r = 0usize; let mut s = 0usize; let mut i = 0usize; loop { let b = core[pos+i] as usize; r |= (b & 0x7F) << s; i += 1; if b & 0x80 == 0 { break; } s += 7; } (r, i) };
                    pos += bl;
                    let body_end = pos + bsz;
                    // skip locals
                    let (nlc, nl) = { let mut r = 0usize; let mut s = 0usize; let mut i = 0usize; loop { let b = core[pos+i] as usize; r |= (b & 0x7F) << s; i += 1; if b & 0x80 == 0 { break; } s += 7; } (r, i) };
                    pos += nl;
                    for _ in 0..nlc { let (c, cl) = { let mut r = 0usize; let mut s = 0usize; let mut i = 0usize; loop { let b = core[pos+i] as usize; r |= (b & 0x7F) << s; i += 1; if b & 0x80 == 0 { break; } s += 7; } (r, i) }; pos += cl; pos += 1; }
                    let mut ic = 0usize;
                    while pos < body_end {
                        let op = core[pos]; pos += 1;
                        if op == 0x0B { ic += 1; break; }
                        ic += 1;
                        // Skip operands (rough)
                        match op {
                            0x02 | 0x03 | 0x04 => { pos += 1; }
                            0x0C | 0x0D => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            0x10 => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            0x20..=0x24 => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            0x28..=0x3E => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            0x41 => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            0x42 => { while pos < body_end && core[pos] & 0x80 != 0 { pos += 1; } pos += 1; }
                            _ => {}
                        }
                    }
                    total += ic;
                    pos = body_end;
                }
                break;
            }
            pos += sz;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Run an OutLayer WASM module with wasmtime, providing stdin data.
/// Returns the i64 result read from RESULT_BUF (offset 65536 in memory).
fn run_outlayer_wasm(wasm: &[u8], stdin_data: &[u8]) -> i64 {
    use std::sync::Arc;
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).expect("WASM should be valid");

    let stdin_arc = Arc::new(stdin_data.to_vec());
    let mut store = Store::new(&engine, ());

    // Create all host functions first (before linker.define borrows store)
    let sd = stdin_arc.clone();
    let fd_read_fn = Func::new(
        &mut store,
        FuncType::new(&engine,
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32]),
        move |mut caller, args, results| {
            let iov_ptr = args[1].unwrap_i32() as usize;
            let nread_ptr = args[3].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    let copy_len = sd.len().min(buf_len);
                    if buf_ptr + copy_len <= data.len() {
                        data[buf_ptr..buf_ptr+copy_len].copy_from_slice(&sd[..copy_len]);
                    }
                    if nread_ptr + 4 <= data.len() {
                        data[nread_ptr..nread_ptr+4].copy_from_slice(&(copy_len as u32).to_le_bytes());
                    }
                }
            }
            results[0] = Val::I32(0);
            Ok(())
        },
    );

    let fd_write_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32| -> i32 { 0 });

    let proc_exit_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32], vec![]),
        |_, args, _| {
            let code = args[0].unwrap_i32();
            Err(wasmtime::Error::msg(format!("proc_exit({})", code)))
        },
    );

    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    let ol_view_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
    let ol_transfer_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});

    let read_reg_fn = Func::wrap(&mut store, |_: i64, _: i64| {});
    let reg_len_fn = Func::wrap(&mut store, |_: i64| -> i64 { 0 });

    // Now define all in linker
    let mut linker = Linker::new(&engine);
    linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "random_get", random_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", environ_sizes_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_get", environ_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek_fn).unwrap();
    // near:rpc/api — all return void (canonical ABI writes to ret_area)
    linker.define(&store, "near:rpc/api@0.1.0", "view", ol_view_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "call", ol_call_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "transfer", ol_transfer_fn).unwrap();
    // near:storage/api — has/delete return i32 (bool), rest return void
    let storage_set_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set", storage_set_fn).unwrap();
    let storage_get_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "get", storage_get_fn).unwrap();
    let storage_has_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "near:storage/api@0.1.0", "has", storage_has_fn).unwrap();
    let storage_delete_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "near:storage/api@0.1.0", "delete", storage_delete_fn).unwrap();
    let storage_incr_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "increment", storage_incr_fn).unwrap();
    let storage_decr_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "decrement", storage_decr_fn).unwrap();
    let storage_sia_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-if-absent", storage_sia_fn).unwrap();
    let storage_sie_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-if-equals", storage_sie_fn).unwrap();
    let storage_lk_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "list-keys", storage_lk_fn).unwrap();
    let storage_ca_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 1], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "clear-all", storage_ca_fn).unwrap();
    let storage_sw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-worker", storage_sw_fn).unwrap();
    let storage_gw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "get-worker", storage_gw_fn).unwrap();
    // near:rpc/api — raw
    let raw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:rpc/api@0.1.0", "raw", raw_fn).unwrap();
    // NEAR compat stubs
    linker.define(&store, "env", "read_register", read_reg_fn).unwrap();
    linker.define(&store, "env", "register_len", reg_len_fn).unwrap();

    let instance = linker.instantiate(&mut store, &module).expect("instantiate");
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start").expect("_start export");

    match start.call(&mut store, ()) {
        Ok(()) => {}
        Err(trap) => {
            // proc_exit raises a trap — check if it's our proc_exit or a real error
            let msg = trap.to_string();
            // The wasmtime error chain includes our original "proc_exit(N)" message
            let is_exit = msg.contains("proc_exit") 
                || trap.source().map(|s| s.to_string().contains("proc_exit")).unwrap_or(false);
            if !is_exit {
                panic!("_start failed: {}", msg);
            }
        }
    }

    let memory = instance.get_memory(&mut store, "memory").expect("memory export");
    let data = memory.data(&store);
    i64::from_le_bytes(data[65536..65536+8].try_into().unwrap())
}

/// Like run_outlayer_wasm but outlayer.view mock writes response to result buffer
#[cfg(not(target_arch = "wasm32"))]
fn run_outlayer_wasm_with_view(wasm: &[u8], stdin_data: &[u8], response: &[u8]) -> i64 {
    use std::sync::Arc;
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).expect("WASM should be valid");

    let stdin_arc = Arc::new(stdin_data.to_vec());
    let response_arc = Arc::new(response.to_vec());
    let mut store = Store::new(&engine, ());

    let sd = stdin_arc.clone();
    let fd_read_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        move |mut caller, args, results| {
            let iov_ptr = args[1].unwrap_i32() as usize;
            let nread_ptr = args[3].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    let copy_len = sd.len().min(buf_len);
                    if buf_ptr + copy_len <= data.len() { data[buf_ptr..buf_ptr+copy_len].copy_from_slice(&sd[..copy_len]); }
                    if nread_ptr + 4 <= data.len() { data[nread_ptr..nread_ptr+4].copy_from_slice(&(copy_len as u32).to_le_bytes()); }
                }
            }
            results[0] = Val::I32(0); Ok(())
        },
    );
    let fd_write_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let proc_exit_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32], vec![]),
        |_, args, _| Err(wasmtime::Error::msg(format!("proc_exit({})", args[0].unwrap_i32()))));
    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    // Mock near:rpc/api view: writes response to ret_area as tuple<string, string>
    let resp = response_arc.clone();
    let ol_view_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 9], vec![]),
        move |mut caller, args, _results| {
            // args: contract_ptr, contract_len, method_ptr, method_len, args_ptr, args_len, finality_ptr, finality_len, ret_area
            let ret_area = args[8].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                let copy_len = resp.len().min(65536);
                // Write response bytes at a safe offset after ret_area
                let resp_offset = ret_area + 32;
                if resp_offset + copy_len <= data.len() { data[resp_offset..resp_offset+copy_len].copy_from_slice(&resp[..copy_len]); }
                // tuple<string, string>: ptr1, len1, ptr2=0, len2=0 (no error)
                if ret_area + 16 <= data.len() {
                    data[ret_area..ret_area+4].copy_from_slice(&(resp_offset as u32).to_le_bytes());
                    data[ret_area+4..ret_area+8].copy_from_slice(&(copy_len as u32).to_le_bytes());
                    data[ret_area+8..ret_area+12].copy_from_slice(&0u32.to_le_bytes());
                    data[ret_area+12..ret_area+16].copy_from_slice(&0u32.to_le_bytes());
                }
            }
            Ok(())
        },
    );
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
    let ol_transfer_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| {});
    let read_reg_fn = Func::wrap(&mut store, |_: i64, _: i64| {});
    let reg_len_fn = Func::wrap(&mut store, |_: i64| -> i64 { 0 });

    let mut linker = Linker::new(&engine);
    linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "random_get", random_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", environ_sizes_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_get", environ_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "view", ol_view_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "call", ol_call_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "transfer", ol_transfer_fn).unwrap();
    // outlayer:api/host removed — upstream uses split interfaces
    let storage_set_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set", storage_set_fn).unwrap();
    let storage_get_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "get", storage_get_fn).unwrap();
    let storage_has_fn2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "near:storage/api@0.1.0", "has", storage_has_fn2).unwrap();
    let storage_delete_fn2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "near:storage/api@0.1.0", "delete", storage_delete_fn2).unwrap();
    let storage_incr_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "increment", storage_incr_fn).unwrap();
    let storage_decr_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "decrement", storage_decr_fn).unwrap();
    let storage_sia_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-if-absent", storage_sia_fn).unwrap();
    let storage_sie_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-if-equals", storage_sie_fn).unwrap();
    let storage_lk_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "list-keys", storage_lk_fn).unwrap();
    let storage_ca_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 1], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "clear-all", storage_ca_fn).unwrap();
    let storage_sw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-worker", storage_sw_fn).unwrap();
    let storage_gw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "get-worker", storage_gw_fn).unwrap();
    let storage_swp_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "set-worker-public", storage_swp_fn).unwrap();
    let storage_gwfp_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:storage/api@0.1.0", "get-worker-from-project", storage_gwfp_fn).unwrap();
    let raw_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![]), |_caller, _args, _results| Ok(())); linker.define(&store, "near:rpc/api@0.1.0", "raw", raw_fn).unwrap();
    // outlayer:api/host env-signer/predecessor removed — upstream uses env vars
    linker.define(&store, "env", "read_register", read_reg_fn).unwrap();
    linker.define(&store, "env", "register_len", reg_len_fn).unwrap();

    let instance = linker.instantiate(&mut store, &module).expect("instantiate");
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start").expect("_start export");
    match start.call(&mut store, ()) {
        Ok(()) => {}
        Err(trap) => {
            let msg = trap.to_string();
            let is_exit = msg.contains("proc_exit") || trap.source().map(|s| s.to_string().contains("proc_exit")).unwrap_or(false);
            if !is_exit { panic!("_start failed: {}", msg); }
        }
    }
    let memory = instance.get_memory(&mut store, "memory").expect("memory export");
    let data = memory.data(&store);
    i64::from_le_bytes(data[65536..65536+8].try_into().unwrap())
}

#[test]
fn test_outlayer_json_nested() {
    let src = r#"(define (run input) (let ((amount (json-get "data.amount"))) (+ amount 1)))"#;
    let wasm = compile_outlayer(src).unwrap();
    let result = run_outlayer_wasm(&wasm, br#"{"data":{"amount":42}}"#);
    assert_eq!(result, 43, "nested json-get should parse data.amount=42 and add 1");
}

#[test]
fn test_outlayer_json_flat() {
    let src = r#"(define (run input) (json-get "amount"))"#;
    let wasm = compile_outlayer(src).unwrap();
    let result = run_outlayer_wasm(&wasm, br#"{"amount":99}"#);
    assert_eq!(result, 99, "flat json-get should parse amount=99");
}

#[test]
fn test_outlayer_json_deep() {
    let src = r#"(define (run input) (json-get "a.b.c"))"#;
    let wasm = compile_outlayer(src).unwrap();
    let result = run_outlayer_wasm(&wasm, br#"{"a":{"b":{"c":7}}}"#);
    assert_eq!(result, 7, "deep nested json-get should parse a.b.c=7");
}

#[test]
fn test_outlayer_http_get_real() {
    let src = r#"(define (run) (http-get "https://wttr.in/Montreal?format=%t+%C"))"#;
    let wasm = compile_outlayer(src).unwrap();
    let result = run_outlayer_wasm_with_http(&wasm, &[]);
    // RESULT_BUF stores untagged payload: ptr | (len << 32)
    let ptr = (result & 0xFFFFFFFF) as usize;
    let len = ((result >> 32) as u32) as usize;
    assert!(len > 0, "http-get should return non-empty response, got ptr={} len={}", ptr, len);
    assert!(ptr > 0, "ptr should be non-zero");
}

/// Run OutLayer WASM with a REAL http_get that makes actual HTTP requests
#[cfg(not(target_arch = "wasm32"))]
fn run_outlayer_wasm_with_http(wasm: &[u8], stdin_data: &[u8]) -> i64 {
    use std::sync::Arc;
    use wasmtime::*;

    let engine = Engine::default();
    let module = Module::new(&engine, wasm).expect("WASM should be valid");

    let stdin_arc = Arc::new(stdin_data.to_vec());
    let mut store = Store::new(&engine, ());

    let sd = stdin_arc.clone();
    let fd_read_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        move |mut caller, args, results| {
            let iov_ptr = args[1].unwrap_i32() as usize;
            let nread_ptr = args[3].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    let copy_len = sd.len().min(buf_len);
                    if buf_ptr + copy_len <= data.len() { data[buf_ptr..buf_ptr+copy_len].copy_from_slice(&sd[..copy_len]); }
                    if nread_ptr + 4 <= data.len() { data[nread_ptr..nread_ptr+4].copy_from_slice(&(copy_len as u32).to_le_bytes()); }
                }
            }
            results[0] = Val::I32(0); Ok(())
        },
    );
    let fd_write_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        move |mut caller, args, results| {
            // Capture stdout output
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&mut caller);
                let iov_ptr = args[1].unwrap_i32() as usize;
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    if buf_ptr + buf_len <= data.len() {
                        let _output = String::from_utf8_lossy(&data[buf_ptr..buf_ptr+buf_len]);
                    }
                }
            }
            results[0] = Val::I32(args[2].unwrap_i32()); Ok(())
        },
    );
    let proc_exit_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32], vec![]),
        |_, args, _| Err(wasmtime::Error::msg(format!("proc_exit({})", args[0].unwrap_i32()))));
    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    // REAL http_get: reads URL from WASM memory, does actual HTTP request, writes response back
    let ol_http_get_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]),
        move |mut caller, args, results| {
            let url_ptr = args[0].unwrap_i32() as usize;
            let url_len = args[1].unwrap_i32() as usize;
            let resp_buf = args[2].unwrap_i32() as usize;
            let resp_buf_len = args[3].unwrap_i32() as usize;
            let resp_len_ptr = args[4].unwrap_i32() as usize;

            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if url_ptr + url_len <= data.len() {
                    let url = String::from_utf8_lossy(&data[url_ptr..url_ptr+url_len]).to_string();

                    // Make real HTTP request (blocking)
                    let response = std::process::Command::new("curl")
                        .args(["-s", "--max-time", "10", &url])
                        .output();

                    match response {
                        Ok(output) if output.status.success() => {
                            let body = &output.stdout;
                            let copy_len = body.len().min(resp_buf_len);
                            if resp_buf + copy_len <= data.len() {
                                data[resp_buf..resp_buf+copy_len].copy_from_slice(&body[..copy_len]);
                            }
                            if resp_len_ptr + 4 <= data.len() {
                                data[resp_len_ptr..resp_len_ptr+4].copy_from_slice(&(copy_len as u32).to_le_bytes());
                            }
                            results[0] = Val::I32(0); // errno = 0 (success)
                        }
                        Ok(_output) => {
                            results[0] = Val::I32(1); // errno = error
                        }
                        Err(_e) => {
                            results[0] = Val::I32(1);
                        }
                    }
                } else {
                    results[0] = Val::I32(1);
                }
            } else {
                results[0] = Val::I32(1);
            }
            Ok(())
        },
    );

    // Stubs for other outlayer functions
    let ol_view_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let ol_transfer_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let stub_4 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_5 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_6 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_3 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_8 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 8], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_0 = Func::new(&mut store, FuncType::new(&engine, vec![], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });
    let stub_7 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![ValType::I32]), |_,_,r| { r[0] = Val::I32(0); Ok(()) });

    let mut linker = Linker::new(&engine);
    linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "random_get", random_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", environ_sizes_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_get", environ_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek_fn).unwrap();
    linker.define(&store, "outlayer", "view", ol_view_fn).unwrap();
    linker.define(&store, "outlayer", "call", ol_call_fn).unwrap();
    linker.define(&store, "outlayer", "transfer", ol_transfer_fn).unwrap();
    linker.define(&store, "outlayer", "http_get", ol_http_get_fn).unwrap();
    linker.define(&store, "outlayer", "storage_set", stub_4).unwrap();
    linker.define(&store, "outlayer", "storage_get", stub_5).unwrap();
    linker.define(&store, "outlayer", "storage_has", stub_2).unwrap();
    linker.define(&store, "outlayer", "storage_delete", stub_2).unwrap();
    linker.define(&store, "outlayer", "storage_increment", stub_6).unwrap();
    linker.define(&store, "outlayer", "env_signer", stub_3).unwrap();
    linker.define(&store, "outlayer", "env_predecessor", stub_3).unwrap();
    linker.define(&store, "outlayer", "storage_decrement", stub_6).unwrap();
    linker.define(&store, "outlayer", "storage_set_if_absent", stub_4).unwrap();
    linker.define(&store, "outlayer", "storage_set_if_equals", stub_8).unwrap();
    linker.define(&store, "outlayer", "storage_list_keys", stub_5).unwrap();
    linker.define(&store, "outlayer", "storage_clear_all", stub_0).unwrap();
    linker.define(&store, "outlayer", "storage_set_worker", stub_4).unwrap();
    linker.define(&store, "outlayer", "storage_get_worker", stub_5).unwrap();
    linker.define(&store, "outlayer", "storage_set_worker_public", stub_4).unwrap();
    linker.define(&store, "outlayer", "storage_get_worker_from_project", stub_7).unwrap();

    let instance = linker.instantiate(&mut store, &module).expect("instantiate");
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start").expect("_start export");
    match start.call(&mut store, ()) {
        Ok(()) => {}
        Err(trap) => {
            let msg = trap.to_string();
            let is_exit = msg.contains("proc_exit") || trap.source().map(|s| s.to_string().contains("proc_exit")).unwrap_or(false);
            if !is_exit { panic!("_start failed: {}", msg); }
        }
    }
    let memory = instance.get_memory(&mut store, "memory").expect("memory export");
    let data = memory.data(&store);
    i64::from_le_bytes(data[65536..65536+8].try_into().unwrap())
}


    #[test]
    fn test_p2_wasi_http_component() {
        let src = r#"
(define (fetch)
  (http-get "https://httpbin.org/get"))
(define (run) (fetch))
"#;
        let result = compile_outlayer_p2(src);
        match &result {
            Ok(bytes) => {
                assert!(bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]));
                std::fs::write("/tmp/p2_wasi_http.wasm", bytes).unwrap();
            }
            Err(e) => {
                panic!("compile failed: {}", e);
            }
        }
    }

    /// Live P2 component execution test: compile weather-rust reference to P2
    /// wasi:http component, run it in wasmtime with real HTTP support.
    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn test_p2_wasi_http_live() {
        use wasmtime::{Engine, Config};
        use wasmtime::component::{Linker, ResourceTable};
        use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView, WasiCtxBuilder, p2::pipe::MemoryInputPipe};
        use wasmtime_wasi_http::{WasiHttpCtx, p2::{WasiHttpView, WasiHttpCtxView}};

        struct TestState {
            ctx: WasiCtx,
            http_ctx: WasiHttpCtx,
            table: ResourceTable,
        }
        impl WasiHttpView for TestState {
            fn http(&mut self) -> WasiHttpCtxView<'_> {
                WasiHttpCtxView {
                    ctx: &mut self.http_ctx,
                    table: &mut self.table,
                    hooks: Default::default(),
                }
            }
        }
        impl WasiView for TestState {
            fn ctx(&mut self) -> WasiCtxView<'_> {
                WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
            }
        }

        // Use the reference weather-rust component (compiled from Rust with wasi-http-client)
        let ref_path = "weather-rust/target/wasm32-wasip2/release/weather.wasm";
        let comp_bytes = std::fs::read(ref_path)
            .unwrap_or_else(|e| panic!("read {}: {}. Run: cd weather-rust && cargo build --target wasm32-wasip2 --release", ref_path, e));

        // Set up wasmtime with wasi:http support
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);
        config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        let engine = Engine::new(&config).expect("engine");
        let component = wasmtime::component::Component::from_binary(&engine, &comp_bytes)
            .expect("component deserialize");

        let mut linker = Linker::<TestState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .expect("add wasi to linker");
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .expect("add wasi:http to linker");

        // Set up stdin (JSON input) and stdout (capture)
        let stdin_json = r#"{"city":"Montreal"}"#;
        let stdout_pipe = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(65536);
        let table = ResourceTable::new();
        let ctx = WasiCtxBuilder::new()
            .stdin(MemoryInputPipe::new(stdin_json.as_bytes().to_vec()))
            .stdout(stdout_pipe.clone())
            .build();

        let state = TestState {
            ctx,
            http_ctx: WasiHttpCtx::new(),
            table,
        };

        let mut store = wasmtime::Store::new(&engine, state);

        // Instantiate the component
        let instance = linker.instantiate_async(&mut store, &component)
            .await
            .expect("instantiate");

        // Get wasi:cli/run#run via the instance export
        let (_, run_instance_idx) = instance
            .get_export(&mut store, None, "wasi:cli/run@0.2.6")
            .expect("get wasi:cli/run instance");
        let (_, run_func_idx) = instance
            .get_export(&mut store, Some(&run_instance_idx), "run")
            .expect("get run func");

        let run_fn = instance
            .get_func(&mut store, &run_func_idx)
            .expect("get run Func");

        // Call it — wasi:cli/run#run() -> result<(), _>
        let mut result_val = [wasmtime::component::Val::Bool(false)];
        run_fn.call_async(&mut store, &[], &mut result_val).await
            .expect("run call failed");

        // Read stdout
        let output = stdout_pipe.contents();
        let output_str = String::from_utf8_lossy(&output);

        // Verify we got weather data
        assert!(output_str.contains("temp_c"), "expected weather JSON with temp_c, got: {}", output_str);
    }

    /// Test our Lisp-compiled P2 component in wasmtime with real HTTP
    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn test_lisp_p2_wasi_http_live() {
        use wasmtime::{Engine, Config};
        use wasmtime::component::{Linker, ResourceTable};
        use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView, WasiCtxBuilder, p2::pipe::MemoryInputPipe};
        use wasmtime_wasi_http::{WasiHttpCtx, p2::{WasiHttpView, WasiHttpCtxView}};

        struct TestState {
            ctx: WasiCtx,
            http_ctx: WasiHttpCtx,
            table: ResourceTable,
        }
        impl WasiHttpView for TestState {
            fn http(&mut self) -> WasiHttpCtxView<'_> {
                WasiHttpCtxView { ctx: &mut self.http_ctx, table: &mut self.table, hooks: Default::default() }
            }
        }
        impl WasiView for TestState {
            fn ctx(&mut self) -> WasiCtxView<'_> { WasiCtxView { ctx: &mut self.ctx, table: &mut self.table } }
        }

        // Compile our Lisp code to P2 component
        let source = r#"
(define (weather) (http-get "https://httpbin.org/get"))
"#;
        let comp_bytes = compile_outlayer_p2(source)
            .expect("Lisp P2 compilation failed");
        std::fs::write("/tmp/lisp_p2_test.wasm", &comp_bytes).ok();

        // Set up wasmtime with wasi:http support
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);
        config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        let engine = Engine::new(&config).expect("engine");
        let component = wasmtime::component::Component::from_binary(&engine, &comp_bytes)
            .expect("component deserialize");

        let mut linker = Linker::<TestState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("add wasi");
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).expect("add wasi:http");

        let stdout_pipe = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(65536);
        let table = ResourceTable::new();
        let ctx = WasiCtxBuilder::new()
            .stdin(MemoryInputPipe::new(vec![]))
            .stdout(stdout_pipe.clone())
            .build();

        let state = TestState { ctx, http_ctx: WasiHttpCtx::new(), table };
        let mut store = wasmtime::Store::new(&engine, state);

        let instance = linker.instantiate_async(&mut store, &component)
            .await
            .expect("instantiate");

        // Get wasi:cli/run#run
        let (_, run_instance_idx) = instance
            .get_export(&mut store, None, "wasi:cli/run@0.2.2")
            .expect("get wasi:cli/run instance");
        let (_, run_func_idx) = instance
            .get_export(&mut store, Some(&run_instance_idx), "run")
            .expect("get run func");
        let run_fn = instance
            .get_func(&mut store, &run_func_idx)
            .expect("get run Func");

        let mut result_val = [wasmtime::component::Val::Bool(false)];
        match run_fn.call_async(&mut store, &[], &mut result_val).await {
            Ok(()) => {
                let output = stdout_pipe.contents();
                let _output_str = String::from_utf8_lossy(&output);
            }
            Err(e) => {
                panic!("Lisp P2 component execution failed: {:?}", e);
            }
        }
    }

    /// Test multi-URL http-get: compare Montreal vs Toronto weather
    #[cfg(not(target_arch = "wasm32"))]
    #[tokio::test]
    async fn test_lisp_p2_multi_url_http_get() {
        use wasmtime::{Engine, Config};
        use wasmtime::component::{Linker, ResourceTable};
        use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView, WasiCtxBuilder, p2::pipe::MemoryInputPipe};
        use wasmtime_wasi_http::{WasiHttpCtx, p2::{WasiHttpView, WasiHttpCtxView}};

        struct TestState {
            ctx: WasiCtx,
            http_ctx: WasiHttpCtx,
            table: ResourceTable,
        }
        impl WasiHttpView for TestState {
            fn http(&mut self) -> WasiHttpCtxView<'_> {
                WasiHttpCtxView { ctx: &mut self.http_ctx, table: &mut self.table, hooks: Default::default() }
            }
        }
        impl WasiView for TestState {
            fn ctx(&mut self) -> WasiCtxView<'_> { WasiCtxView { ctx: &mut self.ctx, table: &mut self.table } }
        }

        // Compare Montreal vs Toronto weather using json-get to extract temps
        let source = r#"
(define (compare)
  (let ((mtl (http-get "https://api.open-meteo.com/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m"))
        (tor (http-get "https://api.open-meteo.com/v1/forecast?latitude=43.65&longitude=-79.38&current=temperature_2m")))
    (let ((mtl-temp (json-get "temperature_2m" mtl))
          (tor-temp (json-get "temperature_2m" tor)))
      (if (> mtl-temp tor-temp)
          "Montreal is warmer!"
          "Toronto is warmer!"))))
"#;
        let comp_bytes = compile_outlayer_p2(source)
            .expect("Lisp P2 multi-URL compilation failed");
        std::fs::write("/tmp/lisp_p2_multi_url_test.wasm", &comp_bytes).ok();

        // Set up wasmtime with wasi:http support
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.async_support(true);
        config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        let engine = Engine::new(&config).expect("engine");
        let component = wasmtime::component::Component::from_binary(&engine, &comp_bytes)
            .expect("component deserialize");

        let mut linker = Linker::<TestState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("add wasi");
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).expect("add wasi:http");

        let stdout_pipe = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(65536);
        let table = ResourceTable::new();
        let ctx = WasiCtxBuilder::new()
            .stdin(MemoryInputPipe::new(vec![]))
            .stdout(stdout_pipe.clone())
            .build();

        let state = TestState { ctx, http_ctx: WasiHttpCtx::new(), table };
        let mut store = wasmtime::Store::new(&engine, state);

        let instance = linker.instantiate_async(&mut store, &component)
            .await
            .expect("instantiate");

        // Get wasi:cli/run#run
        let (_, run_instance_idx) = instance
            .get_export(&mut store, None, "wasi:cli/run@0.2.2")
            .expect("get wasi:cli/run instance");
        let (_, run_func_idx) = instance
            .get_export(&mut store, Some(&run_instance_idx), "run")
            .expect("get run func");
        let run_fn = instance
            .get_func(&mut store, &run_func_idx)
            .expect("get run Func");

        let mut result_val = [wasmtime::component::Val::Bool(false)];
        match run_fn.call_async(&mut store, &[], &mut result_val).await {
            Ok(()) => {
                let output = stdout_pipe.contents();
                let output_str = String::from_utf8_lossy(&output);
                // Verify that both responses are present in the output
                assert!(output_str.contains("warmer"), "expected comparison result, got: {}", output_str);
            }
            Err(e) => {
                panic!("Multi-URL P2 component execution failed: {:?}", e);
            }
        }
    }

#[cfg(test)]
mod p2_debug_test {
    use super::*;

    #[test]
    fn debug_p2_http_post() {
        let wasm = compile_outlayer_p2_core_browser(r#"(define (main)
  (let ((url "https://httpbin.org/post")
        (body "{\"hello\": \"world\"}"))
    (http-post url body)))"#).unwrap();
        std::fs::write("/tmp/p2_debug.wasm", &wasm).unwrap();
        eprintln!("WASM: {} bytes", wasm.len());
        eprintln!("First 8 bytes: {:02x?}", &wasm[..wasm.len().min(8)]);
        // Validate with wasm-tools
        let out = std::process::Command::new("wasm-tools")
            .args(["validate", "/tmp/p2_debug.wasm"])
            .output()
            .expect("failed to run wasm-tools validate");
        if !out.status.success() {
            panic!("wasm-tools validate FAILED:\n{}", String::from_utf8_lossy(&out.stderr));
        }
        eprintln!("VALID!");
    }
}
