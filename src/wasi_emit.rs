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
            params: vec![W; 14], results: vec![W] },
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

/// Compile Lisp source to OutLayer WASI binary.
///
/// Produces a WASM module that:
/// - Exports `_start()` (WASI entry point)
/// - Reads input from stdin → calls last defined function → writes result to stdout
/// - Uses WASI P1 for I/O, random, env vars
/// - Uses OutLayer host functions for NEAR RPC (view/call/transfer)
pub fn compile_outlayer(source: &str) -> Result<Vec<u8>, String> {
    let resolved = crate::wasm_emit::resolve_modules(source, std::path::Path::new("."))?;
    let exprs = crate::parser::parse_all(&resolved)?;
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
/// - Lowers them to core functions that satisfy P1 imports (fd_read, fd_write, etc.)
/// - Instantiates core P1 module with lowered functions
/// - Lifts `_start` and exports as `wasi:cli/run@0.2.1/run`
pub fn compile_outlayer_p2(source: &str) -> Result<Vec<u8>, String> {
    // 1. Compile the core P1 module first
    let resolved = crate::wasm_emit::resolve_modules(source, std::path::Path::new("."))?;
    let exprs = crate::parser::parse_all(&resolved)?;
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

    let mut core_bytes = finish_outlayer(&mut em)?;

    // 2. Use wit-component to wrap the core module into a P2 component
    // First, define the WIT world for our component
    use wit_parser::{Resolve};
    
    // Define WIT world inline
    let wit_source = r#"
package outlayer:host;

interface api {
    view: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32, g: s32, h: s32) -> s32;
    call: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32, g: s32, h: s32, i: s32, j: s32, k: s32, l: s32, m: s32) -> s32;
    transfer: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32, g: s32, h: s32, i: s32) -> s32;
    http-get: func(a: s32, b: s32, c: s32, d: s32, e: s32) -> s32;
    storage-set: func(a: s32, b: s32, c: s32, d: s32) -> s32;
    storage-get: func(a: s32, b: s32, c: s32, d: s32, e: s32) -> s32;
    storage-has: func(a: s32, b: s32) -> s32;
    storage-delete: func(a: s32, b: s32) -> s32;
    storage-increment: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32) -> s32;
    storage-decrement: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32) -> s32;
    env-signer: func(a: s32, b: s32, c: s32) -> s32;
    env-predecessor: func(a: s32, b: s32, c: s32) -> s32;
    storage-set-if-absent: func(a: s32, b: s32, c: s32, d: s32) -> s32;
    storage-set-if-equals: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32, g: s32, h: s32) -> s32;
    storage-list-keys: func(a: s32, b: s32, c: s32, d: s32, e: s32) -> s32;
    storage-clear-all: func() -> s32;
    storage-set-worker: func(a: s32, b: s32, c: s32, d: s32) -> s32;
    storage-get-worker: func(a: s32, b: s32, c: s32, d: s32, e: s32) -> s32;
    storage-set-worker-public: func(a: s32, b: s32, c: s32, d: s32) -> s32;
    storage-get-worker-from-project: func(a: s32, b: s32, c: s32, d: s32, e: s32, f: s32, g: s32) -> s32;
}

world outlayer-world {
    import api;
    export run: func() -> result;
}
"#;

    // Parse the WIT source
    let mut resolve = Resolve::new();
    let pkg = resolve.push_source("outlayer.wit", wit_source)
        .map_err(|e| format!("WIT parse failed: {}", e))?;
    
    // Find the world
    let worlds = &resolve.packages[pkg].worlds;
    eprintln!("Available worlds: {:?}", worlds);
    let world_id = worlds.iter()
        .find(|(name, _)| *name == "outlayer-world")
        .map(|(_, &id)| id)
        .ok_or_else(|| format!("world 'outlayer-world' not found in: {:?}", worlds.keys().collect::<Vec<_>>()))?;
    
    // Embed component metadata into the core module
    wit_component::embed_component_metadata(
        &mut core_bytes,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    ).map_err(|e| format!("embed failed: {}", e))?;
    eprintln!("Embedded component metadata, module size now: {} bytes", core_bytes.len());
    
    // Load the WASI adapter
    let adapter_bytes = include_bytes!("../../lisp-rlm/wasi_adapter.wasm");
    
    // Create the component encoder
    let mut encoder = wit_component::ComponentEncoder::default()
        .module(&core_bytes)
        .map_err(|e| format!("encoder module failed: {}", e))?
        .adapter("wasi_snapshot_preview1", adapter_bytes)
        .map_err(|e| format!("encoder adapter failed: {}", e))?
        .validate(false)
        .realloc_via_memory_grow(true);
    
    let component_bytes = encoder.encode()
        .map_err(|e| format!("encode failed: {}", e))?;
    
    Ok(component_bytes)
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
    if em.funcs.is_empty() {
        return Err("no functions defined".into());
    }

    em.tree_shake();

    let wasi = wasi_p1_imports();
    let ol = outlayer_imports();
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
    // type 0: () -> () — for _start, proc_exit, etc
    types.ty().function([], []);
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
    // type 8: call — 14 i32 params -> i32  
    types.ty().function(vec![W; 14], [W]);
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
    // 4 pages = 256KB (need room for stdin/stdout buffers)
    let pages = em.memory_pages.max(4) as u64;
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
    exps.export("_start", ExportKind::Func, start_func_idx);
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

        // proc_exit(0)
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::Call(2)); // proc_exit

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

#[cfg(test)]
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
    }

    #[test]
    fn test_outlayer_p2_square() {
        let src = "(define (square x) (* x x))";
        let comp_bytes = compile_outlayer_p2(src).unwrap();
        assert!(comp_bytes.starts_with(&[0x00, 0x61, 0x73, 0x6D]),
            "should start with WASM magic");
        eprintln!("P2 component: {} bytes", comp_bytes.len());
    }
}

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
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
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
    let ol_call_fn = Func::wrap(&mut store, |_: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { 0 });
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
