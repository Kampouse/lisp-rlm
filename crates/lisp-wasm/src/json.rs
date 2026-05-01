use crate::emit::WasmEmitter;
use crate::emit::INPUT_BUF;
use wasm_encoder::{BlockType, Instruction, ValType};

impl WasmEmitter {
    pub(crate) fn json_get_int(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
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
        let prev_byte = self.local_idx("__js_prev");
        let ws_byte = self.local_idx("__js_ws_byte");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(7)); // input(0)
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len(0)
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(ib)); v.push(Self::host_call(0)); // read_register(0, ib)

        // pos = 0, depth = 0
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(pos));
        let depth = self.local_idx("__js_depth");
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(depth));

        // Scan loop (block/loop)
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        // if pos + pat_len > ilen: break
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);

        // Track brace depth: load byte at INPUT_BUF+pos
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        let scan_byte = self.local_idx("__js_sb");
        v.push(Instruction::LocalSet(scan_byte));
        // if byte == '{': depth++
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        // if byte == '}': depth--
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);

        // Only try to match at depth == 1 (top level)
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        // depth != 1, skip comparison, just advance pos
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); // back to outer LOOP (skip label 0 = this if)
        v.push(Instruction::End);

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
        v.push(Instruction::I64Const(pat_off as i64)); v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        // Compare
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); // match continues
        v.push(Instruction::Else); // mismatch
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End); // inner loop/block

        // If mi==1: check preceding byte boundary
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        // pos > 0 → check preceding byte
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(0));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        // Load byte at INPUT_BUF[pos-1]
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(prev_byte));
        // Valid if prev_byte in {0x7B '{', 0x2C ',', 0x20 ' ', 0x09 '\t', 0x0A '\n'}
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x7B)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x2C)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        // If NOT valid boundary, reset mi
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End); // end pos > 0 check
        v.push(Instruction::End); // end mi==1 check
        // Now check mi again — if still 1, break outer
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // pos++
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End); // outer loop/block

        // Wrap parse section: if pos >= ilen (key not found), skip parsing; res stays 0
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty)); // if pos < ilen → parse

        // pos at match. Value at pos + pat_len
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));

        // Skip whitespace (space, tab, LF, CR)
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(ws_byte));
        // byte == ' ' || byte == '\t' || byte == '\n' || byte == '\r'
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0D)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End);
        v.push(Instruction::End); v.push(Instruction::End);

        // Skip quote if present
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End); v.push(Instruction::End);

        // Check negative
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x2D)); v.push(Instruction::I64Eq);
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
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // if dg > 0x39: break
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        // res = res*10 + (dg - 0x30)
        v.push(Instruction::LocalGet(res)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x30)); v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(res));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // Apply negative → store to res
        v.push(Instruction::LocalGet(ng)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(res)); v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::End); // end if neg
        v.push(Instruction::End); // end if pos < ilen (parse section)
        // Return res (0 if key not found, parsed value otherwise)
        v.push(Instruction::LocalGet(res));
        Ok(v)
    }

    /// Emit WASM to read input JSON, scan for "key": pattern, parse decimal into u128 at offset.
    /// Returns offset (i64). u128 stored as lo 8 bytes at offset, hi 8 bytes at offset+8.
    pub(crate) fn json_get_u128(&mut self, key: &str, offset_expr: Vec<Instruction<'static>>) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7); self.need_host(0); self.need_host(1);
        let mut pattern = vec![b'"'];
        pattern.extend(key.as_bytes());
        pattern.extend_from_slice(b"\":");
        let pat_off = self.alloc_data(&pattern);
        let pat_len = pattern.len() as i64;
        let pos = self.local_idx("__ju_pos");
        let ilen = self.local_idx("__ju_ilen");
        let mi = self.local_idx("__ju_mi");
        let jj = self.local_idx("__ju_j");
        let lo = self.local_idx("__ju_lo");
        let hi = self.local_idx("__ju_hi");
        let dg = self.local_idx("__ju_dg");
        let prev_byte = self.local_idx("__ju_prev");
        let ws_byte = self.local_idx("__ju_ws_byte");
        let scan_byte = self.local_idx("__ju_sb");
        let depth = self.local_idx("__ju_depth");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let mut v = offset_expr;

        // Store offset to a temp local
        let off_local = self.local_idx("__ju_offset");
        v.push(Instruction::LocalSet(off_local));

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(ib)); v.push(Self::host_call(0));

        // pos = 0, depth = 0
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(depth));

        // ── Scan loop ──
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);

        // Track brace depth
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(scan_byte));
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);

        // Only match at depth == 1
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);

        // Compare bytes
        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(pat_off as i64)); v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // Check preceding byte boundary
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(0));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(prev_byte));
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x7B)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x2C)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // ── Parse section ──
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));

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
        v.push(Instruction::LocalSet(ws_byte));
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0D)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); v.push(Instruction::End);
        v.push(Instruction::End); v.push(Instruction::End);

        // Skip quote if present
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End); v.push(Instruction::End);

        // Init lo = 0, hi = 0
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(lo));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(hi));

        // ── Digit parse loop: hi:lo = hi:lo * 10 + digit ──
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(dg));
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);

        // digit = dg - 0x30
        v.push(Instruction::LocalGet(dg)); v.push(Instruction::I64Const(0x30)); v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(dg));

        // u128 multiply-by-10-and-add-digit using 32-bit split:
        // lo_lo = lo & 0xFFFFFFFF, lo_hi = lo >> 32
        // p0 = lo_lo * 10 + digit, r0 = p0 & 0xFFFFFFFF, c0 = p0 >> 32
        // p1 = lo_hi * 10 + c0, r1 = p1 & 0xFFFFFFFF, c1 = p1 >> 32
        // lo = r0 | (r1 << 32), hi = hi * 10 + c1
        let lo_lo = self.local_idx("__ju_ll");
        let lo_hi = self.local_idx("__ju_lh");
        let p0 = self.local_idx("__ju_p0");
        let r0 = self.local_idx("__ju_r0");
        let c0 = self.local_idx("__ju_c0");
        let p1 = self.local_idx("__ju_p1");
        let r1 = self.local_idx("__ju_r1");
        let c1 = self.local_idx("__ju_c1");

        // lo_lo = lo & 0xFFFFFFFF
        v.push(Instruction::LocalGet(lo)); v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And); v.push(Instruction::LocalSet(lo_lo));
        // lo_hi = lo >> 32
        v.push(Instruction::LocalGet(lo)); v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(lo_hi));
        // p0 = lo_lo * 10 + digit
        v.push(Instruction::LocalGet(lo_lo)); v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul); v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(p0));
        // r0 = p0 & 0xFFFFFFFF
        v.push(Instruction::LocalGet(p0)); v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And); v.push(Instruction::LocalSet(r0));
        // c0 = p0 >> 32
        v.push(Instruction::LocalGet(p0)); v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(c0));
        // p1 = lo_hi * 10 + c0
        v.push(Instruction::LocalGet(lo_hi)); v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul); v.push(Instruction::LocalGet(c0));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(p1));
        // r1 = p1 & 0xFFFFFFFF
        v.push(Instruction::LocalGet(p1)); v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And); v.push(Instruction::LocalSet(r1));
        // c1 = p1 >> 32
        v.push(Instruction::LocalGet(p1)); v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(c1));
        // lo = r0 | (r1 << 32)
        v.push(Instruction::LocalGet(r1)); v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl); v.push(Instruction::LocalGet(r0));
        v.push(Instruction::I64Or); v.push(Instruction::LocalSet(lo));
        // hi = hi * 10 + c1
        v.push(Instruction::LocalGet(hi)); v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul); v.push(Instruction::LocalGet(c1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(hi));

        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // ── Write lo/hi to memory at offset ──
        let ma64 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        v.push(Instruction::LocalGet(off_local));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(lo));
        v.push(Instruction::I64Store(ma64.clone()));
        v.push(Instruction::LocalGet(off_local));
        v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(hi));
        v.push(Instruction::I64Store(ma64));

        v.push(Instruction::End); // end if pos < ilen

        v.push(Instruction::LocalGet(off_local));
        Ok(v)
    }


    pub(crate) fn json_get_str(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
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
        let prev_byte = self.local_idx("__jss_prev");
        let ws_byte = self.local_idx("__jss_ws_byte");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(ib)); v.push(Self::host_call(0));

        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(pos));
        let depth = self.local_idx("__jss_depth");
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(depth));

        // Scan loop
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);

        // Track brace depth
        let scan_byte = self.local_idx("__jss_sb");
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(scan_byte));
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(scan_byte)); v.push(Instruction::I64Const(0x7D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        // Only match at depth == 1
        v.push(Instruction::LocalGet(depth)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); // back to outer LOOP (skip label 0 = this if)
        v.push(Instruction::End);

        v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(pat_off as i64)); v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // If mi==1: check preceding byte boundary
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(0));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(prev_byte));
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x7B)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x2C)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(mi)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // If pos >= ilen, key not found — return 0 (packed as 0)
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));

        // Value at pos + pat_len
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(pos));

        // Skip whitespace (space, tab, LF, CR)
        v.push(Instruction::Block(BlockType::Empty)); v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos)); v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS); v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone())); v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(ws_byte));
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x20)); v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x09)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0A)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte)); v.push(Instruction::I64Const(0x0D)); v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
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
        v.push(Instruction::I64Const(0x22)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
        v.push(Instruction::LocalGet(slen)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

        // Return packed: (slen << 32) | (ib + pos)
        v.push(Instruction::LocalGet(slen)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(pos)); v.push(Instruction::I64Add);
        v.push(Instruction::I64Or);
        v.push(Instruction::Else);
        // Key not found: return 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::End); // end if pos < ilen
        Ok(v)
    }


    pub(crate) fn json_return_int(&mut self, val_expr: Vec<Instruction<'static>>) -> Result<Vec<Instruction<'static>>, String> {
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
        v.push(Instruction::I64Const(prefix_off as i64)); v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64); v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

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
        v.push(Instruction::LocalGet(abs_val)); v.push(Instruction::I64Eqz);
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
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);
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
        // Push dst addr first (deeper), then load byte (top) for I32Store8
        v.push(Instruction::I64Const(ib + prefix_len)); v.push(Instruction::LocalGet(si));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        // Stack: [dst_addr]
        v.push(Instruction::I64Const(ib)); v.push(Instruction::LocalGet(ptr)); v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(si)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        // Stack: [dst_addr, loaded_byte] — I32Store8 pops value=byte, addr=dst_addr
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(si)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(si));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

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


    pub(crate) fn json_return_str(&mut self, packed_expr: Vec<Instruction<'static>>) -> Result<Vec<Instruction<'static>>, String> {
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
        v.push(Instruction::I64Const(prefix_off as i64)); v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci)); v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add); v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

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
        v.push(Instruction::Br(0)); v.push(Instruction::End); v.push(Instruction::End);

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
}
