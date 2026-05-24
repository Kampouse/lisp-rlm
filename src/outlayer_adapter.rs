//! Build the outlayer adapter WASM — a passthrough bridge for outlayer:api/host.
//!
//! Bridges core module imports to runtime host functions via wit-component adapter.
//! The adapter imports from `outlayer-impl:api/host` to avoid wit-component cycles,
//! then import_name_map renames to `outlayer:api/host` in the final component.
//!
//! All functions use proper canonical ABI types, including s64 for increment/decrement.
//! Adapter modules can only import functions — no memory import/definition allowed.
//! cabi_realloc is a stub since the adapter never allocates.

use wasm_encoder::*;

const W: ValType = ValType::I32;
const I64: ValType = ValType::I64;

struct FuncDef {
    name: &'static str,
    params: &'static [ValType],
}

const ADAPTER_FUNCS: &[FuncDef] = &[
    FuncDef { name: "view", params: &[W, W, W, W, W, W, W] },
    FuncDef { name: "call", params: &[W, W, W, W, W, W, W, W, W, W, W, W, W] },
    FuncDef { name: "transfer", params: &[W, W, W, W, W, W, W] },
    FuncDef { name: "http-get", params: &[W, W, W] },
    FuncDef { name: "http-post", params: &[W, W, W, W, W, W, W] },
    FuncDef { name: "storage-set", params: &[W, W, W, W, W] },
    FuncDef { name: "storage-get", params: &[W, W, W] },
    FuncDef { name: "storage-has", params: &[W, W, W] },
    FuncDef { name: "storage-delete", params: &[W, W, W] },
    FuncDef { name: "storage-increment", params: &[W, W, I64, W] },
    FuncDef { name: "storage-decrement", params: &[W, W, I64, W] },
    FuncDef { name: "storage-set-if-absent", params: &[W, W, W, W, W] },
    FuncDef { name: "storage-set-if-equals", params: &[W, W, W, W, W, W, W] },
    FuncDef { name: "storage-list-keys", params: &[W, W, W] },
    FuncDef { name: "storage-clear-all", params: &[W] },
    FuncDef { name: "storage-set-worker", params: &[W, W, W, W, W] },
    FuncDef { name: "storage-get-worker", params: &[W, W, W] },
    FuncDef { name: "storage-set-worker-public", params: &[W, W, W, W, W] },
    FuncDef { name: "storage-get-worker-from-project", params: &[W, W, W, W, W] },
    FuncDef { name: "env-signer", params: &[W] },
    FuncDef { name: "env-predecessor", params: &[W] },
];

const ADAPTER_IMPORT_NS: &str = "outlayer-impl:api/host";

/// WIT with real s64 types matching the production outlayer:api/host interface.
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
    storage-increment: func(key: string, delta: s64) -> result<s64, string>;
    storage-decrement: func(key: string, delta: s64) -> result<s64, string>;
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

    // ═══ Type section ═══
    let mut types = TypeSection::new();
    let mut sig_to_type: std::collections::HashMap<Vec<ValType>, u32> = std::collections::HashMap::new();
    for desc in ADAPTER_FUNCS.iter() {
        if !sig_to_type.contains_key(desc.params) {
            let idx = sig_to_type.len() as u32;
            types.ty().function(desc.params.iter().copied(), std::iter::empty::<ValType>());
            sig_to_type.insert(desc.params.to_vec(), idx);
        }
    }
    // cabi_realloc type: (i32, i32, i32, i32) -> i32
    let cabi_realloc_type_idx = sig_to_type.len() as u32;
    types.ty().function([W, W, W, W], Some(W));
    module.section(&types);

    // ═══ Import section — function imports only (adapters can't import memory) ═══
    let mut imports = ImportSection::new();
    for desc in ADAPTER_FUNCS.iter() {
        let type_idx = sig_to_type[desc.params];
        imports.import(ADAPTER_IMPORT_NS, desc.name, EntityType::Function(type_idx));
    }
    module.section(&imports);

    // ═══ Function section — adapter bodies + cabi_realloc ═══
    let mut funcs = FunctionSection::new();
    for desc in ADAPTER_FUNCS.iter() {
        funcs.function(sig_to_type[desc.params]);
    }
    funcs.function(cabi_realloc_type_idx);
    module.section(&funcs);

    // ═══ Export section ═══
    let mut exports = ExportSection::new();
    for (i, desc) in ADAPTER_FUNCS.iter().enumerate() {
        exports.export(desc.name, ExportKind::Func, (num_funcs + i) as u32);
    }
    let realloc_idx = (num_funcs + ADAPTER_FUNCS.len()) as u32;
    exports.export("cabi_realloc", ExportKind::Func, realloc_idx);
    module.section(&exports);

    // ═══ Code section — passthrough + cabi_realloc stub ═══
    let mut codes = CodeSection::new();
    for (i, desc) in ADAPTER_FUNCS.iter().enumerate() {
        let mut func = Function::new(Vec::new());
        let mut local_idx = 0u32;
        for _ in desc.params.iter() {
            func.instruction(&Instruction::LocalGet(local_idx));
            local_idx += 1;
        }
        func.instruction(&Instruction::Call(i as u32));
        func.instruction(&Instruction::End);
        codes.function(&func);
    }

    // cabi_realloc stub: adapter never allocates, so just return old_ptr
    let mut realloc = Function::new(Vec::new());
    // If new_size == 0, return 0 (free)
    realloc.instruction(&Instruction::LocalGet(3)); // new_size
    realloc.instruction(&Instruction::I32Eqz);
    realloc.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::Return);
    realloc.instruction(&Instruction::End);
    // Otherwise return old_ptr unchanged
    realloc.instruction(&Instruction::LocalGet(0)); // old_ptr
    realloc.instruction(&Instruction::End);
    codes.function(&realloc);

    module.section(&codes);

    let bytes = module.finish();

    // Embed WIT metadata
    let mut resolve = wit_parser::Resolve::new();
    let ast = wit_parser::UnresolvedPackageGroup::parse("wit/outlayer-impl/api/host.wit", ADAPTER_WIT)
        .expect("failed to parse adapter WIT");
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
    )
    .expect("failed to embed WIT metadata");

    bytes
}
