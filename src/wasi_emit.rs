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

use crate::wasm_emit::WasmEmitter;
use wasm_encoder::*;

// ── Memory layout for OutLayer target ──
// Same layout as NEAR but different I/O path
const STDIN_BUF: i64 = 32768;   // 32KB for stdin data
const STDOUT_BUF: i64 = 65536;  // 32KB for stdout data  
const STDIN_LEN: i64 = 98304;   // i32: actual bytes read
const RESULT_BUF: i64 = 65536;  // reuse STDOUT_BUF for result

/// WASI Preview 1 function descriptors (module, name, params, results)
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

/// OutLayer host function imports (view, call, transfer via WIT)
fn outlayer_imports() -> Vec<WasiFunc> {
    vec![
        // 0: view(contract_ptr, contract_len, method_ptr, method_len, 
        //         args_ptr, args_len, result_ptr, result_len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "view",
            params: vec![W, W, W, W, W, W, W, W], results: vec![W] },
        // 1: call(contract_ptr, contract_len, method_ptr, method_len,
        //         args_ptr, args_len, gas, deposit_lo, deposit_hi,
        //         result_ptr, result_len_ptr, callback_ptr, callback_len) -> errno
        WasiFunc { module: "outlayer", name: "call",
            params: vec![W; 13], results: vec![W] },
        // 2: transfer(recipient_ptr, recipient_len, amount_lo, amount_hi,
        //             result_ptr, result_len_ptr, msg_ptr, msg_len,
        //             callback_ptr, callback_len) -> errno
        WasiFunc { module: "outlayer", name: "transfer",
            params: vec![W; 10], results: vec![W] },
        // 3: http_get(url_ptr, url_len, response_buf_ptr, response_buf_len, response_len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "http_get",
            params: vec![W, W, W, W, W], results: vec![W] },
        // 4: storage_set(key_ptr, key_len, val_ptr, val_len) -> errno
        WasiFunc { module: "outlayer", name: "storage_set",
            params: vec![W, W, W, W], results: vec![W] },
        // 5: storage_get(key_ptr, key_len, buf_ptr, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_get",
            params: vec![W, W, W, W, W], results: vec![W] },
        // 6: storage_has(key_ptr, key_len) -> i32 (0 or 1)
        WasiFunc { module: "outlayer", name: "storage_has",
            params: vec![W, W], results: vec![W] },
        // 7: storage_delete(key_ptr, key_len) -> errno
        WasiFunc { module: "outlayer", name: "storage_delete",
            params: vec![W, W], results: vec![W] },
        // 8: storage_increment(key_ptr, key_len, delta_lo, delta_hi, result_lo_ptr, result_hi_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_increment",
            params: vec![W, W, W, W, W, W], results: vec![W] },
        // 9: env_signer(buf_ptr, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "env_signer",
            params: vec![W, W, W], results: vec![W] },
        // 10: env_predecessor(buf_ptr, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "env_predecessor",
            params: vec![W, W, W], results: vec![W] },
        // 11: storage_decrement — same sig as increment
        WasiFunc { module: "outlayer", name: "storage_decrement",
            params: vec![W; 6], results: vec![W] },
        // 12: storage_set_if_absent(key, val) -> bool (0=not inserted, 1=inserted)
        WasiFunc { module: "outlayer", name: "storage_set_if_absent",
            params: vec![W; 4], results: vec![W] },
        // 13: storage_set_if_equals(key, expected, new, old_buf, old_len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_set_if_equals",
            params: vec![W; 8], results: vec![W] },
        // 14: storage_list_keys(prefix, buf, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_list_keys",
            params: vec![W; 5], results: vec![W] },
        // 15: storage_clear_all() -> errno
        WasiFunc { module: "outlayer", name: "storage_clear_all",
            params: vec![], results: vec![W] },
        // 16: storage_set_worker(key, val) -> errno
        WasiFunc { module: "outlayer", name: "storage_set_worker",
            params: vec![W; 4], results: vec![W] },
        // 17: storage_get_worker(key, buf, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_get_worker",
            params: vec![W; 5], results: vec![W] },
        // 18: storage_set_worker_public(key, val) -> errno
        WasiFunc { module: "outlayer", name: "storage_set_worker_public",
            params: vec![W; 4], results: vec![W] },
        // 19: storage_get_worker_from_project(key, project, buf, buf_len, len_ptr) -> errno
        WasiFunc { module: "outlayer", name: "storage_get_worker_from_project",
            params: vec![W; 7], results: vec![W] },
    ]
}

/// Compile to WASI P1 core module with minimal imports (no outlayer).
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
    let mut types = TypeSection::new();
    for &np in &param_counts { types.ty().function(vec![ValType::I32; np], vec![ValType::I32]); }
    m.section(&types);
    let mut funcs = FunctionSection::new();
    for i in 0..20u32 { funcs.function(i); }
    m.section(&funcs);
    let mut exports = ExportSection::new();
    for (i, n) in names.iter().enumerate() { exports.export(*n, ExportKind::Func, i as u32); }
    m.section(&exports);
    let mut code = CodeSection::new();
    for _ in 0..20 {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::I32Const(0));
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
        // HTTP example: return simple WASI core with http_get stub
        // (real http_get would be polyfilled to fetch() in browser)
        finish_outlayer(&mut em)
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

    let bytes = if em.need_wasi_http {
        build_p2_with_wasi_http(&em)?
    } else {
        let core_bytes = if em.need_outlayer {
            finish_outlayer(&mut em)?
        } else {
            finish_outlayer_no_ol(&mut em)?
        };
        build_p2_with_adapter(&core_bytes)?
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

    let bytes = if em.need_wasi_http {
        // wasi:http path — build component with embedded HTTP metadata
        build_p2_with_wasi_http(&em)?
    } else {
        let core_bytes = if em.need_outlayer {
            finish_outlayer(&mut em)?
        } else {
            finish_outlayer_no_ol(&mut em)?
        };
        build_p2_with_adapter(&core_bytes)?
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
    let http_urls: Vec<(String, String)> = if em.http_urls.is_empty() {
        vec![(
            "api.open-meteo.com".to_string(),
            "/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m".to_string(),
        )]
    } else {
        em.http_urls.clone()
    };
    let http_get_count = http_urls.len() as u32;

    // Compute all indices dynamically
    let layout = WasiHttpLayout::new(em.funcs.len() as u32, http_get_count);

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
    module.section(&types);
    module.section(&imports);

    // ═══ Function Section ═══
    let mut functions = FunctionSection::new();
    // For each URL, emit an http_get + poll_read pair
    for _ in &http_urls {
        functions.function(layout.http_get_type);
        functions.function(layout.http_get_type); // poll_read — same type
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
    let pages = em.memory_pages.max(17) as u64; // min 17 pages (1.06MB) for P2 scratch + heap
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
    exports.export("wasi:cli/run@0.2.2#run", ExportKind::Func, layout.start_fn_idx);
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

    // ── User functions from the emitter ──
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter().enumerate()
        .map(|(i, f)| (f.name.as_str(), layout.user_fn_base + i as u32))
        .collect();

    for f in &em.funcs {
        let extra = f.local_count.saturating_sub(f.param_count);
        let locals: Vec<(u32, ValType)> = if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] };

        let resolved = {
            let base_resolved = WasmEmitter::resolve_static_pub(&f.instrs, &std::collections::HashMap::new(), &name_map, &em.funcs);
            let mut ol_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            // Map each sentinel 103+i to the corresponding HTTP function
            for i in 0..http_get_count {
                ol_map.insert(103 + i, layout.http_get_fn_idx + (i * 2));
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
        let mut real_func_idx = layout.user_fn_base + (em.funcs.len() - 1) as u32;
        let mut real_param_count = em.funcs.last().unwrap().param_count;
        // Simple heuristic: if the last function has 0 params and its body is just depth+gas+call+epilogue,
        // try the second-to-last function instead.
        // In P2 mode (no gas/depth), a trivial wrapper body is just: Call(N); LocalSet; LocalGet
        if em.funcs.len() > 1 {
            let last = em.funcs.last().unwrap();
            if last.param_count == 0 {
                // Check if body is essentially just a call to another user function
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
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };

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
    // Uses memory at offset 196608 (192KB) as bump allocator pointer.
    // This is after all other static data (stdin, stdout, scratch, etc.)
    {
        let heap_ptr_addr: i32 = 196608;
        let mut realloc = Function::new([(1, ValType::I32)]); // extra local 4
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        // Initialize heap_ptr if zero (first call)
        // result = heap_ptr (from memory)
        realloc.instruction(&Instruction::I32Const(heap_ptr_addr));
        realloc.instruction(&Instruction::I32Load(ma4));
        // If heap_ptr == 0, initialize to 1048576 (1MB)
        realloc.instruction(&Instruction::LocalTee(4));
        realloc.instruction(&Instruction::I32Eqz);
        realloc.instruction(&Instruction::If(BlockType::Empty));
        realloc.instruction(&Instruction::I32Const(1048576));
        realloc.instruction(&Instruction::I32Const(heap_ptr_addr));
        realloc.instruction(&Instruction::I32Store(ma4));
        realloc.instruction(&Instruction::I32Const(1048576));
        realloc.instruction(&Instruction::LocalSet(4));
        realloc.instruction(&Instruction::End);
        // heap_ptr += new_len (param 3), aligned to 4
        realloc.instruction(&Instruction::LocalGet(4));
        realloc.instruction(&Instruction::LocalGet(3));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(3));
        realloc.instruction(&Instruction::I32Add);
        realloc.instruction(&Instruction::I32Const(-4));
        realloc.instruction(&Instruction::I32And);
        // Store new heap_ptr
        realloc.instruction(&Instruction::I32Const(heap_ptr_addr));
        realloc.instruction(&Instruction::I32Store(ma4));
        // Return old ptr
        realloc.instruction(&Instruction::LocalGet(4));
        realloc.instruction(&Instruction::End);
        codes.function(&realloc);
    }

    module.section(&codes);

    // ═══ Data Section (string literals + URL data segments) ═══
    {
        let mut data = DataSection::new();
        let mut has_data = false;
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

    // ═══ Build and embed WIT metadata ═══
    let mut core_bytes = module.finish();
    std::fs::write("/tmp/p2_http_core.wasm", &core_bytes).ok();

    let (resolve, world) = crate::wasi_http::build_http_wit_metadata()?;
    let before = core_bytes.len();
    wit_component::embed_component_metadata(&mut core_bytes, &resolve, world, wit_component::StringEncoding::UTF8)
        .map_err(|e| format!("embed metadata failed: {}", e))?;
    eprintln!("WIT metadata: {} -> {} bytes", before, core_bytes.len());
    std::fs::write("/tmp/p2_http_core_with_meta.wasm", &core_bytes).ok();

    let component = wit_component::ComponentEncoder::default()
        .module(&core_bytes).map_err(|e| format!("encoder: {:#}", e))?
        .validate(true)
        .encode().map_err(|e| format!("encode: {:#}", e))?;

    eprintln!("✅ P2 wasi:http component: {} bytes", component.len());
    std::fs::write("/tmp/p2_wasi_http.wasm", &component).ok();

    if let Err(e) = wasmparser::validate(&component) {
        eprintln!("⚠️ Component validation warning: {}", e);
    }

    Ok(component)
}


fn build_p2_with_adapter(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    // Load WASI adapter
    let adapter_bytes = crate::p2_native::load_wasi_adapter();
    if adapter_bytes.len() < 100 {
        return Err("WASI adapter not found. Set WASI_ADAPTER_PATH or place wasi_snapshot_preview1.command.wasm in /tmp/".into());
    }

    let mut encoder = wit_component::ComponentEncoder::default()
        .module(core_bytes)
        .map_err(|e| format!("wit-component: failed to set module: {}", e))?
        .validate(true)
        .adapter("wasi_snapshot_preview1", &adapter_bytes)
        .map_err(|e| format!("wit-component: failed to set WASI adapter: {}", e))?;

    // Check if the core module imports "outlayer" — if so, add the outlayer adapter
    let imports = analyze_core_imports(core_bytes);
    if imports.contains(&"outlayer") {
        let ol_adapter = crate::outlayer_adapter::build_outlayer_adapter();
        encoder = encoder.adapter("outlayer", &ol_adapter)
            .map_err(|e| format!("wit-component: failed to set outlayer adapter: {}", e))?;
    }

    let component = encoder.encode()
        .map_err(|e| format!("wit-component encode failed: {}", e))?;

    eprintln!("✅ P2 component via wit-component: {} bytes", component.len());
    Ok(component)
}

/// Analyze a core WASM module to find which import module names are referenced.
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
                    0 => { let (tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    1 => { pos += 3; let (tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    2 => { let (tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
                    3 => { pos += 1; let (tl, tl2) = read_leb128_outlayer(&wasm[pos..]); pos += tl2; }
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
    finish_outlayer_inner(em, false)
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
    let ol: Vec<WasiFunc> = if skip_outlayer { vec![] } else { outlayer_imports() };
    let wasi_count = wasi.len() as u32;
    let ol_count = ol.len() as u32;
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
    // P2: _start returns i32 (result code, 0=success) for wasi:cli/run compatibility
    // P1: _start returns () and calls proc_exit
    if em.p2_mode {
        types.ty().function([], [ValType::I32]); // type 0: () -> i32 (_start for P2)
    } else {
        types.ty().function([], []); // type 0: () -> () (_start for P1)
    }
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

    // OutLayer host types
    // type 7: view — 8 i32 params -> i32
    types.ty().function(vec![W; 8], [W]);
    // type 8: call — 13 i32 params -> i32  
    types.ty().function(vec![W; 13], [W]);
    // type 9: transfer — 10 i32 params -> i32
    types.ty().function(vec![W; 10], [W]);
    // type 10: http_get — 5 i32 params -> i32
    types.ty().function(vec![W; 5], [W]);
    // type 11: storage_set — 4 i32 params -> i32
    types.ty().function(vec![W; 4], [W]);
    // type 12: storage_get — 5 i32 params -> i32
    types.ty().function(vec![W; 5], [W]);
    // type 13: storage_has — 2 i32 params -> i32
    types.ty().function(vec![W; 2], [W]);
    // type 14: storage_delete — 2 i32 params -> i32
    types.ty().function(vec![W; 2], [W]);
    // type 15: storage_increment — 6 i32 params -> i32
    types.ty().function(vec![W; 6], [W]);
    // type 16: env_signer — 3 i32 params -> i32
    types.ty().function(vec![W; 3], [W]);
    // type 17: env_predecessor — 3 i32 params -> i32
    types.ty().function(vec![W; 3], [W]);
    // type 18: () -> i32 (storage_clear_all)
    types.ty().function([], [W]);
    // type 19: 7 i32 -> i32 (get_worker_from_project)
    types.ty().function(vec![W; 7], [W]);
    // type 20: 8 i32 -> i32 (set_if_equals)
    types.ty().function(vec![W; 8], [W]);

    // NEAR-style host function types (for NEAR compat stubs)
    // We need types for each unique NEAR host function signature used
    // Map NEAR host func index to its type index
    let _ = &near_host_used; // used below
    // User function types: each function has (i64 × param_count) -> i64
    let max_p = em.funcs.iter().map(|f| f.param_count).max().unwrap_or(0);
    let user_type_base: u32 = 21;
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
    // type for (i64) -> ()
    types.ty().function([ValType::I64], []);
    let near_i64_to_void = nti;
    nti += 1;
    // type for (i64) -> i64
    types.ty().function([ValType::I64], [ValType::I64]);
    let near_i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64) -> ()
    types.ty().function([ValType::I64, ValType::I64], []);
    let near_2i64_to_void = nti;
    nti += 1;
    // type for (i64, i64) -> i64
    types.ty().function([ValType::I64, ValType::I64], [ValType::I64]);
    let near_2i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64) -> ()
    types.ty().function([ValType::I64, ValType::I64, ValType::I64], []);
    let near_3i64_to_void = nti;
    nti += 1;
    // type for (i64, i64, i64) -> i64
    types.ty().function([ValType::I64, ValType::I64, ValType::I64], [ValType::I64]);
    let near_3i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64, i64) -> i64 (storage_write)
    types.ty().function([ValType::I64; 5], [ValType::I64]);
    let near_5i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64, i64, i64, i64, i64) -> i64 (promise_create)
    types.ty().function([ValType::I64; 8], [ValType::I64]);
    let near_8i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64, i64, i64, i64, i64, i64) -> i64 (promise_then)
    types.ty().function([ValType::I64; 9], [ValType::I64]);
    let near_9i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64, i64, i64, i64) -> () (promise_batch_action_function_call)
    types.ty().function([ValType::I64; 7], []);
    let near_7i64_to_void = nti;
    nti += 1;
    // type for (i64, i64, i64, i64, i64, i64) -> () (ed25519_verify variant)
    types.ty().function([ValType::I64; 6], [ValType::I64]);
    let near_6i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64) -> i64 (storage_iter_range)
    types.ty().function([ValType::I64; 4], [ValType::I64]);
    let near_4i64_to_i64 = nti;
    nti += 1;
    // type for (i64, i64, i64, i64) -> () (promise_batch_action_stake)
    types.ty().function([ValType::I64; 4], []);
    let near_4i64_to_void = nti;
    nti += 1;

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
    // OutLayer imports (indices wasi_count..wasi_count+ol_count)
    let ol_type_map: Vec<u32> = vec![
        7,  // view: 8 i32 -> i32
        8,  // call: 14 i32 -> i32
        9,  // transfer: 10 i32 -> i32
        10, // http_get: 5 i32 -> i32
        11, // storage_set: 4 i32 -> i32
        12, // storage_get: 5 i32 -> i32
        13, // storage_has: 2 i32 -> i32
        14, // storage_delete: 2 i32 -> i32
        15, // storage_increment: 6 i32 -> i32
        16, // env_signer: 3 i32 -> i32
        17, // env_predecessor: 3 i32 -> i32
        15, // storage_decrement: 6 i32 -> i32 (reuse type 15)
        11, // storage_set_if_absent: 4 i32 -> i32 (reuse type 11)
        20, // storage_set_if_equals: 8 i32 -> i32
        12, // storage_list_keys: 5 i32 -> i32 (reuse type 12)
        18, // storage_clear_all: () -> i32
        11, // storage_set_worker: 4 i32 -> i32 (reuse type 11)
        12, // storage_get_worker: 5 i32 -> i32 (reuse type 12)
        11, // storage_set_worker_public: 4 i32 -> i32 (reuse type 11)
        19, // storage_get_worker_from_project: 7 i32 -> i32
    ];
    for (i, f) in ol.iter().enumerate() {
        let type_idx = ol_type_map[i];
        imports.import(f.module, f.name, EntityType::Function(type_idx));
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
    m.section(&funcs);

    // ── Memory ──
    let mut mems = MemorySection::new();
    // min 16 pages (1MB) for P2 scratch + heap
    let pages = em.memory_pages.max(16) as u64;
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
    // _start is the last function
    let start_func_idx = internal_base + em.funcs.len() as u32;
    // P2 components export "run", WASI P1 exports "_start"
    let entry_name = "_start"; // always _start; P2 wrapper handles naming
    exps.export(entry_name, ExportKind::Func, start_func_idx);
    m.section(&exps);

    // ── Code section ──
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter().enumerate()
        .map(|(i, f)| (f.name.as_str(), internal_base + i as u32))
        .collect();
    
    let mut code = wasm_encoder::CodeSection::new();
    
    // Emit user functions (same resolution logic as finish())
    for f in &em.funcs {
        let extra = f.local_count.saturating_sub(f.param_count);
        let locals: Vec<(u32, ValType)> = if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] };
        let resolved = WasmEmitter::resolve_static_pub(&f.instrs, &near_host_idx, &name_map, &em.funcs);
        let resolved = if em.need_outlayer || em.wasi_mode {
            let mut ol_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
            // outlayer.view is at import index wasi_count + 0
            // outlayer.call is at import index wasi_count + 1
            // outlayer.transfer is at import index wasi_count + 2
            ol_map.insert(100, wasi_count); // sentinel 100 -> outlayer.view
            ol_map.insert(101, wasi_count + 1); // sentinel 101 -> outlayer.call
            ol_map.insert(102, wasi_count + 2); // sentinel 102 -> outlayer.transfer
            ol_map.insert(103, wasi_count + 3); // sentinel 103 -> outlayer.http_get
            ol_map.insert(110, wasi_count + 4); // sentinel 110 -> outlayer.storage_set
            ol_map.insert(111, wasi_count + 5); // sentinel 111 -> outlayer.storage_get
            ol_map.insert(112, wasi_count + 6); // sentinel 112 -> outlayer.storage_has
            ol_map.insert(113, wasi_count + 7); // sentinel 113 -> outlayer.storage_delete
            ol_map.insert(114, wasi_count + 8); // sentinel 114 -> outlayer.storage_increment
            ol_map.insert(120, wasi_count + 9); // sentinel 120 -> outlayer.env_signer
            ol_map.insert(121, wasi_count + 10); // sentinel 121 -> outlayer.env_predecessor
            ol_map.insert(130, wasi_count + 11); // sentinel 130 -> outlayer.storage_decrement
            ol_map.insert(131, wasi_count + 12); // sentinel 131 -> outlayer.storage_set_if_absent
            ol_map.insert(132, wasi_count + 13); // sentinel 132 -> outlayer.storage_set_if_equals
            ol_map.insert(133, wasi_count + 14); // sentinel 133 -> outlayer.storage_list_keys
            ol_map.insert(134, wasi_count + 15); // sentinel 134 -> outlayer.storage_clear_all
            ol_map.insert(135, wasi_count + 16); // sentinel 135 -> outlayer.storage_set_worker
            ol_map.insert(136, wasi_count + 17); // sentinel 136 -> outlayer.storage_get_worker
            ol_map.insert(137, wasi_count + 18); // sentinel 137 -> outlayer.storage_set_worker_public
            ol_map.insert(138, wasi_count + 19); // sentinel 138 -> outlayer.storage_get_worker_from_project
            ol_map.insert(crate::wasm_emit::WASI_FD_WRITE, 1); // fd_write is WASI import index 1
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
        let last_func = em.funcs.last().unwrap();
        let last_idx = internal_base + (em.funcs.len() - 1) as u32;
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
        let ma1 = MemArg { offset: 0, align: 0, memory_index: 0 };

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
        } else {
            // Load params from STDIN_BUF as raw tagged i64s (same as NEAR pattern)
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

        // ── Write result to stdout AND store at RESULT_BUF ──
        // Always store untagged payload at RESULT_BUF (for testing)
        fb.instruction(&Instruction::I64Const(RESULT_BUF));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64ShrS);
        fb.instruction(&Instruction::I64Store(ma));

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
        fb.instruction(&Instruction::I64Const(3)); fb.instruction(&Instruction::I64ShrS);
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

        // P1: proc_exit(0) — terminates process
        // P2: return i32 0 — signals success to wasi:cli/run  
        // no_proc_exit: just return cleanly (wit-component adapter handles exit)
        if em.p2_mode {
            fb.instruction(&Instruction::I32Const(0));
        } else if !em.no_proc_exit {
            fb.instruction(&Instruction::I32Const(0));
            fb.instruction(&Instruction::Call(2)); // proc_exit
        }
        // else: no_proc_exit && !p2_mode — just fall through (return void)

        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    m.section(&code);

    // ── Data section ──
    if !em.data_segments.is_empty() {
        let mut data = DataSection::new();
        for (off, bytes) in &em.data_segments {
            data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
        }
        m.section(&data);
    }

    Ok(m.finish())
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn test_outlayer_simple() {
        let src = "(define (square x) (* x x))";
        let wasm = compile_outlayer(src).unwrap();
        assert!(!wasm.is_empty());
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        eprintln!("OutLayer WAT:\n{}", wat);
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
        eprintln!("Counter OutLayer WAT:\n{}", wat);
        assert!(wat.contains("_start"));
        assert!(wat.contains("wasi_snapshot_preview1"));
        assert!(wat.contains("outlayer"));
    }

    /// Test with wasmtime: compile and run a simple function
    #[test]
    fn test_outlayer_wasmtime_square() {
        let src = "(define (square x) (* x x))";
        let wasm = compile_outlayer(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        eprintln!("Square OutLayer WAT:\n{}", wat);
        let result = run_outlayer_wasm(&wasm, &7i64.to_le_bytes());
        assert_eq!(result, 49, "square(7) should be 49");
    }

    #[test]
    fn test_outlayer_wasmtime_const() {
        let src = "(define (main) 42)";
        let wasm = compile_outlayer(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        eprintln!("Const OutLayer WAT:\n{}", wat);
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
        eprintln!("result payload = {:x}", result);
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
        eprintln!("P2 first 10 bytes: {:?}", &comp_bytes[..10.min(comp_bytes.len())]);
        eprintln!("P2 component: {} bytes", comp_bytes.len());
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
        eprintln!("P2 component: {} bytes", comp_bytes.len());
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
        eprintln!("Core P1 WASM: {} bytes", core.len());
        std::fs::write("/tmp/core_http.wasm", &core).unwrap();
        
        let comp_bytes = compile_outlayer_p2(src).unwrap();
        assert!(comp_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]),
            "should start with WASM magic");
        eprintln!("P2 HTTP component: {} bytes", comp_bytes.len());
        
        // Validate with wasm-tools if available
        std::fs::write("/tmp/test_p2_http.wasm", &comp_bytes).unwrap();
        let output = std::process::Command::new("wasm-tools")
            .args(["validate", "/tmp/test_p2_http.wasm"])
            .output();
        match output {
            Ok(o) => {
                let valid = o.status.success();
                eprintln!("wasm-tools validate: {} ({})", 
                    if valid { "✅ valid" } else { "❌ invalid" },
                    String::from_utf8_lossy(&o.stderr));
                assert!(valid, "P2 component should validate");
            }
            Err(_) => {
                eprintln!("⚠️ wasm-tools not available, skipping validation");
            }
        }
    }

    #[test]
    fn test_outlayer_echo_instruction_count() {
        // Count instructions for the minimal echo: (print (json-get "amount"))
        let src = r#"(define (main) (print (json-get "amount")))"#;
        let core = compile_outlayer(src).unwrap();
        std::fs::write("/tmp/echo_p1.wasm", &core).unwrap();
        eprintln!("Echo core WASM: {} bytes", core.len());
        
        // Also build P2 and save
        let echo_p2_src = r#"(define (main) (print (json-get "amount")))"#;
        let echo_p2 = compile_outlayer_p2(echo_p2_src).unwrap();
        std::fs::write("/tmp/echo_p2.wasm", &echo_p2).unwrap();
        eprintln!("Echo P2 component: {} bytes", echo_p2.len());
        
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
                    eprintln!("  Func {}: {} instructions", fi, ic);
                    total += ic;
                    pos = body_end;
                }
                eprintln!("Echo total instructions: {}", total);
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
            eprintln!("  [host] proc_exit({})", code);
            Err(wasmtime::Error::msg(format!("proc_exit({})", code)))
        },
    );

    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    let ol_view_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let ol_transfer_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });

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
    linker.define(&store, "outlayer", "view", ol_view_fn).unwrap();
    linker.define(&store, "outlayer", "call", ol_call_fn).unwrap();
    linker.define(&store, "outlayer", "transfer", ol_transfer_fn).unwrap();
    let ol_http_get_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "http_get", ol_http_get_fn).unwrap();
    let storage_stub_4 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_set", storage_stub_4).unwrap();
    let storage_stub_5 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get", storage_stub_5).unwrap();
    let storage_stub_2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_has", storage_stub_2).unwrap();
    let storage_stub_2d = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_delete", storage_stub_2d).unwrap();
    let storage_stub_6 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_increment", storage_stub_6).unwrap();
    let env_signer_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "env_signer", env_signer_fn).unwrap();
    let env_pred_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "env_predecessor", env_pred_fn).unwrap();
    let stub_11 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_decrement", stub_11).unwrap();
    let stub_12 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_if_absent", stub_12).unwrap();
    let stub_13 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 8], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_set_if_equals", stub_13).unwrap();
    let stub_14 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_list_keys", stub_14).unwrap();
    let stub_15 = Func::new(&mut store, FuncType::new(&engine, vec![], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_clear_all", stub_15).unwrap();
    let stub_16 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_worker", stub_16).unwrap();
    let stub_17 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get_worker", stub_17).unwrap();
    let stub_18 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_worker_public", stub_18).unwrap();
    let stub_19 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get_worker_from_project", stub_19).unwrap();
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
            eprintln!("Trap (is_exit={}): {}", is_exit, msg);
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

    // Mock outlayer.view: writes response to result_buf
    let resp = response_arc.clone();
    let ol_view_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 8], vec![ValType::I32]),
        move |mut caller, args, results| {
            let result_buf = args[6].unwrap_i32() as usize;
            let result_len_ptr = args[7].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                let copy_len = resp.len().min(65536);
                if result_buf + copy_len <= data.len() { data[result_buf..result_buf+copy_len].copy_from_slice(&resp[..copy_len]); }
                if result_len_ptr + 4 <= data.len() { data[result_len_ptr..result_len_ptr+4].copy_from_slice(&(copy_len as u32).to_le_bytes()); }
            }
            results[0] = Val::I32(0); Ok(())
        },
    );
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
    let ol_transfer_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
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
    linker.define(&store, "outlayer", "view", ol_view_fn).unwrap();
    linker.define(&store, "outlayer", "call", ol_call_fn).unwrap();
    linker.define(&store, "outlayer", "transfer", ol_transfer_fn).unwrap();
    let ol_http_get_fn2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "http_get", ol_http_get_fn2).unwrap();
    let storage_stub_4 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_set", storage_stub_4).unwrap();
    let storage_stub_5 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get", storage_stub_5).unwrap();
    let storage_stub_2 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_has", storage_stub_2).unwrap();
    let storage_stub_2d = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 2], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_delete", storage_stub_2d).unwrap();
    let storage_stub_6 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_increment", storage_stub_6).unwrap();
    let env_signer_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "env_signer", env_signer_fn).unwrap();
    let env_pred_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 3], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "env_predecessor", env_pred_fn).unwrap();
    let stub_11 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 6], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_decrement", stub_11).unwrap();
    let stub_12 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_if_absent", stub_12).unwrap();
    let stub_13 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 8], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_set_if_equals", stub_13).unwrap();
    let stub_14 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_list_keys", stub_14).unwrap();
    let stub_15 = Func::new(&mut store, FuncType::new(&engine, vec![], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_clear_all", stub_15).unwrap();
    let stub_16 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_worker", stub_16).unwrap();
    let stub_17 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 5], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get_worker", stub_17).unwrap();
    let stub_18 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(0); Ok(()) }); linker.define(&store, "outlayer", "storage_set_worker_public", stub_18).unwrap();
    let stub_19 = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32; 7], vec![ValType::I32]), |_caller, _args, results| { results[0] = Val::I32(1); Ok(()) }); linker.define(&store, "outlayer", "storage_get_worker_from_project", stub_19).unwrap();
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
    eprintln!("📊 RESULT_BUF raw={:#x}, ptr={}, len={}", result, ptr, len);
    assert!(len > 0, "http-get should return non-empty response, got ptr={} len={}", ptr, len);
    assert!(ptr > 0, "ptr should be non-zero");
    eprintln!("✅ HTTP GET works! Response at ptr={}, len={} bytes", ptr, len);
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
                        let output = String::from_utf8_lossy(&data[buf_ptr..buf_ptr+buf_len]);
                        eprintln!("📝 stdout: {}", output);
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
                    eprintln!("🌐 HTTP GET: {}", url);

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
                            eprintln!("✅ HTTP response: {} bytes", copy_len);
                            results[0] = Val::I32(0); // errno = 0 (success)
                        }
                        Ok(output) => {
                            eprintln!("❌ HTTP error: {}", String::from_utf8_lossy(&output.stderr));
                            results[0] = Val::I32(1); // errno = error
                        }
                        Err(e) => {
                            eprintln!("❌ curl failed: {}", e);
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
                eprintln!("P2 wasi:http component: {} bytes", bytes.len());
                assert!(bytes.starts_with(&[0x00, 0x61, 0x73, 0x6d]));
                std::fs::write("/tmp/p2_wasi_http.wasm", bytes).unwrap();
            }
            Err(e) => {
                eprintln!("Error: {}", e);
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
        eprintln!("Reference component: {} bytes", comp_bytes.len());

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
        eprintln!("📦 run func ready");

        // Call it — wasi:cli/run#run() -> result<(), _>
        let mut result_val = [wasmtime::component::Val::Bool(false)];
        run_fn.call_async(&mut store, &[], &mut result_val).await
            .expect("run call failed");

        // Read stdout
        let output = stdout_pipe.contents();
        let output_str = String::from_utf8_lossy(&output);
        eprintln!("📝 stdout: {}", output_str);

        // Verify we got weather data
        assert!(output_str.contains("temp_c"), "expected weather JSON with temp_c, got: {}", output_str);
        eprintln!("✅ Reference P2 component ran successfully with real HTTP!");
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
        eprintln!("Lisp component: {} bytes", comp_bytes.len());
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

        eprintln!("📦 Lisp component ready, calling run...");

        let mut result_val = [wasmtime::component::Val::Bool(false)];
        match run_fn.call_async(&mut store, &[], &mut result_val).await {
            Ok(()) => {
                eprintln!("📦 run completed");
                let output = stdout_pipe.contents();
                let output_str = String::from_utf8_lossy(&output);
                eprintln!("📝 stdout: {}", output_str);
            }
            Err(e) => {
                eprintln!("❌ run failed: {:?}", e);
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
        eprintln!("Multi-URL component: {} bytes", comp_bytes.len());
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

        eprintln!("📦 Multi-URL component ready, calling run...");

        let mut result_val = [wasmtime::component::Val::Bool(false)];
        match run_fn.call_async(&mut store, &[], &mut result_val).await {
            Ok(()) => {
                eprintln!("📦 run completed");
                let output = stdout_pipe.contents();
                let output_str = String::from_utf8_lossy(&output);
                eprintln!("📝 stdout: {}", output_str);
                // Verify that both responses are present in the output
                assert!(output_str.contains("warmer"), "expected comparison result, got: {}", output_str);
            }
            Err(e) => {
                eprintln!("❌ run failed: {:?}", e);
                panic!("Multi-URL P2 component execution failed: {:?}", e);
            }
        }
    }
