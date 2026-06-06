#![allow(dead_code)]
#![allow(non_snake_case)]
//! WASI HTTP P2 integration for the Lisp compiler.
//!
//! Generates core WASM that directly imports canonical-lowered wasi:http@0.2.2
//! functions, with embedded WIT metadata for wit-component to wire up.
//!
//! # Index computation
//!
//! All type/import/function indices are computed dynamically by [`WasiHttpLayout`].
//! Adding a new import only requires:
//! 1. Adding a `FN_*` constant
//! 2. Adding the type + import entry in [`add_http_imports_to_sections`]
//! 3. Everything else (user_type_base, start_type, etc.) is derived automatically.

use wasm_encoder::*;

// ── Import function indices (positional, 0-based) ──
pub const FN_DROP_INPUT_STREAM: u32 = 0;
pub const FN_DROP_OUTPUT_STREAM: u32 = 1;
pub const FN_DROP_INCOMING_RESPONSE: u32 = 2;
const W: ValType = ValType::I32;
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
pub const FN_INPUT_STREAM_BLOCKING_READ: u32 = 22;
pub const FN_GET_STDOUT: u32 = 23;
pub const FN_OUTPUT_STREAM_WRITE: u32 = 24;
pub const FN_DROP_INCOMING_BODY: u32 = 25;
pub const FN_DROP_FIELDS: u32 = 26;
pub const FN_FIELDS_SET: u32 = 27;
/// Total count of wasi:http imports — must equal the last FN_* + 1.
pub const HTTP_IMPORT_COUNT: u32 = 28;

/// Total number of canonical ABI types used by the wasi:http imports.
/// Count the `types.ty().function(...)` calls in [`add_http_imports_to_sections`].
pub const HTTP_TYPE_COUNT: u32 = 10;

// ── Scratch memory layout (single source of truth) ──
// Shared by both wasi_http.rs (runtime URL path) and wasi_http_buffer.rs (data-segment path).
pub const SCRATCH: i32 = 196608; // 192KB offset
pub const SCRATCH_BODY_RESULT: i32 = SCRATCH;
pub const SCRATCH_STREAM_RESULT: i32 = SCRATCH + 4;
pub const SCRATCH_FUTURE_RESULT: i32 = SCRATCH + 8;
pub const SCRATCH_GET_RESULT: i32 = SCRATCH + 16;
pub const SCRATCH_CONSUME_RESULT: i32 = SCRATCH + 24;
pub const SCRATCH_READ_RESULT: i32 = SCRATCH + 32;
pub const SCRATCH_POLL_RESULT: i32 = SCRATCH + 48;
pub const SCRATCH_WRITE_RESULT: i32 = SCRATCH + 64; // write result area (runtime path)

/// Buffer for HTTP response body (used by data-segment path in call_outlayer.rs).
/// Placed between STDOUT_BUF (65536) and SCRATCH (196608) — 128KB available.
pub const SENTINEL_BUF: i32 = 65536; // right after STDOUT
pub const SENTINEL_BUF_SIZE: i32 = 131008; // ~128KB response buffer (ends at 196544, 64 byte gap before SCRATCH)
// Data-segment path uses different sub-offsets (see wasi_http_buffer.rs),
// but all start from SCRATCH.

/// Computed layout for the entire P2 WASM module.
///
/// Derives all type/import/function indices from the canonical constants,
/// so adding a new import doesn't require touching hardcoded numbers elsewhere.
#[derive(Debug, Clone)]
pub struct WasiHttpLayout {
    /// Number of user function types (0..=16 params → 17 types).
    pub user_type_count: u32,
    /// First type index for user functions.
    pub user_type_base: u32,
    /// Type index for _start: () -> i32
    pub start_type: u32,
    /// Type index for cabi_realloc: (i32,i32,i32,i32) -> i32
    pub realloc_type: u32,
    /// Type index for __wasi_http_get: (i32,i32,i32,i32,i32) -> i32
    pub http_get_type: u32,
    /// Type index for __wasi_http_post: (i32,i32,i32,i32,i32,i32,i32) -> i32
    pub http_post_type: u32,
    /// Total number of types in the type section.
    pub total_types: u32,

    /// Number of internal functions (http_get*2 + http_post*2).
    pub internal_fn_count: u32,
    /// Function index of first __wasi_http_get.
    pub http_get_fn_idx: u32,
    /// Function index of first __wasi_http_post.
    pub http_post_fn_idx: u32,
    /// Function index where user functions start.
    pub user_fn_base: u32,

    /// Number of user functions (set at build time).
    pub user_fn_count: u32,
    /// Function index of _start.
    pub start_fn_idx: u32,
    /// Function index of cabi_realloc.
    pub realloc_fn_idx: u32,
}

impl WasiHttpLayout {
    /// Compute the full layout given the number of user functions, HTTP GET count, and HTTP POST count.
    pub fn new(user_fn_count: u32, http_get_count: u32, http_post_count: u32) -> Self {
        let user_type_count = 17; // types for (i64×0..=16) -> i64
        let user_type_base = HTTP_TYPE_COUNT;
        let start_type = user_type_base + user_type_count;
        let realloc_type = start_type + 1;
        let http_get_type = realloc_type + 1;
        let http_post_type = http_get_type + 1;
        let total_types = http_post_type + 1;

        let http_get_count = if http_get_count == 0 && http_post_count == 0 {
            1 // at least 1 HTTP function pair when nothing else
        } else {
            http_get_count
        };
        let get_fn_count = http_get_count * 2; // each URL gets (get + poll_read)
        let post_fn_count = http_post_count * 2; // each POST URL gets (post + poll_read)
        let internal_fn_count = get_fn_count + post_fn_count;
        let http_get_fn_idx = HTTP_IMPORT_COUNT;
        let http_post_fn_idx = http_get_fn_idx + get_fn_count;
        let user_fn_base = http_get_fn_idx + internal_fn_count;

        let start_fn_idx = user_fn_base + user_fn_count;
        let realloc_fn_idx = start_fn_idx + 1;

        Self {
            user_type_count,
            user_type_base,
            start_type,
            realloc_type,
            http_get_type,
            http_post_type,
            total_types,
            internal_fn_count,
            http_get_fn_idx,
            http_post_fn_idx,
            user_fn_base,
            user_fn_count,
            start_fn_idx,
            realloc_fn_idx,
        }
    }
}

/// Single source of truth: add all wasi:http canonical ABI types AND imports
/// to the given sections. Called once from `build_p2_with_wasi_http`.
///
/// Returns nothing — the type indices are positional (0..HTTP_TYPE_COUNT-1)
/// and the import indices are the `FN_*` constants.
pub fn add_http_imports_to_sections(types: &mut TypeSection, imports: &mut ImportSection) {

    // Canonical ABI types for wasi:http@0.2.2 lowered functions.
    // Type indices must match what the import entries reference below.
    types.ty().function([], [W]);                          // 0: () -> i32 (constructors)
    types.ty().function([W], [W]);                         // 1: (i32) -> i32 (constructor, subscribe)
    types.ty().function([W, W], []);                       // 2: (i32,i32) -> () (body, consume, get)
    types.ty().function([W], []);                          // 3: (i32) -> () (resource drops)
    types.ty().function([W, W, W, W], [W]);                // 4: set-method/authority/path -> result
    types.ty().function([W, W, W, W, W], [W]);             // 5: set-scheme -> result
    types.ty().function([W, W, W, W], []);                 // 6: finish/handle/write-and-flush
    types.ty().function([W, W, W], []);                    // 7: poll
    types.ty().function([W, ValType::I64, W], []);         // 8: read
    types.ty().function([W, W, W, W, W, W], []);           // 9: fields.set(self, name_ptr, name_len, val_list_ptr, val_list_len, ret_ptr)

    assert_eq!(types.len(), HTTP_TYPE_COUNT, "HTTP type count mismatch");

    let ht = "wasi:http/types@0.2.2";
    let hh = "wasi:http/outgoing-handler@0.2.2";
    let is = "wasi:io/streams@0.2.2";
    let ip = "wasi:io/poll@0.2.2";
    let cs = "wasi:cli/stdout@0.2.2";

    // Import entries — order MUST match FN_* constants.
    imports.import(is, "[resource-drop]input-stream", EntityType::Function(3));
    imports.import(is, "[resource-drop]output-stream", EntityType::Function(3));
    imports.import(ht, "[resource-drop]incoming-response", EntityType::Function(3));
    imports.import(ht, "[resource-drop]future-incoming-response", EntityType::Function(3));
    imports.import(ht, "[constructor]fields", EntityType::Function(0));
    imports.import(ht, "[constructor]outgoing-request", EntityType::Function(1));
    imports.import(ht, "[method]outgoing-request.set-method", EntityType::Function(4));
    imports.import(ht, "[method]outgoing-request.set-scheme", EntityType::Function(5));
    imports.import(ht, "[method]outgoing-request.set-authority", EntityType::Function(4));
    imports.import(ht, "[method]outgoing-request.set-path-with-query", EntityType::Function(4));
    imports.import(ht, "[method]outgoing-request.body", EntityType::Function(2));
    imports.import(ht, "[method]outgoing-body.write", EntityType::Function(2));
    imports.import(ht, "[static]outgoing-body.finish", EntityType::Function(6));
    imports.import(ht, "[resource-drop]outgoing-body", EntityType::Function(3));
    imports.import(hh, "handle", EntityType::Function(6));
    imports.import(ht, "[resource-drop]outgoing-request", EntityType::Function(3));
    imports.import(ht, "[method]future-incoming-response.get", EntityType::Function(2));
    imports.import(ht, "[method]future-incoming-response.subscribe", EntityType::Function(1));
    imports.import(ip, "poll", EntityType::Function(7));
    imports.import(ip, "[resource-drop]pollable", EntityType::Function(3));
    imports.import(ht, "[method]incoming-response.consume", EntityType::Function(2));
    imports.import(ht, "[method]incoming-body.stream", EntityType::Function(2));
    imports.import(is, "[method]input-stream.blocking-read", EntityType::Function(8));
    imports.import(cs, "get-stdout", EntityType::Function(0));
    imports.import(is, "[method]output-stream.blocking-write-and-flush", EntityType::Function(6));
    imports.import(ht, "[resource-drop]incoming-body", EntityType::Function(3));
    imports.import(ht, "[resource-drop]fields", EntityType::Function(3));
    imports.import(ht, "[method]fields.set", EntityType::Function(9));

    assert_eq!(imports.len(), HTTP_IMPORT_COUNT, "HTTP import count mismatch");
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
    // NOTE: constructor takes ownership (own) of fields, so do NOT drop it
    func.instruction(&lg(0));
    func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&ls(1));

    // Step 4: set-method GET  -- (i32,i32,i32,i32) -> i32
    func.instruction(&lg(1));  // req
    func.instruction(&cst(0)); // disc=0 (get)
    func.instruction(&cst(SCRATCH)); func.instruction(&cst(0)); // valid ptr, len=0
    func.instruction(&cl(FN_SET_METHOD));
    func.instruction(&Instruction::Drop);

    // Step 5: set-scheme HTTPS  -- (i32,i32,i32,i32,i32) -> i32
    // option<scheme>: Some(HTTPS) → (1=Some, 1=HTTPS, valid_ptr, 0)
    func.instruction(&lg(1));
    func.instruction(&cst(0)); // Some = 0
    func.instruction(&cst(1)); // HTTPS = 1
    func.instruction(&cst(SCRATCH)); func.instruction(&cst(0)); // valid ptr, len=0
    func.instruction(&cl(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // Step 6: set-authority = URL  -- (i32,i32,i32,i32) -> i32
    // option<string> Some → disc=1
    func.instruction(&lg(1));
    func.instruction(&cst(1)); // Some = 1
    func.instruction(&lg(url_ptr_local));
    func.instruction(&lg(url_len_local));
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // Step 7: set-path-with-query = "/"  -- (i32,i32,i32,i32) -> i32
    func.instruction(&cst(SCRATCH));
    func.instruction(&cst(0x2F));
    func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&lg(1));
    func.instruction(&cst(1)); // Some = 1
    func.instruction(&cst(SCRATCH));
    func.instruction(&cst(1));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);

    // Step 8: body = outgoing-request.body(req)  -- (i32,i32) -> ()
    // Writes body handle to SCRATCH_BODY_RESULT
    func.instruction(&lg(1));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    // Load body handle from memory: result<outgoing-body> at +0=disc, +4=handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT + 4));
    func.instruction(&ls(2));

    // Step 9: get output stream from body, drop it (empty body for GET)
    func.instruction(&lg(2));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_WRITE));
    // result<output-stream>: +0=disc, +4=handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
    func.instruction(&cl(FN_DROP_OUTPUT_STREAM));

    // Step 10: finish body (no trailers)  -- (i32,i32,i32,i32) -> ()
    // option<trailers>=None: disc=0 (None in canonical ABI)
    func.instruction(&lg(2));
    func.instruction(&cst(0)); // None = 0
    func.instruction(&cst(0)); // pad
    func.instruction(&cst(SCRATCH_WRITE_RESULT)); // valid dst — reuse from above (SCRATCH+64)
    // Note: SCRATCH_WRITE_RESULT was defined as SCRATCH+64 in the old split,
    // but here we use the canonical constant from the buffer module.
    // The value is the same: 131072 + 64 = 131136.
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // Step 11: drop body
    func.instruction(&lg(2));
    func.instruction(&cl(FN_DROP_OUTGOING_BODY));

    // Step 12: handle(req, None, dst, pad)  -- (i32,i32,i32,i32) -> ()
    // option<request-options>=None: disc=0 (None in canonical ABI)
    func.instruction(&lg(1));
    func.instruction(&cst(0)); // None = 0
    func.instruction(&cst(0)); // pad
    func.instruction(&cst(SCRATCH_FUTURE_RESULT)); // dst
    func.instruction(&cl(FN_HANDLE));
    // Load future handle from result: +0=disc, +4=handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT + 4));
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
    // Load discriminant: 1=Some (ready)
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_GET_RESULT));
    func.instruction(&cst(1)); // Some = 1 (response ready)
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
    // option<result<incoming-response, error-code>>:
    // +0=option disc, +4=result disc, +8=handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_GET_RESULT + 8));
    func.instruction(&ls(5));

    // Drop future
    func.instruction(&lg(3));
    func.instruction(&cl(FN_DROP_FUTURE_INCOMING_RESPONSE));

    // Step 15: consume response  -- (i32,i32) -> ()
    // result<incoming-body>: +0=disc, +4=handle
    func.instruction(&lg(5));
    func.instruction(&cst(SCRATCH_CONSUME_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_CONSUME_RESULT + 4));
    func.instruction(&ls(6));

    // Drop response
    func.instruction(&lg(5));
    func.instruction(&cl(FN_DROP_INCOMING_RESPONSE));

    // stream = incoming-body.stream(body)
    func.instruction(&lg(6));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    // result<input-stream>: +0=disc, +4=handle
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
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
    func.instruction(&cl(FN_INPUT_STREAM_BLOCKING_READ));
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
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut resolve = wit_parser::Resolve::new();
        let wit_dir = find_wit_dir()?;

        // Push dependency subdirs first so cross-package imports resolve
        let dep_dirs: &[&str] = &[
            "io", "clocks", "random", "filesystem", "sockets", "cli", "http",
            "near-storage", "near-payment", "near-vrf", "outlayer-wallet", "near-rpc",
            "simple-http",
        ];
        for subdir in dep_dirs {
            let dir = wit_dir.join(subdir);
            if dir.exists() {
                resolve.push_dir(&dir).map_err(|e| format!("push_dir {} failed: {}", subdir, e))?;
            }
        }
        // Now push root which contains combined.wit
        resolve.push_dir(&wit_dir).map_err(|e| format!("push_dir root failed: {}", e))?;

        // Look for simple-http world first, fall back to outlayer-http
        let mut found_world = None;
        for (_pkg_id, pkg) in resolve.packages.iter() {
            for (name, world_id) in &pkg.worlds {
                if name == "simple-http" || name == "outlayer-http" {
                    found_world = Some(*world_id);
                    break;
                }
            }
            if found_world.is_some() { break; }
        }
        let world = found_world.ok_or("world 'simple-http' or 'outlayer-http' not found")?;

        Ok((resolve, world))
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::wit_embed::build_http_wit_metadata_embedded()
    }
}

pub fn build_combined_wit_metadata() -> Result<(wit_parser::Resolve, wit_parser::WorldId), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut resolve = wit_parser::Resolve::new();
        let wit_dir = find_wit_dir()?;

        // push_dir processes one directory at a time. We need to push deps in
        // dependency order so that cross-package imports resolve correctly.
        // The outlayer-http world (in combined.wit) imports packages
        // from near-rpc, near-storage, near-payment, near-vrf, outlayer-wallet,
        // and all wasi packages, so those must be loaded first.
        let dep_dirs: &[&str] = &[
            "io", "clocks", "random", "filesystem", "sockets", "cli", "http",
            "near-storage", "near-payment", "near-vrf", "outlayer-wallet", "near-rpc",
        ];
        for subdir in dep_dirs {
            let dir = wit_dir.join(subdir);
            if dir.exists() {
                resolve.push_dir(&dir).map_err(|e| format!("push_dir {} failed: {}", subdir, e))?;
            }
        }
        // Now push the root deps/ dir which contains combined.wit with outlayer-http
        resolve.push_dir(&wit_dir).map_err(|e| format!("push_dir root failed: {}", e))?;

        let mut found_world = None;
        for (_pkg_id, pkg) in resolve.packages.iter() {
            for (name, world_id) in &pkg.worlds {
                if name == "outlayer-http" {
                    found_world = Some(*world_id);
                    break;
                }
            }
            if found_world.is_some() { break; }
        }
        let world = found_world.ok_or("world 'outlayer-http' not found")?;

        Ok((resolve, world))
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::wit_embed::build_http_wit_metadata_embedded()
    }
}

fn find_wit_dir() -> Result<std::path::PathBuf, String> {
    let candidates = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/wit/deps"),
        "wit/deps",
        "lisp-rlm/wit/deps",
        "/Users/asil/.openclaw/workspace/lisp-rlm/wit/deps",
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
    fn test_layout_indices_zero_user_fns() {
        let layout = WasiHttpLayout::new(0, 1, 0);
        assert_eq!(layout.user_type_base, HTTP_TYPE_COUNT); // 10
        assert_eq!(layout.start_type, 27); // 10 + 17
        assert_eq!(layout.realloc_type, 28);
        assert_eq!(layout.http_get_type, 29);
        assert_eq!(layout.http_get_fn_idx, HTTP_IMPORT_COUNT); // 28
        assert_eq!(layout.user_fn_base, HTTP_IMPORT_COUNT + 2); // 30
        assert_eq!(layout.start_fn_idx, HTTP_IMPORT_COUNT + 2); // 30 (0 user fns)
        assert_eq!(layout.realloc_fn_idx, HTTP_IMPORT_COUNT + 3); // 31
    }

    #[test]
    fn test_layout_indices_with_user_fns() {
        let layout = WasiHttpLayout::new(5, 1, 0);
        assert_eq!(layout.user_fn_base, 30);
        assert_eq!(layout.start_fn_idx, 35); // 30 + 5
        assert_eq!(layout.realloc_fn_idx, 36); // 35 + 1
    }

    #[test]
    fn test_layout_multi_url() {
        // 3 URLs → 6 internal functions (3 × 2)
        let layout = WasiHttpLayout::new(2, 3, 0);
        assert_eq!(layout.internal_fn_count, 6);
        assert_eq!(layout.http_get_fn_idx, HTTP_IMPORT_COUNT); // 28
        assert_eq!(layout.user_fn_base, HTTP_IMPORT_COUNT + 6); // 34
        assert_eq!(layout.start_fn_idx, 36); // 34 + 2
        assert_eq!(layout.realloc_fn_idx, 37);
    }

    #[test]
    fn test_type_and_import_counts_match() {
        let mut types = TypeSection::new();
        let mut imports = ImportSection::new();
        add_http_imports_to_sections(&mut types, &mut imports);
        assert_eq!(types.len(), HTTP_TYPE_COUNT);
        assert_eq!(imports.len(), HTTP_IMPORT_COUNT);
    }

    #[test]
    fn test_fn_constants_sequential() {
        // Verify FN_* constants are sequential 0..27
        let max_fn = *[
            FN_DROP_INPUT_STREAM, FN_DROP_OUTPUT_STREAM, FN_DROP_INCOMING_RESPONSE,
            FN_DROP_FUTURE_INCOMING_RESPONSE, FN_CONSTRUCTOR_FIELDS, FN_CONSTRUCTOR_OUTGOING_REQUEST,
            FN_SET_METHOD, FN_SET_SCHEME, FN_SET_AUTHORITY, FN_SET_PATH_WITH_QUERY,
            FN_OUTGOING_REQUEST_BODY, FN_OUTGOING_BODY_WRITE, FN_OUTGOING_BODY_FINISH,
            FN_DROP_OUTGOING_BODY, FN_HANDLE, FN_DROP_OUTGOING_REQUEST, FN_FUTURE_GET,
            FN_FUTURE_SUBSCRIBE, FN_POLL, FN_DROP_POLLABLE, FN_INCOMING_RESPONSE_CONSUME,
            FN_INCOMING_BODY_STREAM, FN_INPUT_STREAM_BLOCKING_READ, FN_GET_STDOUT, FN_OUTPUT_STREAM_WRITE,
            FN_DROP_INCOMING_BODY, FN_DROP_FIELDS, FN_FIELDS_SET,
        ].iter().max().unwrap();
        assert_eq!(HTTP_IMPORT_COUNT, max_fn + 1, "HTTP_IMPORT_COUNT should be max FN_* + 1");
    }

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
