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
    }
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
    let st = |off: i32| Instruction::I32Store(MemArg {
        offset: off as u64,
        align: 2,
        memory_index: 0,
    });
    let ld = |off: i32| Instruction::I32Load(MemArg {
        offset: off as u64,
        align: 2,
        memory_index: 0,
    });

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

    // Set authority (from data segment)
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(data.auth_offset));
    func.instruction(&cst(data.auth_len));
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // Set path-with-query (from data segment)
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(data.path_offset));
    func.instruction(&cst(data.path_len));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY));
    func.instruction(&Instruction::Drop);

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
    func.instruction(&lg(6));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
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

    // Read → ptr at +4, len at +8
    func.instruction(&lg(12));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_READ));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&ls(13)); // data ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(14)); // data len

    // Copy to resp_buf (byte-by-byte, no memory.copy — avoids bulk-memory requirement)
    {
        let copy_i = 15u32; // local 15 = copy counter (i32)
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(copy_i));
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));
        func.instruction(&lg(copy_i));
        func.instruction(&lg(14)); // data len
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));
        // resp_buf[i] = data_ptr[i]
        func.instruction(&lg(2)); // resp_buf base
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(13)); // data ptr
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(copy_i));
        func.instruction(&Instruction::Br(0));
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block
    }

    func.instruction(&lg(4));
    func.instruction(&lg(14));
    func.instruction(&st(0));
    func.instruction(&Instruction::I32Const(0));
}

pub fn emit_http_post_to_buffer(func: &mut Function, data: &HttpDataSegments) {
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let st = |off: i32| Instruction::I32Store(MemArg {
        offset: off as u64,
        align: 2,
        memory_index: 0,
    });
    let ld = |off: i32| Instruction::I32Load(MemArg {
        offset: off as u64,
        align: 2,
        memory_index: 0,
    });

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
    func.instruction(&lg(2));  // body_ptr (param 2)
    func.instruction(&lg(3));  // body_len (param 3)
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
    func.instruction(&lg(8));
    func.instruction(&cst(0));
    func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
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

    // Read → ptr at +4, len at +8
    func.instruction(&lg(14));
    func.instruction(&Instruction::I64Const(65536));
    func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_READ));
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&ls(15)); // data ptr
    func.instruction(&cst(0));
    func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&ls(16)); // data len

    // Copy response data to buf_ptr (byte-by-byte)
    {
        let copy_i = 17u32;
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(copy_i));
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));
        func.instruction(&lg(copy_i));
        func.instruction(&lg(16)); // data len
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));
        // Push dest addr first (stays on stack), then load src byte (top)
        // i32.store8 pops val(top), then addr — need [dest_addr, loaded_byte]
        func.instruction(&lg(4)); // resp_buf base (param 4)
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Add);
        func.instruction(&lg(15)); // data ptr
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&lg(copy_i));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(copy_i));
        func.instruction(&Instruction::Br(0));
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block
    }

    func.instruction(&lg(6)); // len_ptr (param 6)
    func.instruction(&lg(16)); // data len
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
        let data = build_url_data_segments(
            b"example.com",
            b"/api/test",
            &[],
        );
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
            assert!(*off >= end, "segment at {} overlaps with previous ending at {}", off, end);
            end = off + bytes.len() as u32;
        }
    }

    #[test]
    fn test_header_value_list_encoding() {
        let data = build_url_data_segments(
            b"host",
            b"/",
            &[(b"K", b"V")],
        );
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
