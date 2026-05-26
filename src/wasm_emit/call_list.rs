use super::*;

impl WasmEmitter {
    pub(crate) fn call_list(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "array" => {
                // (array elem0 elem1 ...) → TAG_ARRAY
                // Allocate on compile-time heap: [count, elem0, elem1, ...]
                let count = a.len() as u32;
                let slots_needed = 1 + count; // count + elements
                let ptr = self.heap_ptr;
                self.heap_ptr += slots_needed * 8;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Store count at ptr[0]
                v.push(Instruction::I64Const(ptr as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(count as i64));
                v.push(Instruction::I64Store(ma));
                // Evaluate and store each element
                for (i, elem) in a.iter().enumerate() {
                    // I64Store expects [i32 addr, i64 val] — push address first
                    v.push(Instruction::I64Const((ptr + ((i as u32 + 1) * 8)) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.expr(elem)?);
                    v.push(Instruction::I64Store(ma));
                }
                // Return tagged array ptr
                v.push(Instruction::I64Const(((ptr as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "vec-length" => {
                if a.len() != 1 { return Err("vec-length: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__vl_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                // Untag: >> TAG_BITS → raw heap ptr
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count from ptr[0]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Tag as number
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "vec-nth" => {
                if a.len() != 2 { return Err("vec-nth: expected 2 args".into()); }
                let arr_tmp = self.local_idx_i32("__vn_arr");
                let idx_tmp = self.local_idx_i32("__vn_idx");
                let count_tmp = self.local_idx_i32("__vn_count");
                let result_tmp = self.local_idx("__vn_result");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save array ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(arr_tmp));
                // Compile and save index
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(idx_tmp));
                // Bounds check: idx < count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Load(ma)); // load count
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(count_tmp));
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(count_tmp));
                v.push(Instruction::I32LtU); // idx < count (unsigned)
                v.push(Instruction::If(BlockType::Empty));
                // In bounds: load element at arr + (1 + idx) * 8
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32Const(8)); // skip count slot
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I32Const(3)); // idx * 8 = idx << 3
                v.push(Instruction::I32Shl);
                v.push(Instruction::I32Add);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(result_tmp));
                v.push(Instruction::Else);
                // Out of bounds: return nil
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::LocalSet(result_tmp));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(result_tmp));
                Ok(v)
            }
            "vec-set!" => {
                if a.len() != 3 { return Err("vec-set!: expected 3 args".into()); }
                let arr_tmp = self.local_idx_i32("__vs_arr");
                let idx_tmp = self.local_idx_i32("__vs_idx");
                let val_tmp = self.local_idx("__vs_val");
                let count_tmp = self.local_idx_i32("__vs_count");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save array ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(arr_tmp));
                // Compile and save index
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(idx_tmp));
                // Compile and save value (stays tagged i64)
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::LocalSet(val_tmp));
                // Bounds check: idx < count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Load(ma)); // load count
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(count_tmp));
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(count_tmp));
                v.push(Instruction::I32LtU);
                v.push(Instruction::If(BlockType::Empty));
                // In bounds: store at arr_ptr + (1 + idx) * 8
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32Const(8));
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I32Const(3)); // idx * 8 = idx << 3
                v.push(Instruction::I32Shl);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "vec-push" => {
                if a.len() != 2 { return Err("vec-push: expected 2 args".into()); }
                let old_arr = self.local_idx("__vp_old");
                let new_arr = self.local_idx("__vp_new");
                let old_count = self.local_idx("__vp_oc");
                let word_idx = self.local_idx("__vp_wi");
                let val_tmp = self.local_idx("__vp_val");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Compile and save old array
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(old_arr));
                // Compile and save value to push
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(val_tmp));
                // Load old count
                v.push(Instruction::LocalGet(old_arr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); // count
                v.push(Instruction::LocalSet(old_count));
                // Allocate new array: (1 + old_count + 1) * 8 bytes
                // = (old_count + 2) * 8
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                // Stack: alloc_size → emit_runtime_alloc reads top of stack? No — it takes n_bytes as param
                // Need to compute size and pass to alloc. But emit_runtime_alloc is a fixed-size alloc.
                // For dynamic size, inline the alloc logic with overflow guard:
                let rha_tmp = self.local_idx("__vp_rha");
                let rha_new = self.local_idx("__vp_rhan");
                v.push(Instruction::LocalSet(rha_tmp)); // save alloc_size
                // Read current runtime heap ptr
                v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(new_arr)); // new_arr = old heap ptr
                // Compute new ptr
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::LocalGet(rha_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rha_new));
                // Guard: new pointer < memory limit
                let mem_limit = (self.memory_pages as i64) * 65536;
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Const(mem_limit));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                // OK: advance heap ptr
                v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rha_new));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::Else);
                // Overflow: trap
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // Copy loop: copy old_count + 1 words (count + all old elements)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(word_idx));
                // Block → Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // Guard: word_idx < old_count + 1
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64LtU);
                // I64LtU returns i32 — no I32WrapI64 needed
                v.push(Instruction::If(BlockType::Empty));
                // Compute dest addr: new_arr + word_idx * 8
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                // Load word from old array: old_arr + word_idx * 8
                v.push(Instruction::LocalGet(old_arr));
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Stack: [i32 dest_addr, i64 loaded_word] → I64Store
                v.push(Instruction::I64Store(ma));
                // word_idx++
                v.push(Instruction::LocalGet(word_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(word_idx));
                // Br(1) targets the Loop to continue
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // close If
                v.push(Instruction::End); // close Loop
                v.push(Instruction::End); // close Block
                // Write new count: new_arr[0] = old_count + 1
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Write new element: new_arr[1 + old_count] = val_tmp
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I64Const(8)); // skip count
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(old_count));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // Return tagged new array
                v.push(Instruction::LocalGet(new_arr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "vec?" => {
                if a.len() != 1 { return Err("vec?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7)); // tag mask
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Eq);      // i32 result
                v.push(Instruction::I64ExtendI32U); // widen to i64 for tagging
                v.extend(self.emit_tag(TAG_BOOL)); // tag the bool
                Ok(v)
            }
            "arr_new" => {
                let offset_expr = self.expr(&a[0])?;
                let size_expr = self.expr(&a[1])?;
                let off_i = self.local_idx("__an_off");
                let sz_i = self.local_idx("__an_sz");
                let i_i = self.local_idx("__an_i");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off_i));
                v.extend(size_expr); v.push(Instruction::LocalSet(sz_i));
                // Store length at offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(sz_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Zero-fill loop
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(sz_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0)); v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[offset + i*8] = 0
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End); // block
                Ok(v)
            }
            "arr_get" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "arr_set" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let val = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "arr_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "arr_push" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__ap_off");
                let len_i = self.local_idx("__ap_len");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // Load current length from offset-8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(len_i));
                // Store val at offset + len*8
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Increment length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(len_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "arr_sort" => {
                // Bubble sort: arr[offset..offset+n*8]
                // Length stored at offset-8
                let off = self.expr(&a[0])?;
                let off_i = self.local_idx("__as_off");
                let n_i = self.local_idx("__as_n");
                let i_i = self.local_idx("__as_i");
                let j_i = self.local_idx("__as_j");
                let tmp_i = self.local_idx("__as_tmp");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                // n = mem[(offset-8)]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                // Outer loop: i = 0..n-1
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n-1: br 2 (exit)
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // j = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(j_i));
                // Inner loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= n-i-1: br 2
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(n_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Sub); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // tmp = arr[j], load arr[j+1]
                // Compare: if arr[j] > arr[j+1], swap
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(tmp_i)); // tmp = arr[j]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); // arr[j+1]
                // stack: arr[j+1]; tmp_i = arr[j]
                // if arr[j] > arr[j+1] → swap
                v.push(Instruction::LocalGet(tmp_i)); // tmp, arr[j+1] on stack
                v.push(Instruction::I64LtS); // arr[j+1] < arr[j] i.e. arr[j] > arr[j+1]
                v.push(Instruction::If(BlockType::Empty));
                // arr[j] = arr[j+1]
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // arr[j+1] = tmp
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // if swap
                v.push(Instruction::LocalGet(j_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "arr_find" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__af_off");
                let val_i = self.local_idx("__af_val");
                let n_i = self.local_idx("__af_n");
                let i_i = self.local_idx("__af_i");
                let mut v = Vec::new();
                v.extend(off); v.push(Instruction::LocalSet(off_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // Load length
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Sub); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::LocalGet(n_i)); v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(-1)); v.push(Instruction::Br(2)); // not found
                v.push(Instruction::End);
                // if arr[i] == val → return i
                v.push(Instruction::LocalGet(off_i)); v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(3)); v.push(Instruction::I64Shl); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(val_i)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::Br(2)); // found
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(-1)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }
            "list" => {
                let count = a.len() as u32;
                let slots_needed = 1 + count;
                let ptr = self.heap_ptr;
                self.heap_ptr += slots_needed * 8;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.push(Instruction::I64Const(ptr as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(count as i64));
                v.push(Instruction::I64Store(ma));
                for (i, elem) in a.iter().enumerate() {
                    v.push(Instruction::I64Const((ptr + ((i as u32 + 1) * 8)) as i64));
                    v.push(Instruction::I32WrapI64);
                    v.extend(self.expr(elem)?);
                    v.push(Instruction::I64Store(ma));
                }
                v.push(Instruction::I64Const(((ptr as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "car" | "first" => {
                if a.len() != 1 { return Err("car: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__car_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // ptr + 8 (skip count word) → first element
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                Ok(v)
            }
            "map" => {
                if a.len() != 2 { return Err("map: need (map fn lst)".into()); }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "map")?;
                let arr_tmp = self.local_idx("__map_arr");
                let n_tmp = self.local_idx("__map_n");
                let i_tmp = self.local_idx("__map_i");
                let new_ptr = self.local_idx("__map_new");
                let res_tmp = self.local_idx("__map_res");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate lst, untag, save
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count from arr[0]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new array at heap
                let new_heap = self.heap_ptr;
                let slots = 1 + 64; // count + max 64 elements
                self.heap_ptr += slots * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store count at new[0]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // i = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n, break
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element: arr[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                // Bind to param
                v.push(Instruction::LocalSet(p_idx));
                // Evaluate body
                v.extend(self.expr(&body)?);
                v.push(Instruction::LocalSet(res_tmp));
                // Store result at new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(res_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return tagged new array
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "filter" => {
                if a.len() != 2 { return Err("filter: need (filter fn lst)".into()); }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "filter")?;
                let arr_tmp = self.local_idx("__fil_arr");
                let n_tmp = self.local_idx("__fil_n");
                let i_tmp = self.local_idx("__fil_i");
                let write_i = self.local_idx("__fil_w");
                let elem_tmp = self.local_idx("__fil_e");
                let _pred_tmp = self.local_idx("__fil_p");
                let new_ptr = self.local_idx("__fil_new");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate lst
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new array
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store initial count 0
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(ma));
                // i=0, write_i=0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(write_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(elem_tmp));
                // Bind param, eval predicate
                v.push(Instruction::LocalGet(elem_tmp));
                v.push(Instruction::LocalSet(p_idx));
                v.extend(self.expr(&body)?);
                // Check truthy: untag, then compare raw value != 0
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                // Store element at new[(write_i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(elem_tmp));
                v.push(Instruction::I64Store(ma));
                // Increment count at new[0]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // write_i++
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::End); // if
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Return tagged new array
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "cdr" | "rest" => {
                if a.len() != 1 { return Err("cdr: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__cdr_arr");
                let n_tmp = self.local_idx("__cdr_n");
                let new_ptr = self.local_idx("__cdr_new");
                let i_tmp = self.local_idx("__cdr_i");
                let val_tmp = self.local_idx("__cdr_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // new_count = count - 1
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store new_count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy elements 1..old_n to new[1..new_n]
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(i+2)*8] (skip count word + skip elem 0)
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // If new_count == 0, return nil instead of empty array
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                v.push(Instruction::End);
                Ok(v)
            }
            "cons" => {
                if a.len() != 2 { return Err("cons: expected 2 args".into()); }
                let item_tmp = self.local_idx("__cons_item");
                let arr_tmp = self.local_idx("__cons_arr");
                let n_tmp = self.local_idx("__cons_n");
                let new_ptr = self.local_idx("__cons_new");
                let i_tmp = self.local_idx("__cons_i");
                let val_tmp = self.local_idx("__cons_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval lst first (so item is evaluated after, but order doesn't matter for pure)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Eval item
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(item_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new: count + 1 elements
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store new_count = old_count + 1
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Store item at new[1]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(item_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy old elements to new[2..]
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+2)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "len" => {
                if a.len() != 1 { return Err("len: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "length" => {
                if a.len() != 1 { return Err("length: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "nth" => {
                if a.len() != 2 { return Err("nth: expected 2 args".into()); }
                let arr_tmp = self.local_idx("__nth_arr");
                let idx_tmp = self.local_idx("__nth_i");
                let len_tmp = self.local_idx("__nth_len");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(idx_tmp));
                // Load list length (ptr[0])
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(len_tmp));
                // Bounds check: idx < len, otherwise trap
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::LocalGet(len_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable); // out of bounds
                v.push(Instruction::End);
                // Load ptr[(idx+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(idx_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                Ok(v)
            }
            "range" => {
                if a.len() != 2 { return Err("range: need (range start end)".into()); }
                let start_tmp = self.local_idx("__rng_s");
                let end_tmp = self.local_idx("__rng_e");
                let i_tmp = self.local_idx("__rng_i");
                let write_i = self.local_idx("__rng_w");
                let new_ptr = self.local_idx("__rng_new");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(start_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(end_tmp));
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // count = 0
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(start_tmp));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(end_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Store i at new[(write_i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(i_tmp));
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Store(ma));
                // count++
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // write_i++, i++
                v.push(Instruction::LocalGet(write_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(write_i));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "reverse" => {
                if a.len() != 1 { return Err("reverse: expected 1 arg".into()); }
                let arr_tmp = self.local_idx("__rev_arr");
                let n_tmp = self.local_idx("__rev_n");
                let i_tmp = self.local_idx("__rev_i");
                let new_ptr = self.local_idx("__rev_new");
                let val_tmp = self.local_idx("__rev_v");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 64) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64Store(ma));
                // Copy in reverse
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load old[(n - i)*8] (1-indexed from count word)
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                // Store new[(i+1)*8]
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "reduce" => {
                if a.len() != 3 { return Err("reduce: need (reduce fn init lst)".into()); }
                let (acc_name, elem_name, body) = self.resolve_lambda_2(&a[0], "reduce")?;
                let arr_tmp = self.local_idx("__red_arr");
                let n_tmp = self.local_idx("__red_n");
                let i_tmp = self.local_idx("__red_i");
                let acc_local = self.local_idx(&acc_name);
                let elem_local = self.local_idx(&elem_name);
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval init → acc
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(acc_local));
                // Eval lst
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // i = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                // Load element arr[(i+1)*8]
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(elem_local));
                // Eval body with acc and elem bound
                v.extend(self.expr(&body)?);
                v.push(Instruction::LocalSet(acc_local));
                // i++
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Result is acc
                v.push(Instruction::LocalGet(acc_local));
                Ok(v)
            }
            "append" => {
                if a.len() != 2 { return Err("append: expected 2 args".into()); }
                let a1_tmp = self.local_idx("__ap_a");
                let a2_tmp = self.local_idx("__ap_b");
                let n1_tmp = self.local_idx("__ap_n1");
                let n2_tmp = self.local_idx("__ap_n2");
                let i_tmp = self.local_idx("__ap_i");
                let val_tmp = self.local_idx("__ap_v");
                let new_ptr = self.local_idx("__ap_new");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a1_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a2_tmp));
                // Load counts
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n1_tmp));
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n2_tmp));
                // Alloc new
                let new_heap = self.heap_ptr;
                self.heap_ptr += (1 + 128) * 8;
                v.push(Instruction::I64Const(new_heap as i64));
                v.push(Instruction::LocalSet(new_ptr));
                // Store total count
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::LocalGet(n2_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma));
                // Copy a1
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Copy a2 starting at offset n1
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::LocalGet(n2_tmp));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(val_tmp));
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(n1_tmp));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(i_tmp));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_tmp));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
