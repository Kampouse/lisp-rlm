// JSON helper methods for WasmEmitter
// These are inserted into the impl WasmEmitter block in wasm_emit.rs

    /// Emit WASM to read input JSON to INPUT_BUF, scan for "key": pattern, return i64 value.
    fn json_get_int(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7); self.need_host(0); self.need_host(1);
        let mut pattern = vec![b'"'];
        pattern.extend(key.as_bytes());
        pattern.extend_from_slice(b"\":");
        let pat_off = self.alloc_data(&pattern);
        let pat_len = pattern.len() as i64;
        let pos = self.local_idx("__js_pos");
        let ilen = self.local_idx("__js_ilen");
        let mi = self.local_idx("__js_mi");
        let jj = self.local_idx("__js_j");
        let res = self.local_idx("__js_res");
        let ng = self.local_idx("__js_ng");
        let dg = self.local_idx("__js_dg");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(7)); // input(0)
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len(0)
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(ib)); v.push(Self::host_call(0)); // read_register(0, ib)

        // pos = 0
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(pos));

        // Scan loop (block/loop)
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        // if pos + pat_len > ilen: break
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);

        // Assume match (mi=1), compare bytes
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        // Load input[ib+pos+j]
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        // Load pattern[pat_off+j]
        v.push(Instruction::I64Const(pat_off)); v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        // Compare
        v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); // match continues
        v.push(Instruction::Else); // mismatch
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End); // inner loop/block

        // If mi==1: found, break outer
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(3)); v.push(Instruction::End);
        // pos++
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End); // outer loop/block

        // pos at match. Value at pos + pat_len
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));

        // Skip whitespace
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End);
        v.push(Instruction::End); v.push(Instruction::End);

        // Skip quote if present
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End); v.push(Instruction::End);

        // Check negative
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x2D)); v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End); v.push(Instruction::End);

        // Parse digits
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(res));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(dg));
        // if dg < 0x30: break
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64LtS); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // if dg > 0x39: break
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // res = res*10 + (dg - 0x30)
        v.push(Instruction::LocalGet(res)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x30)); v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(res));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Apply negative
        v.push(Instruction::LocalGet(ng)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(res)); v.push(Instruction::I64Sub);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::End);
        Ok(v)
    }

    /// Emit WASM to read input JSON, scan for "key": "value", return packed string (ptr|len<<32).
    fn json_get_str(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7); self.need_host(0); self.need_host(1);
        let mut pattern = vec![b'"'];
        pattern.extend(key.as_bytes());
        pattern.extend_from_slice(b"\":");
        let pat_off = self.alloc_data(&pattern);
        let pat_len = pattern.len() as i64;
        let pos = self.local_idx("__jss_pos");
        let ilen = self.local_idx("__jss_ilen");
        let mi = self.local_idx("__jss_mi");
        let jj = self.local_idx("__jss_j");
        let slen = self.local_idx("__jss_slen");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(ib)); v.push(Self::host_call(0));

        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(pos));

        // Scan loop
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);

        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(pat_off)); v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(3)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Value at pos + pat_len
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));

        // Skip whitespace
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End);
        v.push(Instruction::End); v.push(Instruction::End);

        // Skip opening quote (the quote before the string value)
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));

        // Measure string length (scan until closing quote)
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(slen)); v.push(Instruction::I64Add); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I64Eq); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(slen)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Return packed: (slen << 32) | (ib + pos)
        v.push(Instruction::LocalGet(slen)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::I64Or);
        Ok(v)
    }

    /// Emit WASM to write {"result": <digits>} to INPUT_BUF and call value_return.
    fn json_return_int(&mut self, val_expr: Vec<Instruction<'static>>) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(25);
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let abs_val = self.local_idx("__jri_abs");
        let is_neg = self.local_idx("__jri_neg");
        let dc = self.local_idx("__jri_dc");
        let td = self.local_idx("__jri_td");
        let ptr = self.local_idx("__jri_ptr");
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Write prefix: {"result":  (with trailing space for padding-free int write)
        let prefix: &[u8] = b"{\"result\":  ";
        let prefix_len = prefix.len() as i64; // 12
        let prefix_off = self.alloc_data(prefix);

        // Copy prefix to INPUT_BUF
        let ci = self.local_idx("__jri_ci");
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        // addr = ib + ci
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        // val = load from prefix_off + ci
        v.push(Instruction::I64Const(prefix_off)); v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Write integer digits backwards from ib + prefix_len + 20
        v.extend(val_expr);
        v.push(Instruction::LocalSet(abs_val));

        // Check negative
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Const(0));
        v.push(Instruction::I64LtS); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(is_neg));
        v.push(Instruction::LocalGet(is_neg)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Sub);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::End);
        v.push(Instruction::LocalSet(abs_val));

        let digit_end = prefix_len + 21;
        v.push(Instruction::I64Const(digit_end)); v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(dc));

        // Handle 0
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Eqz); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(ptr)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x30)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(dc));
        v.push(Instruction::Else);

        // Digit loop
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Const(10));
        v.push(Instruction::I64RemS); v.push(Instruction::LocalSet(td));
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Const(10));
        v.push(Instruction::I64DivS); v.push(Instruction::LocalSet(abs_val));
        v.push(Instruction::LocalGet(ptr)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(td)); v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(dc)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dc));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);
        v.push(Instruction::End); // end else

        // Add minus sign
        v.push(Instruction::LocalGet(is_neg)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(ptr)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x2D)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(dc)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dc));
        v.push(Instruction::End);

        // Shift digits to position prefix_len
        let si = self.local_idx("__jri_si");
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(si));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(si)); v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        // Load byte from ib+ptr+si
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ptr)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(si)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        // Store to ib+prefix_len+si
        v.push(Instruction::I64Const(ib + prefix_len)); v.push(Instruction::LocalGet(si));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(si)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(si));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Write '}'
        v.push(Instruction::I64Const(ib + prefix_len)); v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(b'}' as i64)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        // total_len = prefix_len + dc + 1
        let tl = self.local_idx("__jri_tl");
        v.push(Instruction::I64Const(prefix_len)); v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Add); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(tl));

        // value_return(total_len, ib)
        v.push(Instruction::LocalGet(tl)); v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(25));

        v.push(Instruction::I64Const(1)); v.push(Instruction::GlobalSet(1));
        v.push(Instruction::I64Const(0));
        Ok(v)
    }

    /// Emit WASM to write {"result": "str"} to INPUT_BUF and call value_return.
    fn json_return_str(&mut self, packed_expr: Vec<Instruction<'static>>) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(25);
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let packed = self.local_idx("__jrs_packed");
        let str_ptr = self.local_idx("__jrs_ptr");
        let str_len = self.local_idx("__jrs_len");
        let ci = self.local_idx("__jrs_ci");
        let mut v = Vec::new();

        // Write prefix: {"result": "
        let prefix: &[u8] = b"{\"result\": \"";
        let prefix_len = prefix.len() as i64; // 12
        let prefix_off = self.alloc_data(prefix);

        // Copy prefix
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(prefix_off)); v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Unpack string
        v.extend(packed_expr);
        v.push(Instruction::LocalSet(packed));
        v.push(Instruction::LocalGet(packed)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(str_ptr));
        v.push(Instruction::LocalGet(packed)); v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(str_len));

        // Copy string bytes to ib + prefix_len
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        // dst
        v.push(Instruction::I64Const(ib + prefix_len)); v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        // src
        v.push(Instruction::LocalGet(str_ptr)); v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(1)); v.push(Instruction::End); v.push(Instruction::End);

        // Write '"}'
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add); v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add); v.push(Instruction::I64Const(prefix_len + 1));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(b'}' as i64)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        // value_return(prefix_len + str_len + 2, ib)
        v.push(Instruction::I64Const(prefix_len)); v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add); v.push(Instruction::I64Const(2));
        v.push(Instruction::I64Add); v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(25));

        v.push(Instruction::I64Const(1)); v.push(Instruction::GlobalSet(1));
        v.push(Instruction::I64Const(0));
        Ok(v)
    }
