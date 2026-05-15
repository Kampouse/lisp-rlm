//! Native P2 component — built from scratch using wasm-encoder's ComponentBuilder.
//!
//! Constructs a valid P2 component that:
//! - If core module uses outlayer: imports outlayer:api/outlayer component interface
//! - If core module is WASI-only: wraps with just WASI P1→P2 adapter
//! - Exports `run` via wasi:cli/run convention

use wasm_encoder::*;

pub fn build_native_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let info = analyze_outlayer(core_bytes);
    let has_outlayer = !info.names.is_empty();
    
    let mut b = ComponentBuilder::default();
    
    // ── Types for _start: () -> result<> ──
    // Define result<> type
    let (result_type, mut renc) = b.ty(None);
    {
        renc.defined_type().result(None, None);
    }
    let (run_type, mut run_enc) = b.ty(None);
    {
        run_enc.function().params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(result_type)));
    }

    if has_outlayer {
        // ── Outlayer component interface ──
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

        b.import("outlayer:api/outlayer", ComponentTypeRef::Instance(outlayer_inst_type));
    }

    // ── Embed core module ──
    let module_idx = b.core_module_raw(None, core_bytes);

    // ── Build WASI P1→P2 adapter (real adapter from wasmtime) ──
    let wasi_adapter_bytes = load_wasi_adapter();
    let wasi_stub_mod = b.core_module_raw(None, &wasi_adapter_bytes);

    let mut lowered_funcs: Vec<u32> = Vec::new();
    if has_outlayer {
        // Lower each outlayer function from the imported instance
        for (i, name) in info.names.iter().enumerate() {
            let kebab = name.replace('_', "-");
            let comp_func = b.alias_export(0, &leak_str(kebab), ComponentExportKind::Func);
            let core_func = b.lower_func(None, comp_func, []);
            lowered_funcs.push(core_func);
        }

        // Create outlayer core instance from lowered funcs
        let outlayer_inst = b.core_instantiate_exports(
            None,
            info.names.iter().zip(lowered_funcs.iter())
                .map(|(n, &idx)| (n.as_str(), ExportKind::Func, idx)),
        );
    }

    // ── Instantiate WASI stub ──
    let wasi_stub_inst = b.core_instantiate(None, wasi_stub_mod, []);

    // ── Instantiate core module with WASI (and optionally outlayer) ──
    let core_inst = if has_outlayer {
        let outlayer_inst_idx = 0u32;
        b.core_instantiate(None, module_idx, [
            ("outlayer", ModuleArg::Instance(outlayer_inst_idx)),
            ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
        ])
    } else {
        b.core_instantiate(None, module_idx, [
            ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
        ])
    };

    // ── Lift _start → run ──
    // _start returns i32 (0=success) when compiled for P2
    let start_func = b.core_alias_export(None, core_inst, "_start", ExportKind::Func);
    let mem = b.core_alias_export(None, core_inst, "memory", ExportKind::Memory);
    let run_func = b.lift_func(None, start_func, run_type, [CanonicalOption::Memory(mem)]);

    // ── Export wasi:cli/run@0.2.2 instance ──
    // Build a shim component that wraps our run function in the wasi:cli/run interface

    let mut shim = ComponentBuilder::default();

    // func type: () -> result<>  (result with no ok type = result<(), _> which maps to i32)
    let (shim_fn_type, mut sf_enc) = shim.ty(None);
    {
        let mut f = sf_enc.function();
        f.params([] as [(&str, ComponentValType); 0]);
        // result<> — encode as a defined type
    }

    // Define result<> as a defined type
    let (result_defined, mut rd_enc) = shim.ty(None);
    {
        rd_enc.defined_type().result(None, None);
    }

    // Now redefine the func type using the result type
    // Actually, let's use the component func type result directly
    drop(shim);

    // Simpler approach: build the shim as raw WASM bytes
    // The shim component: imports "import-func-run" () -> result<>, exports "run" () -> result<>
    let shim_bytes = build_run_shim();
    let shim_idx = b.component_raw(None, &shim_bytes);

    // Instantiate shim with our lifted run function
    let shim_instance = b.instantiate(
        None,
        shim_idx,
        [("import-func-run", ComponentExportKind::Func, run_func)],
    );

    // Export the shim instance as "wasi:cli/run@0.2.2"
    b.export("wasi:cli/run@0.2.2", ComponentExportKind::Instance, shim_instance, None);

    let bytes = b.finish();
    eprintln!("✅ Native P2 component: {} bytes (outlayer={})", bytes.len(), has_outlayer);
    Ok(bytes)
}

/// Load the WASI P1→P2 adapter module.
/// Looks for wasi_snapshot_preview1.command.wasm in known locations.
pub fn load_wasi_adapter() -> Vec<u8> {
    let candidates = [
        std::env::var("WASI_ADAPTER_PATH").unwrap_or_default().leak(),
        "/tmp/wasi_snapshot_preview1.command.wasm",
        "/tmp/wasi_adapter.wasm",
    ];
    for path in &candidates {
        if !path.is_empty() {
            if let Ok(bytes) = std::fs::read(path) {
                if bytes.len() > 100 {
                    eprintln!("📦 Using WASI adapter: {} ({} bytes)", path, bytes.len());
                    return bytes;
                }
            }
        }
    }
    // Fall back to built-in stub
    eprintln!("⚠️ No WASI adapter found, using no-op stub (I/O will not work)");
    build_wasi_stub()
}

fn build_wasi_stub() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

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

fn build_run_shim() -> Vec<u8> {
    let mut shim = ComponentBuilder::default();
    // Define result<> type
    let (result_type, mut renc) = shim.ty(None);
    {
        renc.defined_type().result(None, None);
    }
    // func type: () -> result<>
    let (fn_type, mut fenc) = shim.ty(None);
    {
        fenc.function().params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(result_type)));
    }
    shim.import("import-func-run", ComponentTypeRef::Func(fn_type));
    shim.export("run", ComponentExportKind::Func, 0, Some(ComponentTypeRef::Func(fn_type)));
    shim.finish()
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
                pos += np; // skip param types
                let (nr, rl) = rleb(wasm, pos); pos += rl;
                pos += nr; // skip result types
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
        inst.alias(Alias::Outer { count: 1, index: 0, kind: ComponentOuterAliasKind::Type });
        inst.export("hello", ComponentTypeRef::Func(0));
        enc2.instance(&inst);
    }

    b.import("test:hello/world", ComponentTypeRef::Instance(inst_type));

    let bytes = b.finish();
    std::fs::write("/tmp/test_instance.wasm", &bytes).unwrap();
    eprintln!("Built: {} bytes", bytes.len());
}

#[test]
fn test_shim_component() {
    let shim = build_run_shim();
    eprintln!("Shim size: {} bytes", shim.len());
    std::fs::write("/tmp/test_shim.wasm", &shim).unwrap();
    let output = std::process::Command::new("wasm-tools")
        .args(["validate", "/tmp/test_shim.wasm"])
        .output()
        .unwrap();
    eprintln!("Validate: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn test_shim_standalone() {
    let mut shim = ComponentBuilder::default();
    let (fn_type, mut fenc) = shim.ty(None);
    fenc.function().params([] as [(&str, ComponentValType); 0]).result(None);
    shim.import("import-func-run", ComponentTypeRef::Func(fn_type));
    shim.export("run", ComponentExportKind::Func, 0, Some(ComponentTypeRef::Func(fn_type)));
    let bytes = shim.finish();
    eprintln!("Shim: {} bytes", bytes.len());
    eprintln!("Hex: {:02x?}", bytes);
    std::fs::write("/tmp/test_shim_builder.wasm", &bytes).unwrap();
}
