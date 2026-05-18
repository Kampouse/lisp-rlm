//! Build the outlayer adapter WASM with proper WIT metadata.
//! This adapter maps flat `outlayer.*` imports to canonical `outlayer:api/outlayer@0.1.0`.

use wasm_encoder::*;

const NUM_FUNCS: usize = 20;

const SIGNATURES: [(usize, bool); NUM_FUNCS] = [
    (8, true), (13, true), (10, true), (5, true), (4, true), (5, true),
    (2, true), (2, true), (6, true), (3, true), (3, true), (6, true),
    (4, true), (8, true), (5, true), (0, true), (4, true), (5, true),
    (4, true), (7, true),
];

/// WIT canonical names (hyphenated) — used for imports from the canonical interface
const WIT_NAMES: [&str; NUM_FUNCS] = [
    "view", "call", "transfer", "http-get", "storage-set", "storage-get",
    "storage-has", "storage-delete", "storage-increment", "env-signer",
    "env-predecessor", "storage-decrement", "storage-set-if-absent",
    "storage-set-if-equals", "storage-list-keys", "storage-clear-all",
    "storage-set-worker", "storage-get-worker", "storage-set-worker-public",
    "storage-get-worker-from-project",
];

/// Flat ABI names (underscored) — used for exports that the core module imports
const FLAT_NAMES: [&str; NUM_FUNCS] = [
    "view", "call", "transfer", "http_get", "storage_set", "storage_get",
    "storage_has", "storage_delete", "storage_increment", "env_signer",
    "env_predecessor", "storage_decrement", "storage_set_if_absent",
    "storage_set_if_equals", "storage_list_keys", "storage_clear_all",
    "storage_set_worker", "storage_get_worker", "storage_set_worker_public",
    "storage_get_worker_from_project",
];

pub fn build_outlayer_adapter() -> Vec<u8> {
    let mut module = Module::new();

    // Type section
    let mut types = TypeSection::new();
    for (nparams, has_result) in SIGNATURES {
        let params: Vec<ValType> = (0..nparams).map(|_| ValType::I32).collect();
        if has_result {
            types.ty().function(params, [ValType::I32]);
        } else {
            types.ty().function(params, []);
        }
    }
    module.section(&types);

    // Import section - imports from the canonical interface (hyphenated names)
    let mut imports = ImportSection::new();
    for (i, name) in WIT_NAMES.iter().enumerate() {
        imports.import(
            "outlayer:api/host@0.1.0",
            *name,
            EntityType::Function(i as u32),
        );
    }
    module.section(&imports);

    // Function section
    let mut funcs = FunctionSection::new();
    for i in 0..NUM_FUNCS {
        funcs.function(i as u32);
    }
    module.section(&funcs);

    // Export section - flat function names for the core module (underscored)
    let mut exports = ExportSection::new();
    for (i, name) in FLAT_NAMES.iter().enumerate() {
        exports.export(*name, ExportKind::Func, i as u32);
    }
    module.section(&exports);

    // Code section - passthrough: call import with same args
    let mut codes = CodeSection::new();
    for (i, (nparams, _)) in SIGNATURES.iter().enumerate() {
        let mut func = Function::new(Vec::new());
        for p in 0..*nparams {
            func.instruction(&Instruction::LocalGet(p as u32));
        }
        func.instruction(&Instruction::Call(i as u32));
        func.instruction(&Instruction::End);
        codes.function(&func);
    }
    module.section(&codes);

    // Embed WIT metadata as a custom section (browser-safe: uses include_str!)
    let wit_bytes = build_wit_metadata();
    module.section(&CustomSection {
        name: std::borrow::Cow::Borrowed("component-type:wit-bindgen:0.1.0:outlayer:api@0.1.0:outlayer-world:encoded world"),
        data: std::borrow::Cow::Owned(wit_bytes),
    });

    module.finish()
}

/// Build WIT metadata using embedded WIT (browser-safe, no filesystem access).
fn build_wit_metadata() -> Vec<u8> {
    // Use include_str! for browser compatibility (no filesystem access needed)
    const OUTLAYER_WIT: &str = include_str!("../wit/outlayer/api/outlayer.wit");

    let mut resolve = wit_parser::Resolve::new();
    let ast = wit_parser::UnresolvedPackageGroup::parse("wit/outlayer/api/outlayer.wit", OUTLAYER_WIT)
        .expect("failed to parse WIT");
    let pkg_id = resolve.push_group(ast).expect("failed to push WIT package");
    let world_id = resolve.packages[pkg_id]
        .worlds.iter()
        .find(|(name, _)| *name == "outlayer-world")
        .map(|(_, &id)| id)
        .expect("outlayer-world not found in WIT package");

    wit_component::metadata::encode(&resolve, world_id, wit_component::StringEncoding::UTF8, None)
        .expect("failed to encode WIT metadata")
}