//! Build the outlayer adapter WASM — a passthrough bridge for outlayer:api/host.
//!
//! Bridges core module imports to runtime host functions via wit-component adapter.
//! The adapter imports from `outlayer-impl:api/host` to avoid wit-component cycles,
//! then import_name_map renames to `outlayer:api/host` in the final component.
//!
//! Only includes functions that use pure canonical-ABI-compatible types (strings, lists, bools).
//! Functions with s64 params (storage-increment/decrement) are excluded because the
//! core module uses all-i32 calling convention which doesn't match canonical i64 ABI.

use wasm_encoder::*;

/// Functions included in the adapter — only those with pure i32 canonical ABI params.
const ADAPTER_FUNCS: &[(&str, usize)] = &[
    ("view", 7),            // 3 strings (6 i32) + ret_area
    ("call", 13),           // 6 strings (12 i32) + ret_area
    ("transfer", 7),        // 3 strings (6 i32) + ret_area
    ("http-get", 3),        // 1 string (2 i32) + ret_area
    ("http-post", 7),       // 1 string + 1 list<u8> + 1 string (6 i32) + ret_area
    ("storage-set", 5),     // 1 string + 1 list<u8> (4 i32) + ret_area
    ("storage-get", 3),     // 1 string (2 i32) + ret_area
    ("storage-has", 3),     // 1 string (2 i32) + ret_area
    ("storage-delete", 3),  // 1 string (2 i32) + ret_area
    ("storage-increment", 4), // 1 string (2 i32) + s32 (1 i32) + ret_area
    ("storage-decrement", 4), // same
    ("storage-set-if-absent", 5),   // 1 string + 1 list<u8> (4 i32) + ret_area
    ("storage-set-if-equals", 7),   // 1 string + 2 list<u8> (6 i32) + ret_area
    ("storage-list-keys", 3),       // 1 string (2 i32) + ret_area
    ("storage-clear-all", 1),       // ret_area only
    ("storage-set-worker", 5),      // 1 string + 1 list<u8> (4 i32) + ret_area
    ("storage-get-worker", 3),      // 1 string (2 i32) + ret_area
    ("storage-set-worker-public", 5),  // 1 string + 1 list<u8> (4 i32) + ret_area
    ("storage-get-worker-from-project", 5), // 2 strings (4 i32) + ret_area
    ("env-signer", 1),       // ret_area only
    ("env-predecessor", 1),  // ret_area only
];

const ADAPTER_IMPORT_NS: &str = "outlayer-impl:api/host";

/// WIT with s64 functions replaced by placeholder s32 — allows wit-component to accept all-i32 params.
/// We replace `delta: s64` with `delta: s32` so canonical ABI generates i32 instead of i64.
const ADAPTER_WIT: &str = r#"
package outlayer-impl:api;

interface host {
    view: func(contract-id: string, method-name: string, args-json: string) -> result<string, string>;
    call: func(signer-key: string, receiver-id: string, method-name: string, args-json: string, deposit-yocto: string, gas: string) -> result<string, string>;
    transfer: func(signer-key: string, receiver-id: string, amount-yocto: string) -> result<string, string>;
    http-get: func(url: string) -> result<list<u8>, string>;
    http-post: func(url: string, body: list<u8>, content-type: string) -> result<list<u8>, string>;
    storage-set: func(key: string, value: list<u8>) -> result<_, string>;
    storage-get: func(key: string) -> result<option<list<u8>>, string>;
    storage-has: func(key: string) -> result<bool, string>;
    storage-delete: func(key: string) -> result<_, string>;
    storage-increment: func(key: string, delta: s32) -> result<s32, string>;
    storage-decrement: func(key: string, delta: s32) -> result<s32, string>;
    storage-set-if-absent: func(key: string, value: list<u8>) -> result<bool, string>;
    storage-set-if-equals: func(key: string, expected: list<u8>, new-value: list<u8>) -> result<bool, string>;
    storage-list-keys: func(prefix: string) -> result<list<string>, string>;
    storage-clear-all: func() -> result<_, string>;
    storage-set-worker: func(key: string, value: list<u8>) -> result<_, string>;
    storage-get-worker: func(key: string) -> result<option<list<u8>>, string>;
    storage-set-worker-public: func(key: string, value: list<u8>) -> result<_, string>;
    storage-get-worker-from-project: func(key: string, project: string) -> result<option<list<u8>>, string>;
    env-signer: func() -> string;
    env-predecessor: func() -> string;
}

world outlayer-world {
    import host;
}
"#;

pub fn build_outlayer_adapter() -> Vec<u8> {
    let num_funcs = ADAPTER_FUNCS.len();

    let mut module = Module::new();

    // Type section — all (i32 * N) -> ()
    let mut types = TypeSection::new();
    // Deduplicate param counts
    let mut unique_counts: Vec<usize> = ADAPTER_FUNCS.iter().map(|(_, n)| *n).collect();
    unique_counts.sort();
    unique_counts.dedup();
    let mut count_to_type: std::collections::HashMap<usize, u32> = std::collections::HashMap::new();
    for (i, &count) in unique_counts.iter().enumerate() {
        let params: Vec<ValType> = (0..count).map(|_| ValType::I32).collect();
        types.ty().function(params, []);
        count_to_type.insert(count, i as u32);
    }
    module.section(&types);

    // Import section
    let mut imports = ImportSection::new();
    for (i, (name, nparams)) in ADAPTER_FUNCS.iter().enumerate() {
        let type_idx = count_to_type[nparams];
        imports.import(ADAPTER_IMPORT_NS, *name, EntityType::Function(type_idx));
    }
    module.section(&imports);

    // Function section — adapter bodies with same types
    let mut funcs = FunctionSection::new();
    for (_, nparams) in ADAPTER_FUNCS.iter() {
        funcs.function(count_to_type[nparams]);
    }
    module.section(&funcs);

    // Export section — adapter function indices start at num_funcs
    let mut exports = ExportSection::new();
    for (i, (name, _)) in ADAPTER_FUNCS.iter().enumerate() {
        exports.export(*name, ExportKind::Func, (num_funcs + i) as u32);
    }
    module.section(&exports);

    // Code section — passthrough
    let mut codes = CodeSection::new();
    for (_, nparams) in ADAPTER_FUNCS.iter() {
        let mut func = Function::new(Vec::new());
        for p in 0..*nparams {
            func.instruction(&Instruction::LocalGet(p as u32));
        }
        func.instruction(&Instruction::End);
        codes.function(&func);
    }
    module.section(&codes);

    let bytes = module.finish();

    // Embed WIT metadata with s32 instead of s64 (matches all-i32 canonical ABI)
    let mut resolve = wit_parser::Resolve::new();
    let ast = wit_parser::UnresolvedPackageGroup::parse(
        "wit/outlayer-impl/api/host.wit",
        ADAPTER_WIT,
    ).expect("failed to parse adapter WIT");
    let pkg_id = resolve.push_group(ast).expect("failed to push WIT package");
    let world_id = resolve.packages[pkg_id]
        .worlds
        .iter()
        .find(|(name, _)| *name == "outlayer-world")
        .map(|(_, &id)| id)
        .expect("outlayer-world not found");

    let mut bytes = bytes;
    wit_component::embed_component_metadata(
        &mut bytes,
        &resolve,
        world_id,
        wit_component::StringEncoding::UTF8,
    ).expect("failed to embed WIT metadata");

    bytes
}
