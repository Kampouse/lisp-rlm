//! String-based u128 builtins for the wasm target — exact interpreter semantics
//! (commit 4b1403e): decimal strings in, decimal strings out, hard errors on
//! invalid parse / type misuse / overflow / division by zero.
//!
//! Lowering strategy: strict-parse the operand strings into (lo, hi) i64 limb
//! pairs at fixed scratch cells, run limb math in dedicated internal helper
//! functions (`__u128_*`, emitted once per module and reachable through the
//! normal USER_BASE call graph so tree-shaking keeps them), then render the
//! result limbs back to a decimal heap string.
//!
//! Error mapping (documented deviation): the interpreter raises typed error
//! messages ("u128/add: invalid u128 string 'x'", …); on wasm these hard errors
//! trap via `unreachable` (nonzero exit under wasmtime/near-mock). Message text
//! is not reproduced at runtime.

use super::*;

// Dedicated scratch cells (8480..8528 — free region between KEY_BUF end 8480
// and INPUT_BUF 16384; not part of PROTECTED_REGIONS so mem-set! stays legal).
pub(crate) const U128_A: i64 = 8480; // operand A / arithmetic destination
pub(crate) const U128_B: i64 = 8496; // operand B
pub(crate) const U128_R: i64 = 8512; // remainder (divmod)
pub(crate) const U128_C: i64 = 8528; // muldiv third operand (divisor)
pub(crate) const U128_Q: i64 = 8544; // muldiv quotient result
pub(crate) const U128_RM: i64 = 8560; // muldiv remainder result

/// Function-table indices of the synthesized helpers (positions in `funcs`).
#[derive(Clone, Copy)]
pub(crate) struct U128Helpers {
    pub(crate) parse: u32,
    pub(crate) to_str: u32,
    pub(crate) add: u32,
    pub(crate) sub: u32,
    pub(crate) mul: u32,
    pub(crate) divmod: u32,
    pub(crate) muldiv: u32,
    pub(crate) i64_to_str: u32,
    // Checked variants (try/catch, round 4): same math, but every error trap
    // returns TAGGED_FALSE instead of trapping. Call sites under an active
    // try guard on that sentinel and catch-jump.
    pub(crate) parse_ck: u32,
    pub(crate) add_ck: u32,
    pub(crate) sub_ck: u32,
    pub(crate) mul_ck: u32,
    pub(crate) divmod_ck: u32,
    pub(crate) muldiv_ck: u32,
}

fn ma() -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}
pub(crate) fn ma8() -> wasm_encoder::MemArg {
    wasm_encoder::MemArg {
        offset: 0,
        align: 3,
        memory_index: 0,
    }
}

impl WasmEmitter {
    // ─────────────────────────────────────────────────────────────────────────
    // Helper-function synthesis. Each helper is a plain i64→i64 wasm function
    // pushed into `funcs` (params first, then i64 temporaries). They never use
    // per-user-function emitter state (locals/next_local), so they can be
    // created mid-emit without clobbering the function being compiled.
    // ─────────────────────────────────────────────────────────────────────────

    pub(crate) fn ensure_u128_str_helpers(&mut self) -> U128Helpers {
        if let Some(h) = self.u128h {
            return h;
        }
        let mem_limit = (self.memory_pages as i64) * 65536;
        let parse = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_parse".into(),
            param_count: 2,
            local_count: 13,
            instrs: Self::h_parse(),
            local_entries: None,
            custom_type: None,
        });
        let to_str = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_to_str".into(),
            param_count: 1,
            local_count: 14,
            instrs: Self::h_to_str(mem_limit),
            local_entries: None,
            custom_type: None,
        });
        let add = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_add".into(),
            param_count: 2,
            local_count: 10,
            instrs: Self::h_add(),
            local_entries: None,
            custom_type: None,
        });
        let sub = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_sub".into(),
            param_count: 2,
            local_count: 10,
            instrs: Self::h_sub(),
            local_entries: None,
            custom_type: None,
        });
        let mul = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_mul".into(),
            param_count: 2,
            local_count: 16,
            instrs: Self::h_mul(),
            local_entries: None,
            custom_type: None,
        });
        let divmod = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_divmod".into(),
            param_count: 3,
            local_count: 14,
            instrs: Self::h_divmod(),
            local_entries: None,
            custom_type: None,
        });
        let muldiv = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_muldiv".into(),
            param_count: 5,
            local_count: 44,
            instrs: Self::h_muldiv(),
            local_entries: None,
            custom_type: None,
        });
        let i64_to_str = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_i64_to_str".into(),
            param_count: 1,
            local_count: 7,
            instrs: Self::h_i64_to_str(mem_limit),
            local_entries: None,
            custom_type: None,
        });
        // Checked variants: Unreachable → return TAGGED_FALSE(1). Legit
        // returns from these helpers are nil(4) or tagged strings (tag 5) —
        // never 1, so the sentinel is unambiguous.
        let parse_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_parse_ck".into(),
            param_count: 2,
            local_count: 13,
            instrs: Self::to_checked(Self::h_parse()),
            local_entries: None,
            custom_type: None,
        });
        let add_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_add_ck".into(),
            param_count: 2,
            local_count: 10,
            instrs: Self::to_checked(Self::h_add()),
            local_entries: None,
            custom_type: None,
        });
        let sub_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_sub_ck".into(),
            param_count: 2,
            local_count: 10,
            instrs: Self::to_checked(Self::h_sub()),
            local_entries: None,
            custom_type: None,
        });
        let mul_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_mul_ck".into(),
            param_count: 2,
            local_count: 16,
            instrs: Self::to_checked(Self::h_mul()),
            local_entries: None,
            custom_type: None,
        });
        let divmod_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_divmod_ck".into(),
            param_count: 3,
            local_count: 14,
            instrs: Self::to_checked(Self::h_divmod()),
            local_entries: None,
            custom_type: None,
        });
        let muldiv_ck = self.funcs.len();
        self.funcs.push(FuncDef {
            name: "__h_u128_muldiv_ck".into(),
            param_count: 5,
            local_count: 44,
            instrs: Self::h_muldiv_ck(),
            local_entries: None,
            custom_type: None,
        });
        let h = U128Helpers {
            parse: parse as u32,
            to_str: to_str as u32,
            add: add as u32,
            sub: sub as u32,
            mul: mul as u32,
            divmod: divmod as u32,
            muldiv: muldiv as u32,
            i64_to_str: i64_to_str as u32,
            parse_ck: parse_ck as u32,
            add_ck: add_ck as u32,
            sub_ck: sub_ck as u32,
            mul_ck: mul_ck as u32,
            divmod_ck: divmod_ck as u32,
            muldiv_ck: muldiv_ck as u32,
        };
        self.u128h = Some(h);
        h
    }

    /// Convert an error-trapping helper body into a checked body: every
    /// `unreachable` becomes `return TAGGED_FALSE`. Fall-through returns are
    /// unchanged (nil / tagged string).
    fn to_checked(v: Vec<Instruction<'static>>) -> Vec<Instruction<'static>> {
        let mut out: Vec<Instruction<'static>> = Vec::with_capacity(v.len() + 8);
        let mut pending_return = false;
        for instr in v {
            if pending_return {
                out.push(Instruction::Return);
                pending_return = false;
            }
            if matches!(instr, Instruction::Unreachable) {
                out.push(Instruction::I64Const(1)); // TAGGED_FALSE (0 << 3 | TAG_BOOL)
                pending_return = true;
            } else {
                out.push(instr);
            }
        }
        if pending_return {
            out.push(Instruction::Return);
        }
        out
    }

    pub(crate) fn call_user(idx: u32) -> Instruction<'static> {
        Instruction::Call(USER_BASE | idx)
    }

    // __u128_parse(v: tagged, dst: addr) -> nil — strict decimal parse.
    // Locals: 0=v 1=dst 2=payload 3=ptr 4=len 5=i 6=ch 7..10=l0..l3 11=t 12=carry
    fn h_parse() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        // Type check: (v & 7) == TAG_STR
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
        // empty string → error
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        for l in 7..=10 {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(l));
        }
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(5));
        // loop over digits
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64GeU);
        e(&Instruction::BrIf(1));
        // ch = load8(ptr + i)
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Load8U(ma()));
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(6));
        // ch < '0' || ch > '9' → error
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
        // carry = ch - '0'
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(12));
        // (l3,l2,l1,l0) = (l3,l2,l1,l0)*10 + carry, 32-bit limbs
        for l in 7..=10 {
            e(&Instruction::LocalGet(l));
            e(&Instruction::I64Const(10));
            e(&Instruction::I64Mul);
            e(&Instruction::LocalGet(12));
            e(&Instruction::I64Add);
            e(&Instruction::LocalSet(11));
            e(&Instruction::LocalGet(11));
            e(&Instruction::I64Const(0xFFFF_FFFF));
            e(&Instruction::I64And);
            e(&Instruction::LocalSet(l));
            e(&Instruction::LocalGet(11));
            e(&Instruction::I64Const(32));
            e(&Instruction::I64ShrU);
            e(&Instruction::LocalSet(12));
        }
        // carry != 0 → overflow past 128 bits → error
        e(&Instruction::LocalGet(12));
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
        // store [dst] = l0 | (l1 << 32), [dst+8] = l2 | (l3 << 32)
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(9));
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // __u128_to_str(addr) -> tagged string (decimal render of limbs at addr)
    // Locals: 0=addr 1=lo 2=hi 3=dst(heap) 4=pos 5=qlo 6=qhi 7=rem 8=bit 9=t 10=len 11=tmp
    /// __u128_to_str(addr) -> tagged str — CHUNKED conversion (2026-09-15).
    ///
    /// The old implementation divided by 10 via 128-step binary long
    /// division PER DECIMAL DIGIT — up to 39 x 128 x ~15 ≈ 75k instructions
    /// (~75 Ggas, ~95% of every u128 op's cost; measured acc(100) at 8.04
    /// Tgas ≈ 80 Ggas/iteration). This one divides by D = 10^18 per chunk
    /// (10^18 fits SIGNED i64 — 10^19 overflows i64::MAX and breaks i64
    /// div/rem on the chunk): at most 3 chunks for a 39-digit u128, one
    /// 128-step division each, then cheap i64 digit formatting (~19 i64
    /// divmods per chunk). ~700 instructions total — ~100x cheaper.
    ///
    /// Digit emission: least-significant chunk first, written right-to-left
    /// into a 48-byte runtime-heap buffer (same layout as before). A chunk
    /// is ZERO-PADDED to 18 digits iff the quotient after its division is
    /// nonzero (interior chunks); the most-significant chunk prints bare.
    ///
    /// Locals: 0=addr(param) 1=lo 2=hi 3=dst 4=pos 5=qlo 6=qhi 7=rem
    /// 8=bitctr 9=chunk 10=scratch 11=new-heap-top 12=ndig 13=scratch2
    fn h_to_str(mem_limit: i64) -> Vec<Instruction<'static>> {
        const D: i64 = 1_000_000_000_000_000_000; // 10^18
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        // lo = *(addr), hi = *(addr+8)
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(1));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(2));
        // dst = bump heap 48 bytes (same as before)
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(11));
        e(&Instruction::LocalGet(11));
        e(&Instruction::I64Const(mem_limit));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(11));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::Else);
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        // pos = dst + 48 (write cursor, moves down)
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(4));
        // zero fast-path: "0"
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::I32Store8(ma()));
        // tagged string from POS (where '0' was written) — dst[0] is
        // uninitialized heap. The dst-based pointer returned a 1-byte NUL
        // string instead of "0" (AMM wallet zeros → downstream u128 parse
        // traps; found 2026-09-15 bisecting the published 74900a4)
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::Else);
        // ── outer loop: while (lo|hi) != 0 ──
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        // if (lo|hi) == 0 → exit outer
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        // ── ONE 128-step division of (lo,hi) by D → (qlo,qhi,rem) ──
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(5)); // qlo
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(6)); // qhi
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(7)); // rem
        e(&Instruction::I64Const(128));
        e(&Instruction::LocalSet(8)); // bitctr
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        // if bitctr == 0 → division done
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(8));
        // rem = rem<<1 | dividend bit(bitctr)
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Shl);
        // dividend still in (1,2): bit from lo if bitctr<64 else hi
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(1));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::Else);
        e(&Instruction::LocalGet(2));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64Sub);
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::End);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(7));
        // if rem >=u D: rem -= D; set quotient bit
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(D));
        e(&Instruction::I64GeU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(D));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(7));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(5));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64Sub);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(6));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // chunk = rem; (lo,hi) = (qlo,qhi); scratch10 = pad? (q != 0)
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalSet(9)); // chunk
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(10)); // 10 = "quotient nonzero" (pad flag)
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalSet(1));
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalSet(2));
        // ── write chunk digits right-to-left: do { pos--; *pos='0'+chunk%10; chunk/=10; ndig++ } while chunk != 0 ──
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(12)); // ndig
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(10));
        // chunk is always >= 0 and < 10^18 < i64::MAX — RemS/RemU identical
        e(&Instruction::I64RemU);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::I32Add);
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(10));
        e(&Instruction::I64DivS); // chunk/10 — chunk >= 0, signed div correct
        e(&Instruction::LocalSet(9));
        e(&Instruction::LocalGet(12));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(12));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(0));
        e(&Instruction::I64Ne);
        e(&Instruction::BrIf(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // ── pad to 18 when interior (pad flag local 10 != 0): while ndig < 18 { pos--; *pos='0'; ndig++ } ──
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1)); // no pad → exit
        e(&Instruction::LocalGet(12));
        e(&Instruction::I64Const(18));
        e(&Instruction::I64GeS);
        e(&Instruction::BrIf(1)); // padded enough → exit
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::LocalGet(12));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(12));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::Br(0)); // loop outer
        e(&Instruction::End);
        e(&Instruction::End);
        // ── finalize: len = dst+48-pos; tagged str ──
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(48));
        e(&Instruction::I64Add);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(10));
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::End);
        v
    }

    // __u128_add(dst, src) — dst += src, traps on carry out of 128 bits.
    // Locals: 0=dst 1=src 2=alo 3=ahi 4=blo 5=bhi 6=rlo 7=t 8=c 9=c1
    fn h_add() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(2));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(5));
        // rlo = alo + blo; c = rlo <u blo  (original low carry — must add this)
        e(&Instruction::LocalGet(2));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64LtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(8));
        // t = ahi + bhi; c1 = t <u bhi (high-add carry)
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(7));
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64LtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(9));
        // the +carry wraps iff t was u64::MAX and c set
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(-1));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::I64And);
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(9));
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(7));
        // overflow → trap
        e(&Instruction::LocalGet(9));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // __u128_sub(dst, src) — dst -= src, traps on borrow out of 128 bits.
    // Locals: 0=dst 1=src 2=alo 3=ahi 4=blo 5=bhi 6=rlo 7=t 8=b 9=b2
    fn h_sub() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(2));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(5));
        // rlo = alo - blo; b = rlo >u alo  (original low borrow — must subtract this)
        e(&Instruction::LocalGet(2));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(8));
        // t = ahi - bhi; ov = t >u ahi (high borrow)
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(7));
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(9));
        // t -= b wraps iff t == 0 and b set
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Eqz);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::I64And);
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(9));
        e(&Instruction::LocalGet(7));
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(7));
        // underflow → trap
        e(&Instruction::LocalGet(9));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // __u128_mul(dst, src) — full 128×128→128 schoolbook on 32-bit limbs,
    // traps on overflow. Locals: 0=dst 1=src 2..5=a0..a3 6..9=b0..b3 10..13=r0..r3 14=t 15=u
    fn h_mul() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        // load a limbs
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(14));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(2));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(14));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(5));
        // load b limbs
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(14));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(7));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(14));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(8));
        e(&Instruction::LocalGet(14));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(9));
        for r in 10..=13 {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(r));
        }
        let a: [u32; 4] = [2, 3, 4, 5];
        let b: [u32; 4] = [6, 7, 8, 9];
        let r: [u32; 4] = [10, 11, 12, 13];
        for i in 0..4usize {
            for j in 0..4usize {
                let pos = i + j;
                // t = a_i * b_j (both < 2^32 → product fits i64)
                e(&Instruction::LocalGet(a[i]));
                e(&Instruction::LocalGet(b[j]));
                e(&Instruction::I64Mul);
                e(&Instruction::LocalSet(14));
                if pos > 3 {
                    // any nonzero product above bit 127 → overflow → trap
                    e(&Instruction::LocalGet(14));
                    e(&Instruction::I32WrapI64);
                    e(&Instruction::If(BlockType::Empty));
                    e(&Instruction::Unreachable);
                    e(&Instruction::End);
                    continue;
                }
                // u = r_pos + (t & 0xFFFFFFFF); r_pos = u & 0xFFFFFFFF
                e(&Instruction::LocalGet(r[pos]));
                e(&Instruction::LocalGet(14));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::I64Add);
                e(&Instruction::LocalSet(15));
                e(&Instruction::LocalGet(15));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::LocalSet(r[pos]));
                // t_high = (t >> 32) + (u >> 32)  — add into limbs pos+1..
                e(&Instruction::LocalGet(14));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalGet(15));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::I64Add);
                e(&Instruction::LocalSet(14));
                // propagate t_high through r_{pos+1}..r3 (each r_k < 2^32; adding t_high (<2^32) can carry at most one extra limb)
                let mut k = pos + 1;
                loop {
                    if k > 3 {
                        // carry/remainder must be zero after r3
                        e(&Instruction::LocalGet(14));
                        e(&Instruction::I32WrapI64);
                        e(&Instruction::If(BlockType::Empty));
                        e(&Instruction::Unreachable);
                        e(&Instruction::End);
                        break;
                    }
                    // u = r_k + t_high; r_k = u & 0xFFFFFFFF; t_high = u >> 32
                    e(&Instruction::LocalGet(r[k]));
                    e(&Instruction::LocalGet(14));
                    e(&Instruction::I64Add);
                    e(&Instruction::LocalSet(15));
                    e(&Instruction::LocalGet(15));
                    e(&Instruction::I64Const(0xFFFF_FFFF));
                    e(&Instruction::I64And);
                    e(&Instruction::LocalSet(r[k]));
                    e(&Instruction::LocalGet(15));
                    e(&Instruction::I64Const(32));
                    e(&Instruction::I64ShrU);
                    e(&Instruction::LocalSet(14));
                    k += 1;
                }
            }
        }
        // store: [dst] = r0 | (r1 << 32), [dst+8] = r2 | (r3 << 32)
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(r[0]));
        e(&Instruction::LocalGet(r[1]));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(r[2]));
        e(&Instruction::LocalGet(r[3]));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // __u128_divmod(dst, src, rem) — dst = dst / src, *rem = dst % src.
    // Traps when divisor is zero. Restoring 128-bit binary long division.
    // Locals: 0=dst 1=src 2=rem 3=dvlo 4=dvhi 5=rlo 6=rhi 7=qlo 8=qhi 9=bit 10=ov 11=t 12=cond 13=tmp
    fn h_divmod() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(1));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(4));
        // divisor == 0 → trap
        e(&Instruction::LocalGet(3));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(5));
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(6));
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(7));
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalSet(8));
        e(&Instruction::I64Const(128));
        e(&Instruction::LocalSet(9));
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(9));
        // ov = rhi >> 63; rhi = (rhi << 1) | (rlo >> 63); rlo = (rlo << 1) | dividend bit
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(10));
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(63));
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(6));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalSet(11));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::Else);
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64Sub);
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::End);
        e(&Instruction::LocalGet(11));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(5));
        // cond = ov | (rhi >u dvhi) | ((rhi == dvhi) & (rlo >=u dvlo))
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64GeU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::I64And);
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(10));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(12));
        e(&Instruction::LocalGet(12));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        // rlo -= dvlo (borrow detection via wraparound)
        e(&Instruction::LocalGet(5));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(11));
        e(&Instruction::LocalGet(11));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(13));
        e(&Instruction::LocalGet(11));
        e(&Instruction::LocalSet(5));
        // rhi = rhi - dvhi - borrow
        e(&Instruction::LocalGet(6));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalGet(13));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(6));
        // set quotient bit
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(7));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Const(1));
        e(&Instruction::LocalGet(9));
        e(&Instruction::I64Const(64));
        e(&Instruction::I64Sub);
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(8));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // store quotient into dst, remainder into rem
        e(&Instruction::LocalGet(0));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(7));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(8));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // __h_u128_muldiv(a, b, d, q, rem_ptr) — CLMM core intrinsic:
    // q = a * b / d over the FULL 256-bit product (restoring binary long
    // division), *rem_ptr(8B cell) = a * b % d (skipped when rem_ptr == 0).
    // Traps: d == 0, or quotient ≥ 2^128 — the same contract as Raydium's
    // FullMath::mulDiv (callers pre-size liquidity so the quotient fits).
    // Locals: 0=a 1=b 2=d 3=q 4=rem_ptr | A=5..8 B=9..12 D=13..16 (u32×4)
    //         P=17..24 Q=25..32 R=33..36 | dvlo=37 dvhi=38 t=39 bit=40
    //         cnd=41 tmp=42 subw=43
    fn h_muldiv() -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        let a_l = [5usize, 6, 7, 8];
        let b_l = [9usize, 10, 11, 12];
        let d_l = [13usize, 14, 15, 16];
        let p_l = [17usize, 18, 19, 20, 21, 22, 23, 24];
        let q_l = [25usize, 26, 27, 28, 29, 30, 31, 32];
        let r_l = [33usize, 34, 35, 36];
        let dvlo = 37usize;
        let dvhi = 38usize;
        let t = 39usize;
        let bit = 40usize;
        let cnd = 41usize;
        let tmp = 42usize;
        let subw = 43usize;
        // load a 16-byte cell → 4 u32 limbs (low→high)
        for (ptr_local, base) in [(0usize, a_l[0]), (1usize, b_l[0]), (2usize, d_l[0])] {
            for half in 0..2usize {
                e(&Instruction::LocalGet(ptr_local as u32));
                if half == 1 {
                    e(&Instruction::I64Const(8));
                    e(&Instruction::I64Add);
                }
                e(&Instruction::I32WrapI64);
                e(&Instruction::I64Load(ma8()));
                e(&Instruction::LocalSet(t as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(0xFFFF_FFFF));
                e(&Instruction::I64And);
                e(&Instruction::LocalSet((base + half * 2) as u32));
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalSet((base + half * 2 + 1) as u32));
            }
        }
        // d == 0 → trap
        e(&Instruction::LocalGet(d_l[0] as u32));
        e(&Instruction::LocalGet(d_l[1] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(d_l[2] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(d_l[3] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        // precompute dvlo/dvhi (i64-assembled halves) for the loop compare
        e(&Instruction::LocalGet(d_l[1] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(d_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(dvlo as u32));
        e(&Instruction::LocalGet(d_l[3] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(d_l[2] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(dvhi as u32));
        // P = 0, Q = 0, R = 0
        for l in p_l.iter().chain(q_l.iter()).chain(r_l.iter()) {
            e(&Instruction::I64Const(0));
            e(&Instruction::LocalSet(*l as u32));
        }
        // ── schoolbook 4×4 → 8 ──
        // P += A_i * B_j << (32·(i+j)); carry propagates unconditionally to
        // the top limb — the full product always fits 8 u32 limbs, so the
        // limb-7 carry-out is 0 by math.
        for i in 0..4usize {
            for j in 0..4usize {
                let pos = i + j;
                // t = A_i * B_j (u32·u32 < 2^64 — fits i64)
                e(&Instruction::LocalGet(a_l[i] as u32));
                e(&Instruction::LocalGet(b_l[j] as u32));
                e(&Instruction::I64Mul);
                e(&Instruction::LocalSet(t as u32));
                // u = P_pos + (t & M); P_pos = u & M
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
                // t = (t >> 32) + (u >> 32) — carry
                e(&Instruction::LocalGet(t as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::LocalGet(tmp as u32));
                e(&Instruction::I64Const(32));
                e(&Instruction::I64ShrU);
                e(&Instruction::I64Add);
                e(&Instruction::LocalSet(t as u32));
                // propagate carry pos+1 ..= 7
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
        // ── 256-bit restoring binary long division ──
        // for bit 255..=0: R = (R << 1) | P[bit]; R>=D ⇒ R-=D, Q[bit]=1.
        // R < D ≤ 2^128-1 as the invariant ⇒ R's two high u32 limbs stay
        // 0 (they only matter for the shift-in of P bits ≥ 128, where the
        // invariant R' < D still caps R at 2^128-1). Q's limbs 4..7 collect
        // the quotient's "high" bits — any nonzero ⇒ quotient ≥ 2^128 ⇒
        // overflow trap after the loop.
        e(&Instruction::I64Const(255));
        e(&Instruction::LocalSet(bit as u32));
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        // 129-bit shift: rcarry = r3's old bit 31 (the bit that falls off
        // the 128-bit R when shifted — algebraically it adds 2^128 to
        // R', which we fold into the compare below via cnd; the wraparound
        // subtract stays correct because R' - D < 2^128 always).
        e(&Instruction::LocalGet(r_l[3] as u32));
        e(&Instruction::I64Const(31));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(subw as u32)); // subw reused as rcarry outside the subtract
        for k in (0..4).rev() {
            e(&Instruction::LocalGet(r_l[k] as u32));
            e(&Instruction::I64Const(1));
            e(&Instruction::I64Shl);
            // mask: limbs hold exactly 32 bits — an unmasked shl leaves old
            // bit 31 at bit 32 where it spills upward and double-counts the
            // bit carried via the >>31 or-in below (corrupts rlo/rhi compares)
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
        // select P limb: t = P[bit >> 5] (4-way select on bit>>5 0..3)
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(5));
        e(&Instruction::I64ShrU);
        e(&Instruction::LocalSet(tmp as u32)); // tmp = bit >> 5 (0..7 — P halves)
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[0] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[1] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(2));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[2] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(3));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[3] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(4));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[4] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(5));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[5] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(6));
        e(&Instruction::I64Eq);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(p_l[6] as u32));
        e(&Instruction::Else);
        e(&Instruction::LocalGet(p_l[7] as u32));
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        e(&Instruction::End);
        // t now = P's u32 limb for this bit; shift = bit & 31
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(31));
        e(&Instruction::I64And);
        e(&Instruction::I64ShrU);
        e(&Instruction::I64Const(1));
        e(&Instruction::I64And);
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(r_l[0] as u32));
        // compare R vs D: cnd = (rhi >u dvhi) | ((rhi == dvhi) & (rlo >=u dvlo))
        e(&Instruction::LocalGet(r_l[3] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(r_l[2] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(tmp as u32)); // rhi
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::LocalGet(dvhi as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::LocalGet(dvhi as u32));
        e(&Instruction::I64Eq);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalGet(r_l[1] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(t as u32)); // rlo
        e(&Instruction::LocalGet(t as u32));
        e(&Instruction::LocalGet(dvlo as u32));
        e(&Instruction::I64GeU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::I64And);
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(subw as u32)); // rcarry: R' ≥ 2^128 ⇒ R' > D
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(cnd as u32));
        // Q <<= 1 (8 limbs, shift-in from below = 0 — Q's top 128 bits carry
        // the overflow evidence)
        for k in (0..8).rev() {
            e(&Instruction::LocalGet(q_l[k] as u32));
            e(&Instruction::I64Const(1));
            e(&Instruction::I64Shl);
            // mask: same 32-bit limb invariant as the R shift above —
            // unmasked shl spills old bit 31 upward and double-counts
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
        // if cnd: R -= D; Q |= 1 — cnd is an i64 (0/1); If needs i32
        e(&Instruction::LocalGet(cnd as u32));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        // 4-limb subtract with borrow: per limb u = R_k - D_k (- borrow)
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::LocalGet(d_l[0] as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(tmp as u32));
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(subw as u32)); // borrow out of limb 0 (wraparound)
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(r_l[0] as u32));
        e(&Instruction::LocalGet(r_l[1] as u32));
        e(&Instruction::LocalGet(d_l[1] as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalGet(subw as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(tmp as u32));
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::LocalGet(r_l[1] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(subw as u32));
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(r_l[1] as u32));
        e(&Instruction::LocalGet(r_l[2] as u32));
        e(&Instruction::LocalGet(d_l[2] as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalGet(subw as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(tmp as u32));
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::LocalGet(r_l[2] as u32));
        e(&Instruction::I64GtU);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(subw as u32));
        e(&Instruction::LocalGet(tmp as u32));
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(r_l[2] as u32));
        e(&Instruction::LocalGet(r_l[3] as u32));
        e(&Instruction::LocalGet(d_l[3] as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalGet(subw as u32));
        e(&Instruction::I64Sub);
        e(&Instruction::I64Const(0xFFFF_FFFF));
        e(&Instruction::I64And);
        e(&Instruction::LocalSet(r_l[3] as u32));
        // Q |= 1
        e(&Instruction::LocalGet(q_l[0] as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Or);
        e(&Instruction::LocalSet(q_l[0] as u32));
        e(&Instruction::End);
        // bit -= 1; loop while bit >= 0 (br_if -1 when bit wraps below 0)
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1)); // done at bit 0 (inclusive)
        e(&Instruction::LocalGet(bit as u32));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(bit as u32));
        e(&Instruction::Br(0));
        e(&Instruction::End); // loop
        e(&Instruction::End); // block
                              // quotient overflow: any Q limb ≥ 2^128 nonzero → trap
        e(&Instruction::LocalGet(q_l[4] as u32));
        e(&Instruction::LocalGet(q_l[5] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(q_l[6] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::LocalGet(q_l[7] as u32));
        e(&Instruction::I64Or);
        // Eqz here is load-bearing twice: i64→i32 for If, and polarity —
        // 1 = all high limbs zero = no overflow. If branches on NONZERO,
        // so `If(unreachable)` with the Eqz would trap on the SUCCESS
        // path; the trap must live in the Else arm.
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::Else);
        e(&Instruction::Unreachable); // muldiv: quotient ≥ 2^128
        e(&Instruction::End);
        // store q: [q] = (q1 << 32 | q0), [q+8] = (q3 << 32 | q2)
        e(&Instruction::LocalGet(3 as u32));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(q_l[1] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(q_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::LocalGet(3 as u32));
        e(&Instruction::I64Const(8));
        e(&Instruction::I64Add);
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(q_l[3] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(q_l[2] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        // if rem_ptr != 0: [rem] = (r1 << 32 | r0)
        e(&Instruction::LocalGet(4 as u32));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(4 as u32));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(r_l[1] as u32));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(r_l[0] as u32));
        e(&Instruction::I64Or);
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::End);
        e(&Instruction::I64Const(TAG_NIL));
        v
    }

    // Checked variant: Unreachable → TAGGED_FALSE (same sentinel discipline
    // as the other _ck helpers — legit returns are nil(4), never 1).
    fn h_muldiv_ck() -> Vec<Instruction<'static>> {
        Self::to_checked(Self::h_muldiv())
    }

    // __i64_to_str(n) -> tagged string. Locals: 0=n 1=neg 2=u 3=dst 4=pos 5=digit 6=len
    fn h_i64_to_str(mem_limit: i64) -> Vec<Instruction<'static>> {
        let mut v = vec![];
        let mut e = |i: &Instruction<'static>| v.push(i.clone());
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Const(0));
        e(&Instruction::I64LtS);
        e(&Instruction::I64ExtendI32U);
        e(&Instruction::LocalSet(1));
        // u = |n| — negate ONLY when negative (unconditional 0-n broke positives:
        // 0-42 = -42 → digits of 2^64-42)
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::I64Const(0));
        e(&Instruction::LocalGet(0));
        e(&Instruction::I64Sub);
        e(&Instruction::Else);
        e(&Instruction::LocalGet(0));
        e(&Instruction::End);
        e(&Instruction::LocalSet(2));
        // alloc 24 bytes
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I64Load(ma8()));
        e(&Instruction::LocalSet(3));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(24));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(5));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Const(mem_limit));
        e(&Instruction::I64LtU);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::I64Const(56));
        e(&Instruction::I32WrapI64);
        e(&Instruction::LocalGet(5));
        e(&Instruction::I64Store(ma8()));
        e(&Instruction::Else);
        e(&Instruction::Unreachable);
        e(&Instruction::End);
        // zero fast path
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Eqz);
        e(&Instruction::If(BlockType::Result(ValType::I64)));
        e(&Instruction::LocalGet(3));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::I32Store8(ma()));
        // tagged str = ((1<<32)|dst)<<TAG_BITS | TAG_STR
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::Else);
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(24));
        e(&Instruction::I64Add);
        e(&Instruction::LocalSet(4));
        e(&Instruction::Block(BlockType::Empty));
        e(&Instruction::Loop(BlockType::Empty));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Eqz);
        e(&Instruction::BrIf(1));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Const(10));
        e(&Instruction::I64RemU);
        e(&Instruction::LocalSet(5));
        e(&Instruction::LocalGet(2));
        e(&Instruction::I64Const(10));
        e(&Instruction::I64DivU);
        e(&Instruction::LocalSet(2));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(48));
        e(&Instruction::LocalGet(5));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Add);
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::Br(0));
        e(&Instruction::End);
        e(&Instruction::End);
        // if negative: prepend '-'
        e(&Instruction::LocalGet(1));
        e(&Instruction::I32WrapI64);
        e(&Instruction::If(BlockType::Empty));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Const(1));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(4));
        e(&Instruction::LocalGet(4));
        e(&Instruction::I32WrapI64);
        e(&Instruction::I32Const(45));
        e(&Instruction::I32Store8(ma()));
        e(&Instruction::End);
        e(&Instruction::LocalGet(3));
        e(&Instruction::I64Const(24));
        e(&Instruction::I64Add);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Sub);
        e(&Instruction::LocalSet(6));
        // tagged str = ((len<<32)|pos)<<TAG_BITS | TAG_STR
        e(&Instruction::LocalGet(6));
        e(&Instruction::I64Const(32));
        e(&Instruction::I64Shl);
        e(&Instruction::LocalGet(4));
        e(&Instruction::I64Or);
        e(&Instruction::I64Const(TAG_BITS));
        e(&Instruction::I64Shl);
        e(&Instruction::I64Const(TAG_STR));
        e(&Instruction::I64Or);
        e(&Instruction::End); // close zero fast-path If(Result i64)
        v
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Op emission (string-based, interpreter semantics)
    // ─────────────────────────────────────────────────────────────────────────

    pub(crate) fn call_u128_str(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "u128/add" | "u128/sub" | "u128/mul" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                let h = self.ensure_u128_str_helpers();
                // Limb-aware operand slots (u128 Level 1, 2026-09-15):
                // limb locals and nested u128 arith feed (lo, hi) pairs
                // directly — no tagged string round-trip. Generic operands
                // parse once, exactly as before. Fresh per-gen slots keep
                // the 2026-08-31 nested-clobber rule (operands are saved
                // BEFORE any scratch use).
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let alo = self.local_idx(&format!("__u128la_{gen}"));
                let ahi = self.local_idx(&format!("__u128ha_{gen}"));
                let blo = self.local_idx(&format!("__u128lb_{gen}"));
                let bhi = self.local_idx(&format!("__u128hb_{gen}"));
                let mut v = Vec::new();
                self.emit_u128_operand(&mut v, &a[0], alo, ahi)?;
                self.emit_u128_operand(&mut v, &a[1], blo, bhi)?;
                v.extend(self.limb_pair_store(alo, ahi, U128_A));
                v.extend(self.limb_pair_store(blo, bhi, U128_B));
                let (hf, hf_ck) = match op {
                    "u128/add" => (h.add, h.add_ck),
                    "u128/sub" => (h.sub, h.sub_ck),
                    _ => (h.mul, h.mul_ck),
                };
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I64Const(U128_B));
                if self.try_stack.is_empty() {
                    v.push(Self::call_user(hf));
                    v.push(Instruction::Drop);
                } else {
                    let call = Self::call_user(hf_ck);
                    self.ck_guarded(&mut v, call, "u128: overflow/underflow");
                }
                v.push(Instruction::I64Const(U128_A));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u128/div" | "u128/mod" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                let h = self.ensure_u128_str_helpers();
                // Limb-aware operand slots (u128 Level 1) — same discipline
                // as the add/sub/mul arm: limb locals and nested u128 arith
                // feed (lo, hi) pairs without a tagged round-trip.
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let alo = self.local_idx(&format!("__u128la_{gen}"));
                let ahi = self.local_idx(&format!("__u128ha_{gen}"));
                let blo = self.local_idx(&format!("__u128lb_{gen}"));
                let bhi = self.local_idx(&format!("__u128hb_{gen}"));
                let mut v = Vec::new();
                self.emit_u128_operand(&mut v, &a[0], alo, ahi)?;
                self.emit_u128_operand(&mut v, &a[1], blo, bhi)?;
                v.extend(self.limb_pair_store(alo, ahi, U128_A));
                v.extend(self.limb_pair_store(blo, bhi, U128_B));
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I64Const(U128_B));
                v.push(Instruction::I64Const(U128_R));
                if self.try_stack.is_empty() {
                    v.push(Self::call_user(h.divmod));
                    v.push(Instruction::Drop);
                } else {
                    let call = Self::call_user(h.divmod_ck);
                    self.ck_guarded(&mut v, call, "u128: division by zero");
                }
                let src = if op == "u128/div" { U128_A } else { U128_R };
                v.push(Instruction::I64Const(src));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u128/muldiv" => {
                // CLMM core: q = a * b / d over the full 256-bit product.
                // Same operand discipline as the add/sub/mul arm; the
                // 256-bit product + restoring division live in the
                // __h_u128_muldiv helper (d==0 and quotient ≥ 2^128 trap).
                if a.len() != 3 {
                    return Err("u128/muldiv: need 3 args".into());
                }
                let h = self.ensure_u128_str_helpers();
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let alo = self.local_idx(&format!("__u128la_{gen}"));
                let ahi = self.local_idx(&format!("__u128ha_{gen}"));
                let blo = self.local_idx(&format!("__u128lb_{gen}"));
                let bhi = self.local_idx(&format!("__u128hb_{gen}"));
                let clo = self.local_idx(&format!("__u128lc_{gen}"));
                let chi = self.local_idx(&format!("__u128hc_{gen}"));
                let mut v = Vec::new();
                self.emit_u128_operand(&mut v, &a[0], alo, ahi)?;
                self.emit_u128_operand(&mut v, &a[1], blo, bhi)?;
                self.emit_u128_operand(&mut v, &a[2], clo, chi)?;
                v.extend(self.limb_pair_store(alo, ahi, U128_A));
                v.extend(self.limb_pair_store(blo, bhi, U128_B));
                v.extend(self.limb_pair_store(clo, chi, U128_C));
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I64Const(U128_B));
                v.push(Instruction::I64Const(U128_C));
                v.push(Instruction::I64Const(U128_Q));
                v.push(Instruction::I64Const(U128_RM));
                if self.try_stack.is_empty() {
                    v.push(Self::call_user(h.muldiv));
                    v.push(Instruction::Drop);
                } else {
                    let call = Self::call_user(h.muldiv_ck);
                    self.ck_guarded(&mut v, call, "u128: muldiv overflow");
                }
                v.push(Instruction::I64Const(U128_Q));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u128/lt" | "u128/gt" | "u128/eq" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                // Limb-aware operand slots (u128 Level 1): operands land in
                // (lo, hi) local pairs — limb locals and nested u128 arith
                // with zero stringification; generic operands parse once.
                // The comparison then runs on the LOCALS (no scratch memory
                // round-trip). Fresh per-gen slots keep the 2026-08-31
                // nested-clobber rule; u128_parse_call inside the generic
                // operand path keeps the round-4 CERR fix (catchable parse
                // errors under try, wasm-fuzz find #5).
                let gen = self.limb_call_count;
                self.limb_call_count += 1;
                let alo = self.local_idx(&format!("__u128la_{gen}"));
                let ahi = self.local_idx(&format!("__u128ha_{gen}"));
                let blo = self.local_idx(&format!("__u128lb_{gen}"));
                let bhi = self.local_idx(&format!("__u128hb_{gen}"));
                let mut v = Vec::new();
                self.emit_u128_operand(&mut v, &a[0], alo, ahi)?;
                self.emit_u128_operand(&mut v, &a[1], blo, bhi)?;
                match op {
                    "u128/lt" => {
                        // (a.hi <u b.hi) | ((a.hi == b.hi) & (a.lo <u b.lo))
                        v.push(Instruction::LocalGet(ahi));
                        v.push(Instruction::LocalGet(bhi));
                        v.push(Instruction::I64LtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::LocalGet(ahi));
                        v.push(Instruction::LocalGet(bhi));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::LocalGet(alo));
                        v.push(Instruction::LocalGet(blo));
                        v.push(Instruction::I64LtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64And);
                        v.push(Instruction::I64Or);
                    }
                    "u128/gt" => {
                        v.push(Instruction::LocalGet(ahi));
                        v.push(Instruction::LocalGet(bhi));
                        v.push(Instruction::I64GtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::LocalGet(ahi));
                        v.push(Instruction::LocalGet(bhi));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::LocalGet(alo));
                        v.push(Instruction::LocalGet(blo));
                        v.push(Instruction::I64GtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64And);
                        v.push(Instruction::I64Or);
                    }
                    _ => {
                        v.push(Instruction::LocalGet(alo));
                        v.push(Instruction::LocalGet(blo));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::LocalGet(ahi));
                        v.push(Instruction::LocalGet(bhi));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64And);
                    }
                }
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "u128/from-i64" => {
                if a.len() != 1 {
                    return Err("u128/from-i64: need 1 arg".into());
                }
                let h = self.ensure_u128_str_helpers();
                let av = self.expr(&a[0])?;
                let va = self.local_idx("__u128fi");
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                // type check: (v & 7) == TAG_NUM
                v.push(Instruction::LocalGet(va));
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_NUM));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // payload (signed shift — negatives render with '-')
                v.push(Instruction::LocalGet(va));
                v.push(Instruction::I64Const(TAG_BITS));
                v.push(Instruction::I64ShrS);
                v.push(Self::call_user(h.i64_to_str));
                Ok(v)
            }
            "u128/to-i64" => {
                if a.len() != 1 {
                    return Err("u128/to-i64: need 1 arg".into());
                }
                let h = self.ensure_u128_str_helpers();
                let av = self.expr(&a[0])?;
                let va = self.local_idx("__u128ti");
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                v.push(Instruction::LocalGet(va));
                v.push(Instruction::I64Const(U128_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                // hi != 0 → exceeds i64
                v.push(Instruction::I64Const(U128_A + 8));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8()));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Else);
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // tagged-Num payloads are 61-bit signed: values above 2^60-1
                // cannot round-trip through the tagged ABI → hard error
                // (interpreter accepts up to i64::MAX — documented deviation).
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8()));
                v.push(Instruction::I64Const(0x0FFF_FFFF_FFFF_FFFF)); // 2^60 - 1
                v.push(Instruction::I64LeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Else);
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8()));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "u128/is-zero" => {
                if a.len() != 1 {
                    return Err("u128/is-zero: need 1 arg".into());
                }
                let h = self.ensure_u128_str_helpers();
                let av = self.expr(&a[0])?;
                let va = self.local_idx("__u128iz");
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                v.push(Instruction::LocalGet(va));
                v.push(Instruction::I64Const(U128_A));
                v.push(Self::call_user(h.parse));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(U128_A));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8()));
                v.push(Instruction::I64Const(U128_A + 8));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8()));
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }

    /// Guarded helper call (try-aware): call a _ck helper, and if it returns
    /// TAGGED_FALSE, emit a catch jump. Emits nothing extra when no try is
    /// active (caller should then use the trapping variant instead).
    pub(crate) fn ck_guarded(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        call: Instruction<'static>,
        msg: &str,
    ) {
        v.push(call);
        v.push(Instruction::I64Const(1)); // TAGGED_FALSE
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        self.try_guard(v, msg);
        v.push(Instruction::End);
    }

    /// parse call — trapping or checked depending on try context.
    pub(crate) fn u128_parse_call(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        val_local: u32,
        dst: i64,
        h: &U128Helpers,
    ) {
        let val = Instruction::LocalGet(val_local);
        let dstc = Instruction::I64Const(dst);
        if self.try_stack.is_empty() {
            v.push(val);
            v.push(dstc);
            v.push(Self::call_user(h.parse));
            v.push(Instruction::Drop);
        } else {
            v.push(val);
            v.push(dstc);
            let call = Self::call_user(h.parse_ck);
            self.ck_guarded(v, call, "u128: parse/overflow error");
        }
    }
}
