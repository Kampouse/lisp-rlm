//! Data-segment-based HTTP GET emission for wasi:http P2 components.
//!
//! Unlike [`crate::wasi_http::emit_http_get`] which builds URL at runtime,
//! this module pre-loads URL + headers into WASM data segments for minimal
//! instruction count (no runtime string construction).

use crate::wasi_http::*;
use wasm_encoder::{BlockType, Function, Instruction, MemArg};

// ── Scratch memory sub-offsets for the data-segment path ──
// These differ from the runtime path offsets in wasi_http.rs because the
// instruction sequences are different. Both start from SCRATCH (131072).
pub const SCRATCH_BODY_RESULT: i32 = SCRATCH;
pub const SCRATCH_WRITE_RESULT: i32 = SCRATCH + 16;
pub const SCRATCH_FUTURE_RESULT: i32 = SCRATCH + 32;
pub const SCRATCH_POLLABLES: i32 = SCRATCH + 64;
pub const SCRATCH_POLL_RESULT: i32 = SCRATCH + 96;
pub const SCRATCH_RESPONSE_RESULT: i32 = SCRATCH + 128;
pub const SCRATCH_IBODY_RESULT: i32 = SCRATCH + 192;
pub const SCRATCH_STREAM_RESULT: i32 = SCRATCH + 208;
pub const SCRATCH_READ_RESULT: i32 = SCRATCH + 224;

/// Memory address where data-segment strings start (after all scratch areas).
pub const DATA_BASE: i32 = SCRATCH + 256 + 16; // 131584

/// A single HTTP header entry (name, value).
pub type Header = (&'static [u8], &'static [u8]);

/// Data segment offsets for URL + headers, pre-computed at build time.
pub struct HttpDataSegments {
    pub segments: Vec<(u32, Vec<u8>)>,
    pub auth_offset: i32,
    pub auth_len: i32,
    pub path_offset: i32,
    pub path_len: i32,
    /// Per-header data for fields.set calls: (name_ptr, name_len, val_list_ptr, val_list_len)
    pub headers: Vec<(i32, i32, i32, i32)>,
    /// Response header to capture (x402): name ptr/len in data segments.
    pub hdr_name_offset: i32,
    pub hdr_name_len: i32,
}

/// Pre-compute data segments for a URL + optional headers.
///
/// Breaks the URL into authority (host) and path (path+query).
/// Places all strings and list entries in WASM memory starting at [`DATA_BASE`].
///
/// # Arguments
/// * `authority` — Host part of the URL (e.g. `b"api.open-meteo.com"`)
/// * `path` — Path + query string (e.g. `b"/v1/forecast?lat=45.50"`)
/// * `headers` — Slice of (name, value) pairs (e.g. `[(b"Accept", b"application/json")]`)
pub fn build_url_data_segments(
    authority: &[u8],
    path: &[u8],
    headers: &[Header],
) -> HttpDataSegments {
    build_url_data_segments_with_base(authority, path, headers, DATA_BASE)
}

/// Pre-compute data segments for a URL + optional headers at a custom base offset.
///
/// Same as [`build_url_data_segments`] but places data starting at `base_offset`
/// instead of [`DATA_BASE`]. Use this for multi-URL programs where each URL's
/// data segments are placed at different offsets.
///
/// Returns the [`HttpDataSegments`] and the total size consumed (aligned).
pub fn build_url_data_segments_with_base(
    authority: &[u8],
    path: &[u8],
    headers: &[Header],
    base_offset: i32,
) -> HttpDataSegments {
    let auth_offset = base_offset;
    let path_offset = auth_offset + align4(authority.len());

    let mut segments = vec![
        (auth_offset as u32, authority.to_vec()),
        (path_offset as u32, path.to_vec()),
    ];

    let mut str_offset = path_offset + align4(path.len());
    let mut header_entries: Vec<(i32, i32, i32, i32)> = Vec::new();

    for (name, value) in headers {
        // Name string
        let name_off = str_offset;
        segments.push((str_offset as u32, name.to_vec()));
        str_offset += align4(name.len());

        // Value string
        let val_off = str_offset;
        segments.push((str_offset as u32, value.to_vec()));
        str_offset += align4(value.len());

        // Value list: list<string> with 1 element = 8 bytes: (val_ptr_le, val_len_le)
        let val_list_off = str_offset;
        let mut val_list_data = Vec::with_capacity(8);
        val_list_data.extend_from_slice(&val_off.to_le_bytes());
        val_list_data.extend_from_slice(&(value.len() as i32).to_le_bytes());
        segments.push((str_offset as u32, val_list_data));
        str_offset += 8; // already aligned

        header_entries.push((name_off, name.len() as i32, val_list_off, 1));
    }

    HttpDataSegments {
        segments,
        auth_offset,
        auth_len: authority.len() as i32,
        path_offset,
        path_len: path.len() as i32,
        headers: header_entries,
        hdr_name_offset: 0,
        hdr_name_len: 0,
    }
}

/// Same as [`build_url_data_segments_with_base`] plus a response-header
/// capture: places `capture_name` in the data segments and records its
/// offset/len so the GET core fn runs fields.get(name) and stashes the
/// value at (buf + 65536 - 2048), len at len_ptr+4.
pub fn build_url_data_segments_with_capture(
    authority: &[u8],
    path: &[u8],
    headers: &[Header],
    base_offset: i32,
    capture_name: &[u8],
) -> HttpDataSegments {
    let mut d = build_url_data_segments_with_base(authority, path, headers, base_offset);
    // place the capture name after the last segment
    let last = d.segments.last().map(|(off, b)| *off as i32 + b.len() as i32);
    let end = last.unwrap_or(base_offset);
    let name_off = (end + 3) & !3;
    d.segments.push((name_off as u32, capture_name.to_vec()));
    d.hdr_name_offset = name_off;
    d.hdr_name_len = capture_name.len() as i32;
    d
}

impl HttpDataSegments {
    /// Compute the total byte size spanned by all segments (end of last segment - base offset).
    /// Useful for computing where the next URL's data segments should start.
    pub fn total_span(&self) -> i32 {
        self.segments
            .iter()
            .map(|(off, bytes)| *off as i32 + bytes.len() as i32)
            .max()
            .unwrap_or(0)
    }
}

/// Align `len` up to the next 4-byte boundary, returned as i32.
fn align4(len: usize) -> i32 {
    ((len + 3) & !3) as i32
}

/// Emit the full HTTP GET call sequence using pre-loaded data segments.
///
/// URL bytes are already in memory via data segments — no runtime instructions
/// needed for string construction. Just reference offsets directly.
///
/// Uses locals 2, 4, 5, 6–14 (caller must allocate).
pub fn emit_http_get_to_buffer(func: &mut Function, data: &HttpDataSegments) {
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let st = |off: i32| {
        Instruction::I32Store(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };
    let ld = |off: i32| {
        Instruction::I32Load(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };

    // Fields → Request
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&ls(5)); // fields handle

    // Set headers: one fields.set call per header
    for &(name_ptr, name_len, val_list_ptr, val_list_len) in &data.headers {
        func.instruction(&lg(5)); // fields handle (self)
        func.instruction(&cst(name_ptr));
        func.instruction(&cst(name_len));
        func.instruction(&cst(val_list_ptr));
        func.instruction(&cst(val_list_len));
        func.instruction(&cst(SCRATCH_WRITE_RESULT)); // return area for result<_, header-error>
        func.instruction(&cl(FN_FIELDS_SET));
    }

    func.instruction(&lg(5)); // fields handle (ctor stored it in 5)
    func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&ls(6)); // request handle

    // Set method=GET
    func.instruction(&lg(6));
    func.instruction(&cst(0)); // option none
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_METHOD));
    func.instruction(&Instruction::Drop);

    // Set scheme=HTTPS
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(1)); // variant index 1 = HTTPS
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // Set authority + path. DYNAMIC when capture is wired (httpGetHdr):
    // parse "https://authority/path" from url params (locals 0/1). Static
    // data-segment path otherwise.
    let dynamic = data.hdr_name_len > 0;
    if dynamic {
        // locals: 13=i, 14=auth_len, 15=path_ptr, 16=path_len
        // i = 8 (skip "https://")
        func.instruction(&cst(8));
        func.instruction(&ls(13));
        // auth_len = url_len - 8 (default: no slash)
        func.instruction(&lg(1));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Sub);
        func.instruction(&ls(14));
        func.instruction(&Instruction::Block(BlockType::Empty)); // $done
        func.instruction(&Instruction::Loop(BlockType::Empty)); // $scan
        // if i >= url_len → br $done
        func.instruction(&lg(13));
        func.instruction(&lg(1));
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));
        // if url[i] == '/' → auth_len = i-8; br $done
        func.instruction(&lg(0));
        func.instruction(&lg(13));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&cst(0x2f));
        func.instruction(&Instruction::I32Eq);
        // if eq: compute auth_len then br 1; else drop and continue
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&lg(13));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Sub);
        func.instruction(&ls(14));
        func.instruction(&Instruction::Br(2)); // br $done: If=0, Loop=1, Block=2
        func.instruction(&Instruction::End); // if
        // i++ ; br $scan
        func.instruction(&lg(13));
        func.instruction(&cst(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&ls(13));
        func.instruction(&Instruction::Br(0));
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block
        // NOTE on br depth inside If inside Loop inside Block:
        //   labels innermost-first: If=0, Loop=1, Block=2.
        //   br 2 = exit block; br 0 = loop continue.
        // authority = url+8, len = auth_len
        func.instruction(&lg(6));
        func.instruction(&cst(1)); // option some
        func.instruction(&lg(0));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(14));
        func.instruction(&cl(FN_SET_AUTHORITY));
        func.instruction(&Instruction::Drop);
        // path: if auth_len+8 < url_len → url+8+auth_len+1, url_len-9-auth_len
        // else static "/" segment (data.path_offset)
        func.instruction(&lg(14));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(1));
        func.instruction(&Instruction::I32LtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&lg(6));
        func.instruction(&cst(1));
        func.instruction(&lg(0));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(14));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(1));
        func.instruction(&cst(8));
        func.instruction(&Instruction::I32Sub);
        func.instruction(&lg(14));
        func.instruction(&Instruction::I32Sub);
        func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
        func.instruction(&Instruction::Drop);
        func.instruction(&Instruction::Else);
        func.instruction(&lg(6));
        func.instruction(&cst(1));
        func.instruction(&cst(data.path_offset));
        func.instruction(&cst(data.path_len));
        func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
        func.instruction(&Instruction::Drop);
        func.instruction(&Instruction::End);
    } else {
        func.instruction(&lg(6));
        func.instruction(&cst(1)); // option some
        func.instruction(&cst(data.auth_offset));
        func.instruction(&cst(data.auth_len));
        func.instruction(&cl(FN_SET_AUTHORITY));
        func.instruction(&Instruction::Drop);

        func.instruction(&lg(6));
        func.instruction(&cst(1)); // option some
        func.instruction(&cst(data.path_offset));
        func.instruction(&cst(data.path_len));
        func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
        func.instruction(&Instruction::Drop);
    }

    // Body → handle at +4
    func.instruction(&lg(6));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT + 4));
    func.instruction(&ls(7)); // outgoing-body handle

    // Finish
    func.instruction(&lg(7));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // Handle → future handle at +8
    // responder block: disc-error exits land here (graceful total_len=0 path
    // for runtimes without egress — TEE hosts return result::err from handle)
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(6));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
    // handle() result disc (≠0 = err) → reset total_len, exit responder
    func.instruction(&cst(0));
    func.instruction(&ls(13));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(0));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT + 8));
    func.instruction(&ls(8)); // future handle

    // Subscribe → pollable on stack
    func.instruction(&lg(8));
    func.instruction(&cl(FN_FUTURE_SUBSCRIBE));
    func.instruction(&ls(9)); // pollable handle

    // Poll
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&lg(9));
    func.instruction(&st(0));
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT));
    func.instruction(&cl(FN_POLL));
    func.instruction(&lg(9));
    func.instruction(&cl(FN_DROP_POLLABLE));

    // Future.get → response handle at +24
    func.instruction(&lg(8));
    func.instruction(&cst(SCRATCH_RESPONSE_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_RESPONSE_RESULT + 24));
    func.instruction(&ls(10)); // incoming-response handle

    // ── Header capture (x402): hdr = headers(response);
    //    (disc,ptr,len) = fields.get(hdr, name from data segment);
    //    copy value into HDR region (buf + HDR_REGION_OFF), store its len at
    //    len_ptr+4. Best-effort: silent skip on none/empty.
    func.instruction(&lg(10));
    func.instruction(&cl(FN_INCOMING_RESPONSE_HEADERS));
    func.instruction(&ls(16)); // local 16: response headers fields handle
    // fields.get(fields, name_ptr, name_len) -> (disc, ptr, len) flat
    func.instruction(&lg(16));
    func.instruction(&cst(data.hdr_name_offset));
    func.instruction(&cst(data.hdr_name_len));
    func.instruction(&cst(SCRATCH_WRITE_RESULT)); // ret_area (list = ptr+len)
    func.instruction(&cl(FN_FIELDS_GET));
    // ret_area: +0 entries ptr, +4 count; entry = (ptr,len) pair
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_WRITE_RESULT));
    func.instruction(&ls(18)); // entries ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_WRITE_RESULT + 4));
    func.instruction(&ls(17)); // count
    func.instruction(&cst(0));
    func.instruction(&ls(19)); // value len = 0 default
    func.instruction(&Instruction::Block(BlockType::Empty));
    // skip when no entries
    func.instruction(&lg(17));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(0));
    // entry 0: base = entries ptr; val_ptr = [base+0], val_len = [base+4]
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
    func.instruction(&ls(19)); // val_ptr (local 19 reuse)
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
    func.instruction(&ls(17)); // val_len
    // clamp: len = min(len, 2048) via select (a=len, b=2048, cond=len>2048)
    func.instruction(&lg(17));
    func.instruction(&cst(2048));
    func.instruction(&lg(17));
    func.instruction(&cst(2048));
    func.instruction(&Instruction::I32GtU);
    func.instruction(&Instruction::Select);
    func.instruction(&ls(17));
    // copy value into HDR region at (buf + 65536 - 2048)
    func.instruction(&lg(2));
    func.instruction(&cst(65536 - 2048));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(19));
    func.instruction(&lg(17));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    // hdr len at len_ptr+4
    func.instruction(&lg(4));
    func.instruction(&Instruction::I32Const(4));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(17));
    func.instruction(&st(0));
    func.instruction(&Instruction::End);
    func.instruction(&lg(16));
    func.instruction(&cl(FN_DROP_FIELDS));
    // Consume → incoming-body handle at +4
    func.instruction(&lg(10));
    func.instruction(&cst(SCRATCH_IBODY_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_IBODY_RESULT + 4));
    func.instruction(&ls(11)); // incoming-body handle

    // Stream → input-stream handle at +4
    func.instruction(&lg(11));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
    func.instruction(&ls(12)); // input-stream handle

    // Read loop - keep reading until EOF (len=0)
    // Local 13 = total accumulated length
    // Local 14 = chunk length from blocking-read
    // Local 15 = copy counter
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ls(13)); // total_len = 0

    func.instruction(&Instruction::Block(BlockType::Empty)); // outer block - break here on EOF/error
    func.instruction(&Instruction::Loop(BlockType::Empty)); // read loop

    // blocking-read(stream, 65536, dst)
    func.instruction(&lg(12)); // stream handle
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_BLOCKING_READ));

    // Check result disc: (disc != 0) means error → break outer
    // Stack: push constant 0, load disc, compare for inequality
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT)); // disc at offset 0
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1)); // break outer on error

    // Check len (0=EOF) → break outer
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8)); // len at offset 8
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1)); // break outer on EOF

    // Store chunk_len from result
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(14)); // chunk_len

    // Capacity guard: if chunk <= remaining (CAP - total), copy it and
    // loop; else copy only the remaining bytes and break (buffer full).
    func.instruction(&lg(14));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&Instruction::I32LeU);
    func.instruction(&Instruction::If(BlockType::Empty));
    // chunk fits: copy + accumulate + loop
    func.instruction(&lg(2)); // dst = resp_buf base
    func.instruction(&lg(13)); // + total_len offset
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0)); // load chunk data ptr
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&lg(14)); // len = chunk_len
    func.instruction(&Instruction::MemoryCopy {
        src_mem: 0,
        dst_mem: 0,
    });

    // total_len += chunk_len
    func.instruction(&lg(13));
    func.instruction(&lg(14));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(13));

    func.instruction(&Instruction::Br(1)); // continue loop (depth: If=0, Loop=1)
    func.instruction(&Instruction::End); // end if (fits)
    // chunk exceeds remaining: copy the remainder, total = CAP, break.
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0)); // load chunk data ptr
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub); // remaining
    func.instruction(&Instruction::MemoryCopy {
        src_mem: 0,
        dst_mem: 0,
    });
    func.instruction(&cst(1_048_576));
    func.instruction(&ls(13)); // total = CAP
    func.instruction(&Instruction::Br(2)); // break outer (depth: If=0, Loop=1, Block=2)
    func.instruction(&Instruction::End); // loop ends
    func.instruction(&Instruction::End); // block ends
    func.instruction(&Instruction::End); // responder block ends

    // Store total_len at resp_len_ptr (local 4)
    func.instruction(&lg(4));
    func.instruction(&lg(13)); // total_len
    func.instruction(&st(0));
    func.instruction(&Instruction::I32Const(0));
}

/// Dynamic-URL GET with a runtime request header (x402 settle leg).
/// Clone of emit_http_get_to_buffer: UA/Accept from fixed segment
/// tuples; ONE extra fields.set driven by runtime params 5..8
/// (name ptr/len, value ptr/len), sanitized (&0x7F - P2 string bytes
/// carry a spurious |0x80). Fallback path/auth = static "/" segment.
pub fn emit_http_get_with_header_v2(
    func: &mut Function,
    ua: (i32, i32, i32, i32),       // UA name ptr/len, value ptr/len
    ua_entry: (i32, i32),           // 8-byte list entry (ptr, len)
    accept: (i32, i32, i32, i32),   // Accept name ptr/len, value ptr/len
    accept_entry: (i32, i32),
    fallback_path: (i32, i32),      // static "/" segment (ptr, len)
    fallback_auth: (i32, i32),      // no-slash fallback auth (unreachable)
    capture_name: (i32, i32),       // response capture name (len 0 = none)
) {
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let st = |off: i32| {
        Instruction::I32Store(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };
    let ld = |off: i32| {
        Instruction::I32Load(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };

    // CRITICAL: shadow-copy params 5..8 BEFORE anything else —
    // local 5 is CLOBBERED by the fields ctor below (fn163's
    // local 5 = fields handle). Locals 20..23 = hdr name ptr/len,
    // val ptr/len. (Copying after the ctor stored the handle = 1
    // into the "name ptr" shadow — the bogus-ptr bug.)
    func.instruction(&lg(5));
    func.instruction(&ls(20));
    func.instruction(&lg(6));
    func.instruction(&ls(21));
    func.instruction(&lg(7));
    func.instruction(&ls(22));
    func.instruction(&lg(8));
    func.instruction(&ls(23));

            // Fields ctor → fields handle in local 5
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&ls(5));


    // Set headers: UA + Accept from fixed segments; runtime header
    // (name/val copied into the buf tail by the caller-preceding
    // copy block below) — list len = ENTRY COUNT (1), not bytes.
    func.instruction(&lg(5)); // fields handle (self)
    func.instruction(&cst(ua.0));
    func.instruction(&cst(ua.1));
    func.instruction(&cst(ua_entry.0));
    func.instruction(&cst(ua_entry.1));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_FIELDS_SET));

    func.instruction(&lg(5));
    func.instruction(&cst(accept.0));
    func.instruction(&cst(accept.1));
    func.instruction(&cst(accept_entry.0));
    func.instruction(&cst(accept_entry.1));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_FIELDS_SET));

    // Runtime header: skip entirely when name_len == 0 (param 6).
    // Copy name → buf+62K, val → buf+63K; one-entry list at buf+63.5K.
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(21));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(0));
    // name copy: dst = buf + 65536-1024, src = local20, len = local21
    func.instruction(&lg(2));
    func.instruction(&cst(32768));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(20));
    func.instruction(&lg(21));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    // sanitize name bytes: strip bytes >= 0x80 (high bit) to 0x20
    func.instruction(&cst(0));
    func.instruction(&ls(18)); // i = 0
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    func.instruction(&lg(18));
    func.instruction(&lg(21));
    func.instruction(&Instruction::I32GeU);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(2));
    func.instruction(&cst(32768));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Add);
    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&cst(128));
    func.instruction(&Instruction::I32GeU);
    func.instruction(&Instruction::If(BlockType::Empty));
    func.instruction(&lg(2));
    func.instruction(&cst(32768));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0x20));
    func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&Instruction::End);
    func.instruction(&lg(18));
    func.instruction(&cst(1));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(18));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End);
    // val copy: dst = buf + 65536-3072, src = local22, len = min(local23, 3072)
    // (x402 PAYMENT-SIGNATURE b64 needs ~2KB; old cap 500 truncated it)
    // select returns a if cond≠0: a=3072 (clamped), b=len, cond=len>3072
    func.instruction(&cst(3072));
    func.instruction(&lg(23));
    func.instruction(&lg(23));
    func.instruction(&cst(3072));
    func.instruction(&Instruction::I32GtU);
    func.instruction(&Instruction::Select);
    func.instruction(&ls(17)); // clamped val len
    func.instruction(&lg(2));
    func.instruction(&cst(29696));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(22));
    func.instruction(&lg(17));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    // one-entry list at buf + 65536-512: (val_ptr, val_len)
    func.instruction(&lg(2));
    func.instruction(&cst(32256));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(2));
    func.instruction(&cst(29696));
    func.instruction(&Instruction::I32Add);
    func.instruction(&st(0));
    func.instruction(&lg(2));
    func.instruction(&cst(32256));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(17));
    func.instruction(&st(4));
    // fields.set(fields, name=buf+62K/name_len, entry=buf+63.5K, COUNT=1)
    func.instruction(&lg(5));
    func.instruction(&lg(2));
    func.instruction(&cst(32768));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(21));
    func.instruction(&lg(2));
    func.instruction(&cst(33536));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(1)); // list COUNT: 1 entry
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_FIELDS_SET));
    func.instruction(&Instruction::End); // guard block

    // Request ctor: fields handle in local 5
    func.instruction(&lg(5));
    func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&ls(6)); // request handle

    // Set method=GET
    func.instruction(&lg(6));
    func.instruction(&cst(0)); // option none
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_METHOD));
    func.instruction(&Instruction::Drop);

    // Set scheme=HTTPS
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(1)); // variant index 1 = HTTPS
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // ALWAYS dynamic: parse "https://authority/path" from url
    // (locals 13=i, 14=auth_len, 15/16 scratch)
    func.instruction(&cst(8));
    func.instruction(&ls(13));
    func.instruction(&lg(1));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&ls(14));
    func.instruction(&Instruction::Block(BlockType::Empty)); // $done
    func.instruction(&Instruction::Loop(BlockType::Empty)); // $scan
    func.instruction(&lg(13));
    func.instruction(&lg(1));
    func.instruction(&Instruction::I32GeU);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(0));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&cst(0x2f));
    func.instruction(&Instruction::I32Eq);
    func.instruction(&Instruction::If(BlockType::Empty));
    func.instruction(&lg(13));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&ls(14));
    func.instruction(&Instruction::Br(2));
    func.instruction(&Instruction::End);
    func.instruction(&lg(13));
    func.instruction(&cst(1));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(13));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End);
    // authority = url+8, auth_len
    func.instruction(&lg(6));
    func.instruction(&cst(1));
    func.instruction(&lg(0));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(14));
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);
    // path: dynamic slash → url+8+auth_len+1; else fallback "/"
    func.instruction(&lg(14));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(1));
    func.instruction(&Instruction::I32LtU);
    func.instruction(&Instruction::If(BlockType::Empty));
    func.instruction(&lg(6));
    func.instruction(&cst(1));
    func.instruction(&lg(0));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(14));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(1));
    func.instruction(&cst(8));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&lg(14));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);
    func.instruction(&Instruction::Else);
    func.instruction(&lg(6));
    func.instruction(&cst(1));
    func.instruction(&cst(fallback_path.0));
    func.instruction(&cst(fallback_path.1));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);
    func.instruction(&Instruction::End);

    // Body → handle at +4
    func.instruction(&lg(6));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT + 4));
    func.instruction(&ls(7)); // outgoing-body handle

    // Finish
    func.instruction(&lg(7));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // Responder block
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(6));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
    func.instruction(&cst(0));
    func.instruction(&ls(13));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(0));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT + 8));
    func.instruction(&ls(8)); // future handle

    func.instruction(&lg(8));
    func.instruction(&cl(FN_FUTURE_SUBSCRIBE));
    func.instruction(&ls(9)); // pollable

    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&lg(9));
    func.instruction(&st(0));
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT));
    func.instruction(&cl(FN_POLL));
    func.instruction(&lg(9));
    func.instruction(&cl(FN_DROP_POLLABLE));

    func.instruction(&lg(8));
    func.instruction(&cst(SCRATCH_RESPONSE_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_RESPONSE_RESULT + 24));
    func.instruction(&ls(10)); // incoming-response handle

    // Header capture (best-effort, len-0 name = skip)
    func.instruction(&lg(10));
    func.instruction(&cl(FN_INCOMING_RESPONSE_HEADERS));
    func.instruction(&ls(16));
    func.instruction(&lg(16));
    func.instruction(&cst(capture_name.0));
    func.instruction(&cst(capture_name.1));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_FIELDS_GET));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_WRITE_RESULT));
    func.instruction(&ls(18)); // entries ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_WRITE_RESULT + 4));
    func.instruction(&ls(17)); // count
    func.instruction(&cst(0));
    func.instruction(&ls(19)); // value len = 0 default
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(17));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(0));
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
    func.instruction(&ls(19)); // val_ptr
    func.instruction(&lg(18));
    func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
    func.instruction(&ls(17)); // val_len
    func.instruction(&lg(17));
    func.instruction(&cst(2048));
    func.instruction(&lg(17));
    func.instruction(&cst(2048));
    func.instruction(&Instruction::I32GtU);
    func.instruction(&Instruction::Select);
    func.instruction(&ls(17));
    func.instruction(&lg(2));
    func.instruction(&cst(65536 - 2048));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(19));
    func.instruction(&lg(17));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&lg(4));
    func.instruction(&Instruction::I32Const(4));
    func.instruction(&Instruction::I32Add);
    func.instruction(&lg(17));
    func.instruction(&st(0));
    func.instruction(&Instruction::End);
    func.instruction(&lg(16));
    func.instruction(&cl(FN_DROP_FIELDS));

    // Consume → body → stream
    func.instruction(&lg(10));
    func.instruction(&cst(SCRATCH_IBODY_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_IBODY_RESULT + 4));
    func.instruction(&ls(11));
    func.instruction(&lg(11));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
    func.instruction(&ls(12)); // input-stream

    // Read loop until EOF
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ls(13)); // total
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    func.instruction(&lg(12));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_BLOCKING_READ));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(14)); // chunk_len
    // Capacity guard (resp_buf = HTTP_MAX_RESPONSE): copy min(chunk,
    // remaining); if chunk > remaining, copy remainder and EOF-break.
    func.instruction(&lg(14));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&Instruction::I32LeU);
    func.instruction(&Instruction::If(BlockType::Empty));
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&lg(14));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&lg(13));
    func.instruction(&lg(14));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(13));
    func.instruction(&Instruction::Br(1)); // continue loop (If=0, Loop=1)
    func.instruction(&Instruction::End); // end if (fits)
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&cst(1_048_576));
    func.instruction(&ls(13)); // total = CAP
    func.instruction(&Instruction::Br(2)); // break outer (If=0, Loop=1, Block=2)
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End); // responder block

    // total at len_ptr
    func.instruction(&lg(4));
    func.instruction(&lg(13));
    func.instruction(&st(0));
    func.instruction(&Instruction::I32Const(0));
}

/// Dynamic-URL GET with a runtime request header (x402 settle leg).
/// Params: 0=url_ptr, 1=url_len, 2=buf_ptr, 3=buf_len, 4=len_ptr,
///         5=hdr_name_ptr, 6=hdr_name_len, 7=hdr_val_ptr, 8=hdr_val_len.
/// Result: i32 error disc (0 = ok). Body → (buf, len_ptr); the caller
/// builds the tagged string. Header name/value/one-entry list are copied
/// into the buf tail (buf+64K-1024/-512/-768) because caller scratch is
/// transient. Max body: 62KB (tail reserved).

/// Dynamic-URL GET with a runtime request header (x402 settle leg).
/// Params: 0=url_ptr, 1=url_len, 2=buf_ptr, 3=buf_len, 4=len_ptr,
///         5=hdr_name_ptr, 6=hdr_name_len, 7=hdr_val_ptr, 8=hdr_val_len.
/// Result: i32 error disc (0 = ok). Body → (buf, len_ptr); the caller
/// builds the tagged string. Header name/value/one-entry list are copied
/// into the buf tail (buf+64K-1024/-512/-768) because caller scratch is
/// transient. Max body: 62KB (tail reserved).

/// Shared response-side tail: body read loop into (buf local 2, len →
/// local 4) for the dynamic GET variants. req_handle_local = request
/// handle local; uses locals 2 (buf), 4 (len_ptr) unchanged.
fn emit_get_response_tail(func: &mut Function, req_local: u32) {
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let st = |off: i32| Instruction::I32Store(MemArg { offset: off as u64, align: 2, memory_index: 0 });
    let ld = |off: i32| Instruction::I32Load(MemArg { offset: off as u64, align: 2, memory_index: 0 });

    // outgoing body + finish
    func.instruction(&lg(req_local));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT + 4));
    func.instruction(&ls(11)); // outgoing-body handle (reused pre-response)
    func.instruction(&lg(11));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // responder block (error disc → graceful exit)
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(req_local));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(0));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT + 8));
    func.instruction(&ls(10)); // future handle

    func.instruction(&lg(10));
    func.instruction(&cl(FN_FUTURE_SUBSCRIBE));
    func.instruction(&ls(15)); // pollable
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&lg(15));
    func.instruction(&st(0));
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT));
    func.instruction(&cl(FN_POLL));
    func.instruction(&lg(15));
    func.instruction(&cl(FN_DROP_POLLABLE));

    func.instruction(&lg(10));
    func.instruction(&cst(SCRATCH_RESPONSE_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_RESPONSE_RESULT + 24));
    func.instruction(&ls(10)); // incoming-response

    // stream setup
    func.instruction(&lg(10));
    func.instruction(&cst(SCRATCH_IBODY_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_IBODY_RESULT + 4));
    func.instruction(&ls(11));
    func.instruction(&lg(11));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
    func.instruction(&ls(12));

    // read loop
    func.instruction(&cst(0));
    func.instruction(&ls(13));
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    func.instruction(&lg(12));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_BLOCKING_READ));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(14));
    // Clamp chunk to remaining capacity (resp_buf = HTTP_MAX_RESPONSE):
    // local 14 = min(chunk, CAP - total); drop the read (continue) if 0.
    func.instruction(&lg(14));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&Instruction::I32LeU);
    func.instruction(&Instruction::If(BlockType::Empty));
    // chunk <= remaining: copy it
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&lg(14));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&lg(13));
    func.instruction(&lg(14));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(13));
    func.instruction(&Instruction::Br(1)); // continue loop (If=0, Loop=1)
    func.instruction(&Instruction::End); // end if
    // chunk > remaining: copy only the remaining bytes, then EOF-break.
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(13));
    func.instruction(&Instruction::I32Sub); // remaining
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&cst(1_048_576));
    func.instruction(&ls(13)); // total = CAP
    func.instruction(&Instruction::Br(2)); // break outer (If=0, Loop=1, Block=2)
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End);
    func.instruction(&Instruction::End); // responder block
}

pub fn emit_http_post_to_buffer(func: &mut Function, data: &HttpDataSegments) {
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let st = |off: i32| {
        Instruction::I32Store(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };
    let ld = |off: i32| {
        Instruction::I32Load(MemArg {
            offset: off as u64,
            align: 2,
            memory_index: 0,
        })
    };

    // Params: 0=url_ptr, 1=url_len, 2=body_ptr, 3=body_len, 4=buf_ptr, 5=buf_len, 6=len_ptr
    // Extra locals: 7=fields, 8=req, 9=body, 10=future, 11=pollable,
    //               12=response, 13=resp_body, 14=in_stream,
    //               15=temp_i/path_len, 16=authority_len, 17=authority_ptr,
    //               18=path_ptr, 19=bytes_written

    // Fields → Request
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&ls(7)); // fields handle

    // Set headers (includes Content-Type for POST)
    for &(name_ptr, name_len, val_list_ptr, val_list_len) in &data.headers {
        func.instruction(&lg(7)); // fields handle (self)
        func.instruction(&cst(name_ptr));
        func.instruction(&cst(name_len));
        func.instruction(&cst(val_list_ptr));
        func.instruction(&cst(val_list_len));
        func.instruction(&cst(SCRATCH_WRITE_RESULT)); // return area
        func.instruction(&cl(FN_FIELDS_SET));
    }

    func.instruction(&lg(7));
    func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST));
    func.instruction(&ls(8)); // request handle

    // Set method=POST: try direct variant index (2=POST)
    func.instruction(&lg(8));
    func.instruction(&cst(2)); // POST variant
    func.instruction(&cst(0)); // padding
    func.instruction(&cst(0)); // padding
    func.instruction(&cl(FN_SET_METHOD));
    func.instruction(&Instruction::Drop);

    // Set scheme=HTTPS
    func.instruction(&lg(8));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(1)); // variant index 1 = HTTPS
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_SCHEME));
    func.instruction(&Instruction::Drop);

    // Set authority
    func.instruction(&lg(8));
    func.instruction(&cst(1));
    func.instruction(&cst(data.auth_offset));
    func.instruction(&cst(data.auth_len));
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // Set path-with-query
    func.instruction(&lg(8));
    func.instruction(&cst(1));
    func.instruction(&cst(data.path_offset));
    func.instruction(&cst(data.path_len));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);

    // Body → handle at +4
    func.instruction(&lg(8));
    func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_BODY_RESULT + 4));
    func.instruction(&ls(9)); // outgoing-body handle

    // Get output-stream from outgoing-body, write body, then drop stream
    // outgoing-body.write(body_handle, result_ptr) → output-stream at result_ptr+4
    func.instruction(&lg(9)); // outgoing-body handle
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_WRITE));
    // Load output-stream handle from result
    func.instruction(&cst(0)); // base addr for load
    func.instruction(&ld(SCRATCH_WRITE_RESULT + 4));
    func.instruction(&ls(19)); // save output-stream handle

    // blocking-write-and-flush(stream, body_ptr, body_len, result_ptr)
    func.instruction(&lg(19)); // output-stream handle
    func.instruction(&lg(2)); // body_ptr (param 2)
    func.instruction(&lg(3)); // body_len (param 3)
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTPUT_STREAM_WRITE));

    // Drop the output-stream
    func.instruction(&lg(19));
    func.instruction(&cl(FN_DROP_OUTPUT_STREAM));

    // Finish outgoing body (none trailers)
    func.instruction(&lg(9));
    func.instruction(&cst(0)); // none (disc=0)
    func.instruction(&cst(0)); // padding
    func.instruction(&cst(SCRATCH_WRITE_RESULT)); // valid result ptr
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // Handle → future handle at +8
    // responder block (see GET emit) — disc-error exits are graceful
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&lg(8));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
    // handle() result disc (≠0 = err) → reset total_len, exit responder
    func.instruction(&cst(0));
    func.instruction(&ls(15));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT));
    func.instruction(&cst(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(0));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_FUTURE_RESULT + 8));
    func.instruction(&ls(10)); // future handle

    // Subscribe → pollable
    func.instruction(&lg(10));
    func.instruction(&cl(FN_FUTURE_SUBSCRIBE));
    func.instruction(&ls(11)); // pollable handle

    // Poll
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&lg(11));
    func.instruction(&st(0));
    func.instruction(&cst(SCRATCH_POLLABLES));
    func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT));
    func.instruction(&cl(FN_POLL));
    func.instruction(&lg(11));
    func.instruction(&cl(FN_DROP_POLLABLE));

    // Future.get → response handle at +24
    func.instruction(&lg(10));
    func.instruction(&cst(SCRATCH_RESPONSE_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_RESPONSE_RESULT + 24));
    func.instruction(&ls(12)); // incoming-response handle

    // Consume → incoming-body handle at +4
    func.instruction(&lg(12));
    func.instruction(&cst(SCRATCH_IBODY_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_IBODY_RESULT + 4));
    func.instruction(&ls(13)); // incoming-body handle

    // Stream → input-stream handle at +4
    func.instruction(&lg(13));
    func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_STREAM_RESULT + 4));
    func.instruction(&ls(14)); // input-stream handle

    // Read loop - keep reading until EOF (len=0)
    // Local 15 = total accumulated length
    // Local 16 = chunk length from blocking-read
    // Local 17 = copy counter
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ls(15)); // total_len = 0

    func.instruction(&Instruction::Block(BlockType::Empty)); // outer block - break here on EOF/error
    func.instruction(&Instruction::Loop(BlockType::Empty)); // read loop

    // blocking-read(stream, 65536, dst)
    func.instruction(&lg(14)); // stream handle
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_BLOCKING_READ));

    // Check result disc: (disc != 0) means error → break outer
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT)); // disc at offset 0
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1)); // break outer on error

    // Check len (0=EOF) → break outer
    func.instruction(&Instruction::I32Const(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8)); // len at offset 8
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1)); // break outer on EOF

    // Store chunk_len
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(16)); // chunk_len

    // Store chunk data ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&ls(17)); // chunk_data_ptr

    // Capacity guard (buf = HTTP_MAX_RESPONSE): if chunk <= remaining
    // (CAP - total), copy it and loop; else copy only remaining and
    // break with total = CAP (graceful truncation, no OOB trap).
    // NOTE: chunk_len lives in local 16. 92e764eb wrote lg(19) here —
    // local 19 is the REQUEST output-stream HANDLE, so every chunk copied
    // exactly `handle` bytes (observed: 7-byte body '{"jsonr', handle=7).
    func.instruction(&lg(16));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(15));
    func.instruction(&Instruction::I32Sub);
    func.instruction(&Instruction::I32LeU);
    func.instruction(&Instruction::If(BlockType::Empty));
    // Bulk copy chunk to buf_ptr using memory.copy (dst, src, len)
    func.instruction(&lg(4)); // dst = buf_ptr base (param 4)
    func.instruction(&lg(15)); // + total_len offset
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0)); // load chunk data ptr
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&lg(16)); // len = chunk_len
    func.instruction(&Instruction::MemoryCopy {
        src_mem: 0,
        dst_mem: 0,
    });

    // total_len += chunk_len
    func.instruction(&lg(15));
    func.instruction(&lg(16));
    func.instruction(&Instruction::I32Add);
    func.instruction(&ls(15)); // update total_len

    func.instruction(&Instruction::Br(1)); // continue loop (If=0, Loop=1)
    func.instruction(&Instruction::End); // end if (fits)
    // chunk exceeds remaining: copy only remaining, total = CAP, break.
    func.instruction(&lg(4));
    func.instruction(&lg(15));
    func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0)); // load chunk data ptr
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&cst(1_048_576));
    func.instruction(&lg(15));
    func.instruction(&Instruction::I32Sub); // remaining
    func.instruction(&Instruction::MemoryCopy {
        src_mem: 0,
        dst_mem: 0,
    });
    func.instruction(&cst(1_048_576));
    func.instruction(&ls(15)); // total = CAP
    func.instruction(&Instruction::Br(2)); // break outer (If=0, Loop=1, Block=2)
    func.instruction(&Instruction::End); // loop
    func.instruction(&Instruction::End); // block
    func.instruction(&Instruction::End); // responder block

    func.instruction(&lg(6)); // len_ptr (param 6)
    func.instruction(&lg(15)); // total_len
    func.instruction(&st(0));
    func.instruction(&Instruction::I32Const(0));
}

pub fn emit_http_poll_read(func: &mut Function) {
    func.instruction(&Instruction::Nop);
    func.instruction(&Instruction::I32Const(0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_align4() {
        assert_eq!(align4(0), 0);
        assert_eq!(align4(1), 4);
        assert_eq!(align4(3), 4);
        assert_eq!(align4(4), 4);
        assert_eq!(align4(5), 8);
        assert_eq!(align4(18), 20);
    }

    #[test]
    fn test_build_segments_no_headers() {
        let data = build_url_data_segments(b"example.com", b"/api/test", &[]);
        assert_eq!(data.auth_len, 11);
        assert_eq!(data.path_len, 9);
        assert!(data.headers.is_empty());
        assert_eq!(data.segments.len(), 2); // authority + path only
    }

    #[test]
    fn test_build_segments_with_headers() {
        let data = build_url_data_segments(
            b"api.open-meteo.com",
            b"/v1/forecast",
            &[
                (b"User-Agent", b"lisp-rlm/0.1"),
                (b"Accept", b"application/json"),
            ],
        );
        assert_eq!(data.auth_len, 18);
        assert_eq!(data.path_len, 12);
        assert_eq!(data.headers.len(), 2);
        assert_eq!(data.segments.len(), 8); // auth + path + 2×(name + value + val_list)
    }

    #[test]
    fn test_segments_no_overlap() {
        let data = build_url_data_segments(
            b"host.example.com",
            b"/path?query=1",
            &[(b"X-Custom", b"value")],
        );
        // Verify no segment data overlaps by checking offsets are strictly increasing
        let mut sorted = data.segments.clone();
        sorted.sort_by_key(|(off, _)| *off);
        let mut end = 0u32;
        for (off, bytes) in &sorted {
            assert!(
                *off >= end,
                "segment at {} overlaps with previous ending at {}",
                off,
                end
            );
            end = off + bytes.len() as u32;
        }
    }

    #[test]
    fn test_header_value_list_encoding() {
        let data = build_url_data_segments(b"host", b"/", &[(b"K", b"V")]);
        // The value list entry should be 8 bytes: [val_ptr (i32 LE), val_len (i32 LE)]
        let val_list_seg = &data.segments[4]; // auth(0), path(1), name(2), value(3), val_list(4)
        assert_eq!(val_list_seg.1.len(), 8);
        // val_len should be 1 (for "V")
        let val_len = i32::from_le_bytes(val_list_seg.1[4..8].try_into().unwrap());
        assert_eq!(val_len, 1);
    }

    #[test]
    fn test_data_base_is_after_scratch() {
        assert!(DATA_BASE > SCRATCH + 256);
        // Make sure scratch areas don't overlap with data
        assert!(SCRATCH_READ_RESULT + 16 <= DATA_BASE as i32);
    }
}
