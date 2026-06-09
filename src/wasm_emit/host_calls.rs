use super::*;

impl WasmEmitter {
    pub(crate) fn read_to_register(
        &mut self,
        host_idx: usize,
        _a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        if !self.wasi_mode && !self.p2_mode {
            // NEAR mode: allocate from FP_GLOBAL (avoids overwriting data segment for large inputs)
            self.needs_frame = true;
            let buf_i = self.local_idx("__rr_buf");
            let len_i = self.local_idx("__rr_len");
            let mut v = Vec::new();
            // Call host function to write to register 0
            v.push(Instruction::I64Const(0)); // register_id=0
            v.push(Self::host_call(host_idx));
            // register_len(0) → save
            v.push(Instruction::I64Const(0));
            v.push(Self::host_call(1));
            v.push(Instruction::LocalSet(len_i));
            // Allocate buf from FP_GLOBAL
            v.push(Instruction::GlobalGet(FP_GLOBAL));
            v.push(Instruction::LocalSet(buf_i));
            // read_register(0, buf)
            v.push(Instruction::I64Const(0));
            v.push(Instruction::LocalGet(buf_i));
            v.push(Self::host_call(0));
            // Bump FP by actual length rounded up to 8
            v.push(Instruction::GlobalGet(FP_GLOBAL));
            v.push(Instruction::LocalGet(len_i));
            v.push(Instruction::I64Const(7));
            v.push(Instruction::I64Add);
            v.push(Instruction::I64Const(-8i64 as u64 as i64));
            v.push(Instruction::I64And);
            v.push(Instruction::I64Add);
            v.push(Instruction::GlobalSet(FP_GLOBAL));
            // Pack: (len << 32) | buf — tag as Str
            v.push(Instruction::LocalGet(len_i));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64Shl);
            v.push(Instruction::LocalGet(buf_i));
            v.push(Instruction::I64Or);
            v.extend(self.emit_tag_str());
            Ok(v)
        } else {
            // WASI/P2: use TEMP_MEM (inputs are small, no data segment conflict)
            let mut v = Vec::new();
            v.push(Instruction::I64Const(0)); // register_id=0
            v.push(Self::host_call(host_idx));
            v.push(Instruction::I64Const(0));
            v.push(Instruction::I64Const(TEMP_MEM));
            v.push(Self::host_call(0));
            v.push(Instruction::I64Const(0));
            v.push(Self::host_call(1));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64Shl);
            v.push(Instruction::I64Const(TEMP_MEM));
            v.push(Instruction::I64Or);
            v.extend(self.emit_tag_str());
            Ok(v)
        }
    }

    pub(crate) fn read_u128_low(
        &mut self,
        host_idx: usize,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        // Pass TEMP_MEM as the pointer where host will write u128
        v.push(Instruction::I64Const(TEMP_MEM as i64));
        v.push(Self::host_call(host_idx));
        // Load low 8 bytes (bytes 0..7) from TEMP_MEM — tag as Num
        v.push(Instruction::I32Const(TEMP_MEM as i32));
        v.push(Instruction::I64Load(wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        }));
        v.extend(self.emit_tag_num());
        Ok(v)
    }

    // Helper: same but return high 64 bits of u128

    pub(crate) fn read_u128_high(
        &mut self,
        host_idx: usize,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        // Pass TEMP_MEM as the pointer where host will write u128
        v.push(Instruction::I64Const(TEMP_MEM as i64));
        v.push(Self::host_call(host_idx));
        // Load high 8 bytes (bytes 8..15) from TEMP_MEM — tag as Num
        v.push(Instruction::I32Const(TEMP_MEM as i32));
        v.push(Instruction::I64Load(wasm_encoder::MemArg {
            offset: 8,
            align: 3,
            memory_index: 0,
        }));
        v.extend(self.emit_tag_num());
        Ok(v)
    }

    // Clean int_to_str implementation

    pub(crate) fn int_to_str_clean(
        &mut self,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        let n = self.expr(&a[0])?;
        let n_i = self.local_idx("__its2_n");
        let neg_i = self.local_idx("__its2_neg");
        let len_i = self.local_idx("__its2_len");
        let dst_i = self.local_idx("__its2_dst");
        let tmp_i = self.local_idx("__its2_tmp");
        let dig_i = self.local_idx("__its2_dig");
        let i_i = self.local_idx("__its2_i");
        let src_i = self.local_idx("__its2_src");
        let alloc_base = self.next_data_offset.max(3072);
        self.next_data_offset = (alloc_base + 64) & !7;
        let mut v = Vec::new();
        v.extend(n);
        // Untag the number before converting: val >> TAG_BITS
        v.push(Instruction::I64Const(TAG_BITS));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(n_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(neg_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::I64Const(alloc_base as i64));
        v.push(Instruction::LocalSet(dst_i));
        // Handle negative
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(neg_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(n_i));
        v.push(Instruction::End);
        // Handle n == 0
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(48));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::Else);
        // Extract digits backward at dst+31
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Const(31));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(tmp_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // dig = n % 10
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64RemU);
        v.push(Instruction::LocalSet(dig_i));
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64DivU);
        v.push(Instruction::LocalSet(n_i));
        // mem[tmp] = '0' + dig
        v.push(Instruction::LocalGet(tmp_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(dig_i));
        v.push(Instruction::I64Const(48));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(tmp_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(tmp_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
                                  // Digits are at [tmp+1 .. dst+31], copy to dst[0..len-1]
        v.push(Instruction::LocalGet(tmp_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(src_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(src_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        v.push(Instruction::End); // if/else n==0
                                  // Prepend '-' if negative
        v.push(Instruction::LocalGet(neg_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        // Shift digits right by 1, write '-' at dst[0]
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(45)); // '-'
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::End);
        // Return packed: (len << 32) | dst, tagged as string
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Or);
        v.extend(self.emit_tag_str());
        Ok(v)
    }
}
