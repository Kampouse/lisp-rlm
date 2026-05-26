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

/// Ordered list of all WIT function names (kebab-case) — core module import names.
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

/// Which component interface a function belongs to.
/// Uses production split interfaces for storage and RPC.
/// Returns the core module import param signature for a given outlayer function name.
/// All outlayer imports return () — only params vary. Signatures must match the
/// core module's type section (set by wasi_emit.rs outlayer_imports).
fn core_import_sig(kebab: &str) -> Vec<wasm_encoder::ValType> {
    use wasm_encoder::ValType;
    match kebab {
        // 7 i32 params: ptr pairs + ret_ptr
        "view" | "transfer" | "http-post" | "storage-set-if-equals" => vec![ValType::I32; 7],
        // 13 i32 params: many ptr/len pairs
        "call" => vec![ValType::I32; 13],
        // 5 i32 params
        "storage-set" | "storage-set-if-absent" | "storage-set-worker"
        | "storage-set-worker-public" | "storage-get-worker-from-project" => vec![ValType::I32; 5],
        // 3 i32 params
        "http-get" | "storage-get" | "storage-has" | "storage-delete"
        | "storage-list-keys" | "storage-get-worker" => vec![ValType::I32; 3],
        // s64 signature: 2 i32 + i64 + i32
        "storage-increment" | "storage-decrement" => vec![
            ValType::I32, ValType::I32, ValType::I64, ValType::I32,
        ],
        // 1 i32 param
        "storage-clear-all" | "env-signer" | "env-predecessor" => vec![ValType::I32; 1],
        // Fallback
        _ => vec![ValType::I32; 1],
    }
}

fn func_interface(kebab: &str) -> &'static str {
    match kebab {
        "storage-set"
        | "storage-get"
        | "storage-has"
        | "storage-delete"
        | "storage-increment"
        | "storage-decrement"
        | "storage-set-if-absent"
        | "storage-set-if-equals"
        | "storage-list-keys"
        | "storage-clear-all"
        | "storage-set-worker"
        | "storage-get-worker"
        | "storage-set-worker-public"
        | "storage-get-worker-public"
        | "storage-get-by-version"
        | "storage-clear-version" => "near:storage/api@0.1.0",
        "view" | "call" | "transfer" => "near:rpc/api@0.1.0",
        "http-get" | "http-post" => "lisp:http-adapter/api@0.1.0",
        "env-signer" | "env-predecessor" => "trap",
        _ => "trap",
    }
}

/// Map core module import name to the WIT function name exported by the interface.
/// Storage functions drop the "storage-" prefix in near:storage/api@0.1.0.
/// RPC functions use production names (view, call, transfer) in near:rpc/api@0.1.0.
fn func_wit_name(kebab: &str) -> &str {
    match kebab {
        // Storage: drop "storage-" prefix
        s if s.starts_with("storage-") => &s[8..],
        // RPC: same name
        "view" | "call" | "transfer" => kebab,
        // Legacy: same name
        _ => kebab,
    }
}

/// Returns the WIT param types and result type for a given function name.
/// Takes the defined type indices as input.
///
/// Uses production split WIT types:
/// - Storage functions use `near:storage/api@0.1.0` (tuple returns, not result<>)
/// - RPC functions use `near:rpc/api@0.1.0` (extra signer-id/wait-until params)
/// - HTTP functions stay on `outlayer:api/host` (legacy result<> types)
fn wit_func_signature(
    name: &str,
    string_ty: ComponentValType,
    list_u8_ty: ComponentValType,
    s64_ty: ComponentValType,
    // Legacy result types (for trap stubs matching core module's old canonical ABI)
    result_string_string: ComponentValType,
    result_list_u8_string: ComponentValType,
    result_void_string: ComponentValType,
    result_bool_string: ComponentValType,
    result_s64_string: ComponentValType,
    result_option_list_u8_string: ComponentValType,
    result_list_string_string: ComponentValType,
    // Production tuple types (for near:storage/api functions where ABI matches)
    _tuple_list_u8_string: ComponentValType,
    _tuple_string_string: ComponentValType,
    _tuple_bool_string: ComponentValType,
    _tuple_bool_list_u8_string: ComponentValType,
    _tuple_s64_string: ComponentValType,
    _option_bool_ty: ComponentValType,
    _option_string_ty: ComponentValType,
) -> (
    Vec<(&'static str, ComponentValType)>,
    Option<ComponentValType>,
) {
    match name {
        // ── near:storage/api@0.1.0 (production WIT: bare returns, not result<>) ──
        "storage-set" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(string_ty), // -> string (empty on success, error msg on failure)
        ),
        "storage-get" => (
            vec![("key", string_ty)],
            Some(_tuple_list_u8_string), // -> tuple<list<u8>, string>
        ),
        "storage-has" => (
            vec![("key", string_ty)],
            Some(ComponentValType::Primitive(PrimitiveValType::Bool)), // -> bool
        ),
        "storage-delete" => (
            vec![("key", string_ty)],
            Some(ComponentValType::Primitive(PrimitiveValType::Bool)), // -> bool
        ),
        "storage-increment" => (
            vec![("key", string_ty), ("delta", s64_ty)],
            Some(_tuple_s64_string), // -> tuple<s64, string>
        ),
        "storage-decrement" => (
            vec![("key", string_ty), ("delta", s64_ty)],
            Some(_tuple_s64_string), // -> tuple<s64, string>
        ),
        "storage-set-if-absent" => (
            vec![("key", string_ty), ("value", list_u8_ty)],
            Some(_tuple_bool_string), // -> tuple<bool, string>
        ),
        "storage-set-if-equals" => (
            vec![("key", string_ty), ("expected", list_u8_ty), ("new-value", list_u8_ty)],
            Some(_tuple_bool_list_u8_string), // -> tuple<bool, list<u8>, string>
        ),
        "storage-list-keys" => (
            vec![("prefix", string_ty)],
            Some(_tuple_string_string), // -> tuple<string, string>
        ),
        "storage-clear-all" => (
            vec![],
            Some(string_ty), // -> string
        ),
        "storage-clear-version" => (
            vec![("wasm-hash", string_ty)],
            Some(string_ty), // -> string
        ),
        "storage-set-worker" => (
            vec![("key", string_ty), ("value", list_u8_ty), ("is-encrypted", _option_bool_ty)],
            Some(string_ty), // -> string
        ),
        "storage-get-worker" => (
            vec![("key", string_ty), ("project", _option_string_ty)],
            Some(_tuple_list_u8_string), // -> tuple<list<u8>, string>
        ),
        "storage-set-worker-public" => (
            vec![("key", string_ty), ("value", list_u8_ty), ("is-encrypted", _option_bool_ty)],
            Some(string_ty), // -> string (same as set-worker)
        ),
        "storage-get-worker-from-project" => (
            vec![("key", string_ty), ("project", _option_string_ty)],
            Some(_tuple_list_u8_string), // -> tuple<list<u8>, string>
        ),
        "storage-get-by-version" => (
            vec![("key", string_ty), ("wasm-hash", string_ty)],
            Some(_tuple_list_u8_string), // -> tuple<list<u8>, string>
        ),
        "env-signer" => (
            vec![],
            Some(string_ty),
        ),
        "env-predecessor" => (
            vec![],
            Some(string_ty),
        ),

        // ── near:rpc/api@0.1.0 ──
        "view" => (
            vec![("contract-id", string_ty), ("method-name", string_ty), ("args-json", string_ty)],
            Some(result_string_string),
        ),
        "call" => (
            vec![
                ("signer-key", string_ty), ("receiver-id", string_ty),
                ("method-name", string_ty), ("args-json", string_ty),
                ("deposit-yocto", string_ty), ("gas", string_ty),
            ],
            Some(result_string_string),
        ),
        "transfer" => (
            vec![("signer-key", string_ty), ("receiver-id", string_ty), ("amount-yocto", string_ty)],
            Some(result_string_string),
        ),

        // ── outlayer:api/host (legacy) ──
        "http-get" => (
            vec![("url", string_ty)],
            Some(result_list_u8_string),
        ),
        "http-post" => (
            vec![("url", string_ty), ("body", list_u8_ty), ("content-type", string_ty)],
            Some(result_list_u8_string),
        ),

        _ => panic!("Unknown WIT function: {}", name),
    }
}

pub fn build_native_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let info = analyze_outlayer(core_bytes);
    let has_outlayer = !info.names.is_empty();

    let mut b = ComponentBuilder::default();

    // ── Types for wasi:cli/run: () -> result<()> ──
    // result<()> = result { ok: None, err: None } — canonical i32 discriminant
    let (result_ok_err_type, roe_enc) = b.type_defined(None);
    {
        roe_enc.result(None::<ComponentValType>, None::<ComponentValType>);
    }
    let (run_type, run_enc) = b.ty(None);
    {
        run_enc
            .function()
            .params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(result_ok_err_type)));
    }

    // Set of core module import names (kebab-case from WASM binary)
    let core_names_set: std::collections::HashSet<String> = info.names.iter().cloned().collect();

    // Set of functions actually called (has Call instruction in code section)
    let called_names: std::collections::HashSet<String> = info
        .called
        .iter()
        .map(|&idx| info.names[idx].clone())
        .collect();

    // Determine which WIT functions the core module actually CALLS
    // (not just imports — unused imports get trap stubs, not component-level imports)
    let used_wit_names: Vec<&str> = WIT_FUNC_NAMES
        .iter()
        .filter(|&&kebab| called_names.contains(kebab))
        .copied()
        .collect();

    // Will be populated inside has_outlayer block
    // (interface_import_idx, names, func_type_indices)
    let mut used_interface_imports: Option<(
        std::collections::HashMap<&'static str, u32>,
        Vec<&str>,
        Vec<u32>,
    )> = None;

    // ── Shared type definitions (needed for outlayer WIT and P2 streams) ──
    let string_ty = ComponentValType::Primitive(PrimitiveValType::String);
    let s64_ty = ComponentValType::Primitive(PrimitiveValType::S64);

    // list<u8>
    let (list_u8_idx, list_u8_enc) = b.type_defined(None);
    list_u8_enc.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let list_u8_ty = ComponentValType::Type(list_u8_idx);

    if has_outlayer {

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
        // Production split WIT types (near:storage/api, near:rpc/api)
        // ══════════════════════════════════════════════════════════

        // tuple<list<u8>, string> — storage-get, storage-get-worker
        let (tuple_list_u8_string_idx, tl8s_enc) = b.type_defined(None);
        tl8s_enc.tuple([list_u8_ty, string_ty]);
        let tuple_list_u8_string = ComponentValType::Type(tuple_list_u8_string_idx);

        // tuple<string, string> — view, call, transfer, storage-list-keys
        let (tuple_string_string_idx, tss_enc) = b.type_defined(None);
        tss_enc.tuple([string_ty, string_ty]);
        let tuple_string_string = ComponentValType::Type(tuple_string_string_idx);

        // tuple<bool, string> — storage-set-if-absent
        let (tuple_bool_string_idx, tbs_enc) = b.type_defined(None);
        tbs_enc.tuple([ComponentValType::Type(bool_idx), string_ty]);
        let tuple_bool_string = ComponentValType::Type(tuple_bool_string_idx);

        // tuple<bool, list<u8>, string> — storage-set-if-equals
        let (tuple_bool_list_u8_string_idx, tbl8s_enc) = b.type_defined(None);
        tbl8s_enc.tuple([ComponentValType::Type(bool_idx), list_u8_ty, string_ty]);
        let tuple_bool_list_u8_string = ComponentValType::Type(tuple_bool_list_u8_string_idx);

        // tuple<s64, string> — storage-increment, storage-decrement
        let (tuple_s64_string_idx, ts64s_enc) = b.type_defined(None);
        ts64s_enc.tuple([s64_ty, string_ty]);
        let tuple_s64_string = ComponentValType::Type(tuple_s64_string_idx);

        // option<bool> — storage-set-worker is-encrypted param
        let (option_bool_idx, ob_enc) = b.type_defined(None);
        ob_enc.option(ComponentValType::Type(bool_idx));
        let option_bool_ty = ComponentValType::Type(option_bool_idx);

        // option<string> — storage-get-worker project param
        let (option_string_idx, os_enc) = b.type_defined(None);
        os_enc.option(string_ty);
        let option_string_ty = ComponentValType::Type(option_string_idx);

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
                tuple_list_u8_string,
                tuple_string_string,
                tuple_bool_string,
                tuple_bool_list_u8_string,
                tuple_s64_string,
                option_bool_ty,
                option_string_ty,
            );
            let (ft_idx, mut ft_enc) = b.type_function(None);
            ft_enc.params(params).result(result);
            func_type_indices.push(ft_idx);
        }

        // ══════════════════════════════════════════════════════════
        // 3. Build split instance types for production WIT interfaces
        // ══════════════════════════════════════════════════════════

        // Group functions by interface
        let mut storage_names: Vec<&str> = Vec::new();
        let mut rpc_names: Vec<&str> = Vec::new();
        let mut http_names: Vec<&str> = Vec::new();
        for &name in &used_wit_names {
            match func_interface(name) {
                "near:storage/api@0.1.0" => storage_names.push(name),
                "near:rpc/api@0.1.0" => rpc_names.push(name),
                "lisp:http-adapter/api@0.1.0" => http_names.push(name),
                _ => {} // "trap" — no component-level import needed
            }
        }

        // Build instance type for a group of functions
        let build_inst_type = |b: &mut ComponentBuilder, names: &[&str], all_types: &[u32]| -> u32 {
            let (inst_type, inst_enc) = b.ty(None);
            {
                let mut inst = InstanceType::new();
                // Alias function types from outer scope
                for &name in names {
                    if let Some(idx) = used_wit_names.iter().position(|&n| n == name) {
                        inst.alias(Alias::Outer {
                            kind: ComponentOuterAliasKind::Type,
                            count: 1,
                            index: all_types[idx],
                        });
                    }
                }
                // Export functions with their WIT names
                let mut local_idx = 0u32;
                for &name in names {
                    let wit_name = func_wit_name(name);
                    inst.export(wit_name, ComponentTypeRef::Func(local_idx));
                    local_idx += 1;
                }
                inst_enc.instance(&inst);
            }
            inst_type
        };

        let storage_type_idx = if !storage_names.is_empty() {
            let it = build_inst_type(&mut b, &storage_names, &func_type_indices);
            Some(it)
        } else {
            None
        };
        let rpc_type_idx = if !rpc_names.is_empty() {
            let it = build_inst_type(&mut b, &rpc_names, &func_type_indices);
            Some(it)
        } else {
            None
        };

        let http_type_idx = if !http_names.is_empty() {
            let it = build_inst_type(&mut b, &http_names, &func_type_indices);
            Some(it)
        } else {
            None
        };

        // Import each interface — track component-level import indices
        let mut interface_import_idx: std::collections::HashMap<&str, u32> =
            std::collections::HashMap::new();
        if let Some(st) = storage_type_idx {
            let idx = b.import("near:storage/api@0.1.0", ComponentTypeRef::Instance(st));
            interface_import_idx.insert("storage", idx);
        }
        if let Some(rt) = rpc_type_idx {
            let idx = b.import("near:rpc/api@0.1.0", ComponentTypeRef::Instance(rt));
            interface_import_idx.insert("rpc", idx);
        }
        if let Some(ht) = http_type_idx {
            let idx = b.import("lisp:http-adapter/api@0.1.0", ComponentTypeRef::Instance(ht));
            interface_import_idx.insert("http", idx);
        }
        // Store for later use in canon lower section
        used_interface_imports = Some((interface_import_idx, used_wit_names.clone(), func_type_indices.clone()));
    }

    // ── Embed core module (patched to import memory from "env") ──
    let mem_pages = extract_memory_pages(core_bytes);
    eprintln!("DEBUG: core_bytes {} bytes, mem_pages={}", core_bytes.len(), mem_pages);
    let patched_core = make_memory_import(core_bytes);
    let module_idx = b.core_module_raw(None, &patched_core);

    // ── Build memory module (exports shared memory + trap stubs) ──
    let (mem_mod_bytes, trap_func_map) = build_memory_module(mem_pages);
    let mem_mod = b.core_module_raw(None, &mem_mod_bytes);

    // ── Instantiate memory module FIRST (provides memory + realloc for canon lower) ──
    let mem_inst = b.core_instantiate(None, mem_mod, []);
    let mem = b.core_alias_export(None, mem_inst, "memory", ExportKind::Memory);
    let realloc = b.core_alias_export(None, mem_inst, "cabi_realloc", ExportKind::Func);

    // Alias all trap stubs from memory module for per-signature type matching
    let trap_s64_sig = vec![wasm_encoder::ValType::I32, wasm_encoder::ValType::I32, wasm_encoder::ValType::I64, wasm_encoder::ValType::I32];
    let mut trap_stubs: std::collections::HashMap<Vec<wasm_encoder::ValType>, u32> = std::collections::HashMap::new();
    for (sig, &export_name) in &trap_func_map {
        let name = format!("trap_{}i32", sig.len());
        let core_func = b.core_alias_export(None, mem_inst, &name, ExportKind::Func);
        trap_stubs.insert(sig.clone(), core_func);
    }
    let s64_trap = *trap_stubs.get(&trap_s64_sig).unwrap();

    // ════════════════════════════════════════════════════════════════
    // P2 WASI stream imports — proper resource-owning instance types
    // matching the standard WASI P2 WIT definitions.
    //
    // Pattern (from p2_wasi_http.wasm):
    //   1. wasi:io/streams exports resource types (input-stream, output-stream)
    //      as (sub resource) inside the instance type, plus functions.
    //   2. After import, alias resource types out at component level.
    //   3. wasi:cli/stdin and wasi:cli/stdout alias those resource types
    //      from outer scope, re-export them, and define get-stdin/get-stdout.
    // ════════════════════════════════════════════════════════════════

    // ── wasi:io/error@0.2.2 instance type (needed by streams) ──
    let error_type_idx = {
        let (it, ie) = b.ty(None);
        let mut inst = InstanceType::new();
        inst.export("error", ComponentTypeRef::Type(TypeBounds::SubResource));
        ie.instance(&inst);
        it
    };
    let error_comp_inst = b.import("wasi:io/error@0.2.2", ComponentTypeRef::Instance(error_type_idx));
    let error_type = b.alias_export(error_comp_inst, "error", ComponentExportKind::Type);

    // ── wasi:io/streams@0.2.2 instance type ──
    // Must match the exact shape from WIT: exports input-stream, output-stream (sub resource),
    // error (eq aliased), stream-error (variant), and two methods.
    // Match reference p2_wasi_http.wasm exactly:
    // 0=input-stream(SubResource), 1=output-stream(SubResource),
    // 2=alias error, 3=export error Eq(2), 4=own(3), 5=stream-error variant,
    // 6=export stream-error Eq(5), 7=borrow(0), 8=list<u8> inline,
    // 9=result<8,6>, 10=func(7,u64)->9, export read func(10),
    // 11=borrow(1), 12=result<_,6>, 13=func(11,8)->12, export write func(13)
    let streams_type_idx = {
        let (it, ie) = b.ty(None);
        let mut inst = InstanceType::new();
        // 0: input-stream (SubResource)
        inst.export("input-stream", ComponentTypeRef::Type(TypeBounds::SubResource));
        // 1: output-stream (SubResource)
        inst.export("output-stream", ComponentTypeRef::Type(TypeBounds::SubResource));
        // 2: alias error from outer (component-level error_type)
        inst.alias(Alias::Outer {
            kind: ComponentOuterAliasKind::Type,
            count: 1,
            index: error_type,
        });
        // 3: export "error" (eq 2)
        inst.export("error", ComponentTypeRef::Type(TypeBounds::Eq(2)));
        // 4: own(3) — own<error>
        inst.ty().defined_type().own(3);
        // 5: stream-error variant { last-operation-failed(4), closed }
        inst.ty().defined_type().variant([
            ("last-operation-failed", Some(ComponentValType::Type(4))),
            ("closed", None),
        ]);
        // 6: export "stream-error" (eq 5)
        inst.export("stream-error", ComponentTypeRef::Type(TypeBounds::Eq(5)));
        // 7: borrow(0) — borrow<input-stream>
        inst.ty().defined_type().borrow(0);
        // 8: list<u8> defined inline (NOT aliased from outer)
        inst.ty().defined_type().list(ComponentValType::Primitive(PrimitiveValType::U8));
        // 9: result<list<u8>=8, stream-error=6>
        inst.ty().defined_type().result(
            Some(ComponentValType::Type(8)),
            Some(ComponentValType::Type(6)),
        );
        // 10: func (self:7, len: u64) -> result<list<u8>, stream-error>=9
        inst.ty().function()
            .params([
                ("self", ComponentValType::Type(7)),
                ("len", ComponentValType::Primitive(PrimitiveValType::U64)),
            ])
            .result(Some(ComponentValType::Type(9)));
        // Export "[method]input-stream.read" func(10)
        inst.export("[method]input-stream.read", ComponentTypeRef::Func(10));
        // 11: borrow(1) — borrow<output-stream>
        inst.ty().defined_type().borrow(1);
        // 12: result<_, stream-error=6>
        inst.ty().defined_type().result(None, Some(ComponentValType::Type(6)));
        // 13: func (self:11, contents:8) -> result<_, stream-error>=12
        inst.ty().function()
            .params([
                ("self", ComponentValType::Type(11)),
                ("contents", ComponentValType::Type(8)),
            ])
            .result(Some(ComponentValType::Type(12)));
        // Export "[method]output-stream.blocking-write-and-flush" func(13)
        inst.export("[method]output-stream.blocking-write-and-flush", ComponentTypeRef::Func(13));
        ie.instance(&inst);
        it
    };
    let streams_comp_inst = b.import(
        "wasi:io/streams@0.2.2",
        ComponentTypeRef::Instance(streams_type_idx),
    );
    // ── Alias resource types from streams import to component level ──
    let input_stream_type = b.alias_export(streams_comp_inst, "input-stream", ComponentExportKind::Type);
    let output_stream_type = b.alias_export(streams_comp_inst, "output-stream", ComponentExportKind::Type);

    // ── wasi:cli/stdin@0.2.2 instance type ──
    let stdin_type_idx = {
        let (it, ie) = b.ty(None);
        let mut inst = InstanceType::new();
        // Alias input-stream resource from outer (component-level) — local 0
        inst.alias(Alias::Outer {
            kind: ComponentOuterAliasKind::Type,
            count: 1,
            index: input_stream_type,
        });
        // Re-export as local type — local 1 = (eq 0)
        inst.export("input-stream", ComponentTypeRef::Type(TypeBounds::Eq(0)));
        // own<input-stream> using local 1 — local 2
        inst.ty().defined_type().own(1);
        // get-stdin func: () -> own(1) — local 3
        inst.ty().function()
            .params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(2)));
        inst.export("get-stdin", ComponentTypeRef::Func(3));
        ie.instance(&inst);
        it
    };
    let stdin_comp_inst = b.import(
        "wasi:cli/stdin@0.2.2",
        ComponentTypeRef::Instance(stdin_type_idx),
    );

    // ── wasi:cli/stdout@0.2.2 instance type ──
    let stdout_type_idx = {
        let (it, ie) = b.ty(None);
        let mut inst = InstanceType::new();
        // Alias output-stream resource from outer (component-level) — local 0
        inst.alias(Alias::Outer {
            kind: ComponentOuterAliasKind::Type,
            count: 1,
            index: output_stream_type,
        });
        // Re-export as local type — local 1 = (eq 0)
        inst.export("output-stream", ComponentTypeRef::Type(TypeBounds::Eq(0)));
        // own<output-stream> using local 1 — local 2
        inst.ty().defined_type().own(1);
        // get-stdout func: () -> own(1) — local 3
        inst.ty().function()
            .params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(2)));
        inst.export("get-stdout", ComponentTypeRef::Func(3));
        ie.instance(&inst);
        it
    };
    let stdout_comp_inst = b.import(
        "wasi:cli/stdout@0.2.2",
        ComponentTypeRef::Instance(stdout_type_idx),
    );

    // ── Lower P2 stream functions to core ──
    let get_stdin_comp = b.alias_export(stdin_comp_inst, "get-stdin", ComponentExportKind::Func);
    let get_stdin_core = b.lower_func(None, get_stdin_comp, [CanonicalOption::Memory(mem)]);

    let get_stdout_comp = b.alias_export(stdout_comp_inst, "get-stdout", ComponentExportKind::Func);
    let get_stdout_core = b.lower_func(None, get_stdout_comp, [CanonicalOption::Memory(mem)]);

    // Methods use [method] naming in streams instance type
    // Alias [method]input-stream.read from streams instance (not blocking-read)
    let blocking_read_comp = b.alias_export(
        streams_comp_inst,
        "[method]input-stream.read",
        ComponentExportKind::Func,
    );
    // Realloc needed for blocking_read (returns list<u8> which host writes to memory)
    let blocking_read_core = b.lower_func(None, blocking_read_comp, [
        CanonicalOption::Memory(mem),
        CanonicalOption::Realloc(realloc),
    ]);

    let blocking_write_comp = b.alias_export(
        streams_comp_inst,
        "[method]output-stream.blocking-write-and-flush",
        ComponentExportKind::Func,
    );
    // No realloc needed for blocking_write — list<u8> is a param (guest→host), result is just discriminant
    let blocking_write_core = b.lower_func(None, blocking_write_comp, [
        CanonicalOption::Memory(mem),
    ]);

    // Resource drops use canon resource.drop on the component-level type
    let drop_input_core = b.resource_drop(input_stream_type);
    let drop_output_core = b.resource_drop(output_stream_type);

    // ── Build P2 WASI bridge module ──
    let bridge_bytes = crate::p2_wasi_bridge::build_p2_wasi_bridge();
    let bridge_mod = b.core_module_raw(None, &bridge_bytes);

    let p2_funcs_inst = b.core_instantiate_exports(
        None,
        [
            ("get_stdin", ExportKind::Func, get_stdin_core),
            ("blocking_read", ExportKind::Func, blocking_read_core),
            ("drop_input_stream", ExportKind::Func, drop_input_core),
            ("get_stdout", ExportKind::Func, get_stdout_core),
            ("blocking_write_and_flush", ExportKind::Func, blocking_write_core),
            ("drop_output_stream", ExportKind::Func, drop_output_core),
        ],
    );

    let bridge_inst = b.core_instantiate(None, bridge_mod, [
        ("env", ModuleArg::Instance(mem_inst)),
        ("p2", ModuleArg::Instance(p2_funcs_inst)),
    ]);

    let mut lowered_funcs: Vec<u32> = Vec::new();
    let mut lowered_names: Vec<String> = Vec::new();
    let mut outlayer_inst_idx: u32 = 0;
    if has_outlayer {
        let (ref iface_imports, ref wit_names, _ref_types) = used_interface_imports.as_ref().unwrap();

        // Lower ALL outlayer functions that the core module imports
        for kebab in &info.names {
            // Only lower functions that exist in WIT
            if !WIT_FUNC_NAMES.contains(&kebab.as_str()) {
                eprintln!("⚠️ No WIT type for {}, skipping", kebab);
                continue;
            }
            // Skip functions that are imported but never called —
            // use a type-matched trap stub so the core module's imports are satisfied
            if !called_names.contains(kebab.as_str()) {
                eprintln!("DEBUG: trap stub for unused import {}", kebab);
                let sig = core_import_sig(kebab.as_str());
                let trap = trap_stubs.get(&sig).expect(&format!("no trap stub for {} sig {:?}", kebab, sig));
                lowered_funcs.push(*trap);
                lowered_names.push(kebab.clone());
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
            // Determine which interface this function belongs to
            let iface = func_interface(kebab.as_str());
            if iface == "trap" {
                // No production split interface — use trap stub even for called functions
                eprintln!("⚠️ No production split interface for {}, using trap stub", kebab);
                let sig = core_import_sig(kebab.as_str());
                let trap = trap_stubs
                    .get(&sig)
                    .unwrap_or_else(|| panic!("no trap stub for {} sig {:?}", kebab, sig));
                lowered_funcs.push(*trap);
                lowered_names.push(kebab.clone());
                continue;
            }
            let iface_key = match iface {
                "near:storage/api@0.1.0" => "storage",
                "near:rpc/api@0.1.0" => "rpc",
                "lisp:http-adapter/api@0.1.0" => "http",
                _ => unreachable!(),
            };
            eprintln!(
                "DEBUG: lowering {} iface_key={} keys={:?}",
                kebab,
                iface_key,
                iface_imports.keys().collect::<Vec<_>>()
            );
            let import_idx = *iface_imports
                .get(iface_key)
                .unwrap_or_else(|| panic!("no import for iface_key={} ({})", iface_key, kebab));
            let wit_name = func_wit_name(kebab.as_str());
            let comp_func = b.alias_export(import_idx, wit_name, ComponentExportKind::Func);
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

    // ── Instantiate core module with env(memory) + WASI bridge + outlayer ──
    let core_inst = if has_outlayer {
        b.core_instantiate(
            None,
            module_idx,
            [
                ("env", ModuleArg::Instance(mem_inst)),
                ("outlayer:api/host", ModuleArg::Instance(outlayer_inst_idx)),
                ("wasi_snapshot_preview1", ModuleArg::Instance(bridge_inst)),
            ],
        )
    } else {
        b.core_instantiate(
            None,
            module_idx,
            [
                ("env", ModuleArg::Instance(mem_inst)),
                ("wasi_snapshot_preview1", ModuleArg::Instance(bridge_inst)),
            ],
        )
    };

    // ── Wrap _start to return i32(0) for wasi:cli/run result<()> ──
    // The lift expects () -> result<()> which in canonical ABI = () -> i32 (discriminant)
    // Our core _start returns (), so we wrap it with a tiny adapter module
    let wrapper_bytes = build_run_wrapper();
    let wrapper_mod = b.core_module_raw(None, &wrapper_bytes);
    let wrapper_inst = b.core_instantiate(None, wrapper_mod, [
        ("env", ModuleArg::Instance(core_inst)),
    ]);
    let run_core = b.core_alias_export(None, wrapper_inst, "run", ExportKind::Func);
    let run_func = b.lift_func(None, run_core, run_type, []);

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

/// Build a tiny WASM module that exports memory, a bump-allocator realloc,
/// and per-signature unreachable trap stubs for unused outlayer imports.
fn build_memory_module(min_pages: u64) -> (Vec<u8>, std::collections::HashMap<Vec<wasm_encoder::ValType>, u32>) {
    use wasm_encoder::*;
    let mut m = Module::new();

    // Collect unique param signatures needed for trap stubs
    // All outlayer imports return () — only params vary
    let trap_sigs: Vec<Vec<ValType>> = vec![
        vec![ValType::I32; 1],   // storage-clear-all, env-signer, env-predecessor
        vec![ValType::I32; 3],   // http-get, storage-get, storage-has, storage-delete, storage-list-keys
        vec![ValType::I32; 5],   // storage-set, storage-set-if-absent, storage-set-worker, storage-set-worker-public, storage-get-worker-from-project
        vec![ValType::I32; 7],   // view, transfer, http-post, storage-set-if-equals
        vec![ValType::I32; 13],  // call
        vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], // storage-increment, storage-decrement (s64)
    ];

    // Type section
    let mut types = TypeSection::new();
    // type 0: () -> ()
    types.ty().function([], []);
    // type 1: (i32, i32, i32, i32) -> i32 — realloc signature
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]);
    // types 2..: per-signature trap stubs (all return ())
    let mut trap_type_map: std::collections::HashMap<Vec<ValType>, u32> = std::collections::HashMap::new();
    for (i, sig) in trap_sigs.iter().enumerate() {
        let type_idx = 2 + i as u32;
        types.ty().function(sig.iter().copied(), []);
        trap_type_map.insert(sig.clone(), type_idx);
    }
    m.section(&types);

    // Function section: func 0 = realloc (type 1), funcs 1.. = trap stubs
    let mut funcs = FunctionSection::new();
    funcs.function(1); // realloc: type 1
    for (_, &type_idx) in &trap_type_map {
        funcs.function(type_idx);
    }
    m.section(&funcs);

    // Memory section
    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: min_pages + 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    // Global: bump pointer (i32, mutable)
    let mut globals = GlobalSection::new();
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
    // Export trap stubs by name for reference (func indices 1..)
    let mut func_idx = 1u32;
    let mut trap_func_map: std::collections::HashMap<Vec<ValType>, u32> = std::collections::HashMap::new();
    for (sig, _) in &trap_type_map {
        let name = format!("trap_{}i32", sig.len());
        exports.export(&name, ExportKind::Func, func_idx);
        trap_func_map.insert(sig.clone(), func_idx);
        func_idx += 1;
    }
    m.section(&exports);

    // Code section
    let mut codes = CodeSection::new();
    // realloc: bump allocator
    let mut f = Function::new([(0, ValType::I32), (1, ValType::I32), (2, ValType::I32), (3, ValType::I32)]);
    f.instruction(&Instruction::GlobalGet(0));
    f.instruction(&Instruction::LocalTee(0));
    f.instruction(&Instruction::LocalGet(3));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::GlobalSet(0));
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::End);
    codes.function(&f);

    // Trap stubs: one per unique signature, all just unreachable
    for sig in &trap_sigs {
        let locals: Vec<(u32, ValType)> = sig.iter().enumerate().map(|(i, &vt)| (i as u32, vt)).collect();
        let mut trap = Function::new(locals);
        trap.instruction(&Instruction::Unreachable);
        trap.instruction(&Instruction::End);
        codes.function(&trap);
    }

    m.section(&codes);

    (m.finish(), trap_func_map)
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

/// Tiny core module that calls `_start` from "env" instance and returns i32(0).
/// This bridges the core module's `() -> ()` _start to the canonical ABI's
/// `() -> i32` expected by the lift for `wasi:cli/run` `() -> result<()>`.
fn build_run_wrapper() -> Vec<u8> {
    use wasm_encoder::{Module, TypeSection, ImportSection, FunctionSection, ExportSection, CodeSection, Function, Instruction};

    let mut m = Module::new();
    // Type section
    let mut types = TypeSection::new();
    types.ty().function([], []);          // type 0: () -> ()  (_start signature)
    types.ty().function([], [ValType::I32]); // type 1: () -> i32 (run wrapper signature)
    m.section(&types);

    // Import section: import _start from env
    let mut imports = ImportSection::new();
    imports.import("env", "_start", wasm_encoder::EntityType::Function(0));
    m.section(&imports);

    // Function section: func 1 (after import) = type 1
    let mut funcs = FunctionSection::new();
    funcs.function(1);
    m.section(&funcs);

    // Export section: export "run" = func 1
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    m.section(&exports);

    // Code section: call _start, return 0
    let mut codes = CodeSection::new();
    let mut f = Function::new([]);
    f.instruction(&Instruction::Call(0)); // call imported _start
    f.instruction(&Instruction::I32Const(0)); // result<()> discriminant = 0 (ok)
    f.instruction(&Instruction::End);
    codes.function(&f);
    m.section(&codes);

    m.finish()
}

fn build_run_shim() -> Vec<u8> {
    let mut shim = ComponentBuilder::default();
    // type 0: result<()> — ok: None, err: None
    let (result_type, renc) = shim.type_defined(None);
    {
        renc.result(None::<ComponentValType>, None::<ComponentValType>);
    }
    // type 1: () -> result<()>
    let (fn_type, fenc) = shim.ty(None);
    {
        fenc.function()
            .params([] as [(&str, ComponentValType); 0])
            .result(Some(ComponentValType::Type(result_type)));
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
    /// Indices (0-based among outlayer imports) of functions that have actual Call instructions
    called: std::collections::HashSet<usize>,
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

    // Count total function imports (all modules) to know import func index range
    let mut total_import_funcs = 0u32;
    let mut outlayer_import_start = 0u32; // func index where outlayer imports begin
    let mut outlayer_names_in_order: Vec<String> = Vec::new();

    // Read import section
    pos = 8;
    while pos < wasm.len() {
        let sid = wasm[pos];
        pos += 1;
        let (sz, lb) = rleb(wasm, pos);
        pos += lb;
        if sid == 2 {
            let section_end = pos + sz;
            let (cnt, cl) = rleb(wasm, pos);
            pos += cl;
            let mut found_outlayer = false;
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
                            if !found_outlayer {
                                outlayer_import_start = total_import_funcs;
                                found_outlayer = true;
                            }
                            names.push(name.clone());
                            outlayer_names_in_order.push(name);
                            param_counts.push(type_params.get(tl as usize).copied().unwrap_or(0));
                        }
                        // Detect wasi:http imports (informational — not used here)
                        total_import_funcs += 1;
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
            pos = section_end;
            break;
        }
        pos += sz;
    }

    // Scan code section for Call instructions to find which imported functions are actually used
    let mut called: std::collections::HashSet<usize> = std::collections::HashSet::new();
    pos = 8;
    while pos < wasm.len() {
        let sid = wasm[pos];
        pos += 1;
        let (sz, lb) = rleb(wasm, pos);
        pos += lb;
        if sid == 10 {
            // Code section
            let section_end = pos + sz;
            let (_cnt, cl) = rleb(wasm, pos);
            pos += cl;
            // Scan raw bytes for Call opcodes (0x10) followed by function index
            while pos < section_end {
                // Skip function body header (local count + locals)
                let (_body_size, bsl) = rleb(wasm, pos);
                pos += bsl;
                let body_end = pos + _body_size;
                let (_local_cnt, lcl) = rleb(wasm, pos);
                pos += lcl;
                for _ in 0.._local_cnt {
                    let (_repeat, rl) = rleb(wasm, pos);
                    pos += rl;
                    pos += 1; // type byte
                }
                // Scan instructions
                while pos < body_end {
                    let opcode = wasm[pos];
                    pos += 1;
                    if opcode == 0x10 {
                        // Call
                        let (func_idx, fl) = rleb(wasm, pos);
                        pos += fl;
                        // Check if this is an outlayer import
                        if func_idx >= outlayer_import_start as usize
                            && func_idx < outlayer_import_start as usize + outlayer_names_in_order.len()
                        {
                            called.insert(func_idx - outlayer_import_start as usize);
                        }
                    } else if opcode == 0x0C || opcode == 0x0D {
                        // br / br_if — skip label index
                        let (_, ll) = rleb(wasm, pos);
                        pos += ll;
                    } else if opcode == 0x0E {
                        // br_table — skip count + labels + default
                        let (count, cl) = rleb(wasm, pos);
                        pos += cl;
                        for _ in 0..=count {
                            let (_, ll) = rleb(wasm, pos);
                            pos += ll;
                        }
                    } else if opcode == 0x11 {
                        // CallIndirect — skip type idx + table idx
                        let (_, tl) = rleb(wasm, pos);
                        pos += tl;
                        pos += 1; // table index byte
                    } else if opcode >= 0x20 && opcode <= 0x24 {
                        // local.get/local.set/local.tee/global.get/global.set
                        let (_, ll) = rleb(wasm, pos);
                        pos += ll;
                    } else if opcode == 0x28 || opcode == 0x29 || opcode == 0x2A || opcode == 0x2B
                        || opcode == 0x2C || opcode == 0x2D || opcode == 0x2E || opcode == 0x2F
                    {
                        // Load instructions — skip align + offset
                        pos += 1; // align
                        let (_, ol) = rleb(wasm, pos);
                        pos += ol;
                    } else if opcode >= 0x34 && opcode <= 0x3E {
                        // Store instructions — skip align + offset
                        pos += 1;
                        let (_, ol) = rleb(wasm, pos);
                        pos += ol;
                    } else if opcode == 0x41 {
                        // i32.const
                        let (_, vl) = rleb(wasm, pos);
                        pos += vl;
                    } else if opcode == 0x42 {
                        // i64.const — skip 8 bytes (signed LEB128 up to 10 bytes)
                        let mut _shift = 0u32;
                        loop {
                            let byte = wasm[pos];
                            pos += 1;
                            if byte & 0x80 == 0 {
                                break;
                            }
                            _shift += 7;
                            if _shift >= 64 {
                                break;
                            }
                        }
                    } else if opcode == 0x0B || opcode == 0x00 {
                        // end / unreachable — no operands
                    } else if opcode == 0x01 {
                        // nop
                    } else if opcode == 0x04 || opcode == 0x02 {
                        // if / block — skip block type
                        let bt = wasm[pos];
                        pos += 1;
                        if bt == 0x40 {
                            // void — no type
                        } else if bt == 0x7F || bt == 0x7E || bt == 0x7D || bt == 0x7C {
                            // value type — done
                        } else {
                            // Type index (signed LEB)
                            pos -= 1;
                            let (_, tl) = rleb(wasm, pos);
                            pos += tl;
                        }
                    } else if opcode == 0x03 {
                        // loop — skip block type (same as block)
                        let bt = wasm[pos];
                        pos += 1;
                        if bt != 0x40 && bt != 0x7F && bt != 0x7E && bt != 0x7D && bt != 0x7C {
                            pos -= 1;
                            let (_, tl) = rleb(wasm, pos);
                            pos += tl;
                        }
                    } else if opcode == 0x05 {
                        // else — no operands
                    } else if opcode == 0x0F {
                        // return — no operands
                    } else if opcode == 0x43 || opcode == 0x44 {
                        // f32.const / f64.const
                        let sz = if opcode == 0x43 { 4 } else { 8 };
                        pos += sz;
                    } else if opcode == 0x50 || opcode == 0x51 || opcode == 0x52 || opcode == 0x53
                        || opcode == 0x54 || opcode == 0x55 || opcode == 0x56 || opcode == 0x57
                        || opcode == 0x58 || opcode == 0x59 || opcode == 0x5A || opcode == 0x5B
                        || opcode == 0x5C || opcode == 0x5D || opcode == 0x5E || opcode == 0x5F
                        || opcode == 0x60 || opcode == 0x61 || opcode == 0x62 || opcode == 0x63
                        || opcode == 0x64 || opcode == 0x65 || opcode == 0x66 || opcode == 0x67
                        || opcode == 0x68 || opcode == 0x69 || opcode == 0x6A || opcode == 0x6B
                        || opcode == 0x6C || opcode == 0x6D || opcode == 0x6E || opcode == 0x6F
                        || opcode == 0x70 || opcode == 0x71 || opcode == 0x72 || opcode == 0x73
                        || opcode == 0x74 || opcode == 0x75 || opcode == 0x76 || opcode == 0x77
                        || opcode == 0x78 || opcode == 0x79 || opcode == 0x7A || opcode == 0x7B
                    {
                        // Numeric i32/i64/f32/f64 operations — no operands
                    }
                    // Skip any unknown opcodes — this is a best-effort scanner
                }
                pos = body_end;
            }
            break;
        }
        pos += sz;
    }

    OutlayerInfo {
        names,
        param_counts,
        called,
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

    // Embed WIT metadata (imports + wasi:cli/run export)
    let (resolve, world) =
        crate::wasi_http::build_http_wit_metadata().map_err(|e| format!("WIT metadata: {}", e))?;
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
