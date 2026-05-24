//! Native P2 component — built from scratch using wasm-encoder's ComponentBuilder.
//!
//! Constructs a valid P2 component that:
//! - If core module uses outlayer: imports outlayer:api/host component interface
//!   with REAL WIT types so wasmtime's typed bindgen linker can match them.
//! - If core module is WASI-only: wraps with just WASI P1→P2 adapter
//! - Exports `run` via wasi:cli/run convention

use wasm_encoder::*;

/// Functions that use s64 params — these are skipped because canonical ABI
/// lowers s64 to i64 but the core module was compiled with 2×i32.
const S64_FUNCS: &[&str] = &["storage-increment", "storage-decrement"];

/// Ordered list of all WIT function names (kebab-case) in the outlayer:api/host interface.
/// Used to iterate in a consistent order for both type definition and canon lower.
const WIT_FUNC_NAMES: &[&str] = &[
    "view",
    "call",
    "transfer",
    "http-get",
    "http-post",
    "storage-set",
    "storage-get",
    "storage-has",
    "storage-delete",
    "storage-increment",
    "storage-decrement",
    "storage-set-if-absent",
    "storage-set-if-equals",
    "storage-list-keys",
    "storage-clear-all",
    "storage-set-worker",
    "storage-get-worker",
    "storage-set-worker-public",
    "storage-get-worker-from-project",
    "env-signer",
    "env-predecessor",
];

/// Returns the WIT param types and result type for a given function name.
/// Takes the defined type indices as input.
fn wit_func_signature(
    name: &str,
    string_ty: ComponentValType,
    list_u8_ty: ComponentValType,
    s64_ty: ComponentValType,
    result_string_string: ComponentValType,
    result_list_u8_string: ComponentValType,
    result_void_string: ComponentValType,
    result_bool_string: ComponentValType,
    result_s64_string: ComponentValType,
    result_option_list_u8_string: ComponentValType,
    result_list_string_string: ComponentValType,
) -> (Vec<(&'static str, ComponentValType)>, Option<ComponentValType>) {
    match name {
        "view" => (
            vec![
                ("contract-id", string_ty),
                ("method-name", string_ty),
                ("args-json", string_ty),
            ],
            Some(result_string_string),
        ),
        "call" => (
            vec![
                ("signer-key", string_ty),
                ("receiver-id", string_ty),
                ("method-name", string_ty),
                ("args-json", string_ty),
                ("deposit-yocto", string_ty),
                ("gas", string_ty),
            ],
            Some(result_string_string),
        ),
        "transfer" => (
            vec![
                ("signer-key", string_ty),
                ("receiver-id", string_ty),
                ("amount-yocto", string_ty),
            ],
            Some(result_string_string),
        ),
        "http-get" => (
            vec![("url", string_ty)],
            Some(result_list_u8_string),
        ),
        "http-post" => (
            vec![
                ("url", string_ty),
                ("body", list_u8_ty),
                ("content-type", string_ty),
            ],
            Some(result_list_u8_string),
        ),
        "storage-set" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(result_void_string),
        ),
        "storage-get" => (
            vec![("key", string_ty)],
            Some(result_option_list_u8_string),
        ),
        "storage-has" => (
            vec![("key", string_ty)],
            Some(result_bool_string),
        ),
        "storage-delete" => (
            vec![("key", string_ty)],
            Some(result_void_string),
        ),
        "storage-increment" => (
            vec![("key", string_ty), ("delta", s64_ty)],
            Some(result_s64_string),
        ),
        "storage-decrement" => (
            vec![("key", string_ty), ("delta", s64_ty)],
            Some(result_s64_string),
        ),
        "storage-set-if-absent" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(result_bool_string),
        ),
        "storage-set-if-equals" => (
            vec![
                ("key", string_ty),
                ("expected", list_u8_ty),
                ("new-value", list_u8_ty),
            ],
            Some(result_bool_string),
        ),
        "storage-list-keys" => (
            vec![("prefix", string_ty)],
            Some(result_list_string_string),
        ),
        "storage-clear-all" => (vec![], Some(result_void_string)),
        "storage-set-worker" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(result_void_string),
        ),
        "storage-get-worker" => (
            vec![("key", string_ty)],
            Some(result_option_list_u8_string),
        ),
        "storage-set-worker-public" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(result_void_string),
        ),
        "storage-get-worker-from-project" => (
            vec![("key", string_ty), ("project", string_ty)],
            Some(result_option_list_u8_string),
        ),
        "env-signer" => (vec![], Some(string_ty)),
        "env-predecessor" => (vec![], Some(string_ty)),
        _ => (vec![], None),
    }
}

pub fn build_native_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let info = analyze_outlayer(core_bytes);
    let has_outlayer = !info.names.is_empty();

    let mut b = ComponentBuilder::default();

    // ── Types for _start: () -> () ──
    let (run_type, run_enc) = b.ty(None);
    {
        run_enc
            .function()
            .params([] as [(&str, ComponentValType); 0])
            .result(None);
    }

    // Set of core module import names (snake_case)
    // Set of core module import names (kebab-case from WASM binary)
    let core_names_set: std::collections::HashSet<String> = info.names.iter().cloned().collect();

    // Determine which WIT functions the core module actually uses
    // Core module imports use kebab-case names matching WIT exactly (e.g. "http-get")
    let used_wit_names: Vec<&str> = WIT_FUNC_NAMES
        .iter()
        .filter(|&&kebab| core_names_set.contains(kebab))
        .copied()
        .collect();

    if has_outlayer {
        // ══════════════════════════════════════════════════════════
        // 1. Define all WIT defined types
        // ══════════════════════════════════════════════════════════

        // Primitive shortcuts (these are Copy, no allocation needed)
        let string_ty = ComponentValType::Primitive(PrimitiveValType::String);
        let s64_ty = ComponentValType::Primitive(PrimitiveValType::S64);

        // list<u8>
        let (list_u8_idx, list_u8_enc) = b.type_defined(None);
        list_u8_enc.list(ComponentValType::Primitive(PrimitiveValType::U8));
        let list_u8_ty = ComponentValType::Type(list_u8_idx);

        // bool
        let (bool_idx, bool_enc) = b.type_defined(None);
        bool_enc.primitive(PrimitiveValType::Bool);
        let _bool_ty = ComponentValType::Type(bool_idx);

        // option<list<u8>>
        let (opt_list_u8_idx, opt_enc) = b.type_defined(None);
        opt_enc.option(list_u8_ty);
        let option_list_u8_ty = ComponentValType::Type(opt_list_u8_idx);

        // result<string, string>
        let (result_string_string_idx, rss_enc) = b.type_defined(None);
        rss_enc.result(Some(string_ty), Some(string_ty));
        let result_string_string = ComponentValType::Type(result_string_string_idx);

        // result<list<u8>, string>
        let (result_list_u8_string_idx, rl8s_enc) = b.type_defined(None);
        rl8s_enc.result(Some(list_u8_ty), Some(string_ty));
        let result_list_u8_string = ComponentValType::Type(result_list_u8_string_idx);

        // result<_, string> (ok=None)
        let (result_void_string_idx, rvs_enc) = b.type_defined(None);
        rvs_enc.result(None, Some(string_ty));
        let result_void_string = ComponentValType::Type(result_void_string_idx);

        // result<bool, string>
        let (result_bool_string_idx, rbs_enc) = b.type_defined(None);
        rbs_enc.result(Some(ComponentValType::Type(bool_idx)), Some(string_ty));
        let result_bool_string = ComponentValType::Type(result_bool_string_idx);

        // result<s64, string>
        let (result_s64_string_idx, rs64s_enc) = b.type_defined(None);
        rs64s_enc.result(Some(s64_ty), Some(string_ty));
        let result_s64_string = ComponentValType::Type(result_s64_string_idx);

        // result<option<list<u8>>, string>
        let (result_opt_list_u8_string_idx, rol8s_enc) = b.type_defined(None);
        rol8s_enc.result(Some(option_list_u8_ty), Some(string_ty));
        let result_option_list_u8_string =
            ComponentValType::Type(result_opt_list_u8_string_idx);

        // list<string>
        let (list_string_idx, ls_enc) = b.type_defined(None);
        ls_enc.list(string_ty);
        let list_string_ty = ComponentValType::Type(list_string_idx);

        // result<list<string>, string>
        let (result_list_string_string_idx, rlss_enc) = b.type_defined(None);
        rlss_enc.result(Some(list_string_ty), Some(string_ty));
        let result_list_string_string =
            ComponentValType::Type(result_list_string_string_idx);

        // ══════════════════════════════════════════════════════════
        // 2. Define function types for each used WIT function
        // ══════════════════════════════════════════════════════════
        let mut func_type_indices: Vec<u32> = Vec::new();
        for &name in &used_wit_names {
            let (params, result) = wit_func_signature(
                name,
                string_ty,
                list_u8_ty,
                s64_ty,
                result_string_string,
                result_list_u8_string,
                result_void_string,
                result_bool_string,
                result_s64_string,
                result_option_list_u8_string,
                result_list_string_string,
            );
            let (ft_idx, mut ft_enc) = b.type_function(None);
            ft_enc.params(params).result(result);
            func_type_indices.push(ft_idx);
        }

        // ══════════════════════════════════════════════════════════
        // 3. Build instance type with properly-typed function exports
        // ══════════════════════════════════════════════════════════
        let (outlayer_inst_type, inst_enc) = b.ty(None);
        {
            let mut inst = InstanceType::new();
            for &ft in &func_type_indices {
                inst.alias(Alias::Outer {
                    kind: ComponentOuterAliasKind::Type,
                    count: 1,
                    index: ft,
                });
            }
            for (i, &name) in used_wit_names.iter().enumerate() {
                inst.export(name, ComponentTypeRef::Func(i as u32));
            }
            inst_enc.instance(&inst);
        }

        b.import(
            "outlayer:api/host",
            ComponentTypeRef::Instance(outlayer_inst_type),
        );
    }

    // ── Embed core module (patched to import memory from "env") ──
    let mem_pages = extract_memory_pages(core_bytes);
    eprintln!("DEBUG: core_bytes {} bytes, mem_pages={}", core_bytes.len(), mem_pages);
    let patched_core = make_memory_import(core_bytes);
    let module_idx = b.core_module_raw(None, &patched_core);

    // ── Build memory module (exports shared memory) ──
    let mem_mod_bytes = build_memory_module(mem_pages);
    let mem_mod = b.core_module_raw(None, &mem_mod_bytes);

    // ── Build WASI stub (no-op P1 implementations) ──
    let wasi_stub_bytes = build_wasi_stub();
    let wasi_stub_mod = b.core_module_raw(None, &wasi_stub_bytes);

    // ── Instantiate memory module FIRST (provides memory + realloc for canon lower) ──
    let mem_inst = b.core_instantiate(None, mem_mod, []);
    let mem = b.core_alias_export(None, mem_inst, "memory", ExportKind::Memory);
    let realloc = b.core_alias_export(None, mem_inst, "cabi_realloc", ExportKind::Func);

    // For s64 functions, alias the unreachable stubs from the memory module
    let s64_trap = b.core_alias_export(None, mem_inst, "s64_trap", ExportKind::Func);

    let mut lowered_funcs: Vec<u32> = Vec::new();
    let mut lowered_names: Vec<String> = Vec::new();
    let mut outlayer_inst_idx: u32 = 0;
    if has_outlayer {
        // Lower ALL outlayer functions that the core module imports
        for kebab in &info.names {
            // Only lower functions that exist in WIT
            if !WIT_FUNC_NAMES.contains(&kebab.as_str()) {
                eprintln!("⚠️ No WIT type for {}, skipping", kebab);
                continue;
            }
            // s64 functions: core module emits all-i32 params but canonical ABI
            // uses i64 for s64 — type mismatch. Use trap stub instead.
            if S64_FUNCS.contains(&kebab.as_str()) {
                eprintln!("⚠️ Using trap stub for {} (s64 ABI mismatch)", kebab);
                lowered_funcs.push(s64_trap);
                lowered_names.push(kebab.clone());
                continue;
            }
            let comp_func = b.alias_export(0, kebab.as_str(), ComponentExportKind::Func);
            let core_func = b.lower_func(
                None,
                comp_func,
                [
                    CanonicalOption::Memory(mem),
                    CanonicalOption::Realloc(realloc),
                ],
            );
            lowered_funcs.push(core_func);
            lowered_names.push(kebab.clone());
        }

        // Create outlayer core instance from lowered funcs + s64 trap stubs
        outlayer_inst_idx = b.core_instantiate_exports(
            None,
            lowered_names
                .iter()
                .zip(lowered_funcs.iter())
                .map(|(n, &idx)| (n.as_str(), ExportKind::Func, idx)),
        );
    }

    // ── Instantiate WASI stub ──
    let wasi_stub_inst = b.core_instantiate(None, wasi_stub_mod, []);

    // ── Instantiate core module with env(memory) + WASI + outlayer ──
    let core_inst = if has_outlayer {
        b.core_instantiate(
            None,
            module_idx,
            [
                ("env", ModuleArg::Instance(mem_inst)),
                ("outlayer:api/host", ModuleArg::Instance(outlayer_inst_idx)),
                ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
            ],
        )
    } else {
        b.core_instantiate(
            None,
            module_idx,
            [
                ("env", ModuleArg::Instance(mem_inst)),
                ("wasi_snapshot_preview1", ModuleArg::Instance(wasi_stub_inst)),
            ],
        )
    };

    // ── Lift _start → run (using shared memory) ──
    let start_func = b.core_alias_export(None, core_inst, "_start", ExportKind::Func);
    let run_func = b.lift_func(None, start_func, run_type, [CanonicalOption::Memory(mem)]);

    // ── Export wasi:cli/run@0.2.2 instance ──
    let shim_bytes = build_run_shim();
    let shim_idx = b.component_raw(None, &shim_bytes);

    let shim_instance = b.instantiate(
        None,
        shim_idx,
        [("import-func-run", ComponentExportKind::Func, run_func)],
    );

    // Export both wasi:cli/run@0.2.2 (standard) and bare "run" (for inlayer fallback)
    b.export(
        "wasi:cli/run@0.2.2",
        ComponentExportKind::Instance,
        shim_instance,
        None,
    );
    b.export("run", ComponentExportKind::Func, run_func, None);

    let bytes = b.finish();
    eprintln!(
        "✅ Native P2 component: {} bytes (outlayer={})",
        bytes.len(),
        has_outlayer
    );
    Ok(bytes)
}

/// Load the WASI P1→P2 adapter module.
pub fn load_wasi_adapter() -> Vec<u8> {
    let candidates = [
        std::env::var("WASI_ADAPTER_PATH")
            .unwrap_or_default()
            .leak(),
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
    eprintln!("⚠️ No WASI adapter found, using no-op stub (I/O will not work)");
    build_wasi_stub()
}

/// Build a tiny WASM module that exports memory and a simple bump-allocator realloc.
fn build_memory_module(min_pages: u64) -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Type section
    let mut types = TypeSection::new();
    // type 0: () -> ()
    types.ty().function([], []);
    // type 1: (i32, i32, i32, i32) -> i32 — realloc signature
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
    // type 2: (i32, i32, i64, i32) -> () — s64 trap stub (canonical ABI for s64 params)
    types.ty().function([ValType::I32, ValType::I32, ValType::I64, ValType::I32], []);
    m.section(&types);

    // Function section: func 0 = realloc (type 1), func 1 = s64_trap (type 2)
    let mut funcs = FunctionSection::new();
    funcs.function(1); // realloc: type 1
    funcs.function(2); // s64_trap: type 2
    m.section(&funcs);

    // Memory section
    let mut mems = MemorySection::new();
    // +1 extra page for bump allocator (host function return strings)
    mems.memory(MemoryType { minimum: min_pages + 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    // Global: bump pointer (i32, mutable)
    let mut globals = GlobalSection::new();
    // global 0: bump_ptr, initialized to first page after memory
    let init_val = (min_pages * 65536) as i64;
    globals.global(
        GlobalType { val_type: ValType::I32, mutable: true, shared: false },
        &ConstExpr::i32_const(init_val as i32),
    );
    m.section(&globals);

    // Export section
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("cabi_realloc", ExportKind::Func, 0);
    exports.export("s64_trap", ExportKind::Func, 1);
    m.section(&exports);

    // Code section: realloc implementation (bump allocator)
    let mut codes = CodeSection::new();
    let mut f = Function::new([(0, ValType::I32), (1, ValType::I32), (2, ValType::I32), (3, ValType::I32)]);
    // Align old_ptr to alignment (param 1)
    // Simple bump: just return bump_ptr, then advance by new_size (param 3)
    // global.get 0 (bump_ptr) -> local.tee 0 (reuse old_ptr local as result)
    f.instruction(&Instruction::GlobalGet(0)); // bump_ptr
    f.instruction(&Instruction::LocalTee(0));  // save as result
    // Advance bump_ptr by new_size (param 3 = local 3)
    f.instruction(&Instruction::LocalGet(3));  // new_size
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::GlobalSet(0)); // bump_ptr += new_size
    f.instruction(&Instruction::LocalGet(0));  // return old bump_ptr
    f.instruction(&Instruction::End);
    codes.function(&f);

    // s64_trap: just trap (unreachable) — matches canon-lowered (i32, i32, i64, i32) -> ()
    let mut trap = Function::new([
        (0, ValType::I32),
        (1, ValType::I32),
        (2, ValType::I64),
        (3, ValType::I32),
    ]);
    trap.instruction(&Instruction::Unreachable);
    trap.instruction(&Instruction::End);
    codes.function(&trap);

    m.section(&codes);

    m.finish()
}

/// Extract the minimum pages from the core module's memory section (section id 5).
/// Returns 17 (safe default) if not found.
fn extract_memory_pages(wasm: &[u8]) -> u64 {
    let mut pos = 8usize;
    while pos < wasm.len() {
        let sid = wasm[pos]; pos += 1;
        let (sz, lb) = rleb(wasm, pos); pos += lb;
        if sid == 5 {
            // Memory section: count, then each has flags + min ( + max)
            let (_cnt, cl) = rleb(wasm, pos);
            let p = pos + cl;
            let (flags, fl) = rleb(wasm, p);
            let (min, ml) = rleb(wasm, p + fl);
            let _ = flags;
            return min as u64;
        }
        pos += sz;
    }
    17 // safe default
}

/// Binary-patch the core module: remove memory section (5), add a memory import
/// from ("env", "memory") with the same limits, and remove the "memory" export.
fn make_memory_import(wasm: &[u8]) -> Vec<u8> {
    let mut out = wasm[..8].to_vec(); // keep header

    let mut pos = 8usize;
    let mut mem_limits: Option<(u64, Option<u64>)> = None;
    let mut import_count_delta: i32 = 0;

    // First pass: find memory section to extract limits
    let mut scan = 8usize;
    while scan < wasm.len() {
        let sid = wasm[scan]; scan += 1;
        let (sz, lb) = rleb(wasm, scan); scan += lb;
        if sid == 5 {
            let p = scan;
            let (cnt, cl) = rleb(wasm, p);
            let base = p + cl;
            if cnt > 0 {
                let (flags, fl) = rleb(wasm, base);
                let (min, ml) = rleb(wasm, base + fl);
                let has_max = flags & 1 != 0;
                let max_val = if has_max {
                    let (mx, _) = rleb(wasm, base + fl + ml);
                    Some(mx as u64)
                } else {
                    None
                };
                mem_limits = Some((min as u64, max_val));
            }
            break;
        }
        scan += sz;
    }
    let (min_p, max_p) = mem_limits.unwrap_or((17u64, None));

    // Build the memory import entry bytes
    let mut mem_import = Vec::new();
    // module name "env"
    leb128::write::unsigned(&mut mem_import, 3).unwrap();
    mem_import.extend_from_slice(b"env");
    // field name "memory"
    leb128::write::unsigned(&mut mem_import, 6).unwrap();
    mem_import.extend_from_slice(b"memory");
    // kind = 2 (memory)
    mem_import.push(2);
    // limits
    let flags: u8 = if max_p.is_some() { 1 } else { 0 };
    mem_import.push(flags);
    leb128::write::unsigned(&mut mem_import, min_p).unwrap();
    if let Some(mx) = max_p {
        leb128::write::unsigned(&mut mem_import, mx).unwrap();
    }

    // Second pass: rebuild the module, skipping memory section (5) and removing "memory" export
    while pos < wasm.len() {
        let sid = wasm[pos]; pos += 1;
        let (sz, lb) = rleb(wasm, pos); pos += lb;
        eprintln!("DEBUG: section {} at {}, size {}", sid, pos, sz);

        // Skip memory section entirely
        if sid == 5 {
            pos += sz;
            continue;
        }

        if sid == 2 {
            // Import section — prepend one memory import
            let (cnt, cl) = rleb(wasm, pos);
            eprintln!("DEBUG: import section, cnt={}, cl={}, section_end={}", cnt, cl, pos + sz);
            let header_end = pos + cl;
            let body = &wasm[header_end..pos + sz];

            let mut new_section = Vec::new();
            leb128::write::unsigned(&mut new_section, (cnt + 1) as u64).unwrap();
            new_section.extend_from_slice(&mem_import);
            new_section.extend_from_slice(body);

            out.push(sid);
            let mut section_len = Vec::new();
            leb128::write::unsigned(&mut section_len, new_section.len() as u64).unwrap();
            out.extend_from_slice(&section_len);
            out.extend_from_slice(&new_section);

            pos += sz;
            continue;
        }

        if sid == 7 {
            // Export section — remove the "memory" export
            let section_start = pos;
            let (cnt, cl) = rleb(wasm, pos);
            let header_end = pos + cl;
            let section_end = pos + sz;
            let mut new_exports = Vec::new();
            new_exports.extend_from_slice(&wasm[pos..header_end]); // original count placeholder

            let mut ep = header_end;
            let mut kept = 0u32;
            for _ in 0..cnt {
                let (nl, nll) = rleb(wasm, ep); ep += nll;
                let name = &wasm[ep..ep+nl]; ep += nl;
                let kind = wasm[ep]; ep += 1;
                let (idx, il) = rleb(wasm, ep); ep += il;
                if name == b"memory" && kind == 2 {
                    continue; // skip memory export
                }
                leb128::write::unsigned(&mut new_exports, nl as u64).unwrap();
                new_exports.extend_from_slice(name);
                new_exports.push(kind);
                leb128::write::unsigned(&mut new_exports, idx as u64).unwrap();
                kept += 1;
            }
            // Fix count
            let count_bytes = &mut new_exports[..cl];
            let mut fixed_count = Vec::new();
            leb128::write::unsigned(&mut fixed_count, kept as u64).unwrap();
            // If LEB encoding changed size, we need to adjust — but usually same or smaller
            let count_end = fixed_count.len().min(cl);
            count_bytes[..count_end].copy_from_slice(&fixed_count[..count_end]);

            out.push(sid);
            let mut section_len = Vec::new();
            leb128::write::unsigned(&mut section_len, new_exports.len() as u64).unwrap();
            out.extend_from_slice(&section_len);
            out.extend_from_slice(&new_exports);

            pos += sz;
            continue;
        }

        // Other sections: pass through unchanged
        out.push(sid);
        let mut section_len = Vec::new();
        leb128::write::unsigned(&mut section_len, sz as u64).unwrap();
        out.extend_from_slice(&section_len);
        out.extend_from_slice(&wasm[pos..pos + sz]);
        pos += sz;
    }

    // If there was no import section, add one with just the memory import
    if !wasm[8..].iter().any(|&b| b == 2) && mem_limits.is_some() {
        // This case is unlikely but handle it: insert import section
        let mut import_section = Vec::new();
        leb128::write::unsigned(&mut import_section, 1u64).unwrap(); // count=1
        import_section.extend_from_slice(&mem_import);
        out.push(2); // import section id
        let mut sl = Vec::new();
        leb128::write::unsigned(&mut sl, import_section.len() as u64).unwrap();
        out.extend_from_slice(&sl);
        out.extend_from_slice(&import_section);
    }

    out
}

fn build_wasi_stub() -> Vec<u8> {
    use wasm_encoder::*;
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
    let (fn_type, fenc) = shim.ty(None);
    {
        fenc.function()
            .params([] as [(&str, ComponentValType); 0])
            .result(None);
    }
    shim.import("import-func-run", ComponentTypeRef::Func(fn_type));
    shim.export(
        "run",
        ComponentExportKind::Func,
        0,
        Some(ComponentTypeRef::Func(fn_type)),
    );
    shim.finish()
}

struct OutlayerInfo {
    names: Vec<String>,
    #[allow(dead_code)]
    param_counts: Vec<usize>,
}

fn analyze_outlayer(wasm: &[u8]) -> OutlayerInfo {
    let mut names = Vec::new();
    let mut param_counts = Vec::new();

    // Read type section first
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

    // Read import section
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
                let name = std::str::from_utf8(&wasm[pos..pos + nl])
                    .unwrap_or("")
                    .to_string();
                pos += nl;
                let kind = wasm[pos];
                pos += 1;
                match kind {
                    0 => {
                        let (tl, tll) = rleb(wasm, pos);
                        pos += tll;
                        if module == "outlayer" || module == "outlayer:api/host" {
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
    OutlayerInfo {
        names,
        param_counts,
    }
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
        inst.alias(Alias::Outer {
            count: 1,
            index: 0,
            kind: ComponentOuterAliasKind::Type,
        });
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
    fenc.function()
        .params([] as [(&str, ComponentValType); 0])
        .result(None);
    shim.import("import-func-run", ComponentTypeRef::Func(fn_type));
    shim.export(
        "run",
        ComponentExportKind::Func,
        0,
        Some(ComponentTypeRef::Func(fn_type)),
    );
    let bytes = shim.finish();
    eprintln!("Shim: {} bytes", bytes.len());
    eprintln!("Hex: {:02x?}", bytes);
    std::fs::write("/tmp/test_shim_builder.wasm", &bytes).unwrap();
}
/// Build a P2 component for wasi:http modules.
/// Uses wit-component encoder with relaxed validation.
pub fn build_wasi_http_component(
    core_bytes: &[u8],
    _em: &crate::wasm_emit::WasmEmitter,
) -> Result<Vec<u8>, String> {
    let mut mod_bytes = core_bytes.to_vec();

    // Embed WIT metadata (imports only — no export, runtime uses _start)
    let (resolve, world) = crate::wasi_http::build_http_wit_metadata()
        .map_err(|e| format!("WIT metadata: {}", e))?;
    wit_component::embed_component_metadata(
        &mut mod_bytes,
        &resolve,
        world,
        wit_component::StringEncoding::UTF8,
    )
    .map_err(|e| format!("embed metadata: {}", e))?;

    let component = wit_component::ComponentEncoder::default()
        .module(&mod_bytes)
        .map_err(|e| format!("encoder module: {:#}", e))?
        .validate(false)
        .encode()
        .map_err(|e| format!("encode: {:#}", e))?;
    Ok(component)
}
