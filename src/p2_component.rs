//! P2 Component Model emitter — wraps P1 core WASM into a WASI Preview 2 component.
//!
//! Uses `wit-component` to create a proper WASI command component with outlayer imports.
//! The core module must use P2 naming (compile_p2) for correct import resolution.

use anyhow::Result;

/// Build a P2 WASI command component from core WASM.
pub fn build_p2_component(core_bytes: &[u8]) -> Result<Vec<u8>> {
    // Parse WIT
    let wit_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("wit/world.wit");
    let mut resolve = wit_parser::Resolve::new();
    let (pkg, _) = resolve.push_path(&wit_path)
        .map_err(|e| anyhow::anyhow!("WIT: {}", e))?;
    let world_id = resolve.packages[pkg].worlds.iter()
        .find_map(|(name, &id)| if name == "outlayer-world" { Some(id) } else { None })
        .ok_or_else(|| anyhow::anyhow!("World not found"))?;

    // Embed metadata
    let mut module = core_bytes.to_vec();
    wit_component::embed_component_metadata(
        &mut module, &resolve, world_id, wit_component::StringEncoding::UTF8,
    ).map_err(|e| anyhow::anyhow!("Embed: {}", e))?;

    // Encode component
    let mut encoder = wit_component::ComponentEncoder::default()
        .module(&module)
        .map_err(|e| anyhow::anyhow!("Module: {}", e))?
        .validate(false);

    // WASI adapter
    let adapter_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../near-outlayer/worker/wasi_adapter.wasm");
    if adapter_path.exists() {
        let adapter = std::fs::read(&adapter_path)?;
        encoder = encoder.adapter("wasi_snapshot_preview1", &adapter)
            .map_err(|e| anyhow::anyhow!("Adapter: {}", e))?;
    }

    encoder.encode().map_err(|e| anyhow::anyhow!("Encode: {}", e))
}
