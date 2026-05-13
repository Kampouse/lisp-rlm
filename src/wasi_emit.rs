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

    // NEAR-style host function types (for NEAR compat stubs)
    // We need types for each unique NEAR host function signature used
    // Map NEAR host func index to its type index
    let _ = &near_host_used; // used below
    // User function types: each function has (i64 × param_count) -> i64
    let max_p = em.funcs.iter().map(|f| f.param_count).max().unwrap_or(0);
    let user_type_base: u32 = 10;
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
    for (i, f) in ol.iter().enumerate() {
        let type_idx = 7 + i as u32; // types 7, 8, 9
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
        let mut fb = Function::new(locals);
        for instr in &resolved { fb.instruction(instr); }
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // ── _start() wrapper ──
    // Reads stdin into STDIN_BUF, calls last user function, writes result to stdout
    {
        let last_func = em.funcs.last().unwrap();
        let last_idx = internal_base + (em.funcs.len() - 1) as u32;
        let param_count = last_func.param_count;
        
        // Locals: stdin_len (i32), errno (i32), i (i32), tmp (i64)
        let mut fb = Function::new(vec![
            (1u32, W),  // local 0: stdin_len (i32)
            (1u32, W),  // local 1: errno (i32) 
            (1u32, W),  // local 2: loop counter (i32)
            (1u32, ValType::I64), // local 3: result/tmp (i64)
        ]);

        // fd_read(0, iov_ptr, 1, &stdin_len)
        // iov structure at offset 0: [buf_ptr:i32, buf_len:i32] = [STDIN_BUF, 65536]
        // Store iov at TEMP_MEM (offset 64)
        let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        
        // iov[0].buf = STDIN_BUF
        fb.instruction(&Instruction::I64Const(64)); // TEMP_MEM offset
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(STDIN_BUF as i32));
        fb.instruction(&Instruction::I32Store(ma4));
        // iov[0].len = 65536
        fb.instruction(&Instruction::I64Const(64 + 4));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(65536));
        fb.instruction(&Instruction::I32Store(ma4));

        // fd_read(0, 64, 1, STDIN_LEN)
        fb.instruction(&Instruction::I32Const(0)); // fd=stdin
        fb.instruction(&Instruction::I64Const(64)); // iovs_ptr
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(1)); // iovs_len=1
        fb.instruction(&Instruction::I64Const(STDIN_LEN)); // nread_ptr
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::Call(0)); // fd_read (import 0)
        fb.instruction(&Instruction::Drop); // ignore errno for now

        // Load the actual bytes read
        fb.instruction(&Instruction::I64Const(STDIN_LEN));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::LocalSet(0)); // stdin_len

        // Call the last user function
        // For 0-param: just call
        // For N-param: load N i64s from STDIN_BUF (same pattern as NEAR input)
        if param_count == 0 {
            fb.instruction(&Instruction::Call(last_idx));
            fb.instruction(&Instruction::LocalSet(3)); // save result
        } else {
            // Load params from STDIN_BUF (raw i64 values, like NEAR input)
            for i in 0..param_count {
                fb.instruction(&Instruction::I64Const(STDIN_BUF + (i as i64) * 8));
                fb.instruction(&Instruction::I32WrapI64);
                fb.instruction(&Instruction::I64Load(ma));
                // Tag as Num: (val << 3) | 0
                fb.instruction(&Instruction::I64Const(3));
                fb.instruction(&Instruction::I64Shl);
            }
            fb.instruction(&Instruction::Call(last_idx));
            fb.instruction(&Instruction::LocalSet(3));
        }

        // Untag result and store at RESULT_BUF
        fb.instruction(&Instruction::I64Const(RESULT_BUF));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::LocalGet(3));
        fb.instruction(&Instruction::I64Const(3));
        fb.instruction(&Instruction::I64ShrS); // untag
        fb.instruction(&Instruction::I64Store(ma));

        // fd_write(1, iov_ptr, 1, &written)
        // Build iov for stdout: [RESULT_BUF, 8]
        fb.instruction(&Instruction::I64Const(64));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(RESULT_BUF as i32));
        fb.instruction(&Instruction::I32Store(ma4));
        fb.instruction(&Instruction::I64Const(64 + 4));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(8)); // 8 bytes output
        fb.instruction(&Instruction::I32Store(ma4));

        // fd_write(1, 64, 1, STDIN_LEN) — reuse STDIN_LEN for nwritten
        fb.instruction(&Instruction::I32Const(1)); // fd=stdout
        fb.instruction(&Instruction::I64Const(64));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::I32Const(1));
        fb.instruction(&Instruction::I64Const(STDIN_LEN));
        fb.instruction(&Instruction::I32WrapI64);
        fb.instruction(&Instruction::Call(1)); // fd_write (import 1)
        fb.instruction(&Instruction::Drop);

        // proc_exit(0)
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::Call(2)); // proc_exit (import 2)

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
        0 | 3 | 7 | 25 | 27 | 28 | 29 | 35 => i64_to_void, // read_register, current_account_id, etc
        // (i64) -> i64  
        1 | 12 | 13 | 14 | 20 => i64_to_i64,
        // (i64, i64) -> ()
        2 | 19 | 21 | 22 | 36 => _2i64_to_void, // write_register, storage_remove, sha256, etc
        // (i64, i64) -> i64
        18 | 32 | 39 | 40 => _2i64_to_i64,
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
}
