use super::*;

impl WasmEmitter {
    /// muldiv(a, b, c): computes (a * b) / c with 128-bit intermediate precision.
    /// All inputs are UNTAGGED raw i64 on the stack (dispatch handles untag).
    /// Returns tagged number.
    /// Traps on: division by zero, result overflow (>i64 max after division).
    /// Uses schoolbook 32-bit split multiplication + binary long division.
    pub(crate) fn emit_muldiv(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__md_a");
        let b = self.local_idx("__md_b");
        let c = self.local_idx("__md_c");
        let a_hi = self.local_idx("__md_ahi");
        let a_lo = self.local_idx("__md_alo");
        let b_hi = self.local_idx("__md_bhi");
        let b_lo = self.local_idx("__md_blo");
        let p0 = self.local_idx("__md_p0");
        let p1 = self.local_idx("__md_p1");
        let p2 = self.local_idx("__md_p2");
        let p3 = self.local_idx("__md_p3");
        let mid = self.local_idx("__md_mid");
        let carry_mid = self.local_idx("__md_cmid");
        let mid_lo = self.local_idx("__md_mlo");
        let mid_hi = self.local_idx("__md_mhi");
        let lo = self.local_idx("__md_lo");
        let carry_lo = self.local_idx("__md_clo");
        let hi = self.local_idx("__md_hi");
        let r = self.local_idx("__md_r");
        let q = self.local_idx("__md_q");
        let i = self.local_idx("__md_i");
        let bit = self.local_idx("__md_bit");

        let mask32: i64 = 0xFFFFFFFF;

        vec![
            // Pop c, b, a (reverse order) — already untagged by dispatch
            Instruction::LocalSet(c),
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            // Check c != 0
            Instruction::LocalGet(c),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            // ===== Step 1: Schoolbook 128-bit multiply =====
            // Split a into a_hi:a_lo
            Instruction::LocalGet(a), Instruction::I64Const(32), Instruction::I64ShrU, Instruction::LocalSet(a_hi),
            Instruction::LocalGet(a), Instruction::LocalGet(a_hi), Instruction::I64Const(32), Instruction::I64Shl, Instruction::I64Sub, Instruction::LocalSet(a_lo),
            // Split b into b_hi:b_lo
            Instruction::LocalGet(b), Instruction::I64Const(32), Instruction::I64ShrU, Instruction::LocalSet(b_hi),
            Instruction::LocalGet(b), Instruction::LocalGet(b_hi), Instruction::I64Const(32), Instruction::I64Shl, Instruction::I64Sub, Instruction::LocalSet(b_lo),
            // p0 = a_lo * b_lo
            Instruction::LocalGet(a_lo), Instruction::LocalGet(b_lo), Instruction::I64Mul, Instruction::LocalSet(p0),
            // p1 = a_lo * b_hi
            Instruction::LocalGet(a_lo), Instruction::LocalGet(b_hi), Instruction::I64Mul, Instruction::LocalSet(p1),
            // p2 = a_hi * b_lo
            Instruction::LocalGet(a_hi), Instruction::LocalGet(b_lo), Instruction::I64Mul, Instruction::LocalSet(p2),
            // p3 = a_hi * b_hi
            Instruction::LocalGet(a_hi), Instruction::LocalGet(b_hi), Instruction::I64Mul, Instruction::LocalSet(p3),
            // mid = p1 + p2
            Instruction::LocalGet(p1), Instruction::LocalGet(p2), Instruction::I64Add, Instruction::LocalSet(mid),
            // carry_mid = (mid <u p1) ? 1 : 0
            Instruction::LocalGet(mid), Instruction::LocalGet(p1), Instruction::I64LtU,
            Instruction::I64ExtendI32U, Instruction::LocalSet(carry_mid),
            // mid_lo = mid & 0xFFFFFFFF, mid_hi = mid >> 32
            Instruction::LocalGet(mid), Instruction::I64Const(mask32), Instruction::I64And, Instruction::LocalSet(mid_lo),
            Instruction::LocalGet(mid), Instruction::I64Const(32), Instruction::I64ShrU, Instruction::LocalSet(mid_hi),
            // lo = p0 + (mid_lo << 32)
            Instruction::LocalGet(p0), Instruction::LocalGet(mid_lo), Instruction::I64Const(32), Instruction::I64Shl, Instruction::I64Add, Instruction::LocalSet(lo),
            // carry_lo = (lo <u p0) ? 1 : 0
            Instruction::LocalGet(lo), Instruction::LocalGet(p0), Instruction::I64LtU,
            Instruction::I64ExtendI32U, Instruction::LocalSet(carry_lo),
            // hi = p3 + mid_hi + (carry_mid << 32) + carry_lo
            Instruction::LocalGet(p3),
            Instruction::LocalGet(mid_hi), Instruction::I64Add,
            Instruction::LocalGet(carry_mid), Instruction::I64Const(32), Instruction::I64Shl, Instruction::I64Add,
            Instruction::LocalGet(carry_lo), Instruction::I64Add,
            Instruction::LocalSet(hi),
            // ===== Step 2: Overflow check — hi >= c means result won't fit i64 =====
            Instruction::LocalGet(hi), Instruction::LocalGet(c), Instruction::I64GeU,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            // ===== Step 3: Binary long division [hi:lo] / c =====
            Instruction::LocalGet(hi), Instruction::LocalSet(r),
            Instruction::I64Const(0), Instruction::LocalSet(q),
            Instruction::I64Const(63), Instruction::LocalSet(i),
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            // if i < 0: break
            Instruction::LocalGet(i), Instruction::I64Const(0), Instruction::I64LtS,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            // bit = (lo >> i) & 1
            Instruction::LocalGet(lo), Instruction::LocalGet(i), Instruction::I64ShrU, Instruction::I64Const(1), Instruction::I64And,
            Instruction::LocalSet(bit),
            // r = (r << 1) | bit
            Instruction::LocalGet(r), Instruction::I64Const(1), Instruction::I64Shl,
            Instruction::LocalGet(bit), Instruction::I64Or,
            Instruction::LocalSet(r),
            // if r >=u c: r -= c; q |= (1 << i)
            Instruction::LocalGet(r), Instruction::LocalGet(c), Instruction::I64GeU,
            Instruction::If(BlockType::Empty),
            Instruction::LocalGet(r), Instruction::LocalGet(c), Instruction::I64Sub, Instruction::LocalSet(r),
            Instruction::LocalGet(q),
            Instruction::I64Const(1), Instruction::LocalGet(i), Instruction::I64Shl,
            Instruction::I64Or, Instruction::LocalSet(q),
            Instruction::End,
            // i--
            Instruction::LocalGet(i), Instruction::I64Const(1), Instruction::I64Sub, Instruction::LocalSet(i),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
            // ===== Step 4: Tag and return =====
            Instruction::LocalGet(q),
            Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
        ]
    }

    /// isqrt(x): floor(sqrt(x)) via Newton's method.
    /// Input is UNTAGGED raw i64 on stack. Returns tagged number.
    pub(crate) fn emit_isqrt(&mut self) -> Vec<Instruction<'static>> {
        let n = self.local_idx("__sq_n");
        let x0 = self.local_idx("__sq_x0");
        let x1 = self.local_idx("__sq_x1");

        vec![
            Instruction::LocalSet(n),
            // if n < 2: return n tagged
            Instruction::LocalGet(n), Instruction::I64Const(2), Instruction::I64LtU,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::LocalGet(n), Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
            Instruction::Else,
            // x0 = n >> 1
            Instruction::LocalGet(n), Instruction::I64Const(1), Instruction::I64ShrU, Instruction::LocalSet(x0),
            // Newton loop
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            // x1 = (n / x0 + x0) >> 1
            Instruction::LocalGet(n), Instruction::LocalGet(x0), Instruction::I64DivU,
            Instruction::LocalGet(x0), Instruction::I64Add,
            Instruction::I64Const(1), Instruction::I64ShrU,
            Instruction::LocalSet(x1),
            // if x1 >= x0: converged, break
            Instruction::LocalGet(x1), Instruction::LocalGet(x0), Instruction::I64GeU,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            // x0 = x1
            Instruction::LocalGet(x1), Instruction::LocalSet(x0),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
            // Return x0 tagged
            Instruction::LocalGet(x0), Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
            Instruction::End, // if
        ]
    }

    /// ctz(x): count trailing zero bits.
    /// Input is UNTAGGED raw i64 on the stack. Returns tagged number (0-63).
    /// Traps if x == 0.
    pub(crate) fn emit_ctz(&mut self) -> Vec<Instruction<'static>> {
        let v = self.local_idx("__ctz_v");
        let count = self.local_idx("__ctz_n");

        vec![
            Instruction::LocalSet(v),
            // if v == 0: trap
            Instruction::LocalGet(v), Instruction::I64Eqz,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            // count = 0
            Instruction::I64Const(0), Instruction::LocalSet(count),
            // Loop while (v & 1) == 0
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            Instruction::LocalGet(v), Instruction::I64Const(1), Instruction::I64And,
            Instruction::I64Const(0), Instruction::I64Ne,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            Instruction::LocalGet(v), Instruction::I64Const(1), Instruction::I64ShrU, Instruction::LocalSet(v),
            Instruction::LocalGet(count), Instruction::I64Const(1), Instruction::I64Add, Instruction::LocalSet(count),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
            // Return count tagged
            Instruction::LocalGet(count), Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
        ]
    }

    // ── Linear memory struct intrinsics ──
    // All pointer arguments are UNTAGGED (dispatch handles untag).
    // Offsets are in BYTES (not field indices) for flexibility.

    /// malloc(n_bytes): allocate ZEROED n_bytes, return a HANDLE (index into table).
    /// Handle table at HANDLE_TABLE_BASE stores [real_ptr, size] per entry.
    /// Input: UNTAGGED n_bytes on stack. Returns TAGGED handle index.
    pub(crate) fn emit_malloc(&mut self) -> Vec<Instruction<'static>> {
        let n = self.local_idx("__ma_n");
        let tmp = self.local_idx("__ma_tmp");
        let new_ptr = self.local_idx("__ma_new");
        let cursor = self.local_idx("__ma_cur");
        let hcount = self.local_idx("__ma_hc");
        let entry_addr = self.local_idx("__ma_ea");
        let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let mem_limit = (self.memory_pages as i64) * 65536;
        vec![
            // Pop n_bytes (already untagged by dispatch)
            Instruction::LocalSet(n),
            // Align n to 8 bytes
            Instruction::LocalGet(n),
            Instruction::I64Const(7),
            Instruction::I64Add,
            Instruction::I64Const(-8i64 as u64 as i64),
            Instruction::I64And,
            Instruction::LocalSet(n),
            // Read handle count
            Instruction::I64Const(HANDLE_COUNT_ADDR),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(hcount),
            // Guard: hcount < MAX_HANDLES
            Instruction::LocalGet(hcount),
            Instruction::I64Const(MAX_HANDLES),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // too many allocations
            Instruction::End,
            // ── Bump allocate from heap ──
            // Read current runtime heap ptr
            Instruction::I64Const(RUNTIME_HEAP_PTR),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(tmp),
            // new_ptr = tmp + n
            Instruction::LocalGet(tmp),
            Instruction::LocalGet(n),
            Instruction::I64Add,
            Instruction::LocalSet(new_ptr),
            // Guard: new_ptr < mem_limit
            Instruction::LocalGet(new_ptr),
            Instruction::I64Const(mem_limit),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            // OK: write back new heap ptr
            Instruction::I64Const(RUNTIME_HEAP_PTR),
            Instruction::I32WrapI64,
            Instruction::LocalGet(new_ptr),
            Instruction::I64Store(ma.clone()),
            Instruction::Else,
            Instruction::Unreachable,
            Instruction::End,
            // ── Zero-fill ──
            Instruction::LocalGet(tmp),
            Instruction::LocalSet(cursor),
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            Instruction::LocalGet(cursor),
            Instruction::LocalGet(new_ptr),
            Instruction::I64GeU,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            Instruction::LocalGet(cursor),
            Instruction::I32WrapI64,
            Instruction::I64Const(0),
            Instruction::I64Store(ma.clone()),
            Instruction::LocalGet(cursor),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::LocalSet(cursor),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
            // ── Write handle table entry ──
            // entry_addr = HANDLE_TABLE_BASE + hcount * 16
            Instruction::I64Const(HANDLE_TABLE_BASE),
            Instruction::LocalGet(hcount),
            Instruction::I64Const(16),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::LocalSet(entry_addr),
            // [+0] = real_ptr
            Instruction::LocalGet(entry_addr),
            Instruction::I32WrapI64,
            Instruction::LocalGet(tmp),
            Instruction::I64Store(ma.clone()),
            // [+8] = size
            Instruction::LocalGet(entry_addr),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::LocalGet(n),
            Instruction::I64Store(ma.clone()),
            // Increment handle count
            Instruction::I64Const(HANDLE_COUNT_ADDR),
            Instruction::I32WrapI64,
            Instruction::LocalGet(hcount),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::I64Store(ma.clone()),
            // Return handle index tagged
            Instruction::LocalGet(hcount),
            Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
        ]
    }

    /// store_i64(handle, byte_offset, value): write i64 via handle table.
    /// Validates: handle index < handle_count, offset + 8 <= alloc_size.
    /// All three inputs are UNTAGGED. Returns nil.
    pub(crate) fn emit_store_i64(&mut self) -> Vec<Instruction<'static>> {
        let handle = self.local_idx("__si_h");
        let off = self.local_idx("__si_off");
        let val = self.local_idx("__si_val");
        let hcount = self.local_idx("__si_hc");
        let entry_addr = self.local_idx("__si_ea");
        let real_ptr = self.local_idx("__si_rp");
        let alloc_size = self.local_idx("__si_sz");
        let addr = self.local_idx("__si_addr");
        let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        vec![
            // Pop val, off, handle (reverse stack order)
            Instruction::LocalSet(val),
            Instruction::LocalSet(off),
            Instruction::LocalSet(handle),
            // Read handle count
            Instruction::I64Const(HANDLE_COUNT_ADDR),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(hcount),
            // Validate: handle < hcount
            Instruction::LocalGet(handle),
            Instruction::LocalGet(hcount),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // bad handle
            Instruction::End,
            // Look up entry: entry_addr = HANDLE_TABLE_BASE + handle * 16
            Instruction::I64Const(HANDLE_TABLE_BASE),
            Instruction::LocalGet(handle),
            Instruction::I64Const(16),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::LocalSet(entry_addr),
            // Load real_ptr from entry[+0]
            Instruction::LocalGet(entry_addr),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(real_ptr),
            // Load alloc_size from entry[+8]
            Instruction::LocalGet(entry_addr),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(alloc_size),
            // Bounds check: offset + 8 <= alloc_size
            Instruction::LocalGet(off),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::LocalGet(alloc_size),
            Instruction::I64LeU,
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // out of bounds
            Instruction::End,
            // addr = real_ptr + offset
            Instruction::LocalGet(real_ptr),
            Instruction::LocalGet(off),
            Instruction::I64Add,
            Instruction::LocalSet(addr),
            // i64.store addr
            Instruction::LocalGet(addr),
            Instruction::I32WrapI64,
            Instruction::LocalGet(val),
            Instruction::I64Store(ma),
            // Return tagged nil
            Instruction::I64Const(TAG_NIL),
        ]
    }

    /// load_i64(handle, byte_offset): read i64 via handle table.
    /// Validates: handle index < handle_count, offset + 8 <= alloc_size.
    /// Both inputs are UNTAGGED. Returns TAGGED value.
    pub(crate) fn emit_load_i64(&mut self) -> Vec<Instruction<'static>> {
        let handle = self.local_idx("__li_h");
        let off = self.local_idx("__li_off");
        let hcount = self.local_idx("__li_hc");
        let entry_addr = self.local_idx("__li_ea");
        let real_ptr = self.local_idx("__li_rp");
        let alloc_size = self.local_idx("__li_sz");
        let addr = self.local_idx("__li_addr");
        let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        vec![
            // Pop off, handle
            Instruction::LocalSet(off),
            Instruction::LocalSet(handle),
            // Read handle count
            Instruction::I64Const(HANDLE_COUNT_ADDR),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(hcount),
            // Validate: handle < hcount
            Instruction::LocalGet(handle),
            Instruction::LocalGet(hcount),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // bad handle
            Instruction::End,
            // Look up entry
            Instruction::I64Const(HANDLE_TABLE_BASE),
            Instruction::LocalGet(handle),
            Instruction::I64Const(16),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::LocalSet(entry_addr),
            // Load real_ptr
            Instruction::LocalGet(entry_addr),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(real_ptr),
            // Load alloc_size
            Instruction::LocalGet(entry_addr),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(alloc_size),
            // Bounds check: offset + 8 <= alloc_size
            Instruction::LocalGet(off),
            Instruction::I64Const(8),
            Instruction::I64Add,
            Instruction::LocalGet(alloc_size),
            Instruction::I64LeU,
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // out of bounds
            Instruction::End,
            // addr = real_ptr + offset
            Instruction::LocalGet(real_ptr),
            Instruction::LocalGet(off),
            Instruction::I64Add,
            Instruction::LocalSet(addr),
            // i64.load addr
            Instruction::LocalGet(addr),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma),
            // Tag as number
            Instruction::I64Const(TAG_BITS), Instruction::I64Shl,
        ]
    }
}
