//! WASI HTTP GET to memory buffer
//! Canonical ABI: 0 = None, 1 = Some; 0 = Ok, 1 = Err

use wasm_encoder::*;
use crate::wasi_http::*;

pub const SCRATCH_URL_PARSE: i32 = SCRATCH + 128;

pub fn emit_http_get_to_buffer(func: &mut Function) {
    let st = |off: i32| Instruction::I32Store(MemArg { offset: off as u64, align: 2, memory_index: 0 });
    let cst = |v: i32| Instruction::I32Const(v);
    let lg = |i: u32| Instruction::LocalGet(i);
    let ls = |i: u32| Instruction::LocalSet(i);
    let cl = |i: u32| Instruction::Call(i);
    let ld = |off: i32| Instruction::I32Load(MemArg { offset: off as u64, align: 2, memory_index: 0 });

    // ═══ STEP 1: Parse URL ═══
    // Find ':' separator
    func.instruction(&cst(0)); func.instruction(&ls(13));
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    func.instruction(&lg(13)); func.instruction(&lg(1));
    func.instruction(&Instruction::I32GeU); func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(0)); func.instruction(&lg(13));
    func.instruction(&Instruction::I32Add);
    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&cst(0x3A)); func.instruction(&Instruction::I32Eq);
    func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(13)); func.instruction(&cst(1)); func.instruction(&Instruction::I32Add); func.instruction(&ls(13));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End); func.instruction(&Instruction::End);

    // authority_start = url_ptr + colon_pos + 3
    func.instruction(&lg(0)); func.instruction(&lg(13)); func.instruction(&cst(3));
    func.instruction(&Instruction::I32Add); func.instruction(&Instruction::I32Add); func.instruction(&ls(15));

    // Find authority end
    func.instruction(&cst(0)); func.instruction(&ls(14));
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    func.instruction(&lg(15)); func.instruction(&lg(14)); func.instruction(&Instruction::I32Add);
    func.instruction(&lg(0)); func.instruction(&lg(1)); func.instruction(&Instruction::I32Add);
    func.instruction(&Instruction::I32GeU); func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(15)); func.instruction(&lg(14)); func.instruction(&Instruction::I32Add);
    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&Instruction::LocalTee(13));
    func.instruction(&cst(47)); func.instruction(&Instruction::I32Eq); func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(13)); func.instruction(&cst(63)); func.instruction(&Instruction::I32Eq); func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(13)); func.instruction(&cst(35)); func.instruction(&Instruction::I32Eq); func.instruction(&Instruction::BrIf(1));
    func.instruction(&lg(14)); func.instruction(&cst(1)); func.instruction(&Instruction::I32Add); func.instruction(&ls(14));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End); func.instruction(&Instruction::End);

    // path_start = authority_start + authority_len
    func.instruction(&lg(15)); func.instruction(&lg(14)); func.instruction(&Instruction::I32Add); func.instruction(&ls(16));
    // path_len = url_end - path_start
    func.instruction(&lg(0)); func.instruction(&lg(1)); func.instruction(&Instruction::I32Add);
    func.instruction(&lg(16)); func.instruction(&Instruction::I32Sub); func.instruction(&ls(13));
    // Default path "/" if empty
    func.instruction(&lg(13)); func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::If(BlockType::Empty));
    func.instruction(&cst(47)); func.instruction(&cst(SCRATCH_URL_PARSE));
    func.instruction(&Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
    func.instruction(&cst(SCRATCH_URL_PARSE)); func.instruction(&ls(16));
    func.instruction(&cst(1)); func.instruction(&ls(13));
    func.instruction(&Instruction::End);

    // ═══ STEP 2: Build request ═══
    func.instruction(&cl(FN_CONSTRUCTOR_FIELDS)); func.instruction(&ls(5));
    func.instruction(&lg(5)); func.instruction(&cl(FN_CONSTRUCTOR_OUTGOING_REQUEST)); func.instruction(&ls(6));

    // set-method GET: (req, 0=GET, ptr, len)
    func.instruction(&lg(6)); func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_URL_PARSE)); func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_METHOD)); func.instruction(&Instruction::Drop);

    // set-scheme HTTPS: (req, 1=Some, 1=HTTPS, ptr, len)
    func.instruction(&lg(6)); func.instruction(&cst(1)); func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_URL_PARSE)); func.instruction(&cst(0));
    func.instruction(&cl(FN_SET_SCHEME)); func.instruction(&Instruction::Drop);

    // set-authority: (req, 1=Some, ptr, len)
    func.instruction(&lg(6)); func.instruction(&cst(1));
    func.instruction(&lg(15)); func.instruction(&lg(14));
    func.instruction(&cl(FN_SET_AUTHORITY)); func.instruction(&Instruction::Drop);

    // set-path-with-query: (req, 1=Some, ptr, len)
    func.instruction(&lg(6)); func.instruction(&cst(1));
    func.instruction(&lg(16)); func.instruction(&lg(13));
    func.instruction(&cl(FN_SET_PATH_WITH_QUERY)); func.instruction(&Instruction::Drop);

    // ═══ STEP 3: Body, finish, send ═══
    // body(req, dst): writes result<outgoing-body> to SCRATCH_BODY_RESULT
    func.instruction(&lg(6)); func.instruction(&cst(SCRATCH_BODY_RESULT));
    func.instruction(&cl(FN_OUTGOING_REQUEST_BODY));
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_BODY_RESULT + 4)); func.instruction(&ls(7));

    // finish(body, None): (body, 0, 0, dst)
    func.instruction(&lg(7)); func.instruction(&cst(0)); func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_WRITE_RESULT));
    func.instruction(&cl(FN_OUTGOING_BODY_FINISH));

    // handle(req, None): (req, 0, 0, dst)
    func.instruction(&lg(6)); func.instruction(&cst(0)); func.instruction(&cst(0));
    func.instruction(&cst(SCRATCH_FUTURE_RESULT));
    func.instruction(&cl(FN_HANDLE));
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_FUTURE_RESULT + 4)); func.instruction(&ls(8));

    // ═══ STEP 4: Poll for response ═══
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    // future.get(future, dst)
    func.instruction(&lg(8)); func.instruction(&cst(SCRATCH_GET_RESULT));
    func.instruction(&cl(FN_FUTURE_GET));
    // Check option disc == 1 (Some = ready)
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_GET_RESULT));
    func.instruction(&cst(1)); func.instruction(&Instruction::I32Eq);
    func.instruction(&Instruction::BrIf(1));
    // Subscribe + poll
    func.instruction(&lg(8)); func.instruction(&cl(FN_FUTURE_SUBSCRIBE)); func.instruction(&ls(9));
    func.instruction(&cst(SCRATCH_POLL_RESULT)); func.instruction(&cst(1));
    func.instruction(&cst(SCRATCH_POLL_RESULT + 8));
    func.instruction(&cl(FN_POLL));
    func.instruction(&lg(9)); func.instruction(&cl(FN_DROP_POLLABLE));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End); func.instruction(&Instruction::End);

    // response = option<result<incoming-response>>: +0=opt_disc, +4=res_disc, +8=handle
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_GET_RESULT + 8)); func.instruction(&ls(10));
    func.instruction(&lg(8)); func.instruction(&cl(FN_DROP_FUTURE_INCOMING_RESPONSE));

    // consume(response, dst): result<incoming-body>
    func.instruction(&lg(10)); func.instruction(&cst(SCRATCH_CONSUME_RESULT));
    func.instruction(&cl(FN_INCOMING_RESPONSE_CONSUME));
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_CONSUME_RESULT + 4)); func.instruction(&ls(11));
    func.instruction(&lg(10)); func.instruction(&cl(FN_DROP_INCOMING_RESPONSE));

    // stream(body, dst): result<input-stream>
    func.instruction(&lg(11)); func.instruction(&cst(SCRATCH_STREAM_RESULT));
    func.instruction(&cl(FN_INCOMING_BODY_STREAM));
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_STREAM_RESULT + 4)); func.instruction(&ls(12));
    func.instruction(&lg(11)); func.instruction(&cl(FN_DROP_INCOMING_BODY));

    // ═══ STEP 5: Read response into buffer ═══
    func.instruction(&cst(0)); func.instruction(&ls(17)); // write_pos = 0
    func.instruction(&Instruction::Block(BlockType::Empty));
    func.instruction(&Instruction::Loop(BlockType::Empty));
    // read(stream, len=32768, dst)
    func.instruction(&lg(12)); func.instruction(&Instruction::I64Const(32768)); func.instruction(&cst(SCRATCH_READ_RESULT));
    func.instruction(&cl(FN_INPUT_STREAM_READ));
    // Check result disc != 0 (error)
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_READ_RESULT));
    func.instruction(&cst(0)); func.instruction(&Instruction::I32Ne);
    func.instruction(&Instruction::BrIf(1));
    // Check len == 0 (EOF)
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::I32Eqz);
    func.instruction(&Instruction::BrIf(1));
    // Copy: memory.copy(buf_ptr + write_pos, read_ptr, read_len)
    func.instruction(&lg(2)); func.instruction(&lg(17)); func.instruction(&Instruction::I32Add);
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_READ_RESULT + 4));
    func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
    func.instruction(&lg(17)); func.instruction(&cst(0)); func.instruction(&ld(SCRATCH_READ_RESULT + 8));
    func.instruction(&Instruction::I32Add); func.instruction(&ls(17));
    func.instruction(&Instruction::Br(0));
    func.instruction(&Instruction::End); func.instruction(&Instruction::End);

    // Drop stream
    func.instruction(&lg(12)); func.instruction(&cl(FN_DROP_INPUT_STREAM));

    // Write response length and return success
    func.instruction(&lg(4)); func.instruction(&lg(17)); func.instruction(&st(0));
    func.instruction(&cst(0));
}
