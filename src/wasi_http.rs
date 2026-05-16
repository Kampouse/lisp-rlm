//! WASI HTTP P2 integration for the Lisp compiler.
//!
//! Generates core WASM that directly imports canonical-lowered wasi:http@0.2.2
//! functions, with embedded WIT metadata for wit-component to wire up.

use wasm_encoder::*;

// ── Import function indices ──
pub const FN_DROP_INPUT_STREAM: u32 = 0;
pub const FN_DROP_OUTPUT_STREAM: u32 = 1;
pub const FN_DROP_INCOMING_RESPONSE: u32 = 2;
pub const FN_DROP_FUTURE_INCOMING_RESPONSE: u32 = 3;
pub const FN_CONSTRUCTOR_FIELDS: u32 = 4;
pub const FN_CONSTRUCTOR_OUTGOING_REQUEST: u32 = 5;
pub const FN_SET_METHOD: u32 = 6;
pub const FN_SET_SCHEME: u32 = 7;
pub const FN_SET_AUTHORITY: u32 = 8;
pub const FN_SET_PATH_WITH_QUERY: u32 = 9;
pub const FN_OUTGOING_REQUEST_BODY: u32 = 10;
pub const FN_OUTGOING_BODY_WRITE: u32 = 11;
pub const FN_OUTGOING_BODY_FINISH: u32 = 12;
pub const FN_DROP_OUTGOING_BODY: u32 = 13;
pub const FN_HANDLE: u32 = 14;
pub const FN_DROP_OUTGOING_REQUEST: u32 = 15;
pub const FN_FUTURE_GET: u32 = 16;
pub const FN_FUTURE_SUBSCRIBE: u32 = 17;
pub const FN_POLL: u32 = 18;
pub const FN_DROP_POLLABLE: u32 = 19;
pub const FN_INCOMING_RESPONSE_CONSUME: u32 = 20;
pub const FN_INCOMING_BODY_STREAM: u32 = 21;
pub const FN_INPUT_STREAM_READ: u32 = 22;
pub const FN_GET_STDOUT: u32 = 23;
pub const FN_OUTPUT_STREAM_WRITE: u32 = 24;
pub const FN_DROP_INCOMING_BODY: u32 = 25;
pub const FN_DROP_FIELDS: u32 = 26;
pub const HTTP_IMPORT_COUNT: u32 = 27;

/// Scratch space in memory for canonical ABI result areas
pub const SCRATCH: i32 = 131072; // 128KB offset
pub const SCRATCH_BODY_RESULT: i32 = SCRATCH;       // i32: outgoing body handle
pub const SCRATCH_STREAM_RESULT: i32 = SCRATCH + 4;  // i32: output/input stream handle
pub const SCRATCH_FUTURE_RESULT: i32 = SCRATCH + 8;  // i32: future handle
pub const SCRATCH_GET_RESULT: i32 = SCRATCH + 16;    // 8 bytes: future.get result (discriminant + handle)
pub const SCRATCH_CONSUME_RESULT: i32 = SCRATCH + 24; // i32: incoming body handle
pub const SCRATCH_READ_RESULT: i32 = SCRATCH + 32;   // 8 bytes: read result (ptr + len)
pub const SCRATCH_POLL_RESULT: i32 = SCRATCH + 48;   // poll result list
pub const SCRATCH_WRITE_RESULT: i32 = SCRATCH + 64;  // write result area

/// Add wasi:http@0.2.2 canonical imports to the module.
pub fn add_http_imports(module: &mut Module) {
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    let W = ValType::I32;
    
    // Core function types for canonical ABI lowered functions:
    types.ty().function([], [W]); // 0: () -> i32 (constructor)
    types.ty().function([W], [W]); // 1: (i32) -> i32 (constructor+handle, subscribe)
    types.ty().function([W, W], []); // 2: (i32, i32) -> () (body/stream/consume/get)
    types.ty().function([W], []); // 3: (i32) -> () (resource drop)
    types.ty().function([W, W, W, W], [W]); // 4: set-method/authority/path -> result
    types.ty().function([W, W, W, W, W], [W]); // 5: set-scheme -> result
    types.ty().function([W, W, W, W], []); // 6: finish/handle/write-and-flush
    types.ty().function([W, W, W], []); // 7: poll
    types.ty().function([W, ValType::I64, W], []); // 8: read

    module.section(&types);
    
    let http_types = "wasi:http/types@0.2.2";
    let http_handler = "wasi:http/outgoing-handler@0.2.2";
    let io_streams = "wasi:io/streams@0.2.2";
    let io_poll = "wasi:io/poll@0.2.2";
    let cli_stdout = "wasi:cli/stdout@0.2.2";

    imports.import(io_streams, "[resource-drop]input-stream", EntityType::Function(3));
    imports.import(io_streams, "[resource-drop]output-stream", EntityType::Function(3));
    imports.import(http_types, "[resource-drop]incoming-response", EntityType::Function(3));
    imports.import(http_types, "[resource-drop]future-incoming-response", EntityType::Function(3));
    imports.import(http_types, "[constructor]fields", EntityType::Function(0));
    imports.import(http_types, "[constructor]outgoing-request", EntityType::Function(1));
    imports.import(http_types, "[method]outgoing-request.set-method", EntityType::Function(4));
    imports.import(http_types, "[method]outgoing-request.set-scheme", EntityType::Function(5));
    imports.import(http_types, "[method]outgoing-request.set-authority", EntityType::Function(4));
    imports.import(http_types, "[method]outgoing-request.set-path-with-query", EntityType::Function(4));
    imports.import(http_types, "[method]outgoing-request.body", EntityType::Function(2));
    imports.import(http_types, "[method]outgoing-body.write", EntityType::Function(2));
    imports.import(http_types, "[static]outgoing-body.finish", EntityType::Function(6));
    imports.import(http_types, "[resource-drop]outgoing-body", EntityType::Function(3));
    imports.import(http_handler, "handle", EntityType::Function(6));
    imports.import(http_types, "[resource-drop]outgoing-request", EntityType::Function(3));
    imports.import(http_types, "[method]future-incoming-response.get", EntityType::Function(2));
    imports.import(http_types, "[method]future-incoming-response.subscribe", EntityType::Function(1));
    imports.import(io_poll, "poll", EntityType::Function(7));
    imports.import(io_poll, "[resource-drop]pollable", EntityType::Function(3));
    imports.import(http_types, "[method]incoming-response.consume", EntityType::Function(2));
    imports.import(http_types, "[method]incoming-body.stream", EntityType::Function(2));
    imports.import(io_streams, "[method]input-stream.read", EntityType::Function(8));
    imports.import(cli_stdout, "get-stdout", EntityType::Function(0));
    imports.import(io_streams, "[method]output-stream.blocking-write-and-flush", EntityType::Function(6));
    imports.import(http_types, "[resource-drop]incoming-body", EntityType::Function(3));
    imports.import(http_types, "[resource-drop]fields", EntityType::Function(3));

    module.section(&imports);
}

/// Emit the full HTTP GET call sequence as WASM instructions.
/// 
/// Assumes URL string is already written to memory at (url_ptr, url_len).
/// Parses the URL to extract scheme, authority, and path.
/// Writes response body to stdout.
///
/// Local variables used (must be allocated by caller):
///   locals[0]: fields handle
///   locals[1]: request handle  
///   locals[2]: outgoing body handle
///   locals[3]: output stream handle (unused for GET)
///   locals[4]: future handle
///   locals[5]: pollable handle
///   locals[6]: incoming response handle
///   locals[7]: incoming body handle
///   locals[8]: input stream handle
///   locals[9]: stdout handle
pub fn emit_http_get(func: &mut Function, url_ptr_local: u32, url_len_local: u32) {
    // ── Step 1: Create headers ──
    // fields = [constructor]fields()
    func.instruction(&Instruction::Call(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&Instruction::LocalSet(0)); // local 0 = fields handle

    // ── Step 2: Create outgoing request ──
    // req = [constructor]outgoing-request(fields)
    func.instruction(&Instruction::LocalGet(0));
    func.instruction(&Instruction::Call(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&Instruction::LocalSet(1)); // local 1 = request handle

    // ── Step 3: Drop fields (no longer needed) ──
    func.instruction(&Instruction::LocalGet(0));
    func.instruction(&Instruction::Call(FN_DROP_FIELDS));

    // ── Step 4: Set method = GET (discriminant 0 = "get") ──
    // set-method(req, disc=0, ptr=0, len=0) — for "get" variant, no string payload
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(0)); // discriminant: 0 = get
    func.instruction(&Instruction::I32Const(0)); // ptr (unused for "get")
    func.instruction(&Instruction::I32Const(0)); // len (unused for "get")
    func.instruction(&Instruction::Call(FN_SET_METHOD));
    func.instruction(&Instruction::Drop); // discard result

    // ── Step 5: Set scheme = HTTPS (discriminant 1) ──
    // set-scheme(req, disc=1, ptr=0, len=0, pad=0) — for "HTTPS" variant
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(1)); // discriminant: 1 = HTTPS
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Const(0)); // padding
    func.instruction(&Instruction::Call(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // ── Step 6: Set authority from URL ──
    // For now, assume the full URL is the authority (host)
    // TODO: Parse URL properly to extract host and path
    // set-authority(req, disc=0 (some), ptr, len)
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(0)); // discriminant: 0 = some(string)
    func.instruction(&Instruction::LocalGet(url_ptr_local));
    func.instruction(&Instruction::LocalGet(url_len_local));
    func.instruction(&Instruction::Call(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // ── Step 7: Set path-with-query = "/" ──
    // set-path-with-query(req, disc=0 (some), ptr, len)
    // Write "/" to memory at a known location first
    func.instruction(&Instruction::I32Const(SCRATCH));
    func.instruction(&Instruction::I32Const(0x2F)); // '/'
    func.instruction(&Instruction::I32Store8(MemArg { offset: SCRATCH as u64, align: 0, memory_index: 0 }));
    
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(0)); // some
    func.instruction(&Instruction::I32Const(SCRATCH));
    func.instruction(&Instruction::I32Const(1)); // len = 1
    func.instruction(&Instruction::Call(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);

    // ── Step 8: Get body and finish it (empty body for GET) ──
    // body = outgoing-request.body(req) → writes to SCRATCH_BODY_RESULT
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(SCRATCH_BODY_RESULT));
    func.instruction(&Instruction::Call(FN_OUTGOING_REQUEST_BODY));
    // Load body handle from result
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_BODY_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::LocalSet(2)); // local 2 = body handle

    // stream = outgoing-body.write(body) → SCRATCH_STREAM_RESULT
    func.instruction(&Instruction::LocalGet(2));
    func.instruction(&Instruction::I32Const(SCRATCH_STREAM_RESULT));
    func.instruction(&Instruction::Call(FN_OUTGOING_BODY_WRITE));
    // Load stream handle and drop it (we don't write anything for GET)
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_STREAM_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::Call(FN_DROP_OUTPUT_STREAM));

    // outgoing-body.finish(body, trailers=none(0), ptr=0, len=0)
    func.instruction(&Instruction::LocalGet(2));
    func.instruction(&Instruction::I32Const(0)); // none
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::Call(FN_OUTGOING_BODY_FINISH));
    func.instruction(&Instruction::Call(FN_DROP_OUTGOING_BODY)); // drop body

    // ── Step 9: Send request ──
    // handle(req, options=none) → writes future to SCRATCH_FUTURE_RESULT
    func.instruction(&Instruction::LocalGet(1));
    func.instruction(&Instruction::I32Const(0)); // none (no options)
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::Call(FN_HANDLE));
    
    // Load future handle
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_FUTURE_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::LocalSet(4)); // local 4 = future handle
    
    // Drop request
    func.instruction(&Instruction::Call(FN_DROP_OUTGOING_REQUEST));

    // ── Step 10: Wait for response (poll loop) ──
    // loop {
    //   result = future.get() → SCRATCH_GET_RESULT
    //   if result is Some(Ok(response)) → break with response
    //   if result is Some(Err(_)) → trap/error
    //   if result is None → subscribe + poll
    // }
    // loop { ... }
    // Note: block/loop management is manual with wasm_encoder
    func.instruction(&Instruction::Block(BlockType::Empty));
    
    // future.get(future, SCRATCH_GET_RESULT)
    func.instruction(&Instruction::LocalGet(4));
    func.instruction(&Instruction::I32Const(SCRATCH_GET_RESULT));
    func.instruction(&Instruction::Call(FN_FUTURE_GET));
    
    // Check discriminant at SCRATCH_GET_RESULT
    // 0 = None (not ready), 1 = Some(Ok(response)), 2 = Some(Err(error)), 3 = Some(Err(()))
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_GET_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::I32Const(1)); // Some(Ok) = discriminant 1
    func.instruction(&Instruction::I32Eq);
    func.instruction(&Instruction::BrIf(1)); // break out of block if ready

    // Not ready — subscribe and poll
    func.instruction(&Instruction::LocalGet(4));
    func.instruction(&Instruction::Call(FN_FUTURE_SUBSCRIBE));
    func.instruction(&Instruction::LocalSet(5)); // pollable handle
    
    // poll([pollable], 1, SCRATCH_POLL_RESULT)
    func.instruction(&Instruction::I32Const(SCRATCH_POLL_RESULT)); // pointer to pollable list
    func.instruction(&Instruction::I32Const(1)); // count
    func.instruction(&Instruction::I32Const(SCRATCH_POLL_RESULT + 8)); // result dst
    func.instruction(&Instruction::Call(FN_POLL));
    
    // Drop pollable
    func.instruction(&Instruction::LocalGet(5));
    func.instruction(&Instruction::Call(FN_DROP_POLLABLE));
    
    // Loop back
    func.instruction(&Instruction::Br(0)); // back to block start
    func.instruction(&Instruction::End); // end block

    // ── Step 11: Extract response ──
    // Response handle is at SCRATCH_GET_RESULT + 4
    func.instruction(&Instruction::I32Load(MemArg { offset: (SCRATCH_GET_RESULT + 4) as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::LocalSet(6)); // incoming response handle

    // Drop future
    func.instruction(&Instruction::LocalGet(4));
    func.instruction(&Instruction::Call(FN_DROP_FUTURE_INCOMING_RESPONSE));

    // ── Step 12: Consume response body ──
    // consume(response) → SCRATCH_CONSUME_RESULT
    func.instruction(&Instruction::LocalGet(6));
    func.instruction(&Instruction::I32Const(SCRATCH_CONSUME_RESULT));
    func.instruction(&Instruction::Call(FN_INCOMING_RESPONSE_CONSUME));
    
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_CONSUME_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::LocalSet(7)); // incoming body handle
    
    // Drop response
    func.instruction(&Instruction::LocalGet(6));
    func.instruction(&Instruction::Call(FN_DROP_INCOMING_RESPONSE));

    // stream = incoming-body.stream(body) → SCRATCH_STREAM_RESULT
    func.instruction(&Instruction::LocalGet(7));
    func.instruction(&Instruction::I32Const(SCRATCH_STREAM_RESULT));
    func.instruction(&Instruction::Call(FN_INCOMING_BODY_STREAM));
    
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_STREAM_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::LocalSet(8)); // input stream handle
    
    // Drop body
    func.instruction(&Instruction::LocalGet(7));
    func.instruction(&Instruction::Call(FN_DROP_INCOMING_BODY));

    // ── Step 13: Read response and write to stdout ──
    // Get stdout
    func.instruction(&Instruction::Call(FN_GET_STDOUT));
    func.instruction(&Instruction::LocalSet(9)); // stdout handle

    // Read loop: read 64KB chunks and write to stdout
    func.instruction(&Instruction::Loop(BlockType::Empty));
    
    // input-stream.read(stream, 65536, SCRATCH_READ_RESULT)
    func.instruction(&Instruction::LocalGet(8));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&Instruction::I32Const(SCRATCH_READ_RESULT));
    func.instruction(&Instruction::Call(FN_INPUT_STREAM_READ));
    
    // Read result is a result<list<u8>, stream-error>
    // At SCRATCH_READ_RESULT: discriminant (0=ok, 1=error)
    // If ok: SCRATCH_READ_RESULT+4 = ptr, SCRATCH_READ_RESULT+8 = len
    func.instruction(&Instruction::I32Load(MemArg { offset: SCRATCH_READ_RESULT as u64, align: 2, memory_index: 0 }));
    func.instruction(&Instruction::I32Const(0)); // 0 = ok
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1)); // break on error → end read loop

    // Get length of data
    func.instruction(&Instruction::I32Load(MemArg { offset: (SCRATCH_READ_RESULT + 8) as u64, align: 2, memory_index: 0 }));
    // If len == 0, break (EOF)
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1)); // break if len == 0

    // Write to stdout: blocking-write-and-flush(stdout, ptr, len, _)
    func.instruction(&Instruction::LocalGet(9));
    func.instruction(&Instruction::I32Load(MemArg { offset: (SCRATCH_READ_RESULT + 4) as u64, align: 2, memory_index: 0 })); // ptr
    func.instruction(&Instruction::I32Load(MemArg { offset: (SCRATCH_READ_RESULT + 8) as u64, align: 2, memory_index: 0 })); // len
    func.instruction(&Instruction::I32Const(0)); // padding
    func.instruction(&Instruction::Call(FN_OUTPUT_STREAM_WRITE));
    
    // Loop back
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End); // end loop

    // ── Step 14: Cleanup ──
    func.instruction(&Instruction::LocalGet(8));
    func.instruction(&Instruction::Call(FN_DROP_INPUT_STREAM));
    func.instruction(&Instruction::LocalGet(9));
    func.instruction(&Instruction::Call(FN_DROP_OUTPUT_STREAM));
}

/// Build the WIT metadata custom section for the wasi:http world.
/// This tells wit-component how to wire up the canonical ABI.
pub fn build_http_wit_metadata() -> Result<Vec<u8>, String> {
    let mut resolve = wit_parser::Resolve::new();
    
    // Push the WIT directory containing world.wit and deps/
    let wit_dir = find_wit_dir()?;
    
    let (pkg_id, _) = resolve.push_dir(&wit_dir).map_err(|e| format!("push_dir failed: {}", e))?;
    encode_metadata(&resolve, pkg_id)
}

fn find_wit_dir() -> Result<std::path::PathBuf, String> {
    // Try multiple paths
    let candidates = [
        "wit",
        "lisp-rlm/wit",
        "/Users/asil/.openclaw/workspace/lisp-rlm/wit",
        concat!(env!("CARGO_MANIFEST_DIR"), "/wit"),
    ];
    for dir in &candidates {
        let p = std::path::Path::new(dir);
        if p.exists() { return Ok(p.to_path_buf()); }
    }
    Err(format!("WIT directory not found, tried: {:?}", candidates))
}

fn encode_metadata(resolve: &wit_parser::Resolve, pkg_id: wit_parser::PackageId) -> Result<Vec<u8>, String> {
    // Find the world in this package
    let pkg = &resolve.packages[pkg_id];
    let world = pkg.worlds.iter()
        .find_map(|(name, id)| if name == "wasi-http-world" { Some(*id) } else { None })
        .ok_or("world 'wasi-http-world' not found")?;
    
    // Create a minimal module and embed metadata
    let mut module = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    
    wit_component::embed_component_metadata(&mut module, resolve, world, wit_component::StringEncoding::UTF8)
        .map_err(|e| format!("embed failed: {}", e))?;
    
    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wit_metadata_embedding() {
        // Use CARGO_MANIFEST_DIR for reliable path
        let wit_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wit");
        eprintln!("WIT dir: {:?}", wit_dir);
        assert!(wit_dir.exists(), "wit dir should exist");
        
        let mut resolve = wit_parser::Resolve::new();
        let (pkg_id, _) = resolve.push_dir(&wit_dir).unwrap();
        let metadata = super::encode_metadata(&resolve, pkg_id).unwrap();
        assert!(!metadata.is_empty(), "metadata should not be empty");
        assert!(metadata.starts_with(&[0x00, 0x61, 0x73, 0x6d]), "should be valid WASM");
        eprintln!("WIT metadata module: {} bytes", metadata.len());
    }
}
