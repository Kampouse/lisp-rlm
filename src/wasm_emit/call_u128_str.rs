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
const U128_A: i64 = 8480; // operand A / arithmetic destination
const U128_B: i64 = 8496; // operand B
const U128_R: i64 = 8512; // remainder (divmod)

/// Function-table indices of the synthesized helpers (positions in `funcs`).
#[derive(Clone, Copy)]
pub(crate) struct U128Helpers {
    pub(crate) parse: u32,
    pub(crate) to_str: u32,
    pub(crate) add: u32,
    pub(crate) sub: u32,
    pub(crate) mul: u32,
    pub(crate) divmod: u32,
    pub(crate) i64_to_str: u32,
    // Checked variants (try/catch, round 4): same math, but every error trap
    // returns TAGGED_FALSE instead of trapping. Call sites under an active
    // try guard on that sentinel and catch-jump.
    pub(crate) parse_ck: u32,
    pub(crate) add_ck: u32,
    pub(crate) sub_ck: u32,
    pub(crate) mul_ck: u32,
    pub(crate) divmod_ck: u32,
}

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
        let h = U128Helpers {
            parse: parse as u32,
            to_str: to_str as u32,
            add: add as u32,
            sub: sub as u32,
            mul: mul as u32,
            divmod: divmod as u32,
            i64_to_str: i64_to_str as u32,
            parse_ck: parse_ck as u32,
            add_ck: add_ck as u32,
            sub_ck: sub_ck as u32,
            mul_ck: mul_ck as u32,
            divmod_ck: divmod_ck as u32,
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
                let av = self.expr(&a[0])?;
                let bv = self.expr(&a[1])?;
                // (2026-08-31) FRESH locals per invocation: a u128 op nested
                // in the second operand reused __u128sa/__u128sb and
                // clobbered the outer op's saved first operand —
                // (u128/lt (u128/mul A B) (u128/mul C D)) compared C's
                // operand instead of A*B's result (interp was right).
                let gen = self.u128_call_count;
                self.u128_call_count += 1;
                let va = self.local_idx(&format!("__u128sa_{gen}"));
                let vb = self.local_idx(&format!("__u128sb_{gen}"));
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                v.extend(bv);
                v.push(Instruction::LocalSet(vb));
                self.u128_parse_call(&mut v, va, U128_A, &h);
                self.u128_parse_call(&mut v, vb, U128_B, &h);
                let (hf, hf_ck) = match op {
                    "u128/add" => (h.add, h.add_ck),
                    "u128/sub" => (h.sub, h.sub_ck),
                    _ => (h.mul, h.mul_ck),
                };
                if self.try_stack.is_empty() {
                    v.push(Instruction::I64Const(U128_A));
                    v.push(Instruction::I64Const(U128_B));
                    v.push(Self::call_user(hf));
                    v.push(Instruction::Drop);
                } else {
                    v.push(Instruction::I64Const(U128_A));
                    v.push(Instruction::I64Const(U128_B));
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
                let av = self.expr(&a[0])?;
                let bv = self.expr(&a[1])?;
                // (2026-08-31) FRESH locals per invocation: a u128 op nested
                // in the second operand reused __u128sa/__u128sb and
                // clobbered the outer op's saved first operand —
                // (u128/lt (u128/mul A B) (u128/mul C D)) compared C's
                // operand instead of A*B's result (interp was right).
                let gen = self.u128_call_count;
                self.u128_call_count += 1;
                let va = self.local_idx(&format!("__u128sa_{gen}"));
                let vb = self.local_idx(&format!("__u128sb_{gen}"));
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                v.extend(bv);
                v.push(Instruction::LocalSet(vb));
                self.u128_parse_call(&mut v, va, U128_A, &h);
                self.u128_parse_call(&mut v, vb, U128_B, &h);
                if self.try_stack.is_empty() {
                    v.push(Instruction::I64Const(U128_A));
                    v.push(Instruction::I64Const(U128_B));
                    v.push(Instruction::I64Const(U128_R));
                    v.push(Self::call_user(h.divmod));
                    v.push(Instruction::Drop);
                } else {
                    v.push(Instruction::I64Const(U128_A));
                    v.push(Instruction::I64Const(U128_B));
                    v.push(Instruction::I64Const(U128_R));
                    let call = Self::call_user(h.divmod_ck);
                    self.ck_guarded(&mut v, call, "u128: division by zero");
                }
                let src = if op == "u128/div" { U128_A } else { U128_R };
                v.push(Instruction::I64Const(src));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "u128/lt" | "u128/gt" | "u128/eq" => {
                if a.len() != 2 {
                    return Err(format!("{}: need 2 args", op));
                }
                let h = self.ensure_u128_str_helpers();
                let av = self.expr(&a[0])?;
                let bv = self.expr(&a[1])?;
                // (2026-08-31) FRESH locals per invocation: a u128 op nested
                // in the second operand reused __u128sa/__u128sb and
                // clobbered the outer op's saved first operand —
                // (u128/lt (u128/mul A B) (u128/mul C D)) compared C's
                // operand instead of A*B's result (interp was right).
                let gen = self.u128_call_count;
                self.u128_call_count += 1;
                let va = self.local_idx(&format!("__u128sa_{gen}"));
                let vb = self.local_idx(&format!("__u128sb_{gen}"));
                let mut v = Vec::new();
                v.extend(av);
                v.push(Instruction::LocalSet(va));
                v.extend(bv);
                v.push(Instruction::LocalSet(vb));
                // Round-4 CERR sweep miss (wasm-fuzz find #5, 2026-08-27):
                // these inlined parse calls bypassed u128_parse_call, so
                // invalid operands TRAPPED uncatchably under try instead of
                // raising a catchable error (interp catches).
                self.u128_parse_call(&mut v, va, U128_A, &h);
                self.u128_parse_call(&mut v, vb, U128_B, &h);
                // compare (A) vs (B)
                match op {
                    "u128/lt" => {
                        // (A.hi <u B.hi) | ((A.hi == B.hi) & (A.lo <u B.lo))
                        v.push(Instruction::I64Const(U128_A + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64LtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(U128_A + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(U128_A));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64LtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64And);
                        v.push(Instruction::I64Or);
                    }
                    "u128/gt" => {
                        v.push(Instruction::I64Const(U128_A + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64GtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(U128_A + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(U128_A));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64GtU);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64And);
                        v.push(Instruction::I64Or);
                    }
                    _ => {
                        v.push(Instruction::I64Const(U128_A));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(U128_A + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
                        v.push(Instruction::I64Const(U128_B + 8));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma8()));
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
    fn ck_guarded(
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
    fn u128_parse_call(
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
