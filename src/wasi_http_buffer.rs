use crate::wasi_http::*;
use wasm_encoder::{Function, Instruction, MemArg};

pub const SCRATCH: i32 = 131072;
pub const SCRATCH_BODY_RESULT: i32 = SCRATCH;
pub const SCRATCH_WRITE_RESULT: i32 = SCRATCH + 16;
pub const SCRATCH_FUTURE_RESULT: i32 = SCRATCH + 32;
pub const SCRATCH_POLLABLES: i32 = SCRATCH + 64;
pub const SCRATCH_POLL_RESULT: i32 = SCRATCH + 96;
pub const SCRATCH_RESPONSE_RESULT: i32 = SCRATCH + 128;
pub const SCRATCH_IBODY_RESULT: i32 = SCRATCH + 192;
pub const SCRATCH_STREAM_RESULT: i32 = SCRATCH + 208;
pub const SCRATCH_READ_RESULT: i32 = SCRATCH + 224;

/// Data segment offsets for the URL, written by the caller into the module's data section.
pub struct HttpDataSegments {
    pub segments: Vec<(u32, Vec<u8>)>,
    pub auth_offset: i32,
    pub auth_len: i32,
    pub path_offset: i32,
    pub path_len: i32,
}

/// Pre-compute data segments for the URL. Returns offsets the function will reference.
pub fn build_url_data_segments() -> HttpDataSegments {
    let base = 131584; // After all scratch areas (SCRATCH + 256 + small gap)

    // Authority: "api.open-meteo.com" (18 bytes)
    let authority = b"api.open-meteo.com";
    let auth_offset = base;

    // Path: "/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m" (67 bytes)
    let path = b"/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m";
    // Align to 4 bytes after authority
    let path_offset = auth_offset + ((authority.len() + 3) & !3) as i32;

    HttpDataSegments {
        segments: vec![
            (auth_offset as u32, authority.to_vec()),
            (path_offset as u32, path.to_vec()),
        ],
        auth_offset,
        auth_len: authority.len() as i32,
        path_offset,
        path_len: path.len() as i32,
    }
}

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

    // URL bytes are now in data segments — no runtime instructions needed!
    // Just reference auth_offset and path_offset directly.

    // Fields → Request
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS));
    func.instruction(&ls(5)); // fields handle
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

    // Copy to resp_buf
    func.instruction(&lg(2));
    func.instruction(&lg(13));
    func.instruction(&lg(14));
    func.instruction(&Instruction::MemoryCopy {
        src_mem: 0,
        dst_mem: 0,
    });

    func.instruction(&lg(4));
    func.instruction(&lg(14));
    func.instruction(&st(0));
    func.instruction(&Instruction::I32Const(0));
}

pub fn emit_http_poll_read(func: &mut Function) {
    func.instruction(&Instruction::Nop);
    func.instruction(&Instruction::I32Const(0));
}
