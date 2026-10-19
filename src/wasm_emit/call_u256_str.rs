//! String-based u256 builtins — the 256-bit decimal-string domain.
//!
//! Motivation (clmm TS-core probe, 2026-10-19): the u128 family caps at
//! u128::MAX even where the 256-bit INTERMEDIATE fits (`u128/muldiv`
//! already does 256-bit schoolbook + restoring division internally) — but
//! the fixed-point price grid needs 256-bit values HELD across steps:
//! the grid step g²·10001/10000 at g≈2^64 is ≈3.4032e38, over the u128
//! ceiling by design, and its isqrt is the answer. This module exposes:
//!
//!   u256/mul    (a b)         — u128×u128 → 256-bit decimal string
//!   u256/div    (n d)         — 256/256 → 256-bit quotient (d=0 traps)
//!   u256/shr    (n k)         — logical shift right, k untagged at call site
//!   u256/isqrt  (n)           — floor sqrt, 256 → 128-bit decimal string
//!   u256/lt|gt|eq             — tagged-bool comparisons
//!
//! Same discipline as call_u128_str: decimal strings in/out, strict parse,
//! trap (unreachable) on overflow/div-zero, helpers synthesized once per
//! module and tree-shaken. Limb representation: 4×i64 (64-bit limbs) for
//! storage/compare/shift, 8×u32 for schoolbook multiply and long division
//! (each u32·u32 partial product < 2^64 fits i64).

use super::*;

// Scratch cells (32 bytes each). 8608.. follows the u128 block (ends 8608),
// well below INPUT_BUF 16384 — same free-region argument as U128_*.
pub(crate) const U256_A: i64 = 8608;
pub(crate) const U256_B: i64 = 8640;
pub(crate) const U256_Q: i64 = 8672;

fn ma() -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}
fn ma8() -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset: 0,
        align: 3,
        memory_index: 0,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct U256Helpers {
    pub(crate) parse: u32,
    pub(crate) to_str: u32,
    pub(crate) mul: u32,
    pub(crate) div: u32,
    pub(crate) shr: u32,
    pub(crate) isqrt: u32,
}

impl WasmEmitter {
    pub(crate) fn ensure_u256_str_helpers(&mut self) -> U256Helpers {
        if let Some(h) = self.u256h {
            return h;
        }
        let parse = self.funcs.len() as u32;
        self.funcs.push(FuncDef {
            name: "__h_u256_parse".into(),
            param_count: 2,
            local_count: 17,
            instrs: Self::h_u256_parse(),
            local_entries: None,
            custom_type: None,
        });
        let to_str = self.funcs.len() as u32;
        let mem_limit = (self.memory_pages as i64) * 65536;
        self.funcs.push(FuncDef {
            name: "__h_u256_to_str".into(),
            param_count: 1,
            local_count: 16,
            instrs: Self::h_u256_to_str(mem_limit),
            local_entries: None,
            custom_type: None,
        });
        let mul = self.funcs.len() as u32;
        self.funcs.push(FuncDef {
            name: "__h_u256_mul".into(),
            param_count: 3,
            local_count: 26,
            instrs: Self::h_u256_mul(),
            local_entries: None,
            custom_type: None,
        });
        let div = self.funcs.len() as u32;
        self.funcs.push(FuncDef {
            name: "__h_u256_div".into(),
            param_count: 3,
            local_count: 49,
            instrs: Self::h_u256_div(),
            local_entries: None,
            custom_type: None,
        });
        let shr = self.funcs.len() as u32;
        self.funcs.push(FuncDef {
            name: "__h_u256_shr".into(),
            param_count: 3,
            local_count: 13,
            instrs: Self::h_u256_shr(),
            local_entries: None,
            custom_type: None,
        });
        let isqrt = self.funcs.len() as u32;
        self.funcs.push(FuncDef {
            name: "__h_u256_isqrt".into(),
            param_count: 2,
            local_count: 21,
            instrs: Self::h_u256_isqrt(),
            local_entries: None,
            custom_type: None,
        });
        let h = U256Helpers {
            parse,
            to_str,
            mul,
            div,
            shr,
            isqrt,
        };
        self.u256h = Some(h);
        h
    }

    // ── parse: tagged decimal string → 32 bytes (4×i64 limbs) at dst ──
    // Locals: 0=s 1=dst 2=payload 3=ptr 4=len 5=i 6=ch 7..14=limb u32s 15=carry
    fn h_u256_parse() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        // type check: (s & 7) == TAG_STR
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(7));
        e(&Instruction::I64And);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Ne);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        // payload / ptr / len
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(2));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        for l in 7..=15 {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(l));
        }
        // digit loop: (l7..l0) = (l7..l0)*10 + digit, 8×u32 limbs
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64GeU);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Load8U(ma()));
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64LtU);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(57));
        e(&Instruction::I64GtU);
        e(&Instruction::I32Or);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(15));
        for l in 7..=14 {
            e(&Instruction::LocalGet(l));
            e(&Instruction::I64Const(10));
            e(&Instruction::I64Mul);
            e(&Instruction::LocalGet(15));
            e(&Instruction::I64Add);
            e(&Instruction::LocalSet(16));
            e(&Instruction::LocalGet(16));
            e(&Instruction::I64Const(0xFFFF_FFFF));
            e(&Instruction::I64And);
            e(&Instruction::LocalSet(l));
            e(&Instruction::LocalGet(16));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64ShrU);
            e(&Instruction::LocalSet(15));
        }
        // carry past limb 7 → value ≥ 2^256
        e(&Instruction::LocalGet(15));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(5));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // store 4×i64: pairs of u32 limbs
        for c in 0..4usize {
            let l0 = 7 + c * 2;
            let l1 = l0 + 1;
            e(&Instruction::LocalGet(1));
            e(&Instruction::I64Const(8 * c as i64));
            e(&Instruction::I64Add);
            e(&Instruction::I32WrapI64);
            e(&Instruction::LocalGet(l1 as u32));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64Shl);
            e(&Instruction::LocalGet(l0 as u32));
            e(&Instruction::I64Or);
            e(&Instruction::I64Store(ma8()));
        }
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // ── to_str: 32 bytes (4×i64 limbs) at addr → tagged decimal string ──
    // Chunked by D = 10^18 (fits signed i64): per chunk one 256-step binary
    // division, then i64 digit formatting. Locals: 0=addr 1..4=v0..v3 5=dst
    // 6=pos 7=R 8=bitctr 9=qbit-tmp 10=newtop 11=chunkend 12=t 13=limbsel 14=q
    fn h_u256_to_str(mem_limit: i64) -> Vec<Instruction<'static>> {
        const D: i64 = 1_000_000_000_000_000_000;
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        for (c, l) in [(0i64, 1u32), (8, 2), (16, 3), (24, 4)] {
            e(&Instruction::LocalGet(0));
            if c != 0 {
                e(&Instruction::I64Const(c));
                e(&Instruction::I64Add);
            }
            e(&Instruction::I32WrapI64);
            e(&Instruction::I64Load(ma8()));
            e(&Instruction::LocalSet(l));
        }
        // dst = bump heap 80 bytes
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(5));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(80));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(10));
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Const(mem_limit));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::Else);
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(80));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(6)); // pos = dst + 80
        // zero fast-path: "0"
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::Else);
        // ── chunk loop ──
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        // one 256-step division by D → Q in (v0..v3), R in local 7
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(7)); // R
        e(&Instruction::I64Const(256));
        e(&Instruction::LocalSet(8)); // bitctr — decremented FIRST, bit 255..0
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(8));
        // R = R<<1 | bit(bitctr of v)
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Shl);
        // select limb: t = bitctr / 64 (0..3)
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(6));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(13)); // limbsel
        // t = 1 << (bitctr & 63) — the bit mask (local 9)
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64Shl);
        e(&Instruction::LocalSet(9));
        // nested select v0..v3 by limbsel
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(1));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(2));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(2));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(3));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(4));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(7));
        // quotient bit: cnd = R >=u D; if cnd: R -= D.
        // The in-place trick (V's consumed bit i is overwritten by Q's bit i)
        // REQUIRES clearing the consumed bit — V must become exactly Q, so:
        //   v[limbsel] = (v[limbsel] & ~(1<<sh)) | (cnd ? 1<<sh : 0)
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(D));
        e(&Instruction::I64GeU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(14)); // cnd
        e(&Instruction::LocalGet(14));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(D));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(7));
        e(&Instruction::End);
        // clear the consumed bit: v[limbsel] &= ~(1 << (bitctr & 63))
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(-1i64));
        e(&Instruction::I64Xor);
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(1));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(2));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(-1i64));
        e(&Instruction::I64Xor);
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(2));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(2));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(-1i64));
        e(&Instruction::I64Xor);
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(3));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(4));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(-1i64));
        e(&Instruction::I64Xor);
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(4));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        // set the quotient bit if cnd: v[limbsel] |= 1 << (bitctr & 63)
        e(&Instruction::LocalGet(14));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(1));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(2));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(2));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Const(2));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(3));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(4));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(4));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        // loop control: bitctr hit 0 after processing bit 0 → done
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::Br(0));
        e(&Instruction::End); // div loop
        e(&Instruction::End); // div block
        // emit 18 digits of R right-to-left
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalSet(11)); // chunkend = pos (before write)
        e(&Instruction::I64Const(18));
        e(&Instruction::LocalSet(8)); // ndig counter
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(10));
        e(&Instruction::I64RemU);
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(10));
        e(&Instruction::I64DivU);
        e(&Instruction::LocalSet(7));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(8));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // most-significant chunk (value now 0): trim leading zeros
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        // while pos < chunkend - 1 AND mem[pos] == '0': pos += 1
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(11));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::I64GeU);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Load8U(ma()));
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Ne);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(6));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::Br(0));
        e(&Instruction::End); // chunk loop
        e(&Instruction::End); // chunk block
        // tagged string: payload = pos | (len << 32), len = (dst+80) - pos
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(80));
        e(&Instruction::I64Add);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Sub);
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::End); // zero-path if/else result
        v
    }

    // ── mul: schoolbook 4×4 u32 → 8×u32 (256-bit) ──
    // Locals: 0=a 1=b 2=dst 3..6=a_l 7..10=b_l 11..18=p_l 19=t 20=tmp
    fn h_u256_mul() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        let a_l = [3usize, 4, 5, 6];
        let b_l = [7usize, 8, 9, 10];
        let p_l = [11usize, 12, 13, 14, 15, 16, 17, 18];
        let t = 19usize;
        let tmp = 20usize;
        // load limbs: t = *(a + 8c) → a_l[2c]=t&M, a_l[2c+1]=t>>32
        for (base, addr) in [(a_l, 0i64), (b_l, 1i64)] {
            for c in 0..2usize {
                e(&Instruction::LocalGet(0 + addr as u32));
                if c == 1 {
                    e(&Instruction::I64Const(8));
                    e(&Instruction::I64Add);
                }
                e(&Instruction::I32WrapI64);
                e(&Instruction::I64Load(ma8()));
                e(&Instruction::LocalSet(t as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::LocalSet(base[c * 2] as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalSet(base[c * 2 + 1] as u32));
            }
        }
        for l in &p_l {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(*l as u32));
        }
        // schoolbook — copied from __h_u128_muldiv
        for i in 0..4usize {
            for j in 0..4usize {
                let pos = i + j;
                e(&Instruction::LocalGet(a_l[i] as u32));
                e(&Instruction::LocalGet(b_l[j] as u32));
                e(&Instruction::I64Mul);
                e(&Instruction::LocalSet(t as u32));
                e(&Instruction::LocalGet(p_l[pos] as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::I64Add);
                e(&Instruction::LocalSet(tmp as u32));
                e(&Instruction::LocalGet(tmp as u32));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::LocalSet(p_l[pos] as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalGet(tmp as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::I64Add);
                e(&Instruction::LocalSet(t as u32));
                for k in (pos + 1)..8 {
                    e(&Instruction::LocalGet(p_l[k] as u32));
                    e(&Instruction::LocalGet(t as u32));
                    e(&Instruction::I64Add);
                    e(&Instruction::LocalSet(tmp as u32));
                    e(&Instruction::LocalGet(tmp as u32));
                    e(&Instruction::I64Const(0xFFFF_FFFF));
                    e(&Instruction::I64And);
                    e(&Instruction::LocalSet(p_l[k] as u32));
                    e(&Instruction::LocalGet(tmp as u32));
                    e(&Instruction::I64Const(32));
                    e(&Instruction::I64ShrU);
                    e(&Instruction::LocalSet(t as u32));
                }
            }
        }
        for c in 0..4usize {
            let l0 = p_l[c * 2];
            let l1 = p_l[c * 2 + 1];
            e(&Instruction::LocalGet(2));
            e(&Instruction::I64Const(8 * c as i64));
            e(&Instruction::I64Add);
            e(&Instruction::I32WrapI64);
            e(&Instruction::LocalGet(l1 as u32));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64Shl);
            e(&Instruction::LocalGet(l0 as u32));
            e(&Instruction::I64Or);
            e(&Instruction::I64Store(ma8()));
        }
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // ── div: general 256/256 restoring division ──
    // Locals: 0=n 1=d 2=dst 3..10=n_l 11..18=d_l 19..26=q_l 27..34=r_l
    //         35=r76 36=r54 37=r32 38=r10 39=d76 40=d54 41=d32 42=d10
    //         43=t 44=tmp 45=cnd 46=subw 47=bit
    fn h_u256_div() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        let n_l: Vec<usize> = (3..11).collect();
        let d_l: Vec<usize> = (11..19).collect();
        let q_l: Vec<usize> = (19..27).collect();
        let r_l: Vec<usize> = (27..35).collect();
        let (r76, r54, r32, r10) = (35usize, 36usize, 37usize, 38usize);
        let (d76, d54, d32, d10) = (39usize, 40usize, 41usize, 42usize);
        let (t, tmp, cnd, subw, bit) = (43usize, 44usize, 45usize, 46usize, 47usize);
        // load 8 u32 limbs from 4 i64 cells
        for (limbs, addr) in [(&n_l, 0i64), (&d_l, 1i64)] {
            for c in 0..4usize {
                e(&Instruction::LocalGet(addr as u32));
                if c != 0 {
                    e(&Instruction::I64Const(8 * c as i64));
                    e(&Instruction::I64Add);
                }
                e(&Instruction::I32WrapI64);
                e(&Instruction::I64Load(ma8()));
                e(&Instruction::LocalSet(t as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::LocalSet(limbs[c * 2] as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalSet(limbs[c * 2 + 1] as u32));
            }
        }
        // d == 0 → trap
        for l in &d_l {
            e(&Instruction::LocalGet(*l as u32));
        }
        for _ in 0..7 {
            e(&Instruction::I64Or);
        }
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        // divisor windows for the flat compare
        for (win, hi, lo) in [
            (d76, d_l[7], d_l[6]),
            (d54, d_l[5], d_l[4]),
            (d32, d_l[3], d_l[2]),
            (d10, d_l[1], d_l[0]),
        ] {
            e(&Instruction::LocalGet(hi as u32));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64Shl);
            e(&Instruction::LocalGet(lo as u32));
            e(&Instruction::I64Or);
            e(&Instruction::LocalSet(win as u32));
        }
        for l in q_l.iter().chain(r_l.iter()) {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(*l as u32));
        }
        e(&Instruction::I64Const(256));
        e(&Instruction::LocalSet(bit as u32));
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(bit as u32));
        // rcarry = old r_l[7] bit 31 (the 2^256 term of R')
        e(&Instruction::LocalGet(r_l[7] as u32));
        e(&Instruction::I64Const(31));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(subw as u32));
        // R = R<<1 | n-bit(bit)
        for k in (0..8).rev() {
            e(&Instruction::LocalGet(r_l[k] as u32));
            e(&Instruction::I64Const(1));
            e(&Instruction::I64Shl);
            e(&Instruction::I64Const(0xFFFF_FFFF));
            e(&Instruction::I64And);
            e(&Instruction::LocalSet(r_l[k] as u32));
            if k > 0 {
                e(&Instruction::LocalGet(r_l[k - 1] as u32));
                e(&Instruction::I64Const(31));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalGet(r_l[k] as u32));
                e(&Instruction::I64Or);
                e(&Instruction::LocalSet(r_l[k] as u32));
            }
        }
        // select n limb: t = n_l[bit >> 5] — 8-way nested If
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(5));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(tmp as u32));
        for k in 0..8usize {
            if k < 7 {
                e(&Instruction::LocalGet(tmp as u32));
                e(&Instruction::I64Const(k as i64));
                e(&Instruction::I64Eq);
                e(&Instruction::If(BlockType::Result(ValType::I64)));
                e(&Instruction::LocalGet(n_l[k] as u32));
                e(&Instruction::Else);
            } else {
                e(&Instruction::LocalGet(n_l[k] as u32));
                for _ in 0..7 {
                    e(&Instruction::End);
                }
            }
        }
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(31));
        e(&Instruction::I64And);
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(r_l[0] as u32));
        // flat compare R vs D via 4 assembled windows
        for (win, hi, lo) in [
            (r76, r_l[7], r_l[6]),
            (r54, r_l[5], r_l[4]),
            (r32, r_l[3], r_l[2]),
            (r10, r_l[1], r_l[0]),
        ] {
            e(&Instruction::LocalGet(hi as u32));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64Shl);
            e(&Instruction::LocalGet(lo as u32));
            e(&Instruction::I64Or);
            e(&Instruction::LocalSet(win as u32));
        }
        // cnd = rcarry | (r76>d76) | ((r76==d76) & ((r54>d54) | ((r54==d54) &
        //                  ((r32>d32) | ((r32==d32) & (r10>=u d10))))))
        e(&Instruction::LocalGet(r76 as u32));
        e(&Instruction::LocalGet(d76 as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r76 as u32));
        e(&Instruction::LocalGet(d76 as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r54 as u32));
        e(&Instruction::LocalGet(d54 as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r54 as u32));
        e(&Instruction::LocalGet(d54 as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r32 as u32));
        e(&Instruction::LocalGet(d32 as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r32 as u32));
        e(&Instruction::LocalGet(d32 as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r10 as u32));
        e(&Instruction::LocalGet(d10 as u32));
        e(&Instruction::I64GeU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::I64And); // (r32==d32) & (r10>=d10)
        e(&Instruction::I64Or); // | (r32>d32)
        e(&Instruction::I64And); // (r54==d54) & (...)
        e(&Instruction::I64Or); // | (r54>d54)
        e(&Instruction::I64And); // (r76==d76) & (...)
        e(&Instruction::I64Or); // | (r76>d76)
        e(&Instruction::LocalGet(subw as u32)); // rcarry
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(cnd as u32));
        // Q <<= 1
        for k in (0..8).rev() {
            e(&Instruction::LocalGet(q_l[k] as u32));
            e(&Instruction::I64Const(1));
            e(&Instruction::I64Shl);
            e(&Instruction::I64Const(0xFFFF_FFFF));
            e(&Instruction::I64And);
            e(&Instruction::LocalSet(q_l[k] as u32));
            if k > 0 {
                e(&Instruction::LocalGet(q_l[k - 1] as u32));
                e(&Instruction::I64Const(31));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalGet(q_l[k] as u32));
                e(&Instruction::I64Or);
                e(&Instruction::LocalSet(q_l[k] as u32));
            }
        }
        // if cnd: R -= D (8-limb borrow subtract); Q |= 1
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        for k in 0..8usize {
            e(&Instruction::LocalGet(r_l[k] as u32));
            e(&Instruction::LocalGet(d_l[k] as u32));
            e(&Instruction::I64Sub);
            if k > 0 {
                e(&Instruction::LocalGet(subw as u32));
                e(&Instruction::I64Sub);
            }
            e(&Instruction::LocalSet(tmp as u32));
            if k < 7 {
                e(&Instruction::LocalGet(tmp as u32));
                e(&Instruction::LocalGet(r_l[k] as u32));
                e(&Instruction::I64GtU);
                e(&Instruction::I64ExtendI32U);
                e(&Instruction::LocalSet(subw as u32));
            }
            e(&Instruction::LocalGet(tmp as u32));
            e(&Instruction::I64Const(0xFFFF_FFFF));
            e(&Instruction::I64And);
            e(&Instruction::LocalSet(r_l[k] as u32));
        }
        e(&Instruction::LocalGet(q_l[0] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(q_l[0] as u32));
        e(&Instruction::End);
        // loop control: bit hit 0 after processing bit 0 → done
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // store Q — full 256 bits, no overflow trap
        for c in 0..4usize {
            let l0 = q_l[c * 2];
            let l1 = q_l[c * 2 + 1];
            e(&Instruction::LocalGet(2));
            e(&Instruction::I64Const(8 * c as i64));
            e(&Instruction::I64Add);
            e(&Instruction::I32WrapI64);
            e(&Instruction::LocalGet(l1 as u32));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64Shl);
            e(&Instruction::LocalGet(l0 as u32));
            e(&Instruction::I64Or);
            e(&Instruction::I64Store(ma8()));
        }
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // ── shr: logical shift right by k (i64, untagged) ──
    // Locals: 0=n 1=k 2=dst 3..6=v0..v3 7..10=w0..w3 11=whole 12=bits
    fn h_u256_shr() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        for (c, l) in [(0i64, 3u32), (8, 4), (16, 5), (24, 6)] {
            e(&Instruction::LocalGet(0));
            if c != 0 {
                e(&Instruction::I64Const(c));
                e(&Instruction::I64Add);
            }
            e(&Instruction::I32WrapI64);
            e(&Instruction::I64Load(ma8()));
            e(&Instruction::LocalSet(l));
        }
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(6));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(11)); // whole = k / 64
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(12)); // bits = k & 63
        // whole-limb shifts: s = 1..=3
        for s in 1..=3usize {
            e(&Instruction::LocalGet(11));
            e(&Instruction::I64Const(s as i64));
            e(&Instruction::I64GeU);
            e(&Instruction::If(BlockType::Empty));
            for j in 0..4usize {
                if j + s < 4 {
                    e(&Instruction::LocalGet((3 + j + s) as u32));
                } else {
                    e(&Instruction::I64Const(0));
                }
                e(&Instruction::LocalSet((3 + j) as u32));
            }
            e(&Instruction::End);
        }
        // bit shift: w[j] = (v[j] >> bits) | (bits != 0 && j < 3 ? v[j+1] << (64-bits) : 0)
        for j in 0..4usize {
            e(&Instruction::LocalGet((3 + j) as u32));
            e(&Instruction::LocalGet(12));
            e(&Instruction::I64ShrU);
            e(&Instruction::LocalSet((7 + j) as u32));
            if j < 3 {
                // bring down v[j+1]'s high bits only when bits != 0
                // (<<64 would wrap to <<0 and smear the whole limb)
                e(&Instruction::LocalGet(12));
                e(&Instruction::I64Const(0));
                e(&Instruction::I64Ne);
                e(&Instruction::If(BlockType::Empty));
                e(&Instruction::LocalGet((7 + j) as u32));
                e(&Instruction::LocalGet((3 + j + 1) as u32));
                // amount = 64 − bits: [0, bits] Sub = 0−bits (no rev-sub),
                // then +64. (bits,0,Sub would give bits−0 = bits.)
                e(&Instruction::I64Const(0));
                e(&Instruction::LocalGet(12));
                e(&Instruction::I64Sub);
                // (64 - bits) mod 64: 64-bits for bits in 1..63
                e(&Instruction::I64Const(64));
                e(&Instruction::I64Add);
                e(&Instruction::I64Shl);
                e(&Instruction::I64Or);
                e(&Instruction::LocalSet((7 + j) as u32));
                e(&Instruction::End);
            }
        }
        for c in 0..4usize {
            e(&Instruction::LocalGet(2));
            e(&Instruction::I64Const(8 * c as i64));
            e(&Instruction::I64Add);
            e(&Instruction::I32WrapI64);
            e(&Instruction::LocalGet((7 + c) as u32));
            e(&Instruction::I64Store(ma8()));
        }
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // ── isqrt: restoring digit-by-digit floor sqrt, 256 → 128-bit ──
    // for bit i = 127..0: tmp = ((g<<1)|1) << i; rem ≥ tmp ⇒ rem -= tmp, g |= 1<<i
    // Locals: 0=n 1=dst 2..5=m0..m3 6=g0 7=g1 8..10=b0..b2 11..14=t0..t3
    //         15=whole 16=sh 17=i 18=cnd 19=subw 20=tmp
    fn h_u256_isqrt() -> Vec<Instruction<'static>> {
        // Hacker's Delight fig 11-1, 256-bit: for m = 4^127 .. 4^0:
        //   b = y | m; y >>= 1; if x >= b: x -= b; y |= m
        // y is a shifting accumulator (< 2^130 mid-run — 4×i64 is safe);
        // the final y is the 128-bit floor sqrt.
        // Locals: 0=n 1=dst 2..5=x 6..9=y 10..13=m 14..17=b 18=k 19=t 20=cnd
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        let x = [2usize, 3, 4, 5];
        let y = [6usize, 7, 8, 9];
        let m = [10usize, 11, 12, 13];
        let b = [14usize, 15, 16, 17];
        let (k, t, cnd) = (18usize, 19usize, 20usize);
        // x = 4×i64 load
        for c in 0..4 {
            e(&Instruction::LocalGet(0));
            if c != 0 {
                e(&Instruction::I64Const(8 * c as i64));
                e(&Instruction::I64Add);
            }
            e(&Instruction::I32WrapI64);
            e(&Instruction::I64Load(ma8()));
            e(&Instruction::LocalSet(x[c] as u32));
        }
        for l in &y {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(*l as u32));
        }
        // m must start at the highest power of 4 ≤ x (HD precondition).
        // H = position of x's highest set bit; m = 2^(H & ~1).
        // Scan limbs high→low: H = base + (63 − clz(top nonzero limb)).
        e(&Instruction::LocalGet(x[3] as u32));
        e(&Instruction::I64Const(0));
        e(&Instruction::I64Ne);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::I64Const(192));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(x[2] as u32));
        e(&Instruction::I64Const(0));
        e(&Instruction::I64Ne);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::I64Const(128));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(x[1] as u32));
        e(&Instruction::I64Const(0));
        e(&Instruction::I64Ne);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::I64Const(64));
        e(&Instruction::Else);
        e(&Instruction::I64Const(0));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::LocalSet(cnd as u32)); // cnd = limb base (192/128/64/0)
        // select the top nonzero limb (same nesting) → clz
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Const(192));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(x[3] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Const(128));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(x[2] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(x[1] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(x[0] as u32));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::I64Clz);
        // 63 − clz: the selected limb is nonzero so clz ≤ 63 ⇒ XOR works
        // (i64.clz returns i64 — no extend needed)
        e(&Instruction::I64Const(63));
        e(&Instruction::I64Xor);
        e(&Instruction::LocalSet(t as u32)); // t = bit offset in limb
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(t as u32)); // t = H (0..255)
        // m = 2^(H & ~1): zero 4 limbs, set bit (H&~1) in limb (H&~1)/64
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(-2i64));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(t as u32)); // t = pos (even)
        for j in 0..4 {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(m[j] as u32));
        }
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(6));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(cnd as u32)); // cnd = limb index 0..3
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(m[0] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(m[0] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(m[1] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(m[1] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I64Const(2));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(m[2] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(m[2] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(m[3] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64And);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(m[3] as u32));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        // b = y | m
        for j in 0..4 {
            e(&Instruction::LocalGet(y[j] as u32));
            e(&Instruction::LocalGet(m[j] as u32));
            e(&Instruction::I64Or);
            e(&Instruction::LocalSet(b[j] as u32));
        }
        // y >>= 1 (ASCENDING j: y[j+1] must be unshifted when read)
        for j in 0..4 {
            e(&Instruction::LocalGet(y[j] as u32));
            e(&Instruction::I64Const(1));
            e(&Instruction::I64ShrU);
            if j < 3 {
                e(&Instruction::LocalGet(y[j + 1] as u32));
                e(&Instruction::I64Const(63));
                e(&Instruction::I64Shl);
                e(&Instruction::I64Or);
            }
            e(&Instruction::LocalSet(y[j] as u32));
        }
        // cnd = x >=u b (flat 4×i64 compare, high pair first)
        e(&Instruction::LocalGet(x[3] as u32));
        e(&Instruction::LocalGet(b[3] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[3] as u32));
        e(&Instruction::LocalGet(b[3] as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[2] as u32));
        e(&Instruction::LocalGet(b[2] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[2] as u32));
        e(&Instruction::LocalGet(b[2] as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[1] as u32));
        e(&Instruction::LocalGet(b[1] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[1] as u32));
        e(&Instruction::LocalGet(b[1] as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(x[0] as u32));
        e(&Instruction::LocalGet(b[0] as u32));
        e(&Instruction::I64GeU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::I64And);
        e(&Instruction::I64Or);
        e(&Instruction::I64And);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(cnd as u32));
        // if cnd: x -= b (4-limb borrow); y |= m
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        for j in 0..4 {
            e(&Instruction::LocalGet(x[j] as u32));
            e(&Instruction::LocalGet(b[j] as u32));
            e(&Instruction::I64Sub);
            if j > 0 {
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Sub);
            }
            e(&Instruction::LocalSet(t as u32)); // t = limb difference
            if j < 3 {
                // borrow out of this limb: diff >u x[j] means it wrapped
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::LocalGet(x[j] as u32));
                e(&Instruction::I64GtU);
                e(&Instruction::I64ExtendI32U);
                e(&Instruction::LocalSet(cnd as u32));
            }
            e(&Instruction::LocalGet(t as u32));
            e(&Instruction::LocalSet(x[j] as u32));
            if j < 3 {
                // carry-in for the next limb
                e(&Instruction::LocalGet(cnd as u32));
                e(&Instruction::LocalSet(t as u32));
            }
        }
        for j in 0..4 {
            e(&Instruction::LocalGet(y[j] as u32));
            e(&Instruction::LocalGet(m[j] as u32));
            e(&Instruction::I64Or);
            e(&Instruction::LocalSet(y[j] as u32));
        }
        e(&Instruction::End);
        // m >>= 2 (ASCENDING j: m[j+1] must be unshifted when read)
        for j in 0..4 {
            e(&Instruction::LocalGet(m[j] as u32));
            e(&Instruction::I64Const(2));
            e(&Instruction::I64ShrU);
            if j < 3 {
                e(&Instruction::LocalGet(m[j + 1] as u32));
                e(&Instruction::I64Const(62));
                e(&Instruction::I64Shl);
                e(&Instruction::I64Or);
            }
            e(&Instruction::LocalSet(m[j] as u32));
        }
        // loop control: exit when m == 0 (all 4^k consumed)
        for l in &m {
            e(&Instruction::LocalGet(*l as u32));
        }
        for _ in 0..3 {
            e(&Instruction::I64Or);
        }
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // store root (2×i64, u128 layout — rendered by __h_u128_to_str)
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // ───────────────────────── call sites ─────────────────────────

    /// Dispatch for the `u256/*` string-builtins (see module doc).
    /// Falls through with "__not_handled__" like call_u128_str.
    pub(crate) fn call_u256_str(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "u256/mul" | "u256/div" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                let h = self.ensure_u256_str_helpers();
                // clobber rule (2026-10-19): save operand A into fresh i64
                // locals BEFORE evaluating B — a nested u256 op inside B
                // would otherwise overwrite the U256_A scratch cell.
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let mut saved: Vec<u32> = Vec::new();
                for w in 0..4 {
                    saved.push(self.local_idx(&format!("__u256a{}_{}", gen, w)));
                }
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(U256_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8()));
                    v.push(Instruction::LocalSet(*l));
                }
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64Const(U256_B));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(*l));
                    v.push(Instruction::I64Store(ma8()));
                }
                v.push(Instruction::I64Const(U256_A));
                v.push(Instruction::I64Const(U256_B));
                v.push(Instruction::I64Const(U256_Q));
                let hf = if op == "u256/mul" { h.mul } else { h.div };
                v.push(Self::call_user(hf));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(U256_Q));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u256/shr" => {
                if a.len() != 2 {
                    return Err("u256/shr: need 2 args".into());
                }
                let h = self.ensure_u256_str_helpers();
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let mut saved: Vec<u32> = Vec::new();
                for w in 0..4 {
                    saved.push(self.local_idx(&format!("__u256s{}_{}", gen, w)));
                }
                let kloc = self.local_idx(&format!("__u256k_{}", gen));
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(U256_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8()));
                    v.push(Instruction::LocalSet(*l));
                }
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64Const(TAG_BITS));
                v.push(Instruction::I64ShrU); // untag
                v.push(Instruction::LocalSet(kloc));
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(*l));
                    v.push(Instruction::I64Store(ma8()));
                }
                v.push(Instruction::I64Const(U256_A));
                v.push(Instruction::LocalGet(kloc));
                v.push(Instruction::I64Const(U256_Q));
                v.push(Self::call_user(h.shr));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(U256_Q));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u256/isqrt" => {
                if a.len() != 1 {
                    return Err("u256/isqrt: need 1 arg".into());
                }
                let h = self.ensure_u256_str_helpers();
                let hu = self.ensure_u128_str_helpers();
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(U256_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(U256_A));
                v.push(Instruction::I64Const(U256_Q));
                v.push(Self::call_user(h.isqrt));
                v.push(Instruction::Drop);
                // root ≤ 2^128 → the u128 renderer covers it
                v.push(Instruction::I64Const(U256_Q));
                v.push(Self::call_user(hu.to_str));
                Ok(v)
            }
            "u256/lt" | "u256/gt" | "u256/eq" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                let h = self.ensure_u256_str_helpers();
                // clobber rule: parse a0 into A, save to locals, then parse B
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let mut saved: Vec<u32> = Vec::new();
                for w in 0..4 {
                    saved.push(self.local_idx(&format!("__u256ca{}_{}", gen, w)));
                }
                let mut av = Vec::new();
                let mut bv = Vec::new();
                for w in 0..4 {
                    av.push(self.local_idx(&format!("__u256cw{}_{}", gen, w)));
                    bv.push(self.local_idx(&format!("__u256dw{}_{}", gen, w)));
                }
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(U256_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8()));
                    v.push(Instruction::LocalSet(*l));
                }
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64Const(U256_B));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                for (w, l) in saved.iter().enumerate() {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(*l));
                    v.push(Instruction::I64Store(ma8()));
                }
                // load 4×i64 of each operand into fresh locals
                for w in 0..4 {
                    v.push(Instruction::I64Const(U256_A + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8()));
                    v.push(Instruction::LocalSet(av[w]));
                    v.push(Instruction::I64Const(U256_B + 8 * w as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8()));
                    v.push(Instruction::LocalSet(bv[w]));
                }
                // flat compare high→low. gt/lt: the (gt | (eq & (gt | (eq &
                // (gt | (eq & gt))))) cascade over pairs (3,2),(1,0) — lt
                // swaps the operands. eq: the four pairwise equalities ANDed.
                let mut push_pair = |v: &mut Vec<Instruction<'static>>,
                                     x: u32,
                                     y: u32,
                                     icmp: Instruction<'static>| {
                    v.push(Instruction::LocalGet(x));
                    v.push(Instruction::LocalGet(y));
                    v.push(icmp);
                    v.push(Instruction::I64ExtendI32U);
                };
                if op == "u256/eq" {
                    for w in 0..4 {
                        push_pair(&mut v, av[w], bv[w], Instruction::I64Eq);
                        if w > 0 {
                            v.push(Instruction::I64And);
                        }
                    }
                } else {
                    let (first, second) = if op == "u256/gt" {
                        (av.clone(), bv.clone())
                    } else {
                        // lt = gt with operands swapped for EVERY pair
                        (bv.clone(), av.clone())
                    };
                    push_pair(&mut v, first[3], second[3], Instruction::I64GtU);
                    v.push(Instruction::LocalGet(first[3]));
                    v.push(Instruction::LocalGet(second[3]));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::I64ExtendI32U);
                    push_pair(&mut v, first[2], second[2], Instruction::I64GtU);
                    v.push(Instruction::LocalGet(first[2]));
                    v.push(Instruction::LocalGet(second[2]));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::I64ExtendI32U);
                    push_pair(&mut v, first[1], second[1], Instruction::I64GtU);
                    v.push(Instruction::LocalGet(first[1]));
                    v.push(Instruction::LocalGet(second[1]));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::I64ExtendI32U);
                    push_pair(&mut v, first[0], second[0], Instruction::I64GtU);
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64And);
                    v.push(Instruction::I64Or);
                }
                v.extend(self.emit_tag_bool());

                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
