use super::*;

impl WasmEmitter {
    pub(crate) fn call_string(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "str_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }
            "str_cat" => {
                // (str-cat s1 s2) — runtime bump-allocated string concatenation
                // Uses emit_runtime_alloc (reads/writes RUNTIME_HEAP_PTR at addr 56)
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__sc1");
                let s2_i = self.local_idx("__sc2");
                let l1_i = self.local_idx("__scl1");
                let l2_i = self.local_idx("__scl2");
                let dst_i = self.local_idx("__scdst");
                let i_i = self.local_idx("__sci");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Save tagged strings, then untag to get raw packed (len<<32|ptr)
                v.extend(s1); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(s2_i));
                // Extract lengths: len = raw >> 32
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l2_i));
                // Extract pointers: ptr = raw & 0xFFFFFFFF
                let ptr1_i = self.local_idx("__scp1");
                let ptr2_i = self.local_idx("__scp2");
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr2_i));
                // Compute total_len = l1 + l2
                let total_i = self.local_idx("__sctot");
                v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(total_i));
                // Allocate via runtime bump allocator
                // We need to emit: compute total_i at runtime, then call emit_runtime_alloc
                // emit_runtime_alloc expects a compile-time constant, so we inline the alloc here
                let rha_tmp = self.local_idx("__rha_tmp");
                let rha_new = self.local_idx("__rha_new");
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mem_limit = (self.memory_pages as i64) * 65536;
                // Read current heap ptr from RUNTIME_HEAP_PTR (addr 56)
                v.push(Instruction::I64Const(56));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::LocalSet(rha_tmp));
                // new_ptr = old + total_len
                v.push(Instruction::LocalGet(rha_tmp));
                v.push(Instruction::LocalGet(total_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rha_new));
                // Guard: new_ptr < mem_limit
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Const(mem_limit));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                // Write back new ptr
                v.push(Instruction::I64Const(56));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Store(ma8));
                v.push(Instruction::Else);
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // dst = old ptr
                v.push(Instruction::LocalGet(rha_tmp));
                v.push(Instruction::LocalSet(dst_i));
                // ── Copy s1 bytes: dst[0..l1] = ptr1[0..l1] ──
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(ptr1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // ── Copy s2 bytes: dst[l1..l1+l2] = ptr2[0..l2] ──
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l2_i)); v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(ptr2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Return tagged string: (total_len << 32) | dst, tagged TAG_STR
                v.push(Instruction::LocalGet(total_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "str_eq" => {
                let s1 = self.expr(&a[0])?;
                let s2 = self.expr(&a[1])?;
                let s1_i = self.local_idx("__se1");
                let s2_i = self.local_idx("__se2");
                let l1_i = self.local_idx("__sel1");
                let i_i = self.local_idx("__sei");
                let res_i = self.local_idx("__seres");
                let mut v = Vec::new();
                v.extend(s1); v.push(Instruction::LocalSet(s1_i));
                v.extend(s2); v.push(Instruction::LocalSet(s2_i));
                // l1 = s1 >> 32
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(l1_i));
                // if l1 != (s2 >> 32) → 0
                v.push(Instruction::LocalGet(l1_i));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Compare byte by byte
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(res_i)); // assume equal
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(l1_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if s1_ptr[i] != s2_ptr[i]: res=0, break
                v.push(Instruction::LocalGet(s1_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(s2_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Ne);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(res_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(res_i));
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            "u32-to-bytes" => {
                let val_expr = self.expr(&a[0])?;
                let val_i = self.local_idx("__u32b_val");
                let buf_i = self.local_idx("__u32b_buf");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 1, align: 0, memory_index: 0 };
                let ma2 = wasm_encoder::MemArg { offset: 2, align: 0, memory_index: 0 };
                let ma3 = wasm_encoder::MemArg { offset: 3, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate arg, untag, store in val_i
                v.extend(val_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(val_i));
                if !self.p2_mode && !self.wasi_mode {
                    // NEAR: allocate from FP_GLOBAL
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::LocalSet(buf_i));
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::GlobalSet(FP_GLOBAL));
                } else {
                    // P2/WASI: compile-time allocation
                    let alloc_base = self.next_data_offset.max(3072);
                    self.next_data_offset = (alloc_base + 8) & !7;
                    v.push(Instruction::I64Const(alloc_base as i64)); v.push(Instruction::LocalSet(buf_i));
                }
                // byte 0: store (val & 0xFF) at buf+0
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // byte 1: store ((val >> 8) & 0xFF) at buf+1
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma1));
                // byte 2: store ((val >> 16) & 0xFF) at buf+2
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma2));
                // byte 3: store ((val >> 24) & 0xFF) at buf+3
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Const(24)); v.push(Instruction::I64ShrU); v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma3));
                // Build tagged string: (4 << 32) | buf_addr, then tag with TAG_STR
                v.push(Instruction::I64Const(4)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(buf_i)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "bytes-to-u32" => {
                let s_expr = self.expr(&a[0])?;
                let packed_i = self.local_idx("__b32u_packed");
                let ptr_i = self.local_idx("__b32u_ptr");
                let b0_i = self.local_idx("__b32u_b0");
                let b1_i = self.local_idx("__b32u_b1");
                let b2_i = self.local_idx("__b32u_b2");
                let b3_i = self.local_idx("__b32u_b3");
                let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma1 = wasm_encoder::MemArg { offset: 1, align: 0, memory_index: 0 };
                let ma2 = wasm_encoder::MemArg { offset: 2, align: 0, memory_index: 0 };
                let ma3 = wasm_encoder::MemArg { offset: 3, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate arg, untag string tag, store packed (len<<32|ptr)
                v.extend(s_expr);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(packed_i));
                // Extract ptr = low 32 bits of packed
                v.push(Instruction::LocalGet(packed_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ptr_i));
                // b0 = I32Load8U(ptr+0)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma0));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b0_i));
                // b1 = I32Load8U(ptr+1)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma1));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b1_i));
                // b2 = I32Load8U(ptr+2)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma2));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b2_i));
                // b3 = I32Load8U(ptr+3)
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma3));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b3_i));
                // result = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
                v.push(Instruction::LocalGet(b0_i));
                v.push(Instruction::LocalGet(b1_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.push(Instruction::LocalGet(b2_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.push(Instruction::LocalGet(b3_i)); v.push(Instruction::I64Const(24)); v.push(Instruction::I64Shl); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "str_to_int" => {
                let s = self.expr(&a[0])?;
                let s_i = self.local_idx("__sti_s");
                let len_i = self.local_idx("__sti_len");
                let i_i = self.local_idx("__sti_i");
                let acc_i = self.local_idx("__sti_acc");
                let ch_i = self.local_idx("__sti_ch");
                let neg_i = self.local_idx("__sti_neg");
                let mut v = Vec::new();
                v.extend(s); v.push(Instruction::LocalSet(s_i));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Check for leading '-'
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // if ch == '-' (45)
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(45)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(i_i)); // skip '-'
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // ch = s_ptr[i]
                v.push(Instruction::LocalGet(s_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(ch_i));
                // acc = acc * 10 + (ch - 48)
                v.push(Instruction::LocalGet(acc_i)); v.push(Instruction::I64Const(10)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(ch_i)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(acc_i));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0)); // fallback
                v.push(Instruction::End); // block
                // Apply negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Sub); // 0 - acc
                v.push(Instruction::Else);
                // Identity — but we need the value on stack. It's already there from the block.
                // Hmm, the block result is already on the stack. The if consumes it.
                // We need to save it to a local first.
                v.pop(); // remove the Else we just added
                // Save block result, then branch
                v.push(Instruction::LocalSet(acc_i)); // save
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(acc_i));
                v.push(Instruction::End);
                Ok(v)
            }
            "str-len" => {
                if a.len() != 1 { return Err("str-len: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                // Untag string → raw = (len << 32) | ptr
                v.extend(self.emit_untag());
                // Extract len: raw >> 32
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "str-ptr" => {
                if a.len() != 1 { return Err("str-ptr: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                // Low 32 bits = ptr: wrap to i32 then extend back to i64
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "str-slice" => {
                if a.len() != 3 { return Err("str-slice: expected 3 args (string, start, end)".into()); }
                if self.p2_mode || self.wasi_mode {
                    // P2/WASI: zero-copy (no FP_GLOBAL available)
                    let raw_i = self.local_idx("__ss_raw");
                    let start_i = self.local_idx("__ss_start");
                    let end_i = self.local_idx("__ss_end");
                    let orig_len_i = self.local_idx("__ss_olen");
                    let mut v = Vec::new();
                    v.extend(self.expr(&a[0])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw_i));
                    v.push(Instruction::LocalGet(raw_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(orig_len_i));
                    v.extend(self.expr(&a[1])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(start_i));
                    v.extend(self.expr(&a[2])?);
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(end_i));
                    v.push(Instruction::LocalGet(end_i)); v.push(Instruction::LocalGet(orig_len_i)); v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                    v.push(Instruction::LocalGet(start_i)); v.push(Instruction::LocalGet(end_i)); v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                    v.push(Instruction::LocalGet(raw_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalGet(start_i)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(end_i)); v.push(Instruction::LocalGet(start_i)); v.push(Instruction::I64Sub);
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    v.extend(self.emit_tag_num());
                    v.push(Instruction::I64Const(5)); v.push(Instruction::I64Or);
                    return Ok(v);
                }
                // NEAR mode: copy-based str-slice
                let raw_i = self.local_idx("__ss_raw");
                let start_i = self.local_idx("__ss_start");
                let end_i = self.local_idx("__ss_end");
                let new_len_i = self.local_idx("__ss_nlen");
                let src_ptr_i = self.local_idx("__ss_srcp");
                let dst_i = self.local_idx("__ss_dst");
                let dst_save_i = self.local_idx("__ss_dst_save");
                let qwords_i = self.local_idx("__ss_qw");
                let remain_i = self.local_idx("__ss_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                // Get raw string descriptor: untag → (len << 32) | ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(raw_i));
                // Evaluate and store start/end
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(start_i));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(end_i));
                // new_len = end - start
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(new_len_i));
                // Bounds check: end > (raw >> 32) → trap
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::LocalGet(raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: start > end → trap
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::LocalGet(end_i));
                v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // src_ptr = (raw & 0xFFFFFFFF) + start
                v.push(Instruction::LocalGet(raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(start_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(src_ptr_i));
                // Allocate dst from FP_GLOBAL
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalSet(dst_save_i)); // save original dst
                // Bounds check: FP + new_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Advance FP: aligned up to 8
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalGet(new_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                v.push(Instruction::GlobalSet(FP_GLOBAL));
                // Word copy: qwords = new_len / 8, remain = new_len & 7
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(new_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                // Word loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder byte copy
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(ma8));
                v.push(Instruction::I64Store8(ma8));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Build result: (new_len << 32) | dst_save, tagged as Str
                v.push(Instruction::LocalGet(new_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_save_i));
                v.push(Instruction::I64Or);
                // Tag as Str
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Const(5)); // TAG_STR
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "str-contains-byte" => {
                if a.len() != 2 { return Err("str-contains-byte: expected 2 args".into()); }
                let str_i = self.local_idx("__scb_str");
                let byte_i = self.local_idx("__scb_byte");
                let len_i = self.local_idx("__scb_len");
                let ptr_i = self.local_idx("__scb_ptr");
                let idx_i = self.local_idx("__scb_idx");
                let found_i = self.local_idx("__scb_found");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 16) & !7;
                let mut v = Vec::new();
                // Eval string, untag, store raw
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(str_i));
                // Eval byte value, untag, store
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(byte_i));
                // Extract len and ptr
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(ptr_i));
                // found = 0, idx = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(found_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx_i));
                // Loop: while idx < len && !found
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if idx >= len, break
                v.push(Instruction::LocalGet(idx_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break
                // load byte at ptr + idx
                v.push(Instruction::LocalGet(ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(idx_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64ExtendI32U);
                // compare with target byte
                v.push(Instruction::LocalGet(byte_i));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(found_i));
                v.push(Instruction::Br(1)); // break outer block (found)
                v.push(Instruction::End);
                // idx++
                v.push(Instruction::LocalGet(idx_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx_i));
                v.push(Instruction::Br(0)); // continue loop
                v.push(Instruction::End); // end loop
                v.push(Instruction::End); // end block
                // Return found as tagged bool
                v.push(Instruction::LocalGet(found_i));
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "strlcpy" => {
                if a.len() != 3 { return Err("strlcpy: expected 3 args (dst_ptr src_ptr len)".into()); }
                let src_i = self.local_idx("__slc_src");
                let dst_i = self.local_idx("__slc_dst");
                let len_i = self.local_idx("__slc_len");
                let qwords_i = self.local_idx("__slc_qw");
                let remain_i = self.local_idx("__slc_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                // Evaluate args, untag, store
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(dst_i));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(src_i));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(len_i));
                // Bounds check: src_ptr + len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: dst_ptr + len ≤ mem_limit
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // qwords = len / 8
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(qwords_i));
                // remain = len & 7
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(remain_i));
                // Word copy loop: while qwords > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                // dst[i64.load src]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder tail: while remain > 0, copy 1 byte via I64Load8U + I64Store8
                // (Note: I64Load8U is different from I32Load8U — worth testing)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return total length (like strlcpy returns strlen(src))
                v.push(Instruction::LocalGet(len_i));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "strlcat" => {
                if a.len() != 4 { return Err("strlcat: expected 4 args (dst_ptr src_ptr dst_offset len)".into()); }
                // Just emit: (strlcpy (i64.add dst_ptr dst_offset) src_ptr len)
                // We inline it to avoid a recursive call
                let src_i = self.local_idx("__slt_src");
                let dst_i = self.local_idx("__slt_dst");
                let off_i = self.local_idx("__slt_off");
                let len_i = self.local_idx("__slt_len");
                let qwords_i = self.local_idx("__slt_qw");
                let remain_i = self.local_idx("__slt_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                v.extend(self.expr(&a[0])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(dst_i));
                v.extend(self.expr(&a[1])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(src_i));
                v.extend(self.expr(&a[2])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(off_i));
                v.extend(self.expr(&a[3])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(len_i));
                // Bounds check: src_ptr + len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Bounds check: dst_ptr + offset + len ≤ mem_limit
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // dst += offset
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                // qwords = len / 8
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(remain_i));
                // Word copy loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder tail
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return total length
                v.push(Instruction::LocalGet(len_i));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "str-cat" => {
                // Variadic str-cat: (str-cat s1 s2 ... sN) — single alloc, N copies
                if a.is_empty() { return Err("str-cat: need at least 1 arg".into()); }
                if a.len() == 1 { return self.expr(&a[0]); }
                if self.p2_mode || self.wasi_mode {
                    // P2/WASI: heap_ptr bump allocator, variadic
                    let d = self.str_cat_depth;
                    self.str_cat_depth += 1;
                    let n = a.len();
                    // Locals: one raw (i64) + len (i32) + ptr (i32) per arg, plus total_len (i32), dst (i32), dst_save (i32)
                    let raw_is: Vec<_> = (0..n).map(|i| self.local_idx(&format!("__sc{}_r{}", d, i))).collect();
                    let len_is: Vec<_> = (0..n).map(|i| self.local_idx_i32(&format!("__sc{}_l{}", d, i))).collect();
                    let ptr_is: Vec<_> = (0..n).map(|i| self.local_idx_i32(&format!("__sc{}_p{}", d, i))).collect();
                    let total_len_i = self.local_idx_i32(&format!("__sc{}_tot", d));
                    let dst_i = self.local_idx_i32(&format!("__sc{}_dst", d));
                    let dst_save_i = self.local_idx_i32(&format!("__sc{}_dsav", d));
                    let mut v = Vec::new();
                    // Phase 1: eval all args, extract len/ptr, sum lengths
                    for i in 0..n {
                        v.extend(self.expr(&a[i])?); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(raw_is[i]));
                        // len = raw >> 32 → i32 via wrap_i64
                        v.push(Instruction::LocalGet(raw_is[i])); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                        v.push(Instruction::I32WrapI64); v.push(Instruction::LocalSet(len_is[i]));
                        // ptr = raw & 0xFFFFFFFF → i32 via wrap_i64
                        v.push(Instruction::LocalGet(raw_is[i])); v.push(Instruction::I32WrapI64); v.push(Instruction::LocalSet(ptr_is[i]));
                    }
                    // Sum all lengths (i32 arithmetic)
                    v.push(Instruction::LocalGet(len_is[0])); v.push(Instruction::LocalSet(total_len_i));
                    for i in 1..n {
                        v.push(Instruction::LocalGet(total_len_i)); v.push(Instruction::LocalGet(len_is[i])); v.push(Instruction::I32Add); v.push(Instruction::LocalSet(total_len_i));
                    }
                    // Allocate from heap_ptr
                    let hp = self.heap_ptr as i32;
                    let max_reserve: i64 = a.iter().map(|_| 4096i64).sum::<i64>().max(4096);
                    self.heap_ptr = (self.heap_ptr as i64 + max_reserve) as u32;
                    v.push(Instruction::I32Const(hp)); v.push(Instruction::LocalSet(dst_i));
                    v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::LocalSet(dst_save_i));
                    // Phase 2: copy each arg into dst using shared memcpy helper
                    for i in 0..n {
                        v.push(Instruction::LocalGet(dst_i));
                        v.push(Instruction::LocalGet(ptr_is[i]));
                        v.push(Instruction::LocalGet(len_is[i]));
                        v.push(Instruction::Call(crate::wasm_emit::MEMCPY_SENTINEL));
                        // Advance dst by len_is[i] (i32 add)
                        v.push(Instruction::LocalGet(dst_i));
                        v.push(Instruction::LocalGet(len_is[i]));
                        v.push(Instruction::I32Add);
                        v.push(Instruction::LocalSet(dst_i));
                    }
                    // Build result: ((total_len << 32) | dst_save) tagged as TAG_STR
                    v.push(Instruction::LocalGet(total_len_i)); v.push(Instruction::I64ExtendI32U); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(dst_save_i)); v.push(Instruction::I64ExtendI32U); v.push(Instruction::I64Or);
                    v.extend(self.emit_tag_str());
                    self.str_cat_depth -= 1;
                    return Ok(v);
                }
                // NEAR mode: frame-based allocation
                let a_raw_i = self.local_idx("__sc_a");
                let b_raw_i = self.local_idx("__sc_b");
                let a_len_i = self.local_idx("__sc_a_len");
                let a_ptr_i = self.local_idx("__sc_a_ptr");
                let b_len_i = self.local_idx("__sc_b_len");
                let b_ptr_i = self.local_idx("__sc_b_ptr");
                let dst_i = self.local_idx("__sc_dst");
                let dst_save_i = self.local_idx("__sc_dst_save");
                let total_len_i = self.local_idx("__sc_total");
                let qwords_i = self.local_idx("__sc_qw");
                let remain_i = self.local_idx("__sc_rem");
                let mut v = Vec::new();
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                // Evaluate a, extract raw descriptor
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a_raw_i));
                v.push(Instruction::LocalGet(a_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(a_len_i));
                v.push(Instruction::LocalGet(a_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(a_ptr_i));
                // Evaluate b, extract raw descriptor
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(b_raw_i));
                v.push(Instruction::LocalGet(b_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(b_len_i));
                v.push(Instruction::LocalGet(b_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(b_ptr_i));
                // total_len = a_len + b_len
                v.push(Instruction::LocalGet(a_len_i));
                v.push(Instruction::LocalGet(b_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(total_len_i));
                // Allocate dst at FP_GLOBAL (frame pointer)
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalSet(dst_save_i));
                // Bounds check: FP + total_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::LocalGet(total_len_i)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // Round up FP advance to 8-byte boundary for safe word copies
                v.push(Instruction::GlobalGet(FP_GLOBAL));
                v.push(Instruction::LocalGet(total_len_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And); // align up to 8
                v.push(Instruction::GlobalSet(FP_GLOBAL));
                // ── Copy A: word-copy loop (I64Load/I64Store) ──
                // qwords = a_len / 8, remain = a_len & 7
                v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(a_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                // Word loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder bytes via I64Load8U/I64Store8
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(a_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(a_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // ── Copy B: same word-copy loop, dst is now at a_len offset (strlcat) ──
                v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::LocalGet(b_len_i)); v.push(Instruction::I64Const(7)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(qwords_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(qwords_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Remainder bytes for B
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq); v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I64Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::LocalGet(b_ptr_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(b_ptr_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(remain_i)); v.push(Instruction::I64Const(-1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(remain_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Build result: ((total_len << 32) | dst_save) << 3 | TAG_STR
                v.push(Instruction::LocalGet(total_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_save_i));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Const(5)); // TAG_STR
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "str-concat" | "string-append" => {
                // Variadic: delegate directly to str-cat (now variadic)
                self.call_string("str-cat", a)
            }
            "str-repeat" => {
                if a.len() != 2 { return Err("str-repeat: expected 2 args".into()); }
                let src_i = self.local_idx("__sr_src");
                let count_i = self.local_idx("__sr_count");
                let src_len_i = self.local_idx("__sr_src_len");
                let src_ptr_i = self.local_idx("__sr_src_ptr");
                let dst_i = self.local_idx("__sr_dst");
                let rep_i = self.local_idx("__sr_rep");
                let off_i = self.local_idx("__sr_off");
                let j_i = self.local_idx("__sr_j");
                let alloc_base = self.next_data_offset.max(3072);
                // We'll allocate at alloc_base; advance next_data_offset later
                let mut v = Vec::new();
                // Eval string arg
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                // Eval count arg
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(count_i));
                // Extract src len and ptr
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // Total size = src_len * count
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::LocalGet(count_i));
                v.push(Instruction::I64Mul);
                // Allocate that many bytes
                v.push(Instruction::LocalSet(off_i));
                let total_size_local = off_i;
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Advance next_data_offset
                let _new_offset = format!("{} + total_size rounded up", alloc_base);
                // We'll fix next_data_offset after we know total_size... but it's runtime.
                // For now, allocate a generous fixed buffer and advance by a worst-case amount.
                // Actually, since count is often a literal, we can handle that. For runtime count,
                // use a generous upper bound.
                // Use a 4096-byte buffer at alloc_base.
                self.next_data_offset = (alloc_base + 4096) & !7;
                // rep = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rep_i));
                // outer loop: for rep in 0..count
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(rep_i));
                v.push(Instruction::LocalGet(count_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break
                // j = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                // inner loop: copy src byte by byte
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1)); // break inner
                // dst[rep*src_len + j] = src[j]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rep_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                // Load src[j]
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                v.push(Instruction::I32Store8(wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0)); // continue inner
                v.push(Instruction::End); // end inner loop
                v.push(Instruction::End); // end inner block
                // rep++
                v.push(Instruction::LocalGet(rep_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rep_i));
                v.push(Instruction::Br(0)); // continue outer
                v.push(Instruction::End); // end outer loop
                v.push(Instruction::End); // end outer block
                // Return tagged string: (total_size << 32) | alloc_base, tagged as Str
                v.push(Instruction::LocalGet(total_size_local));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "hex-encode" => {
                if a.len() != 1 { return Err("hex-encode: expected 1 arg".into()); }
                let src_i = self.local_idx("__he_src");
                let src_len_i = self.local_idx("__he_src_len");
                let src_ptr_i = self.local_idx("__he_src_ptr");
                let dst_i = self.local_idx("__he_dst");
                let i_i = self.local_idx("__he_i");
                let b_i = self.local_idx("__he_b");
                let off_i = self.local_idx("__he_off"); // src byte offset
                let shift_i = self.local_idx("__he_shift");
                let hex_byte_i = self.local_idx("__he_hb");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 4096) & !7;
                let hex_table_off = self.alloc_data(b"0123456789abcdef");
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                // Extract len and ptr
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // dst = alloc_base (8-byte aligned)
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Zero the dst buffer first (clear 512 bytes = max 256 input bytes → 512 hex chars)
                // Write 64 zero-words (512 / 8 = 64)
                for off in (0..512).step_by(8) {
                    v.push(Instruction::I64Const((alloc_base + off) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::I64Store(ma8));
                }
                // i = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load source byte via I64Load word-read
                // off = src_ptr + i
                v.push(Instruction::LocalGet(src_ptr_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(off_i));
                // shift = (off & 7) * 8
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalSet(shift_i));
                // b = (i64.load(off & ~7) >> shift) & 0xFF
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::LocalGet(shift_i));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(b_i));
                // Lookup hi nibble: hex_table[b >> 4]
                // hex_table_off + (b >> 4) is the byte address
                // Load word at (hex_table_off + hi) & ~7, shift by ((hex_table_off + hi) & 7) * 8
                {
                    v.push(Instruction::LocalGet(b_i));
                    v.push(Instruction::I64Const(4)); v.push(Instruction::I64ShrU); // hi = b >> 4
                    v.push(Instruction::I64Const(hex_table_off as i64));
                    v.push(Instruction::I64Add); // hex_table_off + hi
                    v.push(Instruction::LocalSet(off_i)); // reuse off_i
                    // shift
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    // load & extract
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                    // Save hex byte before computing dst offset
                    v.push(Instruction::LocalSet(hex_byte_i));
                    // Store at dst + 2*i — read-modify-write using I64Store
                    // dst_off = dst + 2*i
                    v.push(Instruction::LocalGet(dst_i));
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i)); // reuse
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i)); // dst byte shift
                    // Read-modify-write
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    // word is on stack, hex_byte saved in local
                    // Clear bit: word & ~(0xFF << shift_i)
                    v.push(Instruction::I64Const(0xFF));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl); // 0xFF << shift
                    v.push(Instruction::I64Const(-1i64 as u64 as i64));
                    v.push(Instruction::I64Xor); // ~(0xFF << shift)
                    v.push(Instruction::I64And); // word & ~mask
                    // Set bit: | (hex_byte << shift_i)
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    // I64Store needs [i32 addr, i64 val] — save val, push addr, push val
                    v.push(Instruction::LocalSet(hex_byte_i)); // reuse hex_byte_i as temp
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::I64Store(ma8));
                }
                // Lookup lo nibble: hex_table[b & 0xF]
                {
                    v.push(Instruction::LocalGet(b_i));
                    v.push(Instruction::I64Const(15)); v.push(Instruction::I64And); // lo = b & 0xF
                    v.push(Instruction::I64Const(hex_table_off as i64));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Const(0xFF)); v.push(Instruction::I64And);
                    // Save hex byte before computing dst offset
                    v.push(Instruction::LocalSet(hex_byte_i));
                    // Store at dst + 2*i + 1
                    v.push(Instruction::LocalGet(dst_i));
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalSet(shift_i));
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma8));
                    v.push(Instruction::I64Const(0xFF));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(-1i64 as u64 as i64));
                    v.push(Instruction::I64Xor);
                    v.push(Instruction::I64And);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::LocalGet(shift_i));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or);
                    // I64Store needs [i32 addr, i64 val] — save val, push addr, push val
                    v.push(Instruction::LocalSet(hex_byte_i)); // reuse as temp
                    v.push(Instruction::LocalGet(off_i));
                    v.push(Instruction::I64Const(-8i64 as u64 as i64)); v.push(Instruction::I64And);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(hex_byte_i));
                    v.push(Instruction::I64Store(ma8));
                }
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return: (src_len*2 << 32) | alloc_base, tagged Str
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Shl); // * 2
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "base64-decode" => {
                if a.len() != 1 { return Err("base64-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b64d_src");
                let src_len_i = self.local_idx("__b64d_src_len");
                let src_ptr_i = self.local_idx("__b64d_src_ptr");
                let dst_i = self.local_idx("__b64d_dst");
                let i_i = self.local_idx("__b64d_i");
                let out_len_i = self.local_idx("__b64d_out_len");
                let a_i = self.local_idx("__b64d_a");
                let b_i = self.local_idx("__b64d_b");
                let c_i = self.local_idx("__b64d_c");
                let d_i = self.local_idx("__b64d_d");
                let val_i = self.local_idx("__b64d_val");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base64 decode table: 256 bytes, 0-63 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: process 4 chars at a time
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i + 3 >= src_len, break
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // a = table[src[i]]: load src[i] as i32, add table_off, load8_u, extend to i64
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma)); // loads src[i] (the char)
                v.push(Instruction::I32Add); // table_off + char_value
                v.push(Instruction::I32Load8U(ma)); // loads table[char] (decoded 0-63)
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(a_i));
                // b = table[src[i+1]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // c = table[src[i+2]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(2)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // d = table[src[i+3]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(3)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(d_i));
                // byte1 = (a << 2) | (b >> 4) — all i64
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // byte2 = ((b & 0xF) << 4) | (c >> 2)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(15));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte2
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // byte3 = ((c & 0x3) << 6) | d
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(d_i));
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte3
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // i += 4
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "base64url-decode" => {
                if a.len() != 1 { return Err("base64url-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b64ud_src");
                let src_len_i = self.local_idx("__b64ud_src_len");
                let src_ptr_i = self.local_idx("__b64ud_src_ptr");
                let dst_i = self.local_idx("__b64ud_dst");
                let i_i = self.local_idx("__b64ud_i");
                let out_len_i = self.local_idx("__b64ud_out_len");
                let a_i = self.local_idx("__b64ud_a");
                let b_i = self.local_idx("__b64ud_b");
                let c_i = self.local_idx("__b64ud_c");
                let d_i = self.local_idx("__b64ud_d");
                let val_i = self.local_idx("__b64ud_val");
                let remain_i = self.local_idx("__b64ud_remain");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base64url decode table: 256 bytes, 0-63 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: process groups of 2-4 chars (no padding)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // remain = src_len - i
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(remain_i));
                // Load a = table[src[i]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(a_i));
                // Load b = table[src[i+1]] (we know remain >= 2 because we checked i < src_len, but need at least 2 chars)
                // Actually: since i < src_len, we need to check remain >= 2 — but base64url groups are at least 2.
                // If remain == 1, treat as single char (shouldn't happen in valid base64url, but handle gracefully: skip)
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64LeU);
                v.push(Instruction::BrIf(1)); // break if <= 1 char left (invalid, but safe)
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // byte0 = (a << 2) | (b >> 4)
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte0
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // If remain >= 3: load c and emit byte1
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64LtU);
                // If remain < 3, skip to end-of-group
                v.push(Instruction::If(BlockType::Empty));
                // remain < 3 → just update i and continue
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                // remain >= 3: load c
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(2)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // byte1 = ((b & 0xF) << 4) | (c >> 2)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(15));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(2));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte1
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // If remain >= 4: load d and emit byte2
                v.push(Instruction::LocalGet(remain_i));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                // remain < 4 → update i += 3 and continue
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                // remain >= 4: load d
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(3)); v.push(Instruction::I32Add);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(d_i));
                // byte2 = ((c & 0x3) << 6) | d
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Const(3));
                v.push(Instruction::I64And); v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(d_i));
                v.push(Instruction::I64Or); v.push(Instruction::LocalSet(val_i));
                // dst[out_len] = byte2
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // i += 4
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::End); // end if remain >= 4 else
                v.push(Instruction::End); // end if remain >= 3 else
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "base58-decode" => {
                if a.len() != 1 { return Err("base58-decode: expected 1 arg".into()); }
                let src_i = self.local_idx("__b58d_src");
                let src_len_i = self.local_idx("__b58d_src_len");
                let src_ptr_i = self.local_idx("__b58d_src_ptr");
                let dst_i = self.local_idx("__b58d_dst");
                let i_i = self.local_idx("__b58d_i");
                let j_i = self.local_idx("__b58d_j");
                let out_len_i = self.local_idx("__b58d_out_len");
                let carry_i = self.local_idx("__b58d_carry");
                let decoded_i = self.local_idx("__b58d_decoded");
                let tmp_i = self.local_idx("__b58d_tmp");
                let leading_i = self.local_idx("__b58d_leading");
                let alloc_base = self.next_data_offset.max(3072);
                self.next_data_offset = (alloc_base + 8192) & !7;
                // Base58 decode table: 256 bytes, 0-57 for valid, 255 for invalid
                let mut decode_table = vec![255u8; 256];
                for (i, ch) in b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".iter().enumerate() {
                    decode_table[*ch as usize] = i as u8;
                }
                let table_off = self.alloc_data(&decode_table);
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(src_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(src_len_i));
                v.push(Instruction::LocalGet(src_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(src_ptr_i));
                // dst = alloc_base + src_len (worst case: output can be up to src_len bytes)
                // Actually we need dst to be separate from the src data. Use alloc_base as the dst buffer.
                // Zero out the dst buffer first (we'll use up to src_len + a few bytes)
                // Actually, just use alloc_base directly. We'll keep output in little-endian there.
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::LocalSet(dst_i));
                // Zero out the buffer area we'll use (src_len + 32 bytes to be safe)
                // For simplicity, zero 256 bytes — enough headroom
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(256));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // out_len = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(out_len_i));
                // i = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Main loop: for each input char
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // decoded = table[src[i]]
                v.push(Instruction::I32Const(table_off as i32));
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(decoded_i));
                // carry = decoded
                v.push(Instruction::LocalGet(decoded_i));
                v.push(Instruction::LocalSet(carry_i));
                // Inner loop: j = 0..out_len-1
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= out_len, break
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // tmp = dst[j] (as i64)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U);
                // carry += tmp * 58
                v.push(Instruction::I64Const(58));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));
                // dst[j] = carry & 0xFF
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Const(255));
                v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // carry >>= 8
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(carry_i));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                // While carry > 0: dst[out_len] = carry & 0xFF; out_len++; carry >>= 8
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::BrIf(1)); // break if carry == 0
                // dst[out_len] = carry & 0xFF
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Const(255));
                v.push(Instruction::I64And); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // out_len++
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // carry >>= 8
                v.push(Instruction::LocalGet(carry_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(carry_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // carry loop
                v.push(Instruction::End); // carry block
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                // Post-processing: count leading '1' chars (0x31) in input
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(leading_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= src_len, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(src_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // if src[i] != 0x31, break
                v.push(Instruction::LocalGet(src_ptr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Const(0x31));
                v.push(Instruction::I32Ne);
                v.push(Instruction::BrIf(1));
                // leading++
                v.push(Instruction::LocalGet(leading_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(leading_i));
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Now we need to:
                // 1. Reverse the output buffer (little-endian → big-endian)
                //    Reverse bytes at [dst .. dst+out_len-1]
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= out_len/2, break
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64ShrU); // out_len / 2
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // tmp = dst[i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp_i));
                // dst[i] = dst[out_len - 1 - i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                // Load dst[out_len - 1 - i]
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                // dst[out_len - 1 - i] = tmp
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(tmp_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(ma));
                // i++
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // reverse loop
                v.push(Instruction::End); // reverse block
                // 2. Prepend leading zeros: shift the reversed data right by 'leading' bytes
                //    Use a secondary buffer at alloc_base + 512 for the final result
                //    Actually, we can do it in-place by shifting from the end
                //    Final layout: [leading zero bytes] [reversed data]
                //    We need to move data from offset 0 to offset 'leading'
                //    Since we already reversed (big-endian), move bytes from end to start
                //    Use backward copy to avoid overwriting
                //    New out_len = leading + old_out_len
                //    dst[out_len-1 - k] = dst[old_out_len-1 - k] for k = 0..old_out_len-1
                //    First, shift data right by 'leading' bytes
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= out_len, break
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // dst[leading + out_len - 1 - j] = dst[out_len - 1 - j]
                // Store address: dst + leading + out_len - 1 - j
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(leading_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                // Load address: dst + out_len - 1 - j
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(out_len_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64); v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Sub);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Store8(ma));
                // j++
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // copy loop
                v.push(Instruction::End); // copy block
                // Fill leading zeros: dst[0..leading-1] = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(leading_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // zero loop
                v.push(Instruction::End); // zero block
                // out_len += leading
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::LocalGet(leading_i));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(out_len_i));
                // Return (out_len << 32) | alloc_base tagged Str
                v.push(Instruction::LocalGet(out_len_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(alloc_base as i64));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/store-bytes" => {
                if a.len() != 2 { return Err("near/store-bytes: expected 2 args".into()); }
                self.need_host(17);
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // Extract val ptr and len
                let val_raw_i = self.local_idx("__sb_vr");
                v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(val_raw_i));
                // Bounds check: val_ptr + val_len ≤ mem_limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // val_ptr
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // val_len
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(mem_limit)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Unreachable); v.push(Instruction::End);
                // storage_write(key_len, key_ptr, val_len, val_ptr, register_id=0)
                // Pass val_ptr directly to storage_write (no copy needed)
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // key_len
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // val_len
                v.push(Instruction::LocalGet(val_raw_i));
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // val_ptr
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load-bytes" => {
                if a.len() != 1 { return Err("near/load-bytes: expected 1 arg".into()); }
                self.need_host(18); self.need_host(0); self.need_host(1);
                let key = self.expr(&a[0])?;
                let len_i = self.local_idx("__lb_len");
                let buf_i = self.local_idx("__lb_buf");
                let mut v = Vec::new();
                // storage_read(key_len, key_ptr, register_id=1)
                v.extend(key.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                // register_len(1) → save to local
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(1));
                v.push(Instruction::LocalSet(len_i));
                // Check if -1 (not found)
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(-1i64 as u64 as i64));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return nil
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                if !self.p2_mode && !self.wasi_mode {
                    // NEAR: allocate from FP_GLOBAL (bump by max storage value size)
                    v.push(Instruction::GlobalGet(FP_GLOBAL)); v.push(Instruction::LocalSet(buf_i));
                    // read_register(1, buf)
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::LocalGet(buf_i));
                    v.push(Self::host_call(0));
                    // Bump FP by actual length (rounded up to 8) so next allocs don't overlap
                    // FP += (len + 7) & ~7
                    v.push(Instruction::GlobalGet(FP_GLOBAL));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(7)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Const(-8)); v.push(Instruction::I64And);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::GlobalSet(FP_GLOBAL));
                    // Return tagged string: (len << 32) | buf
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(buf_i));
                    v.push(Instruction::I64Or);
                } else {
                    // P2/WASI: compile-time allocation
                    let alloc_base = self.next_data_offset.max(3072);
                    self.next_data_offset = (alloc_base + 8192) & !7;
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Const(alloc_base as i64));
                    v.push(Self::host_call(0));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(alloc_base as i64));
                    v.push(Instruction::I64Or);
                }
                v.extend(self.emit_tag_str());
                v.push(Instruction::End);
                Ok(v)
            }
            "to-string" | "int_to_str" => {
                return self.int_to_str_clean(&a);
            }
            "str-len" => {
                if a.len() != 1 { return Err("str-len: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
