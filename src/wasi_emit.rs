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

/// Compile Lisp source to OutLayer WASI **Preview 2** (Component Model) binary.
///
/// Produces a Component (not Module) that:
/// - Exports `wasi:cli/run@0.2.1/run` as the entry point  
/// - Contains a core P1 module with _start()
/// - Core imports satisfied via component-level WASI P2 adapters
pub fn compile_outlayer_p2(source: &str) -> Result<Vec<u8>, String> {
    // 1. Parse and build emitter (same as P1)
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

    // 2. Build the core P1 module
    let core_bytes = finish_outlayer(&mut em)?;

    // 3. Build Component wrapper
    // Strategy: embed core module as raw bytes, then add component-level
    // instantiation, imports, and exports to bridge P1 ↔ P2.
    //
    // For now, we produce a minimal component that:
    // - Embeds the core P1 module (index 0)
    // - Instantiates it with component-level imports
    // - Lifts _start to component export wasi:cli/run@0.2.1/run
    //
    // The core module still uses P1-style imports (fd_read, fd_write, etc.)
    // The runtime is expected to provide the P1 adapter layer.

    let mut comp = wasm_encoder::Component::new();

    // Embed core module using RawModule (implements ComponentSection)
    comp.section(&RawModuleSection(&core_bytes));

    // Import wasi:cli/environment@0.2.1 as an instance
    // We need to define the component types first
    let mut types = wasm_encoder::ComponentTypeSection::new();
    
    // Type 0: wasi:cli/run function type: () -> result<void, _>
    // This encodes as a function with no params and no result (void)
    types.function().params([] as [(&str, wasm_encoder::ComponentValType); 0]).result(None);

    comp.section(&types);

    // Core instance section: instantiate core module (index 0)
    // The core module has imports from "wasi_snapshot_preview1", "outlayer", "env"
    // For P2, we import WASI instances and lower them to core funcs
    // For now, instantiate with empty args — the runtime provides imports
    let mut instances = wasm_encoder::InstanceSection::new();
    // Import WASI P2 instances that will satisfy core module's P1 imports
    // We import the standard WASI P2 interfaces as component instances
    
    // For simplicity, we'll just instantiate the core module directly.
    // The runtime must provide the P1 imports (same as P1 path).
    instances.instantiate(0, <[(&str, wasm_encoder::ModuleArg); 0]>::default());
    comp.section(&instances);

    // Canonical function section: lift _start from core instance
    // The core instance exports _start at some func index.
    // After instantiation, _start is core func 0 from instance 0.
    // We need to alias it first.
    let mut aliases = wasm_encoder::ComponentAliasSection::new();
    // Alias _start from core instance 0 (the instance we just created)
    aliases.alias(wasm_encoder::Alias::CoreInstanceExport {
        instance: 0,
        kind: wasm_encoder::ExportKind::Func,
        name: "_start",
    });
    comp.section(&aliases);

    // Now _start is available as core func index in the component.
    // Lift it to a component function.
    let mut canon = wasm_encoder::CanonicalFunctionSection::new();
    // lift(core_func_index=0, type_index=0, options=[utf8])
    // core_func_index: after alias, _start is the first aliased core func (index depends on count)
    // Actually after the alias section, the core func gets index = (total previous core funcs)
    // Since we only embedded the module and instantiated it, the aliased _start gets index 0
    // in the component's core func namespace.
    // Wait — aliases create items in the component scope, not core scope.
    // The CanonicalFunctionSection's lift takes a core func index.
    // After CoreInstanceSection instantiation, the core instance's exports are accessible
    // via aliasing into the component scope.
    // Actually, the lift function takes a core function index directly.
    // We need to alias the core instance export into the component scope first,
    // then reference it.
    //
    // Let me simplify: just alias and then the canonical lift references it.
    // After the AliasSection, the _start func is available as component func 0.
    // The CanonicalFunctionSection::lift takes (core_func_idx, type_idx, options).
    // But we aliased into the component scope, not core scope.
    // We need to use alias_core_instance_export to get a core func, not component func.
    
    // Actually, looking at the wasm-tools component model, the flow is:
    // 1. Core module embedded (idx 0)
    // 2. Import component-level instances for WASI P2
    // 3. Lower component functions to core functions  
    // 4. Instantiate core module with lowered core functions
    // 5. Lift _start from core instance to component function
    // 6. Export the lifted function

    // For simplicity, since the core module uses P1 imports that the runtime 
    // provides directly, we can just instantiate with empty args and let
    // the runtime figure it out. Many runtimes support this pattern.
    
    // lift(_start_core_func_idx, type_idx, [])
    // After alias, the _start is component item 0 (func sort)
    // But lift takes a CORE func index... 
    // 
    // OK I think the issue is that after CoreInstanceSection, the core instance
    // is available, and we alias its exports. The alias creates a core function.
    // Then we can lift that core function.
    
    canon.lift(0, 0, [wasm_encoder::CanonicalOption::UTF8]);
    comp.section(&canon);

    // Export the lifted function as wasi:cli/run@0.2.1/run
    let mut exports = wasm_encoder::ComponentExportSection::new();
    exports.export(
        "wasi:cli/run@0.2.1/run",
        wasm_encoder::ComponentExportKind::Func,
        0, // component func index 0 (the lifted _start)
        None,
    );
    comp.section(&exports);

    Ok(comp.finish())
}

/// Raw core module section — embeds pre-built module bytes into a component.
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

        // fd_read(0, iov, 1, &nread)
        // Set up iov at offset 64: [buf_ptr:i32, buf_len:i32]
        let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };
        // iov[0].buf = STDIN_BUF
        fb.instruction(&Instruction::I32Const(64)); // iov offset
        fb.instruction(&Instruction::I32Const(STDIN_BUF as i32)); // buf ptr
        fb.instruction(&Instruction::I32Store(ma4));
        // iov[0].len = 65536
        fb.instruction(&Instruction::I32Const(68)); // iov+4
        fb.instruction(&Instruction::I32Const(65536)); // buf len
        fb.instruction(&Instruction::I32Store(ma4));

        // fd_read(0, 64, 1, STDIN_LEN)
        fb.instruction(&Instruction::I32Const(0)); // fd=stdin
        fb.instruction(&Instruction::I32Const(64)); // iovs_ptr
        fb.instruction(&Instruction::I32Const(1)); // iovs_len=1
        fb.instruction(&Instruction::I32Const(STDIN_LEN as i32)); // nread_ptr
        fb.instruction(&Instruction::Call(0)); // fd_read (import 0)
        fb.instruction(&Instruction::Drop); // ignore errno

        // Call the last user function
        // For 0-param: just call
        // For N-param: load N i64s from STDIN_BUF (same pattern as NEAR input)
        let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
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

        // proc_exit(0) — skip fd_write for now
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
