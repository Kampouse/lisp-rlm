//! Native P2 component emission (no WASI adapter).
//!
//! Instead of using the P1→P2 adapter (48K instructions overhead),
//! this emits a component that directly imports P2 stream interfaces
//! and uses canonical lowering/lifting to connect to the core module.

use wasm_encoder::*;
use crate::wasm_emit::WasmEmitter;

/// Build a native P2 component from the core module bytes.
/// The core module must export `_start` and `memory`, and must NOT
/// import `wasi_snapshot_preview1` (those are provided by the component layer).
pub fn build_native_p2(core_bytes: &[u8], _em: &WasmEmitter) -> Result<Vec<u8>, String> {
    let mut b = ComponentBuilder::default();

    // ── Component Types ──
    
    // Type 0: wasi:io/error@0.2.6 (resource)
    let error_res = b.type_resource(Some("wasi:io/error"), ValType::I32, None);
    
    // Type 1: wasi:io/streams@0.2.6/input-stream (resource)
    let input_stream_res = b.type_resource(Some("input-stream"), ValType::I32, None);
    
    // Type 2: wasi:io/streams@0.2.6/output-stream (resource)
    let output_stream_res = b.type_resource(Some("output-stream"), ValType::I32, None);

    // Type 3: pollable resource
    let pollable_res = b.type_resource(Some("pollable"), ValType::I32, None);

    // Define wasi:io/streams instance type
    // We only need:
    //   - [method]output-stream.blocking-write-and-flush: (self, list<u8>) -> result<tuple<u64, list<u8>>
    //   - [method]input-stream.blocking-read: (self, u64) -> result<tuple<u64, list<u8>>
    //   - [method]input-stream.subscribe: (self) -> pollable
    //   - [method]output-stream.subscribe: (self) -> pollable
    // For simplicity, we'll import the full streams interface
    
    // Actually, the simplest approach: import wasi:cli/stdin and wasi:cli/stdout
    // which give us get-stdin() -> input-stream and get-stdout() -> output-stream
    // Then we use blocking-read and blocking-write-and-flush
    
    // But defining all these types is hundreds of lines. Let me take a shortcut:
    // Import the component instances as "real" wasi interfaces.
    // The wasmtime runtime provides these via add_to_linker_async.
    
    // The key insight: we don't need to define all the types ourselves.
    // We just need to import the right interface names and the runtime provides them.
    // But wasm-encoder requires explicit type definitions for imports.
    
    // Let me define minimal instance types for stdin/stdout:
    
    // Instance type for wasi:cli/stdin@0.2.6
    let (stdin_type, stdin_enc) = b.ty(Some("wasi:cli/stdin"));
    // get-stdin: () -> own<input-stream>
    stdin_enc.function().params([]).result(
        Some(ComponentValType::Own(input_stream_res))
    );
    // This is a function type, not instance. Need instance type.
    // Actually, ComponentTypeEncoder has methods for this...
    
    // Hmm, this is getting complex. Let me try a different approach.
    // Since the runtime already provides all wasi:cli and wasi:io instances,
    // I can import them with the correct names and the runtime will match them.
    
    // The issue is that wasm-encoder requires me to fully define the types.
    // But the types need to match EXACTLY what wasmtime expects.
    
    // Alternative: use raw bytes for the component header (types + imports),
    // then embed the core module and wire it up.
    
    // Actually, the cleanest approach: extract the type/import section from
    // an existing working component (kv-writer) and reuse those bytes,
    // then add our core module and wiring.
    
    // Let me try yet another approach: build the component using
    // the P1 adapter approach but with a MUCH smaller custom adapter
    // that only implements fd_read/fd_write natively.
    
    // For now, return error to indicate this is not yet implemented
    Err("Native P2 component emission not yet implemented. Use outlayer-p2 target for adapter-based P2.".into())
}
