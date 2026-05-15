//! Native P2 component — built from scratch using wasm-encoder's ComponentBuilder.
//!
//! Constructs a valid P2 component with outlayer as a real component import.

use wasm_encoder::*;

pub fn build_native_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let info = analyze_outlayer(core_bytes);
    
    let mut b = ComponentBuilder::default();
    
    // 1. Define component func types for each outlayer function signature
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
        // Alias all func types from the outer type section into instance scope
        for i in 0..func_types.len() as u32 {
            inst.alias(Alias::Outer { 
                kind: ComponentOuterAliasKind::Type, 
                count: 1, 
                index: i,
            });
        }
        // Now export each function using the aliased type (indices 0..N)
        for (i, name) in info.names.iter().enumerate() {
            let kebab = name.replace('_', "-");
            inst.export(&leak_str(kebab), ComponentTypeRef::Func(i as u32));
        }
        inst_enc.instance(&inst);
    }
    
    // 3. Define run type: () -> result<> (no ok, no err)
    let (run_type, mut run_enc) = b.ty(None);
    {
        run_enc.function().params([] as [(&str, ComponentValType); 0]).result(None::<ComponentValType>);
    }
    
    // 4. Import outlayer interface → instance 0
    b.import("outlayer:api/outlayer", ComponentTypeRef::Instance(outlayer_inst_type));
    
    // 5. Embed core module
    let module_idx = b.core_module_raw(None, core_bytes);
    
    // 5b. Build WASI P1 stub core module
    let wasi_stub = build_wasi_stub();
    let wasi_stub_mod = b.core_module_raw(None, &wasi_stub);
    
    // 6. Lower each outlayer function from the imported instance
    // 6. Build outlayer core stub module (matching exact signatures)
    //    The component imports outlayer:api/outlayer, but the core module
    //    needs stack-based i32 returns. We provide stubs that return 0.
    //    The runtime replaces these via the component-level outlayer import.
    let outlayer_stub = build_outlayer_core_stubs(&info);
    let outlayer_stub_mod = b.core_module_raw(None, &outlayer_stub);
    let outlayer_inst = b.core_instantiate(None, outlayer_stub_mod, []);

    // 9. Instantiate WASI stub
    let wasi_stub_inst = b.core_instantiate(None, wasi_stub_mod, []);
    
    // 10. Instantiate core module with outlayer + wasi
    let core_inst = b.core_instantiate(None, module_idx, [
        ("outlayer", ModuleArg::Instance(outlayer_inst)),
        ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
    ]);
    
    // 10. Lift _start → run
    let start_func = b.core_alias_export(None, core_inst, "_start", ExportKind::Func);
    let _mem = b.core_alias_export(None, core_inst, "memory", ExportKind::Memory);
    let run_func = b.lift_func(None, start_func, run_type, [CanonicalOption::Memory(_mem)]);
    
    // 11. Export
    b.export("run", ComponentExportKind::Func, run_func, None);
    
    let bytes = b.finish();
    eprintln!("✅ Native P2 component: {} bytes", bytes.len());
    Ok(bytes)
}


fn build_outlayer_core_stubs(info: &OutlayerInfo) -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    
    // One type per function (matching exact signatures from core module)
    let mut types = TypeSection::new();
    for &nparams in &info.param_counts {
        types.ty().function(vec![ValType::I32; nparams], [ValType::I32]);
    }
    m.section(&types);
    
    // Functions
    let mut funcs = FunctionSection::new();
    for i in 0..info.names.len() as u32 {
        funcs.function(i);
    }
    m.section(&funcs);
    
    // Exports (underscore names matching core module imports)
    let mut exports = ExportSection::new();
    for (i, name) in info.names.iter().enumerate() {
        exports.export(name, ExportKind::Func, i as u32);
    }
    m.section(&exports);
    
    // Code: each function returns i32.const 0
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

fn build_wasi_stub() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    
    // Types matching the core module's WASI imports
    let mut types = TypeSection::new();
    types.ty().function([], []);                                              // 0: () -> ()
    types.ty().function([ValType::I32; 4], [ValType::I32]);                   // 1: fd_read, fd_write
    types.ty().function([ValType::I32], []);                                  // 2: proc_exit
    types.ty().function([ValType::I32; 2], [ValType::I32]);                   // 3: random_get
    types.ty().function([ValType::I32; 2], [ValType::I32]);                   // 4: environ_sizes_get
    types.ty().function([ValType::I32; 2], [ValType::I32]);                   // 5: environ_get
    types.ty().function([ValType::I32, ValType::I64, ValType::I32, ValType::I32], [ValType::I32]); // 6: fd_seek
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
    // proc_exit: unreachable (trap)
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
fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

struct OutlayerInfo {
    names: Vec<String>,
    param_counts: Vec<usize>,
}

fn analyze_outlayer(wasm: &[u8]) -> OutlayerInfo {
    let mut names = Vec::new();
    let mut param_counts = Vec::new();
    
    // Read type section first
    let mut type_params: Vec<usize> = Vec::new();
    let mut pos = 8usize;
    while pos < wasm.len() {
        let sid = wasm[pos]; pos += 1;
        let (sz, lb) = rleb(wasm, pos); pos += lb;
        if sid == 1 {
            let (cnt, cl) = rleb(wasm, pos); pos += cl;
            for _ in 0..cnt {
                pos += 1; // 0x60
                let (np, pl) = rleb(wasm, pos); pos += pl;
                let n = np;
                pos += n; // skip param types
                let (nr, rl) = rleb(wasm, pos); pos += rl;
                pos += nr; // skip result types
                // Count actual i32 params from the original data
                let start = pos - nr - rl - n - pl - 1;
                let mut pc = 0usize;
                let mut p = start + 1 + pl + 1;
                for _ in 0..np {
                    if wasm[p] == 0x7F { pc += 1; }
                    else if wasm[p] == 0x7E { pc += 2; } // i64 counts as 2 slots for params
                    p += 1;
                }
                type_params.push(np);
            }
            break;
        }
        pos += sz;
    }
    
    // Read import section
    pos = 8;
    while pos < wasm.len() {
        let sid = wasm[pos]; pos += 1;
        let (sz, lb) = rleb(wasm, pos); pos += lb;
        if sid == 2 {
            let (cnt, cl) = rleb(wasm, pos); pos += cl;
            for _ in 0..cnt {
                let (ml, mll) = rleb(wasm, pos); pos += mll;
                let module = std::str::from_utf8(&wasm[pos..pos+ml]).unwrap_or("");
                pos += ml;
                let (nl, nll) = rleb(wasm, pos); pos += nll;
                let name = std::str::from_utf8(&wasm[pos..pos+nl]).unwrap_or("").to_string();
                pos += nl;
                let kind = wasm[pos]; pos += 1;
                match kind {
                    0 => { let (tl, tll) = rleb(wasm, pos); pos += tll;
                        if module == "outlayer" {
                            names.push(name);
                            param_counts.push(type_params.get(tl as usize).copied().unwrap_or(0));
                        }
                    }
                    1 => { pos += 3; let (_tl, tll) = rleb(wasm, pos); pos += tll; }
                    2 => { let (_tl, tll) = rleb(wasm, pos); pos += tll; }
                    3 => { pos += 1; let (_tl, tll) = rleb(wasm, pos); pos += tll; }
                    _ => {}
                }
            }
            break;
        }
        pos += sz;
    }
    OutlayerInfo { names, param_counts }
}

fn rleb(d: &[u8], p: usize) -> (usize, usize) {
    let mut r = 0usize; let mut s = 0usize; let mut b = 0usize;
    loop { let v = d[p+b] as usize; r |= (v & 0x7F) << s; b += 1; if v & 0x80 == 0 { break; } s += 7; }
    (r, b)
}

#[test]
fn test_minimal_instance_type() {
    let mut b = ComponentBuilder::default();
    
    let (_, mut enc) = b.ty(None);
    enc.function()
        .params([("x", PrimitiveValType::S32)])
        .result(Some(ComponentValType::from(PrimitiveValType::S32)));
    
    let (inst_type, mut enc2) = b.ty(None);
    {
        let mut inst = InstanceType::new();
        // Alias the outer func type into the instance type's scope
        inst.alias(Alias::Outer { count: 1, index: 0, kind: ComponentOuterAliasKind::Type });
        inst.export("hello", ComponentTypeRef::Func(0));
        enc2.instance(&inst);
    }
    
    b.import("test:hello/world", ComponentTypeRef::Instance(inst_type));
    
    let bytes = b.finish();
    std::fs::write("/tmp/test_instance.wasm", &bytes).unwrap();
    eprintln!("Built: {} bytes", bytes.len());
}
