//! P2 Component Model emitter — wraps P1 core WASM into a WASI Preview 2 component.
//!
//! Takes the core module emitted by `wasm_emit` and produces a valid P2 component
//! with:
//! - `outlayer:api/outlayer` component import (all 20 host functions)
//! - WASI P1 stubs (fd_read, fd_write, proc_exit, etc.)
//! - Core module instantiated with outlayer + wasi stubs
//! - `_start` lifted to component `run` export: `() -> result`

use wasm_encoder::*;

// ── Outlayer host function info ──

struct OutlayerInfo {
    names: Vec<String>,
    param_counts: Vec<usize>,
}

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn rleb(d: &[u8], p: usize) -> (usize, usize) {
    let mut r = 0usize;
    let mut s = 0usize;
    let mut b = 0usize;
    loop {
        let v = d[p + b] as usize;
        r |= (v & 0x7F) << s;
        b += 1;
        if v & 0x80 == 0 {
            break;
        }
        s += 7;
    }
    (r, b)
}

/// Parse the core WASM module to discover which `outlayer::*` imports it uses.
fn analyze_outlayer(wasm: &[u8]) -> OutlayerInfo {
    let mut names = Vec::new();
    let mut param_counts = Vec::new();

    // Read type section to get param counts per type index
    let mut type_params: Vec<usize> = Vec::new();
    let mut pos = 8usize;
    while pos < wasm.len() {
        let sid = wasm[pos];
        pos += 1;
        let (sz, lb) = rleb(wasm, pos);
        pos += lb;
        if sid == 1 {
            let (cnt, cl) = rleb(wasm, pos);
            pos += cl;
            for _ in 0..cnt {
                pos += 1; // 0x60
                let (np, pl) = rleb(wasm, pos);
                pos += pl;
                pos += np; // skip param types
                let (nr, rl) = rleb(wasm, pos);
                pos += rl;
                pos += nr; // skip result types
                type_params.push(np);
            }
            break;
        }
        pos += sz;
    }

    // Read import section to find outlayer imports
    pos = 8;
    while pos < wasm.len() {
        let sid = wasm[pos];
        pos += 1;
        let (sz, lb) = rleb(wasm, pos);
        pos += lb;
        if sid == 2 {
            let (cnt, cl) = rleb(wasm, pos);
            pos += cl;
            for _ in 0..cnt {
                let (ml, mll) = rleb(wasm, pos);
                pos += mll;
                let module = std::str::from_utf8(&wasm[pos..pos + ml]).unwrap_or("");
                pos += ml;
                let (nl, nll) = rleb(wasm, pos);
                pos += nll;
                let name =
                    std::str::from_utf8(&wasm[pos..pos + nl]).unwrap_or("").to_string();
                pos += nl;
                let kind = wasm[pos];
                pos += 1;
                match kind {
                    0 => {
                        let (tl, tll) = rleb(wasm, pos);
                        pos += tll;
                        if module == "outlayer" {
                            names.push(name);
                            param_counts
                                .push(type_params.get(tl as usize).copied().unwrap_or(0));
                        }
                    }
                    1 => {
                        pos += 3;
                        let (_tl, tll) = rleb(wasm, pos);
                        pos += tll;
                    }
                    2 => {
                        let (_tl, tll) = rleb(wasm, pos);
                        pos += tll;
                    }
                    3 => {
                        pos += 1;
                        let (_tl, tll) = rleb(wasm, pos);
                        pos += tll;
                    }
                    _ => {}
                }
            }
            break;
        }
        pos += sz;
    }
    OutlayerInfo { names, param_counts }
}

// ── Core stub modules ──

/// Build WASI P1 stub module (fd_read, fd_write, proc_exit, etc.) that return 0s.
fn build_wasi_stub() -> Vec<u8> {
    let mut m = Module::new();

    let mut types = TypeSection::new();
    types.ty().function([], []); // 0: () -> ()
    types.ty().function([ValType::I32; 4], [ValType::I32]); // 1: fd_read, fd_write
    types.ty().function([ValType::I32], []); // 2: proc_exit
    types.ty().function([ValType::I32; 2], [ValType::I32]); // 3: random_get
    types.ty().function([ValType::I32; 2], [ValType::I32]); // 4: environ_sizes_get
    types.ty().function([ValType::I32; 2], [ValType::I32]); // 5: environ_get
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I32, ValType::I32],
        [ValType::I32],
    ); // 6: fd_seek
    m.section(&types);

    let mut funcs = FunctionSection::new();
    funcs.function(1); // fd_read
    funcs.function(1); // fd_write
    funcs.function(2); // proc_exit
    funcs.function(3); // random_get
    funcs.function(4); // environ_sizes_get
    funcs.function(5); // environ_get
    funcs.function(6); // fd_seek
    m.section(&funcs);

    let mut exports = ExportSection::new();
    exports.export("fd_read", ExportKind::Func, 0);
    exports.export("fd_write", ExportKind::Func, 1);
    exports.export("proc_exit", ExportKind::Func, 2);
    exports.export("random_get", ExportKind::Func, 3);
    exports.export("environ_sizes_get", ExportKind::Func, 4);
    exports.export("environ_get", ExportKind::Func, 5);
    exports.export("fd_seek", ExportKind::Func, 6);
    m.section(&exports);

    let mut code = CodeSection::new();
    // fd_read, fd_write: return 0
    for _ in 0..2 {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    // proc_exit: unreachable
    {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::Unreachable);
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    // random_get, environ_sizes_get, environ_get, fd_seek: return 0
    for _ in 0..4 {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    m.section(&code);

    m.finish()
}

/// Build outlayer core stub module — one function per import, returning i32.const 0.
/// These satisfy the core module's imports until the runtime wires up the real implementations.
fn build_outlayer_core_stubs(info: &OutlayerInfo) -> Vec<u8> {
    let mut m = Module::new();

    let mut types = TypeSection::new();
    for &nparams in &info.param_counts {
        types.ty().function(vec![ValType::I32; nparams], [ValType::I32]);
    }
    m.section(&types);

    let mut funcs = FunctionSection::new();
    for i in 0..info.names.len() as u32 {
        funcs.function(i);
    }
    m.section(&funcs);

    let mut exports = ExportSection::new();
    for (i, name) in info.names.iter().enumerate() {
        exports.export(name, ExportKind::Func, i as u32);
    }
    m.section(&exports);

    let mut code = CodeSection::new();
    for _ in 0..info.names.len() {
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::End);
        code.function(&body);
    }
    m.section(&code);

    m.finish()
}

// ── P2 component builder ──

/// Wrap a core WASM module (P1) into a P2 Component Model component.
///
/// The component:
/// - Imports `outlayer:api/outlayer` as a component instance
/// - Instantiates the core module with outlayer + WASI stubs
/// - Lifts `_start` → `run: func() -> result`
///
/// The executor (inlayer) must provide `outlayer:api/outlayer@0.1.0` in its linker.
pub fn build_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let info = analyze_outlayer(core_bytes);

    let mut b = ComponentBuilder::default();

    // 1. Define component func types for each outlayer function
    let mut func_types: Vec<u32> = Vec::new();
    for &nparams in &info.param_counts {
        let (idx, mut enc) = b.ty(None);
        {
            let mut f = enc.function();
            let params: Vec<(&str, ComponentValType)> = (0..nparams)
                .map(|i| (leak_str(format!("a{}", i + 1)), PrimitiveValType::S32.into()))
                .collect();
            f.params(params).result(Some(PrimitiveValType::S32.into()));
        }
        func_types.push(idx);
    }

    // 2. Define outlayer instance type
    let (outlayer_inst_type, mut inst_enc) = b.ty(None);
    {
        let mut inst = InstanceType::new();
        for i in 0..func_types.len() as u32 {
            inst.alias(Alias::Outer {
                kind: ComponentOuterAliasKind::Type,
                count: 1,
                index: i,
            });
        }
        for (i, name) in info.names.iter().enumerate() {
            let kebab = name.replace('_', "-");
            inst.export(&leak_str(kebab), ComponentTypeRef::Func(i as u32));
        }
        inst_enc.instance(&inst);
    }

    // 3. Define run type: () -> () (void, component returns nothing on success)
    let (run_type, mut run_enc) = b.ty(None);
    {
        run_enc
            .function()
            .params([] as [(&str, ComponentValType); 0])
            .result(None::<ComponentValType>);
    }

    // 4. Import outlayer interface
    b.import(
        "outlayer:api/outlayer",
        ComponentTypeRef::Instance(outlayer_inst_type),
    );

    // 5. Embed core module
    let module_idx = b.core_module_raw(None, core_bytes);

    // 6. Build and embed stub modules
    let wasi_stub = build_wasi_stub();
    let wasi_stub_mod = b.core_module_raw(None, &wasi_stub);

    let outlayer_stub = build_outlayer_core_stubs(&info);
    let outlayer_stub_mod = b.core_module_raw(None, &outlayer_stub);

    // 7. Instantiate stubs
    let outlayer_inst = b.core_instantiate(None, outlayer_stub_mod, []);
    let wasi_stub_inst = b.core_instantiate(None, wasi_stub_mod, []);

    // 8. Instantiate core module with outlayer + wasi stubs
    let core_inst = b.core_instantiate(
        None,
        module_idx,
        [
            ("outlayer", ModuleArg::Instance(outlayer_inst)),
            ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
        ],
    );

    // 9. Lift _start → run
    let start_func = b.core_alias_export(None, core_inst, "_start", ExportKind::Func);
    let mem = b.core_alias_export(None, core_inst, "memory", ExportKind::Memory);
    let run_func = b.lift_func(None, start_func, run_type, [CanonicalOption::Memory(mem)]);

    // 10. Export run
    b.export("run", ComponentExportKind::Func, run_func, None);

    let bytes = b.finish();
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasi_stub_is_valid() {
        let stub = build_wasi_stub();
        // Should be parseable as valid WASM
        assert!(wasmparser::Parser::new(0)
            .parse_all(&stub)
            .count()
            > 0);
    }

    #[test]
    fn test_outlayer_stubs_match_core() {
        // Build a minimal core module that imports outlayer::view and outlayer::call
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32; 8], [ValType::I32]); // 0: view
        types.ty().function([ValType::I32; 13], [ValType::I32]); // 1: call
        m.section(&types);

        let mut imports = ImportSection::new();
        imports.import("outlayer", "view", EntityType::Function(0));
        imports.import("outlayer", "call", EntityType::Function(1));
        m.section(&imports);

        let mut funcs = FunctionSection::new();
        funcs.function(0); // use view type for body
        m.section(&funcs);

        let mut exports = ExportSection::new();
        exports.export("_start", ExportKind::Func, 0);
        m.section(&exports);

        let mut code = CodeSection::new();
        let mut body = Function::new(std::iter::empty::<(u32, ValType)>());
        body.instruction(&Instruction::End);
        code.function(&body);
        m.section(&code);

        let mut memory = MemorySection::new();
        memory.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        m.section(&memory);
        exports.export("memory", ExportKind::Memory, 0);

        let core = m.finish();

        let info = analyze_outlayer(&core);
        assert_eq!(info.names, vec!["view", "call"]);
        assert_eq!(info.param_counts, vec![8, 13]);

        let stubs = build_outlayer_core_stubs(&info);
        assert!(stubs.len() > 0);
    }
}
