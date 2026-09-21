    /// (json-quote x) — tag-aware JSON scalar encoder (2026-08-30).
    /// STR → "…" with escapes (\" \\ \n \t \r, control → \u00XY)
    /// other tags → __to_string (tagged in, tagged out)
    /// Over-allocates 6*len+2 (worst case \u00XY per byte); actual length
    /// is w − dst after the write pass — no counting pass needed.
    pub(crate) fn json_quote_emit(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        use wasm_encoder::Instruction as I;
        if a.len() != 1 {
            return Err("json-quote: expected 1 arg".into());
        }
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let v_i = self.local_idx("__jq_v");
        let p_i = self.local_idx("__jq_p");
        let len_i = self.local_idx("__jq_len");
        let ptr_i = self.local_idx("__jq_ptr");
        let i_i = self.local_idx("__jq_i");
        let c_i = self.local_idx("__jq_c");
        let q_i = self.local_idx("__jq_q"); // alloc size
        let w_i = self.local_idx("__jq_w");
        let dst_i = self.local_idx("__jq_dst");
        let ts_idx = self.ensure_to_string_func();
        let mut v = Vec::new();
        // eval arg once
        v.extend(self.expr(&a[0])?);
        v.push(I::LocalSet(v_i));
        // runtime dispatch on tag
        v.push(I::Block(BlockType::Result(ValType::I64)));
        v.push(I::LocalGet(v_i));
        v.push(I::I64Const(5)); // TAG_STR
        v.push(I::I64And);
        v.push(I::I64Const(5));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Result(ValType::I64)));
        // ── STR path ──
        v.push(I::LocalGet(v_i));
        v.push(I::I64Const(3));
        v.push(I::I64ShrU); // payload
        v.push(I::LocalSet(p_i));
        v.push(I::LocalGet(p_i));
        v.push(I::I64Const(32));
        v.push(I::I64ShrU);
        v.push(I::LocalSet(len_i));
        v.push(I::LocalGet(p_i));
        v.push(I::I64Const(0xFFFFFFFF));
        v.push(I::I64And);
        v.push(I::LocalSet(ptr_i));
        // q = 6*len + 2 ; dst = rtheap_alloc(q) ; w = dst
        v.push(I::LocalGet(len_i));
        v.push(I::I64Const(6));
        v.push(I::I64Mul);
        v.push(I::I64Const(2));
        v.push(I::I64Add);
        v.push(I::LocalSet(q_i));
        v.extend(self.emit_rtheap_alloc(dst_i, q_i));
        v.push(I::LocalGet(dst_i));
        v.push(I::LocalSet(w_i));
        // write '"'
        v.push(I::LocalGet(w_i));
        v.push(I::I32WrapI64);
        v.push(I::I64Const(0x22));
        v.push(I::I32WrapI64);
        v.push(I::I64Store8(ma8.clone()));
        v.push(I::LocalGet(w_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(w_i));
        // write-escape loop
        v.push(I::I64Const(0));
        v.push(I::LocalSet(i_i));
        v.push(I::Block(BlockType::Empty));
        v.push(I::Loop(BlockType::Empty));
        v.push(I::LocalGet(i_i));
        v.push(I::LocalGet(len_i));
        v.push(I::I64GeU);
        v.push(I::BrIf(1));
        // c = load8(ptr + i)
        v.push(I::LocalGet(ptr_i));
        v.push(I::I32WrapI64);
        v.push(I::LocalGet(i_i));
        v.push(I::I32WrapI64);
        v.push(I::I32Add);
        v.push(I::I32Load8U(ma8.clone()));
        v.push(I::I64ExtendI32U);
        v.push(I::LocalSet(c_i));
        // c == '"' → \"
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x22));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x22));
        v.push(I::Else);
        // c == '\' → \\
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x5C));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x5C));
        v.push(I::Else);
        // c == '\n' → \n
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x0A));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x6E));
        v.push(I::Else);
        // c == '\t' → \t
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x09));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x74));
        v.push(I::Else);
        // c == '\r' → \r
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x0D));
        v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x72));
        v.push(I::Else);
        // c < 0x20 → \u00XY
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x20));
        v.push(I::I64LtU);
        v.push(I::If(BlockType::Empty));
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x5C, 0x75)); // \u
        v.extend(wr2(&mut vec![], w_i, ma8.clone(), 0x30, 0x30)); // 00
        // hi nibble: 48 + d + (d>=10)*39
        v.push(I::LocalGet(w_i));
        v.push(I::I32WrapI64);
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(4));
        v.push(I::I64ShrU);
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(4));
        v.push(I::I64ShrU);
        v.push(I::I64Const(10));
        v.push(I::I64GeU);
        v.push(I::I64Const(39));
        v.push(I::I64Mul);
        v.push(I::I64Add);
        v.push(I::I64Const(48));
        v.push(I::I64Add);
        v.push(I::I32WrapI64);
        v.push(I::I64Store8(ma8.clone()));
        v.push(I::LocalGet(w_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(w_i));
        // lo nibble
        v.push(I::LocalGet(w_i));
        v.push(I::I32WrapI64);
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x0F));
        v.push(I::I64And);
        v.push(I::LocalGet(c_i));
        v.push(I::I64Const(0x0F));
        v.push(I::I64And);
        v.push(I::I64Const(10));
        v.push(I::I64GeU);
        v.push(I::I64Const(39));
        v.push(I::I64Mul);
        v.push(I::I64Add);
        v.push(I::I64Const(48));
        v.push(I::I64Add);
        v.push(I::I32WrapI64);
        v.push(I::I64Store8(ma8.clone()));
        v.push(I::LocalGet(w_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(w_i));
        v.push(I::Else);
        // raw byte
        v.push(I::LocalGet(w_i));
        v.push(I::I32WrapI64);
        v.push(I::LocalGet(c_i));
        v.push(I::I32WrapI64);
        v.push(I::I64Store8(ma8.clone()));
        v.push(I::LocalGet(w_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(w_i));
        for _ in 0..6 {
            v.push(I::End);
        }
        v.push(I::LocalGet(i_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(i_i));
        v.push(I::Br(0));
        v.push(I::End); // loop
        v.push(I::End); // block
        // write closing '"'
        v.push(I::LocalGet(w_i));
        v.push(I::I32WrapI64);
        v.push(I::I64Const(0x22));
        v.push(I::I32WrapI64);
        v.push(I::I64Store8(ma8.clone()));
        v.push(I::LocalGet(w_i));
        v.push(I::I64Const(1));
        v.push(I::I64Add);
        v.push(I::LocalSet(w_i));
        // result: ((w - dst) << 32 | dst) << 3 | 5
        v.push(I::LocalGet(w_i));
        v.push(I::LocalGet(dst_i));
        v.push(I::I64Sub);
        v.push(I::I64Const(32));
        v.push(I::I64Shl);
        v.push(I::LocalGet(dst_i));
        v.push(I::I64Or);
        v.push(I::I64Const(TAG_BITS));
        v.push(I::I64Shl);
        v.push(I::I64Const(5));
        v.push(I::I64Or);
        v.push(I::Else);
        // ── non-STR: __to_string(v) ──
        v.push(I::LocalGet(v_i));
        v.push(I::Call(ts_idx as u32));
        v.push(I::End); // if
        v.push(I::End); // block
        Ok(v)
    }
