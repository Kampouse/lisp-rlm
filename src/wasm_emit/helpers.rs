use super::*;

impl WasmEmitter {
    pub(crate) fn emit_tag(&self, tag: i64) -> Vec<Instruction<'static>> {
        vec![
            Instruction::I64Const(TAG_BITS),
            Instruction::I64Shl,
            Instruction::I64Const(tag),
            Instruction::I64Or,
        ]
    }

    pub(crate) fn emit_untag(&self) -> Vec<Instruction<'static>> {
        vec![Instruction::I64Const(TAG_BITS), Instruction::I64ShrU]
    }

    pub(crate) fn emit_i32_to_i64(&self) -> Vec<Instruction<'static>> {
        vec![Instruction::I64ExtendI32U]
    }

    pub(crate) fn emit_num_coerce(&mut self) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__coerce_tmp");
        let result = self.local_idx("__coerce_result");
        let v = vec![
            Instruction::LocalSet(tmp), // save val
            Instruction::LocalGet(tmp),
            Instruction::I64Const(7), // mask tag bits
            Instruction::I64And,
            Instruction::I64Const(TAG_NUM),
            Instruction::I64Eq, // is it TAG_NUM? (i32 on stack)
            Instruction::If(BlockType::Empty),
            Instruction::LocalGet(tmp),
            Instruction::I64Const(TAG_BITS),
            Instruction::I64ShrU, // untag payload
            Instruction::LocalSet(result),
            Instruction::Else,
            Instruction::I64Const(0), // non-numeric → 0
            Instruction::LocalSet(result),
            Instruction::End,
            Instruction::LocalGet(result),
        ];
        v
    }

    pub(crate) fn emit_tag_num(&self) -> Vec<Instruction<'static>> {
        self.emit_tag(TAG_NUM)
    }

    pub(crate) fn emit_safe_div(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__div_a");
        let b = self.local_idx("__div_b");
        vec![
            // Pop b then a into locals
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            // Check if b == 0
            Instruction::LocalGet(b),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Result(ValType::I64)),
            // b is zero → trap (division by zero)
            Instruction::Unreachable,
            Instruction::Else,
            // b is non-zero → do the division
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64DivS,
            Instruction::End,
        ]
    }

    pub(crate) fn emit_safe_rem(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__rem_a");
        let b = self.local_idx("__rem_b");
        let result = self.local_idx("__rem_result");
        vec![
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            Instruction::LocalGet(b),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Empty),
            // b is zero → trap (division by zero in mod)
            Instruction::Unreachable,
            Instruction::Else,
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64RemS,
            Instruction::LocalSet(result),
            // Euclidean fixup: if result < 0, add |b| to make it non-negative
            Instruction::LocalGet(result),
            Instruction::I64Const(0),
            Instruction::I64LtS,
            Instruction::If(BlockType::Empty),
            Instruction::LocalGet(result),
            Instruction::LocalGet(b),
            Instruction::I64Const(0),
            Instruction::I64LtS,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(0),
            Instruction::LocalGet(b),
            Instruction::I64Sub,
            Instruction::Else,
            Instruction::LocalGet(b),
            Instruction::End,
            Instruction::I64Add,
            Instruction::LocalSet(result),
            Instruction::End,
            Instruction::End,
            Instruction::LocalGet(result),
        ]
    }

    pub(crate) fn emit_checked_add(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__ck_add_a");
        let b = self.local_idx("__ck_add_b");
        let r = self.local_idx("__ck_add_r");
        // Overflow: (a ^ b) >= 0 && (r ^ a) < 0
        vec![
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64Add,
            Instruction::LocalSet(r),
            // Check: same-sign inputs
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64Xor,
            Instruction::I64Const(0),
            Instruction::I64GeS, // (a^b) >= 0 → same sign
            Instruction::If(BlockType::Empty),
            // Same sign: check if result flipped
            Instruction::LocalGet(r),
            Instruction::LocalGet(a),
            Instruction::I64Xor,
            Instruction::I64Const(0),
            Instruction::I64LtS, // (r^a) < 0 → overflow
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable, // trap on overflow
            Instruction::End,
            Instruction::End,
            Instruction::LocalGet(r),
        ]
    }

    pub(crate) fn emit_checked_sub(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__ck_sub_a");
        let b = self.local_idx("__ck_sub_b");
        let r = self.local_idx("__ck_sub_r");
        // Overflow: (a ^ b) < 0 && (r ^ a) < 0
        vec![
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64Sub,
            Instruction::LocalSet(r),
            // Check: different-sign inputs
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64Xor,
            Instruction::I64Const(0),
            Instruction::I64LtS, // (a^b) < 0 → different sign
            Instruction::If(BlockType::Empty),
            // Different sign: check if result flipped
            Instruction::LocalGet(r),
            Instruction::LocalGet(a),
            Instruction::I64Xor,
            Instruction::I64Const(0),
            Instruction::I64LtS, // (r^a) < 0 → overflow
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable, // trap on overflow
            Instruction::End,
            Instruction::End,
            Instruction::LocalGet(r),
        ]
    }

    pub(crate) fn emit_checked_mul(&mut self) -> Vec<Instruction<'static>> {
        let a = self.local_idx("__ck_mul_a");
        let b = self.local_idx("__ck_mul_b");
        let r = self.local_idx("__ck_mul_r");
        vec![
            Instruction::LocalSet(b),
            Instruction::LocalSet(a),
            // Special case: a == 0 or b == 0 → result is 0, no overflow
            Instruction::LocalGet(a),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(0),
            Instruction::Else,
            Instruction::LocalGet(b),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(0),
            Instruction::Else,
            Instruction::LocalGet(a),
            Instruction::LocalGet(b),
            Instruction::I64Mul,
            Instruction::LocalSet(r),
            // Check: r / b == a (with b != -1 edge case)
            // If b == -1: overflow only if a == i64::MIN
            Instruction::LocalGet(b),
            Instruction::I64Const(-1),
            Instruction::I64Eq,
            Instruction::If(BlockType::Empty),
            // b == -1: overflow iff a == i64::MIN
            Instruction::LocalGet(a),
            Instruction::I64Const(i64::MIN),
            Instruction::I64Eq,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            Instruction::Else,
            // General: r / b != a → overflow
            Instruction::LocalGet(r),
            Instruction::LocalGet(b),
            Instruction::I64DivS,
            Instruction::LocalGet(a),
            Instruction::I64Ne,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            Instruction::End,
            Instruction::LocalGet(r),
            Instruction::End,
            Instruction::End,
        ]
    }

    pub(crate) fn emit_tag_bool(&self) -> Vec<Instruction<'static>> {
        self.emit_tag(TAG_BOOL)
    }

    pub(crate) fn emit_tag_str(&self) -> Vec<Instruction<'static>> {
        self.emit_tag(TAG_STR)
    }

    pub(crate) fn emit_tag_array(&self) -> Vec<Instruction<'static>> {
        self.emit_tag(TAG_ARRAY)
    }

    pub(crate) fn emit_tagged_const(&self, val: i64, tag: i64) -> Vec<Instruction<'static>> {
        vec![Instruction::I64Const((val << TAG_BITS) | tag)]
    }

    pub(crate) fn emit_str_eq(&mut self) -> Vec<Instruction<'static>> {
        let a_raw = self.local_idx("__seq_a");
        let b_raw = self.local_idx("__seq_b");
        let a_len = self.local_idx("__seq_alen");
        let a_ptr = self.local_idx("__seq_aptr");
        let b_ptr = self.local_idx("__seq_bptr");
        let n_words = self.local_idx("__seq_nw");
        let tail_len = self.local_idx("__seq_tl");
        let i = self.local_idx("__seq_i");
        let wa = self.local_idx("__seq_wa");
        let wb = self.local_idx("__seq_wb");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        vec![
            // Untag both → raw (ptr | len << 32)
            Instruction::I64Const(TAG_BITS),
            Instruction::I64ShrU,
            Instruction::LocalSet(a_raw),
            Instruction::I64Const(TAG_BITS),
            Instruction::I64ShrU,
            Instruction::LocalSet(b_raw),
            // Fast path: if raw_a == raw_b → true (same pointer + same length)
            Instruction::LocalGet(a_raw),
            Instruction::LocalGet(b_raw),
            Instruction::I64Eq,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(8), // tagged true
            Instruction::Else,
            // Compare lengths: len = raw >> 32
            Instruction::LocalGet(a_raw),
            Instruction::I64Const(32),
            Instruction::I64ShrU,
            Instruction::LocalSet(a_len),
            // If lengths differ → false
            Instruction::LocalGet(a_len),
            Instruction::LocalGet(b_raw),
            Instruction::I64Const(32),
            Instruction::I64ShrU,
            Instruction::I64Ne,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(1), // tagged false
            Instruction::Else,
            // Extract pointers: ptr = raw & 0xFFFFFFFF
            Instruction::LocalGet(a_raw),
            Instruction::I64Const(0xFFFFFFFF),
            Instruction::I64And,
            Instruction::LocalSet(a_ptr),
            Instruction::LocalGet(b_raw),
            Instruction::I64Const(0xFFFFFFFF),
            Instruction::I64And,
            Instruction::LocalSet(b_ptr),
            // n_words = len / 8, tail_len = len % 8
            Instruction::LocalGet(a_len),
            Instruction::I64Const(3),
            Instruction::I64ShrU,
            Instruction::LocalSet(n_words),
            Instruction::LocalGet(a_len),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::LocalSet(tail_len),
            // Word comparison loop: i = 0..n_words
            Instruction::I64Const(0),
            Instruction::LocalSet(i),
            Instruction::Block(BlockType::Result(ValType::I64)), // $break
            Instruction::Loop(BlockType::Empty),                 // $loop
            // if i >= n_words → done with full words, check tail
            Instruction::LocalGet(i),
            Instruction::LocalGet(n_words),
            Instruction::I64GeU,
            Instruction::If(BlockType::Empty),
            // Tail comparison: if tail_len == 0 → equal
            Instruction::LocalGet(tail_len),
            Instruction::I64Eqz,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(8), // tagged true
            Instruction::Else,
            // Load last word from a (at ptr + n_words*8), overlapping is fine
            Instruction::LocalGet(a_ptr),
            Instruction::LocalGet(n_words),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8.clone()),
            Instruction::LocalSet(wa),
            // Load last word from b
            Instruction::LocalGet(b_ptr),
            Instruction::LocalGet(n_words),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8.clone()),
            Instruction::LocalSet(wb),
            // Mask: (1 << (tail_len * 8)) - 1
            Instruction::I64Const(1),
            Instruction::LocalGet(tail_len),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Shl,
            Instruction::I64Const(1),
            Instruction::I64Sub,
            // Apply mask to wa and wb, compare
            Instruction::LocalGet(wa),
            Instruction::I64And,
            Instruction::LocalGet(wb),
            Instruction::I64Const(1),
            Instruction::LocalGet(tail_len),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Shl,
            Instruction::I64Const(1),
            Instruction::I64Sub,
            Instruction::I64And,
            Instruction::I64Eq,
            Instruction::If(BlockType::Result(ValType::I64)),
            Instruction::I64Const(8), // tagged true
            Instruction::Else,
            Instruction::I64Const(1), // tagged false
            Instruction::End,
            Instruction::End,
            Instruction::Br(2), // break out of Block with result
            Instruction::End,
            // Load word from a: mem[a_ptr + i*8]
            Instruction::LocalGet(a_ptr),
            Instruction::LocalGet(i),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8.clone()),
            Instruction::LocalSet(wa),
            // Load word from b: mem[b_ptr + i*8]
            Instruction::LocalGet(b_ptr),
            Instruction::LocalGet(i),
            Instruction::I64Const(8),
            Instruction::I64Mul,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8),
            Instruction::LocalSet(wb),
            // if wa != wb → not equal
            Instruction::LocalGet(wa),
            Instruction::LocalGet(wb),
            Instruction::I64Ne,
            Instruction::If(BlockType::Empty),
            Instruction::I64Const(1), // tagged false
            Instruction::Br(2),       // break out of Block with false
            Instruction::End,
            // i++, continue loop
            Instruction::LocalGet(i),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::LocalSet(i),
            Instruction::Br(0),       // continue
            Instruction::End,         // loop
            Instruction::Unreachable, // unreachable
            Instruction::End,         // block
            Instruction::End,         // if lengths differ
            Instruction::End,         // if raw_a == raw_b (fast path)
        ]
    }

    pub(crate) fn emit_safe_store8(&mut self) -> Vec<Instruction<'static>> {
        let addr = self.local_idx("__sb_addr");
        let byte = self.local_idx("__sb_byte");
        let word = self.local_idx("__sb_word");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        vec![
            Instruction::LocalSet(byte), // save byte (i32)
            Instruction::LocalSet(addr), // save addr (i64)
            // Align addr down to 8-byte boundary: aligned = addr & ~7
            Instruction::LocalGet(addr),
            Instruction::I64Const(!7i64 as i64), // 0xFFFFFFFFFFFFFFF8
            Instruction::I64And,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8.clone()),
            Instruction::LocalSet(word),
            // Compute shift amount: (addr & 7) * 8
            Instruction::LocalGet(addr),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::I64Const(3),
            Instruction::I64Shl, // shift_amount
            // Create mask: ~(0xFF << shift_amount)
            Instruction::I64Const(-1), // 0xFFFFFFFFFFFFFFFF
            Instruction::LocalGet(addr),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Const(8),
            Instruction::I64Shl, // 0xFF << shift_amount
            Instruction::I64Xor, // ~(0xFF << shift_amount) via XOR with -1
            // Mask out the old byte: word & mask
            Instruction::LocalGet(word),
            Instruction::I64And,
            // OR in new byte at the right position
            Instruction::LocalGet(byte),
            Instruction::I64ExtendI32U,
            Instruction::LocalGet(addr),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Shl,
            Instruction::I64Or,
            // Store back
            Instruction::LocalGet(addr),
            Instruction::I64Const(!7i64 as i64),
            Instruction::I64And,
            Instruction::I32WrapI64,
            Instruction::I64Store(ma8),
        ]
    }

    /// Load a single byte from a given address.
    /// Stack: [i64 addr]  →  [i64 byte_value (0-255)]

    pub(crate) fn emit_safe_load8(&mut self) -> Vec<Instruction<'static>> {
        let addr = self.local_idx("__lb_addr");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        vec![
            Instruction::LocalSet(addr),
            // Align addr down to 8-byte boundary
            Instruction::LocalGet(addr),
            Instruction::I64Const(!7i64 as i64),
            Instruction::I64And,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8),
            // Shift right by (addr & 7) * 8 bits
            Instruction::LocalGet(addr),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64ShrU,
            // Mask to get just the byte
            Instruction::I64Const(0xFF),
            Instruction::I64And,
        ]
    }

    /// Copy `n_bytes` bytes from src to dst using 8-byte word operations.
    /// Src and dst addresses are already in locals `src_local` and `dst_local`.
    /// Uses self.local_idx() for temporary locals.

    pub(crate) fn emit_word_copy(
        &mut self,
        n_bytes: u64,
        src_local: u32,
        dst_local: u32,
    ) -> Vec<Instruction<'static>> {
        let _ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let mut v = Vec::new();
        let full_words = n_bytes / 8;
        let tail = n_bytes % 8;

        // Copy full 8-byte words
        for i in 0..full_words {
            let off = (i * 8) as u64;
            let ma_off = wasm_encoder::MemArg {
                offset: off,
                align: 3,
                memory_index: 0,
            };
            // Load word from src + off
            v.push(Instruction::LocalGet(src_local));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Load(ma_off.clone()));
            // Store word to dst + off
            v.push(Instruction::LocalGet(dst_local));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Store(ma_off));
        }

        // Copy tail bytes: load last word from src (at src + full_words*8)
        // and store masked to dst (at dst + full_words*8)
        if tail > 0 {
            let tail_off = (full_words * 8) as u64;
            let w = self.local_idx("__wc_word");
            let ma_tail = wasm_encoder::MemArg {
                offset: tail_off,
                align: 0,
                memory_index: 0,
            };
            // Load last word from src
            v.push(Instruction::LocalGet(src_local));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Load(ma_tail.clone()));
            v.push(Instruction::LocalSet(w));
            // If we also wrote full words, we need to mask the tail
            // to avoid overwriting bytes beyond the string in dst.
            // Mask: (1 << (tail * 8)) - 1
            // First, load the existing bytes at dst tail position
            // Load existing word at dst tail position and merge
            v.push(Instruction::LocalGet(dst_local));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Load(ma_tail.clone()));
            // Keep upper bytes (above tail) from dst: dst_word & ~((1 << tail_bits) - 1)
            v.push(Instruction::I64Const(-1));
            v.push(Instruction::I64Const((1i64 << (tail * 8)) - 1));
            v.push(Instruction::I64Xor); // ~tail_mask
            v.push(Instruction::I64And);
            // OR in src tail bytes (masked)
            v.push(Instruction::LocalGet(w));
            v.push(Instruction::I64Const((1i64 << (tail * 8)) - 1));
            v.push(Instruction::I64And);
            v.push(Instruction::I64Or);
            // Store
            v.push(Instruction::LocalGet(dst_local));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Store(ma_tail));
        }

        v
    }

    pub(crate) fn emit_runtime_word_copy(
        &mut self,
        src_local: u32,
        dst_local: u32,
        len_local: u32,
    ) -> Vec<Instruction<'static>> {
        let ma0 = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let wc = self.local_idx("__rwc_words");
        let tl = self.local_idx("__rwc_tail");
        let wi = self.local_idx("__rwc_i");
        let tmp = self.local_idx("__rwc_tmp");
        let src_i = self.local_idx("__rwc_src");
        let dst_i = self.local_idx("__rwc_dst");

        vec![
            // Save base pointers
            Instruction::LocalGet(src_local),
            Instruction::LocalSet(src_i),
            Instruction::LocalGet(dst_local),
            Instruction::LocalSet(dst_i),
            // Compute word count and tail
            Instruction::LocalGet(len_local),
            Instruction::I64Const(3),
            Instruction::I64ShrU,
            Instruction::LocalSet(wc),
            Instruction::LocalGet(len_local),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::LocalSet(tl),
            // Copy full words
            Instruction::I64Const(0),
            Instruction::LocalSet(wi),
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            Instruction::LocalGet(wi),
            Instruction::LocalGet(wc),
            Instruction::I64GeU,
            Instruction::BrIf(1),
            // Load 8 bytes from src + i*8
            Instruction::LocalGet(src_i),
            Instruction::LocalGet(wi),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma0.clone()),
            // Store to dst + i*8
            Instruction::LocalGet(dst_i),
            Instruction::LocalGet(wi),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Store(ma0.clone()),
            // i++
            Instruction::LocalGet(wi),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::LocalSet(wi),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
            // Tail: if tail > 0, load word from src+full*8, mask, merge with dst+full*8, store
            Instruction::LocalGet(tl),
            Instruction::I64Const(0),
            Instruction::I64Eq,
            Instruction::BrIf(0),
            // Only do tail if there are words before it (otherwise just store directly — fresh allocation)
            Instruction::LocalGet(wc),
            Instruction::I64Const(0),
            Instruction::I64Eq,
            Instruction::If(BlockType::Empty),
            // No full words: just load src word and store to dst
            Instruction::LocalGet(src_i),
            Instruction::LocalGet(wc),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma0.clone()),
            Instruction::I64Const((1i64 << 56) - 1), // mask for up to 7 bytes — we'll mask properly below
            Instruction::I64And, // rough mask — may include extra bytes but fresh alloc so OK
            Instruction::LocalSet(tmp),
            Instruction::LocalGet(dst_i),
            Instruction::LocalGet(wc),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::LocalGet(tmp),
            Instruction::I64Store(ma0.clone()),
            Instruction::Else,
            // Has full words: read-modify-write to preserve upper bytes in dst
            // Load existing dst word at tail offset
            Instruction::LocalGet(dst_i),
            Instruction::LocalGet(wc),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma0.clone()),
            // Mask out tail portion: keep upper bytes only
            Instruction::I64Const(-1),
            // Compute ~(1 << (tl * 8) - 1) = mask for upper bytes
            // Actually: upper_mask = -1 ^ ((1 << (tl*8)) - 1)
            // But tl is runtime... so compute dynamically
            Instruction::I64Const(1),
            Instruction::LocalGet(tl),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Shl, // 1 << (tl*8)
            Instruction::I64Const(1),
            Instruction::I64Sub, // (1 << tl*8) - 1 = tail_mask
            Instruction::I64Xor, // ~tail_mask = upper_mask
            Instruction::I64And, // dst_word & upper_mask
            // Load src word at tail offset and mask tail bytes only
            Instruction::LocalGet(src_i),
            Instruction::LocalGet(wc),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Load(ma0.clone()),
            Instruction::I64Const(1),
            Instruction::LocalGet(tl),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Shl,
            Instruction::I64Const(1),
            Instruction::I64Sub,
            Instruction::I64And, // src_word & tail_mask
            // Merge
            Instruction::I64Or,
            // Store
            Instruction::LocalGet(dst_i),
            Instruction::LocalGet(wc),
            Instruction::I64Const(3),
            Instruction::I64Shl,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I64Store(ma0),
            Instruction::End, // if
        ]
    }

    pub(crate) fn emit_is_truthy(&mut self) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__truthy_tmp");
        let tag = self.local_idx("__truthy_tag");
        let len = self.local_idx("__truthy_len");
        vec![
            Instruction::LocalSet(tmp), // save tagged val
            // Check val == 1 (Bool false)
            Instruction::LocalGet(tmp),
            Instruction::I64Const(1),
            Instruction::I64Eq, // → i32
            // Check val == 4 (Nil)
            Instruction::LocalGet(tmp),
            Instruction::I64Const(TAGGED_NIL),
            Instruction::I64Eq, // → i32
            Instruction::I32Or, // → i32
            // Check empty TAG_STR (tag==5 && len==0)
            Instruction::LocalGet(tmp),
            Instruction::I64Const(7),
            Instruction::I64And,
            Instruction::LocalSet(tag),
            Instruction::LocalGet(tag),
            Instruction::I64Const(TAG_STR),
            Instruction::I64Eq, // → i32
            Instruction::LocalGet(tmp),
            Instruction::I64Const(35), // >> 3 + >> 32 equivalent: bits 35+
            Instruction::I64ShrU,
            Instruction::I64Const(0xFFFFFFFF),
            Instruction::I64And,
            Instruction::LocalSet(len),
            Instruction::LocalGet(len),
            Instruction::I64Eqz, // len == 0 → i32
            Instruction::I32And, // tag==5 AND len==0
            Instruction::I32Or, // falsy if nil OR empty-str
            // invert: 0 → truthy, 1 → falsy
            Instruction::I32Eqz,        // → i32
            Instruction::I64ExtendI32U, // → i64 for callers
        ]
    }

    pub(crate) fn emit_cond_branch(&mut self) -> Vec<Instruction<'static>> {
        let mut v = self.emit_is_truthy();
        v.push(Instruction::I32WrapI64); // i64 → i32 for If
        v
    }

    pub(crate) fn emit_str_concat(&mut self) -> Vec<Instruction<'static>> {
        let a_local = self.local_idx("__str_a");
        let b_local = self.local_idx("__str_b");
        let a_raw = self.local_idx("__str_araw");
        let b_raw = self.local_idx("__str_braw");
        let a_off = self.local_idx("__str_aoff");
        let a_len = self.local_idx("__str_alen");
        let b_off = self.local_idx("__str_boff");
        let b_len = self.local_idx("__str_blen");
        let dst = self.local_idx("__str_dst");
        let i = self.local_idx("__str_i");
        let total = self.local_idx("__str_total");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };

        // Allocate buffer after data segments (128 bytes is plenty for keys)
        let alloc_base = self.next_data_offset.max(3072);
        self.next_data_offset = (alloc_base + 128 + 7) & !7;

        let mut v = vec![
            // Save tagged args: stack has [... b a] (b on top)
            Instruction::LocalSet(b_local),
            Instruction::LocalSet(a_local),
        ];

        // --- Extract A: untag → packed (len<<32|ptr) → extract len and ptr ---
        v.extend(vec![
            Instruction::LocalGet(a_local),
            Instruction::I64Const(TAG_BITS),
            Instruction::I64ShrU, // raw packed
            Instruction::LocalSet(a_raw),
            // len_a = raw >> 32
            Instruction::LocalGet(a_raw),
            Instruction::I64Const(32),
            Instruction::I64ShrU,
            Instruction::LocalSet(a_len),
            // ptr_a = raw & 0xFFFFFFFF
            Instruction::LocalGet(a_raw),
            Instruction::I64Const(0xFFFFFFFF),
            Instruction::I64And,
            Instruction::LocalSet(a_off),
        ]);

        // --- Extract B similarly ---
        v.extend(vec![
            Instruction::LocalGet(b_local),
            Instruction::I64Const(TAG_BITS),
            Instruction::I64ShrU,
            Instruction::LocalSet(b_raw),
            Instruction::LocalGet(b_raw),
            Instruction::I64Const(32),
            Instruction::I64ShrU,
            Instruction::LocalSet(b_len),
            Instruction::LocalGet(b_raw),
            Instruction::I64Const(0xFFFFFFFF),
            Instruction::I64And,
            Instruction::LocalSet(b_off),
        ]);

        // --- Copy A to dst ---
        v.extend(vec![
            Instruction::I64Const(alloc_base as i64),
            Instruction::LocalSet(dst),
            Instruction::I64Const(0),
            Instruction::LocalSet(i),
            // block $exit_a
            Instruction::Block(BlockType::Empty),
            // loop $copy_a
            Instruction::Loop(BlockType::Empty),
            // if i >= len_a → break
            Instruction::LocalGet(i),
            Instruction::LocalGet(a_len),
            Instruction::I64GeS,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            // dst[i] = a_off[i]
            Instruction::LocalGet(dst),
            Instruction::LocalGet(i),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::LocalGet(a_off),
            Instruction::LocalGet(i),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I32Load8U(ma8),
            Instruction::I32Store8(ma8),
            // i++
            Instruction::LocalGet(i),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::LocalSet(i),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
        ]);

        // --- Copy B to dst + len_a ---
        v.extend(vec![
            Instruction::I64Const(0),
            Instruction::LocalSet(i),
            Instruction::Block(BlockType::Empty),
            Instruction::Loop(BlockType::Empty),
            Instruction::LocalGet(i),
            Instruction::LocalGet(b_len),
            Instruction::I64GeS,
            Instruction::If(BlockType::Empty),
            Instruction::Br(2),
            Instruction::End,
            // dst[len_a + i] = b_off[i]
            Instruction::LocalGet(dst),
            Instruction::LocalGet(a_len),
            Instruction::LocalGet(i),
            Instruction::I64Add,
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::LocalGet(b_off),
            Instruction::LocalGet(i),
            Instruction::I64Add,
            Instruction::I32WrapI64,
            Instruction::I32Load8U(ma8),
            Instruction::I32Store8(ma8),
            Instruction::LocalGet(i),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::LocalSet(i),
            Instruction::Br(0),
            Instruction::End, // loop
            Instruction::End, // block
        ]);

        // --- Build tagged result: tag_str((total_len << 32) | dst) ---
        v.extend(vec![
            // total = len_a + len_b
            Instruction::LocalGet(a_len),
            Instruction::LocalGet(b_len),
            Instruction::I64Add,
            Instruction::LocalSet(total),
            // packed = (total << 32) | dst
            Instruction::LocalGet(total),
            Instruction::I64Const(32),
            Instruction::I64Shl,
            Instruction::LocalGet(dst),
            Instruction::I64Or,
        ]);
        v.extend(self.emit_tag_str());
        v
    }

    pub(crate) fn alloc_data(&mut self, bytes: &[u8]) -> u32 {
        // Dedup: reuse existing data segment with same bytes
        if let Some((off, _)) = self
            .data_segments
            .iter()
            .find(|(_, existing)| existing == bytes)
        {
            return *off;
        }
        let off = self.next_data_offset;
        self.data_segments.push((off, bytes.to_vec()));
        self.next_data_offset += bytes.len() as u32;
        self.next_data_offset = (self.next_data_offset + 7) & !7;
        off
    }

    pub(crate) fn process_hex_escapes(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            if input[i] == b'\\' && i + 1 < input.len() {
                let c = input[i + 1];
                match c {
                    b'x' if i + 3 < input.len() => {
                        let hi = input[i + 2];
                        let lo = input[i + 3];
                        let hex_val = |b: u8| -> Option<u8> {
                            if b.is_ascii_digit() {
                                Some(b - b'0')
                            } else if (b'A'..=b'F').contains(&b) {
                                Some(b - b'A' + 10)
                            } else if (b'a'..=b'f').contains(&b) {
                                Some(b - b'a' + 10)
                            } else {
                                None
                            }
                        };
                        if let (Some(h), Some(l)) = (hex_val(hi), hex_val(lo)) {
                            out.push(h << 4 | l);
                            i += 4;
                            continue;
                        }
                    }
                    b'n' => {
                        out.push(b'\n');
                        i += 2;
                        continue;
                    }
                    b't' => {
                        out.push(b'\t');
                        i += 2;
                        continue;
                    }
                    b'r' => {
                        out.push(b'\r');
                        i += 2;
                        continue;
                    }
                    b'0' => {
                        out.push(0);
                        i += 2;
                        continue;
                    }
                    b'\\' => {
                        out.push(b'\\');
                        i += 2;
                        continue;
                    }
                    b'"' => {
                        out.push(b'"');
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            out.push(input[i]);
            i += 1;
        }
        out
    }

    pub(crate) fn emit_runtime_alloc(&mut self, n_bytes: i64) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__rha_tmp");
        let new_ptr = self.local_idx("__rha_new");
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let mem_limit = (self.memory_pages as i64) * 65536;
        let v = vec![
            // Read current runtime heap ptr
            Instruction::I64Const(RUNTIME_HEAP_PTR),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma),
            Instruction::LocalSet(tmp),
            // Compute new ptr
            Instruction::LocalGet(tmp),
            Instruction::I64Const(n_bytes),
            Instruction::I64Add,
            Instruction::LocalSet(new_ptr),
            // Guard: new_ptr must be < mem_limit (otherwise trap)
            Instruction::LocalGet(new_ptr),
            Instruction::I64Const(mem_limit),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            // OK: write back new ptr
            Instruction::I64Const(RUNTIME_HEAP_PTR),
            Instruction::I32WrapI64,
            Instruction::LocalGet(new_ptr),
            Instruction::I64Store(ma),
            Instruction::Else,
            // Overflow: trap
            Instruction::Unreachable,
            Instruction::End,
            // Return old ptr
            Instruction::LocalGet(tmp),
        ];
        v
    }

    pub(crate) fn need_host(&mut self, idx: usize) {
        self.host_needed.insert(idx);
    }

    pub(crate) fn host_call(idx: usize) -> Instruction<'static> {
        Instruction::Call(HOST_BASE | idx as u32)
    }

    // ── Memory safety helpers ──

    /// Tag validation: check that low 3 bits of value are a valid tag (0–6).
    /// Traps if tag bits == 7 (TAG_INVALID).
    /// Stack: [val] → [val]  (passes through unchanged, or traps)
    pub(crate) fn emit_tag_validate(&mut self) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__tv_tmp");
        vec![
            Instruction::LocalTee(tmp),
            Instruction::I64Const(TAG_INVALID), // 7
            Instruction::I64And,
            Instruction::I64Const(TAG_INVALID),
            Instruction::I64Ne, // valid if (val & 7) != 7
            Instruction::If(BlockType::Empty),
            Instruction::Else,
            Instruction::Unreachable, // invalid tag — trap
            Instruction::End,
            Instruction::LocalGet(tmp), // restore val
        ]
    }

    /// Recursion depth guard: increments depth counter, traps if >= MAX_DEPTH.
    /// Call at entry of every user function. Pairs with emit_depth_dec on return.
    pub(crate) fn emit_depth_inc(&mut self) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__di_tmp");
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        vec![
            // Read current depth
            Instruction::I64Const(DEPTH_COUNTER),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::LocalTee(tmp),
            // Check: depth < MAX_DEPTH
            Instruction::I64Const(MAX_DEPTH),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            // OK: increment and write back
            Instruction::I64Const(DEPTH_COUNTER),
            Instruction::I32WrapI64,
            Instruction::LocalGet(tmp),
            Instruction::I64Const(1),
            Instruction::I64Add,
            Instruction::I64Store(ma),
            Instruction::Else,
            Instruction::Unreachable, // stack overflow — trap
            Instruction::End,
        ]
    }

    /// Decrement recursion depth counter on function return.
    pub(crate) fn emit_depth_dec(&mut self) -> Vec<Instruction<'static>> {
        let _tmp = self.local_idx("__dd_tmp");
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        vec![
            // addr first (for I64Store: [i32 addr, i64 val])
            Instruction::I64Const(DEPTH_COUNTER),
            Instruction::I32WrapI64,
            // val: load current, subtract 1
            Instruction::I64Const(DEPTH_COUNTER),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma.clone()),
            Instruction::I64Const(1),
            Instruction::I64Sub,
            Instruction::I64Store(ma),
        ]
    }

    /// Raw memory write bounds check for mem-set!: verifies [addr, addr+8) doesn't
    /// overlap any protected region. Stack: [addr] → [addr] or trap.
    /// addr is UNTAGGED (already untagged by dispatch).
    pub(crate) fn emit_raw_write_bounds_check(&mut self) -> Vec<Instruction<'static>> {
        let addr = self.local_idx("__bc_addr");
        let mut v = vec![Instruction::LocalSet(addr)];
        let _ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        // For each protected region [start, end):
        //   if addr+8 > start AND addr < end → trap
        for &(start, end) in PROTECTED_REGIONS {
            v.push(Instruction::LocalGet(addr));
            v.push(Instruction::I64Const(8));
            v.push(Instruction::I64Add);
            v.push(Instruction::I64Const(start));
            v.push(Instruction::I64GtU); // addr+8 > start
            v.push(Instruction::LocalGet(addr));
            v.push(Instruction::I64Const(end));
            v.push(Instruction::I64LtU); // addr < end
            v.push(Instruction::I64And);
            v.push(Instruction::If(BlockType::Empty));
            v.push(Instruction::Unreachable); // protected region write — trap
            v.push(Instruction::End);
        }
        v.push(Instruction::LocalGet(addr));
        v
    }

    /// Ensure __to_string helper exists. Takes tagged i64, returns TAG_STR.
    /// Converts TAG_NUM->decimal, TAG_BOOL->"true"/"false", TAG_NIL->"nil".
    /// TAG_STR passes through unchanged.
    pub(crate) fn ensure_to_string_func(&mut self) -> u32 {
        if let Some(idx) = self.funcs.iter().position(|f| f.name == "__to_string") {
            return idx as u32;
        }
        let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let mut ins: Vec<Instruction<'static>> = Vec::new();
        // Locals: 1=tagged(i64), 2=tag(i32), 3=raw(i64), 4=heap(i64),
        //         5=buf(i32), 6=len(i32), 7=neg(i32), 8=digits(i32),
        //         9=widx(i32), 10=val(i64)
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I64Const(7));
        ins.push(Instruction::I64And);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(2));
        // TAG_STR(5) -> pass through
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(5));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::LocalSet(3));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(4));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        // "nil"
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Load(ma8));
        ins.push(Instruction::LocalTee(4));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalTee(5));
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Add);
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Store(ma8));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(110));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(105));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(108));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Or);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I64Const(1));
        ins.push(Instruction::I64Eq);
        ins.push(Instruction::If(BlockType::Empty));
        // "true"
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Load(ma8));
        ins.push(Instruction::LocalTee(4));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalTee(5));
        ins.push(Instruction::I64Const(4));
        ins.push(Instruction::I64Add);
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Store(ma8));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(116));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(114));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(117));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(3));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(101));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::I64Const(4));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Or);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        // "false"
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Load(ma8));
        ins.push(Instruction::LocalTee(4));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalTee(5));
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Add);
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Store(ma8));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(102));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(97));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(108));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(3));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(115));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(4));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(101));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Or);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Load(ma8));
        ins.push(Instruction::LocalTee(4));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalTee(5));
        ins.push(Instruction::I64Const(21));
        ins.push(Instruction::I64Add);
        ins.push(Instruction::I64Const(56));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Store(ma8));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::I64LtS);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I64Sub);
        ins.push(Instruction::LocalSet(10));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I32Const(45));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(8));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(7));
        ins.push(Instruction::Else);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::LocalSet(10));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(8));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(7));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(10));
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::I64Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(48));
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(10));
        ins.push(Instruction::LocalSet(3));
        ins.push(Instruction::Block(BlockType::Empty));
        ins.push(Instruction::Loop(BlockType::Empty));
        ins.push(Instruction::LocalGet(10));
        ins.push(Instruction::I64Const(10));
        ins.push(Instruction::I64DivU);
        ins.push(Instruction::LocalSet(10));
        ins.push(Instruction::LocalGet(10));
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::I64Eq);
        ins.push(Instruction::BrIf(1));
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(8));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(9));
        ins.push(Instruction::Block(BlockType::Empty));
        ins.push(Instruction::Loop(BlockType::Empty));
        ins.push(Instruction::LocalGet(9));
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::LocalGet(7));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32LtS);
        ins.push(Instruction::BrIf(1));
        ins.push(Instruction::LocalGet(9));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I64Const(10));
        ins.push(Instruction::I64RemU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I32Const(48));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Store8(ma0));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I64Const(10));
        ins.push(Instruction::I64DivU);
        ins.push(Instruction::LocalSet(3));
        ins.push(Instruction::LocalGet(9));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(9));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::End);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Or);
        self.funcs.push(FuncDef {
            name: "__to_string".to_string(),
            param_count: 1,
            local_count: 10,
            instrs: ins,
            local_entries: Some(vec![(1u32, ValType::I64), (1u32, ValType::I32), (2u32, ValType::I64), (5u32, ValType::I32), (1u32, ValType::I64)]),
        });
        (self.funcs.len() - 1) as u32
    }
}
