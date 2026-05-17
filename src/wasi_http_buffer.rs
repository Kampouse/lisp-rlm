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

pub fn emit_http_get_to_buffer(func: &mut Function) {
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

    // Write authority: "api.open-meteo.com" (18 chars)
    let auth_scratch = SCRATCH + 512;
    func.instruction(&cst(auth_scratch));
    func.instruction(&cst(0x2E697061)); // "api."
    func.instruction(&st(0));
    func.instruction(&cst(auth_scratch + 4));
    func.instruction(&cst(0x6E65706F)); // "open"
    func.instruction(&st(0));
    func.instruction(&cst(auth_scratch + 8));
    func.instruction(&cst(0x74656D2D)); // "-met"
    func.instruction(&st(0));
    func.instruction(&cst(auth_scratch + 12));
    func.instruction(&cst(0x632E6F65)); // "eo.c"
    func.instruction(&st(0));
    func.instruction(&cst(auth_scratch + 16));
    func.instruction(&cst(0x00006D6F)); // "om\0\0"
    func.instruction(&st(0));

    // Write path: "/v1/forecast?latitude=45.50&longitude=-73.57&current=temperature_2m" (67 chars)
    let path_scratch = SCRATCH + 640;
    let path_words: [u32; 17] = [
        0x2F31762F, 0x65726F66, 0x74736163, 0x74616C3F,
        0x64757469, 0x35343D65, 0x2630352E, 0x676E6F6C,
        0x64757469, 0x372D3D65, 0x37352E33, 0x72756326,
        0x746E6572, 0x6D65743D, 0x61726570, 0x65727574,
        0x006D325F,
    ];
    for (i, word) in path_words.iter().enumerate() {
        func.instruction(&cst(path_scratch + (i as i32) * 4));
        func.instruction(&cst(*word as i32));
        func.instruction(&st(0));
    }

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

    // Set authority
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(auth_scratch));
    func.instruction(&cst(18)); // length
    func.instruction(&cl(FN_SET_AUTHORITY));
    func.instruction(&Instruction::Drop);

    // Set path-with-query
    func.instruction(&lg(6));
    func.instruction(&cst(1)); // option some
    func.instruction(&cst(path_scratch));
    func.instruction(&cst(67)); // length
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
