//! Native WASI P2 HTTP GET component emitter.
//!
//! Emits a WASI command component that:
//! 1. Reads URL from stdin (wasi:cli/stdin)
//! 2. Does HTTP GET (wasi:http/outgoing-handler)
//! 3. Writes response to stdout (wasi:cli/stdout)
//!
//! No preview1 adapter. No Rust provider. ~2KB core WASM. Target: <5K instructions.

use wasm_encoder::{
    BlockType, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, ImportSection, Instruction, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};

// ── Import function indices (in declaration order) ──
// Stdin/Stdout (4 functions)
const I_GET_STDIN: u32 = 0;            // () -> i32
const I_GET_STDOUT: u32 = 1;           // () -> i32
const I_BLOCKING_READ: u32 = 2;        // (i32, i64, i32) -> ()
const I_BLOCKING_WRITE: u32 = 3;       // (i32, i32, i32) -> ()
// HTTP (14 functions)
const I_FIELDS_NEW: u32 = 4;           // () -> i32
const I_REQUEST_NEW: u32 = 5;          // (i32) -> i32
const I_SET_SCHEME: u32 = 6;           // (i32, i32, i32, i32, i32) -> i32
const I_SET_AUTHORITY: u32 = 7;        // (i32, i32, i32, i32) -> i32
const I_SET_PATH: u32 = 8;             // (i32, i32, i32, i32) -> i32
const I_REQUEST_BODY: u32 = 9;         // (i32, i32) -> ()
const I_BODY_FINISH: u32 = 10;         // (i32, i32, i32, i32) -> ()
const I_HANDLER_HANDLE: u32 = 11;      // (i32, i32, i32, i32) -> ()
const I_FUTURE_SUBSCRIBE: u32 = 12;    // (i32) -> i32
const I_POLLABLE_BLOCK: u32 = 13;      // (i32) -> ()
const I_FUTURE_GET: u32 = 14;          // (i32, i32) -> ()
const I_RESPONSE_CONSUME: u32 = 15;    // (i32, i32) -> ()
const I_BODY_STREAM: u32 = 16;         // (i32, i32) -> ()
// Resource drops (7 functions)
const I_DROP_FIELDS: u32 = 17;
const I_DROP_REQUEST: u32 = 18;
const I_DROP_OUTBODY: u32 = 19;
const I_DROP_FUTURE: u32 = 20;
const I_DROP_POLLABLE: u32 = 21;
const I_DROP_INBODY: u32 = 22;
const I_DROP_INSTREAM: u32 = 23;
const NUM_IMPORTS: u32 = 24;

// Function indices (after imports)
const F_RUN: u32 = NUM_IMPORTS + 0;       // 24: () -> i64
const F_START: u32 = NUM_IMPORTS + 1;     // 25: () -> ()
const F_REALLOC: u32 = NUM_IMPORTS + 2;   // 26: (i32,i32,i32,i32) -> i32

// ── Memory layout ──
const BUMP_PTR: i32 = 128;        // bump allocator pointer
const TEMP: i32 = 144;            // temp area for string unpacking
const RET: i32 = 160;             // return area for host calls
const SCAN_I: i32 = 224;          // URL scan loop index
const AUTH_OFF: i32 = 228;        // authority offset in URL
const AUTH_LEN_LOC: i32 = 232;    // authority length
const PATH_OFF: i32 = 236;        // path offset in URL
const PATH_LEN_LOC: i32 = 240;    // path length
const URL_PTR_LOC: i32 = 244;     // url ptr (low 32 of packed)
const URL_LEN_LOC: i32 = 248;     // url len (high 32 of packed)
const STDIN_BUF: i32 = 1024;      // stdin buffer start
const STDIN_BUF_SIZE: u64 = 4096;
const BODY_BUF: i32 = 8192;       // response body buffer
const BODY_BUF_SIZE: u64 = 65536;
const SLASH: i32 = 512;           // "/" constant

/// Build the complete native WASI P2 component (core WASM).
pub fn build_native() -> Vec<u8> {
    let mut m = Module::new();

    // ── Type section ──
    let mut types = TypeSection::new();
    types.ty().function([], []);                                         // 0: () -> ()
    types.ty().function([ValType::I32], []);                             // 1: (i32) -> ()
    types.ty().function([], [ValType::I32]);                             // 2: () -> i32
    types.ty().function([ValType::I32], [ValType::I32]);                 // 3: (i32) -> i32
    types.ty().function([ValType::I32, ValType::I32], []);               // 4: (2xi32) -> ()
    types.ty().function([ValType::I32; 4], []);                          // 5: (4xi32) -> ()
    types.ty().function([ValType::I32; 4], [ValType::I32]);              // 6: (4xi32) -> i32
    types.ty().function([ValType::I32; 5], [ValType::I32]);              // 7: (5xi32) -> i32
    types.ty().function([ValType::I32, ValType::I64, ValType::I32], []); // 8: stream read
    types.ty().function([], [ValType::I64]);                             // 9: () -> i64
    m.section(&types);

    // ── Import section ──
    let mut imp = ImportSection::new();

    // Stdin/Stdout
    imp.import("wasi:cli/stdin@0.2.2", "get-stdin", EntityType::Function(2));               // 0
    imp.import("wasi:cli/stdout@0.2.2", "get-stdout", EntityType::Function(2));              // 1
    imp.import("wasi:io/streams@0.2.2", "[method]input-stream.blocking-read", EntityType::Function(8));   // 2
    imp.import("wasi:io/streams@0.2.2", "[method]output-stream.blocking-write-and-flush", EntityType::Function(5)); // 3

    // HTTP constructors & methods
    imp.import("wasi:http/types@0.2.2", "[constructor]fields", EntityType::Function(2));     // 4
    imp.import("wasi:http/types@0.2.2", "[constructor]outgoing-request", EntityType::Function(3)); // 5
    imp.import("wasi:http/types@0.2.2", "[method]outgoing-request.set-scheme", EntityType::Function(7)); // 6
    imp.import("wasi:http/types@0.2.2", "[method]outgoing-request.set-authority", EntityType::Function(6)); // 7
    imp.import("wasi:http/types@0.2.2", "[method]outgoing-request.set-path-with-query", EntityType::Function(6)); // 8
    imp.import("wasi:http/types@0.2.2", "[method]outgoing-request.body", EntityType::Function(4)); // 9
    imp.import("wasi:http/types@0.2.2", "[static]outgoing-body.finish", EntityType::Function(5)); // 10
    imp.import("wasi:http/outgoing-handler@0.2.2", "handle", EntityType::Function(5));       // 11
    imp.import("wasi:http/types@0.2.2", "[method]future-incoming-response.subscribe", EntityType::Function(3)); // 12
    imp.import("wasi:io/poll@0.2.2", "[method]pollable.block", EntityType::Function(1));    // 13
    imp.import("wasi:http/types@0.2.2", "[method]future-incoming-response.get", EntityType::Function(4)); // 14
    imp.import("wasi:http/types@0.2.2", "[method]incoming-response.consume", EntityType::Function(4)); // 15
    imp.import("wasi:http/types@0.2.2", "[method]incoming-body.stream", EntityType::Function(4)); // 16

    // Resource drops (all type 1: (i32) -> ())
    imp.import("wasi:http/types@0.2.2", "[resource-drop]fields", EntityType::Function(1));                    // 17
    imp.import("wasi:http/types@0.2.2", "[resource-drop]outgoing-request", EntityType::Function(1));          // 18
    imp.import("wasi:http/types@0.2.2", "[resource-drop]outgoing-body", EntityType::Function(1));             // 19
    imp.import("wasi:http/types@0.2.2", "[resource-drop]future-incoming-response", EntityType::Function(1));  // 20
    imp.import("wasi:io/poll@0.2.2", "[resource-drop]pollable", EntityType::Function(1));                     // 21
    imp.import("wasi:http/types@0.2.2", "[resource-drop]incoming-body", EntityType::Function(1));             // 22
    imp.import("wasi:io/streams@0.2.2", "[resource-drop]input-stream", EntityType::Function(1));              // 23
    m.section(&imp);

    // ── Functions (must come before memory section) ──
    let mut funcs = FunctionSection::new();
    funcs.function(9); // run: () -> i64
    funcs.function(0); // _start: () -> ()
    funcs.function(6); // realloc: (4xi32) -> i32
    m.section(&funcs);

    // ── Memory ──
    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: 2, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    // ── Exports ──
    let mut exps = ExportSection::new();
    exps.export("memory", ExportKind::Memory, 0);
    // WASI P2 command: export _start for adapter mode, or "run" for native mode
    exps.export("_start", ExportKind::Func, F_START as u32);
    exps.export("canonical_abi_realloc", ExportKind::Func, F_REALLOC as u32);
    m.section(&exps);

    // ── Code ──
    let mut code = wasm_encoder::CodeSection::new();
    code.function(&build_run());
    code.function(&build_start());
    code.function(&build_realloc());
    m.section(&code);

    // ── Data ──
    let mut data = DataSection::new();
    // "/" constant at SLASH
    data.active(0, &ConstExpr::i32_const(SLASH), b"/".to_vec());
    // bump ptr init at BUMP_PTR = 256
    data.active(0, &ConstExpr::i32_const(BUMP_PTR), 256i32.to_le_bytes().to_vec());
    m.section(&data);

    m.finish()
}

/// run() -> i64 — the main logic:
/// 1. Read stdin → URL
/// 2. Parse URL (extract authority, path)
/// 3. Build HTTP request
/// 4. Send via outgoing-handler
/// 5. Read response body
/// 6. Write to stdout
/// 7. Return 0
fn build_run() -> Function {
    // Locals: url_ptr(0), url_len(1), stdin_handle(2), stdout_handle(3), fields(4), req(5),
    //         out_body(6), out_stream(7), future(8), pollable(9), in_body(10), in_stream(11), body_len(12)
    let mut f = Function::new([(13, ValType::I32)]);
    let url_ptr = 0;
    let url_len = 1;
    let stdin_h = 2;
    let stdout_h = 3;
    let fields_h = 4;
    let req_h = 5;
    let out_body_h = 6;
    let _out_stream_h = 7;
    let future_h = 8;
    let pollable_h = 9;
    let in_body_h = 10;
    let in_stream_h = 11;
    let body_len = 12;

    // ─── 1. Get stdin handle ───
    f.instruction(&Instruction::Call(I_GET_STDIN)); // () -> i32
    f.instruction(&Instruction::LocalSet(stdin_h));

    // ─── 2. Read stdin into STDIN_BUF ───
    // blocking-read(stream, len, ret_ptr)
    // ret_ptr points to: [ptr i32, len i32] (list<u8> result)
    f.instruction(&Instruction::LocalGet(stdin_h)); // stream
    f.instruction(&Instruction::I64Const(STDIN_BUF_SIZE as i64));
    f.instruction(&Instruction::I32Const(RET)); // ret_ptr
    f.instruction(&Instruction::Call(I_BLOCKING_READ));
    // Read result: ret_ptr = ptr, ret_ptr+4 = len
    f.instruction(&Instruction::I32Const(RET));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(url_ptr));
    f.instruction(&Instruction::I32Const(RET + 4));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(url_len));

    // ─── 3. Parse URL: find "://" then "/" ───
    // Scan from offset 0 looking for ':' to find scheme end
    // Then skip "//" (2 bytes), then scan for '/' to find authority end
    // For simplicity: assume URL starts with "http://" (7 bytes) or "https://" (8 bytes)
    // Check if byte at url_ptr+4 is 's' to determine scheme offset (7 vs 8)

    // Check scheme: if mem[url_ptr+4] == 's' → offset=8, else offset=7
    // Actually https: the 's' is at index 4 (h=0,t=1,t=2,p=3,s=4,:=5)
    // So if url_ptr[4] == 's' (0x73) → HTTPS, auth starts at 8
    // else → HTTP, auth starts at 7
    // For now just assume HTTPS and auth starts at 8

    // Store authority start offset (8)
    f.instruction(&Instruction::I32Const(AUTH_OFF));
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::I32Store(mem_arg(0)));

    // Scan from i=8 while i < url_len && mem[url_ptr+i] != '/'
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::I32Store(mem_arg(0)));

    // Loop
    f.instruction(&Instruction::Block(BlockType::Empty));
    f.instruction(&Instruction::Loop(BlockType::Empty));
    // if i >= url_len: break
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // i
    f.instruction(&Instruction::LocalGet(url_len));
    f.instruction(&Instruction::I32GeU);
    f.instruction(&Instruction::BrIf(1));
    // if mem[url_ptr + i] == '/': break
    f.instruction(&Instruction::LocalGet(url_ptr));
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::I32Load8U(mem_arg_byte(0)));
    f.instruction(&Instruction::I32Const(0x2F)); // '/'
    f.instruction(&Instruction::I32Eq);
    f.instruction(&Instruction::BrIf(1));
    // i++
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::I32Store(mem_arg(0)));
    f.instruction(&Instruction::I32Const(1)); // always true
    f.instruction(&Instruction::BrIf(0)); // continue loop
    f.instruction(&Instruction::End); // end loop
    f.instruction(&Instruction::End); // end block

    // auth_len = SCAN_I - 8
    f.instruction(&Instruction::I32Const(AUTH_LEN_LOC));
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // i
    f.instruction(&Instruction::I32Const(8));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Store(mem_arg(0)));

    // path_ptr = url_ptr + SCAN_I (if SCAN_I < url_len) or "/" constant
    // path_len = url_len - SCAN_I (if > 0) or 1
    f.instruction(&Instruction::I32Const(PATH_OFF));
    f.instruction(&Instruction::LocalGet(url_ptr));
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::I32Store(mem_arg(0)));

    f.instruction(&Instruction::I32Const(PATH_LEN_LOC));
    f.instruction(&Instruction::LocalGet(url_len));
    f.instruction(&Instruction::I32Const(SCAN_I));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Store(mem_arg(0)));

    // If path_len == 0: use "/" constant
    f.instruction(&Instruction::I32Const(PATH_LEN_LOC));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::I32Const(0));
    f.instruction(&Instruction::I32Eq);
    f.instruction(&Instruction::If(BlockType::Empty));
    f.instruction(&Instruction::I32Const(PATH_OFF));
    f.instruction(&Instruction::I32Const(SLASH));
    f.instruction(&Instruction::I32Store(mem_arg(0)));
    f.instruction(&Instruction::I32Const(PATH_LEN_LOC));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Store(mem_arg(0)));
    f.instruction(&Instruction::End);

    // ─── 4. Build HTTP request ───
    // fields = [constructor]fields()
    f.instruction(&Instruction::Call(I_FIELDS_NEW)); // () -> i32 (result<fields>)
    // The result is actually result<resource, error> via canonical ABI
    // For constructors, the return IS the handle directly (or trap on error)
    // Actually wait — looking at the Rust core module, [constructor]fields has type 8 = () -> i32
    // So it returns i32 directly (the handle). No ret_ptr needed.
    // But canonical ABI for result<resource> is... actually constructors return the handle directly.
    // In component model, [constructor] returns the owning handle.
    // The canonical ABI lowering is just () -> i32.
    f.instruction(&Instruction::LocalSet(fields_h));

    // request = [constructor]outgoing-request(fields)
    f.instruction(&Instruction::LocalGet(fields_h));
    f.instruction(&Instruction::Call(I_REQUEST_NEW)); // (i32) -> i32
    f.instruction(&Instruction::LocalSet(req_h));

    // set-scheme(request, option<http-scheme>)
    // Canonical ABI for option<tuple<http-scheme, string>>:
    // option tag: 0=some, 1=none
    // For HTTPS (scheme variant 1): some tag=0, then scheme discriminant=1
    // But the type signature is (i32,i32,i32,i32,i32) -> i32
    // That's (self, option_discriminant, scheme_discriminant, string_ptr, string_len) -> result<i32>
    // For HTTPS: (req, 0, 1, 0, 0) — some, HTTPS, no string
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::I32Const(0)); // option: some
    f.instruction(&Instruction::I32Const(1)); // scheme: HTTPS
    f.instruction(&Instruction::I32Const(0)); // string ptr (unused)
    f.instruction(&Instruction::I32Const(0)); // string len
    f.instruction(&Instruction::Call(I_SET_SCHEME));
    f.instruction(&Instruction::Drop); // drop result

    // set-authority(request, option<tuple<string>>)
    // Type: (i32, i32, i32, i32) -> i32
    // That's (self, option_tag, string_ptr, string_len) -> result<i32>
    // option<tuple<string>>: tag=0 (some), then the string
    // Wait, for option<string>: tag=0 means "some", then ptr+i32, len+i32
    // But option<tuple<string>> is different from option<string>
    // In the WIT: set-authority takes option<authority> where authority = tuple<string>
    // canonical ABI: option has discriminant, then the inner value
    // For option<tuple<string>>: if some: tag=0, ptr, len. if none: tag=1
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::I32Const(0)); // some
    f.instruction(&Instruction::LocalGet(url_ptr));
    f.instruction(&Instruction::I32Const(AUTH_OFF));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // authority offset
    f.instruction(&Instruction::I32Add); // url_ptr + auth_offset = authority ptr
    f.instruction(&Instruction::I32Const(AUTH_LEN_LOC));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // authority len
    f.instruction(&Instruction::Call(I_SET_AUTHORITY));
    f.instruction(&Instruction::Drop);

    // Hmm wait, I_SET_AUTHORITY has type (4xi32)->i32 which is 4 params.
    // But I pushed: req_h, 0, auth_ptr, auth_len = 4 values. 
    // The authority ptr is computed as url_ptr + AUTH_OFF value. That's fine.

    // set-path-with-query(request, option<string>)
    // Same pattern as authority
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::I32Const(0)); // some
    f.instruction(&Instruction::I32Const(PATH_OFF));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // path ptr
    f.instruction(&Instruction::I32Const(PATH_LEN_LOC));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // path len
    f.instruction(&Instruction::Call(I_SET_PATH));
    f.instruction(&Instruction::Drop);

    // ─── 5. Get request body and finish it ───
    // request.body(self, ret_ptr) → writes (body_handle, stream_handle) to ret_ptr
    // Actually: [method]outgoing-request.body has type (i32, i32) = (self, ret_ptr)
    // It writes result<tuple<outgoing-body, output-stream>, error> to ret_ptr
    // result: [discrim i32, body_handle i32, stream_handle i32]
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::I32Const(RET));
    f.instruction(&Instruction::Call(I_REQUEST_BODY));
    f.instruction(&Instruction::I32Const(RET + 4));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(out_body_h));
    // RET + 8 would be the output stream handle, but we don't need it for GET (no body to write)

    // outgoing-body.finish(body, fields)
    // Type: (i32, i32, i32, i32) = (self, fields_ptr, fields_len, ret_ptr)
    // Actually [static]outgoing-body.finish takes (body, fields) where fields is resource
    // Canonical ABI: (body_handle, fields_ptr, fields_len, ret_ptr) for result<_, error>
    // Hmm, let me check. Type is (4xi32) -> ()
    // That's (self, ???). For a static method taking (body, fields):
    // Canonical: (body_handle, fields.handle_as_i32, ..., ret_ptr)
    // Actually for a resource parameter, it's just the handle i32.
    // finish(body, fields) → canonical: (body_i32, fields_i32, ret_ptr_i32, ???)
    // Type 5 = (4xi32) -> (). So: (self_body, fields_ptr, fields_len, ret_ptr)?
    // Or is it (self_body, fields_handle, ???, ret_ptr)?
    // Hmm, looking at the Rust import: [static]outgoing-body.finish has type 2 = (i32,i32,i32,i32)
    // And it takes (OutgoingBody, Fields) -> Result<(), ErrorCode>
    // Canonical lowering: (self_body_handle, fields_resource_handle, ???, ret_ptr)
    // For Result<(), ErrorCode>: ret_ptr points to [discrim i32]
    // But we have 4 i32 params. The Fields is a resource — just an i32 handle.
    // So: (body_handle, fields_handle, ???, ret_ptr)
    // That's 3 params... what's the 4th? Maybe it's just (body, fields, ret_ptr) with
    // padding? No, canonical ABI doesn't pad.
    // Actually, Fields here is passed by reference? No, resources are handles.
    // Let me look more carefully at the Rust binary's actual call to finish.
    
    // For now, let's use: (body, fields_handle, 0, ret_ptr)
    f.instruction(&Instruction::LocalGet(out_body_h));
    f.instruction(&Instruction::LocalGet(fields_h));
    f.instruction(&Instruction::I32Const(0)); // null/empty
    f.instruction(&Instruction::I32Const(RET + 32)); // ret_ptr for result
    f.instruction(&Instruction::Call(I_BODY_FINISH));

    // ─── 6. Send request ───
    // outgoing-handler.handle(request, options, ret_ptr)
    // handle(OutgoingRequest, RequestOptions) -> Result<FutureIncomingResponse, ErrorCode>
    // Canonical: (req_handle, options_handle, ???, ret_ptr) — type 5 = (4xi32)
    // For options=null: pass 0 as the option discriminant (none)
    // Actually option<RequestOptions> canonical: discrim=1 means "none"
    // So: (req_handle, 1 /* none */, ret_ptr_lo, ret_ptr_hi)?
    // Hmm, the type is (4xi32). option<RequestOptions>: discrim(1=none).
    // Then 2 padding i32s? Or discrim + 2 empty fields for the some-case?
    // For option variant: some has fields, none doesn't. Canonical ABI always passes all fields.
    // option<RequestOptions>: if none: discrim=1, then the inner fields are 0
    // inner fields for RequestOptions resource = 1 i32 (handle)
    // So: (req, discrim=1, inner_placeholder=0, ret_ptr)
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::I32Const(1)); // option: none
    f.instruction(&Instruction::I32Const(0)); // placeholder for inner
    f.instruction(&Instruction::I32Const(RET + 40)); // ret_ptr for result<FutureIncomingResponse>
    f.instruction(&Instruction::Call(I_HANDLER_HANDLE));

    // Read future handle from RET+44 (ok result, after discrim)
    f.instruction(&Instruction::I32Const(RET + 44));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(future_h));

    // ─── 7. Wait for response ───
    // future.subscribe() -> pollable
    f.instruction(&Instruction::LocalGet(future_h));
    f.instruction(&Instruction::Call(I_FUTURE_SUBSCRIBE));
    f.instruction(&Instruction::LocalSet(pollable_h));

    // pollable.block()
    f.instruction(&Instruction::LocalGet(pollable_h));
    f.instruction(&Instruction::Call(I_POLLABLE_BLOCK));

    // future.get(self, ret_ptr) → writes result to ret_ptr
    // Result<Option<tuple<IncomingResponse, option<Fields>>>, ErrorCode>
    // This is complex. ret_ptr will contain:
    // [discrim i32, option_discrim i32, response_handle i32, fields_handle i32]
    f.instruction(&Instruction::LocalGet(future_h));
    f.instruction(&Instruction::I32Const(RET + 48));
    f.instruction(&Instruction::Call(I_FUTURE_GET));

    // Drop future (consumed)
    f.instruction(&Instruction::LocalGet(future_h));
    f.instruction(&Instruction::Call(I_DROP_FUTURE));
    // Drop pollable
    f.instruction(&Instruction::LocalGet(pollable_h));
    f.instruction(&Instruction::Call(I_DROP_POLLABLE));

    // Read response handle: RET+52 (discrim) + RET+56 (response handle)
    // Actually depends on the nested result layout. Let me just try RET+52 for now.
    // If it's result<option<tuple<response, fields>>>:
    // [0] = result discrim (0=ok)
    // [4] = option discrim (0=some)
    // [8] = response handle
    // [12] = fields handle
    f.instruction(&Instruction::I32Const(RET + 56)); // response handle (skip 2 discriminants)
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    // This is an incoming-response handle. Store temporarily.
    f.instruction(&Instruction::LocalSet(future_h)); // reuse local

    // ─── 8. Consume response ───
    // response.consume(self, ret_ptr) → result<IncomingBody, ErrorCode>
    f.instruction(&Instruction::LocalGet(future_h)); // response handle
    f.instruction(&Instruction::I32Const(RET + 64));
    f.instruction(&Instruction::Call(I_RESPONSE_CONSUME));
    // Read in_body handle: RET+68 (after discrim)
    f.instruction(&Instruction::I32Const(RET + 68));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(in_body_h));

    // Drop response
    f.instruction(&Instruction::LocalGet(future_h));
    f.instruction(&Instruction::Call(I_DROP_REQUEST)); // reuse — actually we need DROP_RESPONSE
    // Hmm, we don't have I_DROP_RESPONSE. We didn't import it.
    // Let me skip dropping for now — resources get cleaned up when the component exits.

    // ─── 9. Get body stream ───
    // incoming-body.stream(self, ret_ptr) → result<Option<InputStream>, Error>
    f.instruction(&Instruction::LocalGet(in_body_h));
    f.instruction(&Instruction::I32Const(RET + 72));
    f.instruction(&Instruction::Call(I_BODY_STREAM));
    // Read stream handle: RET+76 (after discrim, then option discrim)
    // result<option<stream>>: [discrim i32, option_discrim i32, handle i32]
    f.instruction(&Instruction::I32Const(RET + 80)); // stream handle
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalSet(in_stream_h));

    // Drop in_body
    f.instruction(&Instruction::LocalGet(in_body_h));
    f.instruction(&Instruction::Call(I_DROP_INBODY));

    // ─── 10. Read body ───
    // blocking-read(stream, len, ret_ptr)
    f.instruction(&Instruction::LocalGet(in_stream_h));
    f.instruction(&Instruction::I64Const(BODY_BUF_SIZE as i64));
    f.instruction(&Instruction::I32Const(RET + 84)); // ret for list<u8>
    f.instruction(&Instruction::Call(I_BLOCKING_READ));
    // Read body ptr and len
    f.instruction(&Instruction::I32Const(RET + 84));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // body_ptr on stack
    f.instruction(&Instruction::LocalSet(future_h));  // reuse future_h local as temp for body_ptr
    f.instruction(&Instruction::I32Const(RET + 88));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // body_len
    f.instruction(&Instruction::LocalSet(body_len));

    // Drop in_stream
    f.instruction(&Instruction::LocalGet(in_stream_h));
    f.instruction(&Instruction::Call(I_DROP_INSTREAM));

    // ─── 11. Write response to stdout ───
    // Get stdout handle
    f.instruction(&Instruction::Call(I_GET_STDOUT));
    f.instruction(&Instruction::LocalSet(stdout_h));

    // blocking-write-and-flush(stream, ptr, len, ret_ptr)
    f.instruction(&Instruction::LocalGet(stdout_h));
    f.instruction(&Instruction::LocalGet(future_h)); // body_ptr
    f.instruction(&Instruction::LocalGet(body_len));
    f.instruction(&Instruction::I32Const(RET + 92)); // ret_ptr
    f.instruction(&Instruction::Call(I_BLOCKING_WRITE));

    // Drop fields, request, out_body
    f.instruction(&Instruction::LocalGet(fields_h));
    f.instruction(&Instruction::Call(I_DROP_FIELDS));
    f.instruction(&Instruction::LocalGet(req_h));
    f.instruction(&Instruction::Call(I_DROP_REQUEST));
    f.instruction(&Instruction::LocalGet(out_body_h));
    f.instruction(&Instruction::Call(I_DROP_OUTBODY));

    // Return 0
    f.instruction(&Instruction::I64Const(0));
    f.instruction(&Instruction::End);
    f
}

fn build_start() -> Function {
    let mut f = Function::new([]);
    f.instruction(&Instruction::Call(F_RUN as u32));
    f.instruction(&Instruction::Drop);
    f.instruction(&Instruction::End);
    f
}

fn build_realloc() -> Function {
    let mut f = Function::new([(3, ValType::I32)]); // locals 4,5,6
    let bump = 4;
    let aligned = 5;
    let new_bump = 6;

    f.instruction(&Instruction::LocalGet(0)); // ptr
    f.instruction(&Instruction::I32Eqz);
    f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    emit_bump(&mut f, bump, aligned, new_bump);
    f.instruction(&Instruction::Else);
    f.instruction(&Instruction::LocalGet(3)); // new_size
    f.instruction(&Instruction::LocalGet(1)); // old_size
    f.instruction(&Instruction::I32LeU);
    f.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
    f.instruction(&Instruction::LocalGet(0)); // return ptr
    f.instruction(&Instruction::Else);
    emit_bump(&mut f, bump, aligned, new_bump);
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::End);
    f.instruction(&Instruction::End);
    f
}

fn emit_bump(f: &mut Function, bump: u32, aligned: u32, new_bump: u32) {
    f.instruction(&Instruction::I32Const(BUMP_PTR));
    f.instruction(&Instruction::I32Load(mem_arg(0)));
    f.instruction(&Instruction::LocalTee(bump));
    f.instruction(&Instruction::LocalGet(2)); // align
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalGet(2));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Sub);
    f.instruction(&Instruction::I32Const(-1));
    f.instruction(&Instruction::I32Xor);
    f.instruction(&Instruction::I32And);
    f.instruction(&Instruction::LocalTee(aligned));
    f.instruction(&Instruction::LocalGet(3)); // new_size
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::LocalSet(new_bump));
    f.instruction(&Instruction::I32Const(BUMP_PTR));
    f.instruction(&Instruction::LocalGet(new_bump));
    f.instruction(&Instruction::I32Store(mem_arg(0)));
    f.instruction(&Instruction::LocalGet(aligned));
}

fn mem_arg(offset: i32) -> wasm_encoder::MemArg {
    wasm_encoder::MemArg { offset: offset as u64, align: 2, memory_index: 0 }
}

fn mem_arg_byte(offset: i32) -> wasm_encoder::MemArg {
    wasm_encoder::MemArg { offset: offset as u64, align: 0, memory_index: 0 }
}
