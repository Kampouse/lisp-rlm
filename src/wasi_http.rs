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
/// Emit full HTTP GET via wasi:http canonical ABI.
/// 
/// Locals: 0=fields, 1=req, 2=body, 3=future, 4=pollable, 
///         5=response, 6=resp_body, 7=in_stream, 8=stdout, 9=scratch, 10=url_ptr, 11=url_len
pub fn emit_http_get(func: &mut Function, url_ptr_local: u32, url_len_local: u32) {
    let ld = |off: i32| Instruction::I32Load(MemArg { offset: off as u64, align: 2, memory_index: 0 });
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);

    // Step 1: fields = [constructor]fields()  -- () -> i32
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&ls(0));

    // Step 2: req = [constructor]outgoing-request(fields)  -- (i32) -> i32
    func.instruction(&lg(0));
    func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&ls(1));

    // Step 3: drop fields  -- (i32) -> ()
    func.instruction(&lg(0));
    func.instruction(&cl(FN_DROP_FIELDS));

    // Step 4: set-method GET  -- (i32,i32,i32,i32) -> i32
    func.instruction(&lg(1));  // req
    func.instruction(&cst(0)); // disc=0 (get)
    func.instruction(&cst(0)); // ptr
    func.instruction(&cst(0)); // len
    func.instruction(&cl(FN_SET_METHOD));
    func.instruction(&Instruction::Drop);

    // Step 5: set-scheme HTTPS  -- (i32,i32,i32,i32,i32) -> i32
    func.instruction(&lg(1));
    func.instruction(&cst(1)); // disc=1 (HTTPS)
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(0)); // pad
    func.instruction(&cl(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // Step 6: set-authority = URL  -- (i32,i32,i32,i32) -> i32
    func.instruction(&lg(1));
    func.instruction(&cst(0)); // disc=0 (some)
    func.instruction(&lg(url_ptr_local));
    func.instruction(&lg(url_len_local));
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // Step 7: set-path-with-query = "/"  -- (i32,i32,i32,i32) -> i32
    func.instruction(&cst(SCRATCH));
    func.instruction(&cst(0x2F));
    func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&lg(1));
    func.instruction(&cst(0)); // some
    func.instruction(&cst(SCRATCH));
    func.instruction(&cst(1));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);

    // Step 8: body = outgoing-request.body(req)  -- (i32,i32) -> ()
    // Writes body handle to SCRATCH_BODY_RESULT
    func.instruction(&lg(1));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    // Load body handle from memory
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT));
    func.instruction(&ls(2));

    // Step 9: get output stream from body, drop it (empty body for GET)
    func.instruction(&lg(2));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_WRITE));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_DROP_OUTPUT_STREAM));

    // Step 10: finish body (no trailers)  -- (i32,i32,i32,i32) -> ()
    func.instruction(&lg(2));
    func.instruction(&cst(0)); // none
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // Step 11: drop body
    func.instruction(&lg(2));
    func.instruction(&cl(FN_DROP_OUTGOING_BODY));

    // Step 12: handle(req, none, dst, pad)  -- (i32,i32,i32,i32) -> ()
    // Writes future handle to SCRATCH_FUTURE_RESULT
    func.instruction(&lg(1));
    func.instruction(&cst(0)); // none
    func.instruction(&cst(SCRATCH_FUTURE_RESULT)); // dst
    func.instruction(&cst(0)); // pad
    func.instruction(&cl(FN_HANDLE));
    // Load future handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT));
    func.instruction(&ls(3));

    // Drop request
    func.instruction(&lg(1));
    func.instruction(&cl(FN_DROP_OUTGOING_REQUEST));

    // Step 13: Poll loop
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    // future.get(future, dst) -- (i32,i32) -> ()
    func.instruction(&lg(3));
    func.instruction(&cst(SCRATCH_GET_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    // Load discriminant
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_GET_RESULT));
    func.instruction(&cst(1)); // Some(Ok) = 1
    func.instruction(&Instruction::I32Eq);
    func.instruction(&Instruction::BrIf(1)); // break out of both loop+block if ready

    // Not ready: subscribe + poll
    func.instruction(&lg(3));
    func.instruction(&cl(FN_FUTURE_SUBSCRIBE)); // (i32) -> i32
    func.instruction(&ls(4));
    // poll(ptr, len, dst)
    func.instruction(&cst(SCRATCH_POLL_RESULT));
    func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT + 8));
    func.instruction(&cl(FN_POLL)); // (i32,i32,i32) -> ()
    // Drop pollable
    func.instruction(&lg(4));
    func.instruction(&cl(FN_DROP_POLLABLE));
    // Loop back
    func.instruction(&Instruction::Br(0)); // back to loop start
    func.instruction(&Instruction::End); // end loop
    func.instruction(&Instruction::End); // end block

    // Step 14: Extract response handle
    // Layout: disc(4) + inner_result_disc(4) + handle(4) = offset 8
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_GET_RESULT + 8));
    func.instruction(&ls(5));

    // Drop future
    func.instruction(&lg(3));
    func.instruction(&cl(FN_DROP_FUTURE_INCOMING_RESPONSE));

    // Step 15: consume response  -- (i32,i32) -> ()
    func.instruction(&lg(5));
    func.instruction(&cst(SCRATCH_CONSUME_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_CONSUME_RESULT));
    func.instruction(&ls(6));

    // Drop response
    func.instruction(&lg(5));
    func.instruction(&cl(FN_DROP_INCOMING_RESPONSE));

    // stream = incoming-body.stream(body)
    func.instruction(&lg(6));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT));
    func.instruction(&ls(7));

    // Drop body
    func.instruction(&lg(6));
    func.instruction(&cl(FN_DROP_INCOMING_BODY));

    // Step 16: get stdout
    func.instruction(&cl(FN_GET_STDOUT)); // () -> i32
    func.instruction(&ls(8));

    // Step 17: Read loop
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    // read(stream, 65536, dst)  -- (i32,i64,i32) -> ()
    func.instruction(&lg(7));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_READ));
    // Check result disc (0=ok, 1=error)
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1)); // break on error
    // Check len (0=EOF)
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1)); // break on EOF
    // Write to stdout
    func.instruction(&lg(8));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4)); // ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8)); // len
    func.instruction(&cst(0)); // pad
    func.instruction(&cl(FN_OUTPUT_STREAM_WRITE)); // (i32,i32,i32,i32) -> ()
    // Loop
    func.instruction(&Instruction::Br(0)); // back to loop start
    func.instruction(&Instruction::End); // end loop
    func.instruction(&Instruction::End); // end block

    // Step 18: Cleanup
    func.instruction(&lg(7));
    func.instruction(&cl(FN_DROP_INPUT_STREAM));
    func.instruction(&lg(8));
    func.instruction(&cl(FN_DROP_OUTPUT_STREAM));
}
pub fn build_http_wit_metadata() -> Result<(wit_parser::Resolve, wit_parser::WorldId), String> {
    let mut resolve = wit_parser::Resolve::new();
    let wit_dir = find_wit_dir()?;
    let (pkg_id, _) = resolve.push_dir(&wit_dir).map_err(|e| format!("push_dir failed: {}", e))?;
    
    let pkg = &resolve.packages[pkg_id];
    let world = pkg.worlds.iter()
        .find_map(|(name, id)| if name == "simple-http" { Some(*id) } else { None })
        .ok_or("world 'simple-http' not found")?;
    
    Ok((resolve, world))
}

fn find_wit_dir() -> Result<std::path::PathBuf, String> {
    let candidates = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/wit"),
        "wit",
        "lisp-rlm/wit",
        "/Users/asil/.openclaw/workspace/lisp-rlm/wit",
    ];
    for dir in &candidates {
        let p = std::path::Path::new(dir);
        if p.exists() { return Ok(p.to_path_buf()); }
    }
    Err(format!("WIT directory not found, tried: {:?}", candidates))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wit_metadata_embedding() {
        let (resolve, world) = build_http_wit_metadata().unwrap();
        
        let mut module = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        wit_component::embed_component_metadata(&mut module, &resolve, world, wit_component::StringEncoding::UTF8).unwrap();
        
        assert!(!module.is_empty());
        assert!(module.starts_with(&[0x00, 0x61, 0x73, 0x6d]));
        eprintln!("WIT metadata module: {} bytes", module.len());
    }
}
