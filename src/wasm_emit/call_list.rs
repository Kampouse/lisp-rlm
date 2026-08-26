use super::*;

impl WasmEmitter {
    pub(crate) fn call_list(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "array" => {
                // (array elem0 elem1 ...) → TAG_ARRAY
                // Allocate on compile-time heap: [count, elem0, elem1, ...]
                let count = a.len() as u32;
                let slots_needed = 1 + count; // count + elements
                let alloc_size = slots_needed * 8;
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                if self.p2_mode || self.wasi_mode {
                    let alloc_local = self.local_idx("__arr_alloc");
                    v.extend(self.heap_bump_runtime(alloc_size, "__arr_alloc"));
                    // Store count at ptr[0]
                    v.push(Instruction::LocalGet(alloc_local));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(count as i64));
                    v.push(Instruction::I64Store(ma));
                    // Evaluate and store each element
                    for (i, elem) in a.iter().enumerate() {
                        // I64Store expects [i32 addr, i64 val] — push address first
                        v.push(Instruction::LocalGet(alloc_local));
                        v.push(Instruction::I64Const(((i as u32 + 1) * 8) as i64));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.extend(self.expr(elem)?);
                        v.push(Instruction::I64Store(ma));
                    }
                    // Return tagged array ptr
                    v.push(Instruction::LocalGet(alloc_local));
                    v.push(Instruction::I64Const(TAG_BITS as i64));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(TAG_ARRAY));
                    v.push(Instruction::I64Or);
                } else {
                    let ptr = self.heap_bump(alloc_size);
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
                    v.push(Instruction::I64Const(
                        ((ptr as i64) << TAG_BITS) | TAG_ARRAY,
                    ));
                }
                Ok(v)
            }
            "vec-length" => {
                if a.len() != 1 {
                    return Err("vec-length: expected 1 arg".into());
                }
                let arr_tmp = self.local_idx("__vl_arr");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                if a.len() != 2 {
                    return Err("vec-nth: expected 2 args".into());
                }
                let arr_tmp = self.local_idx_i32("__vn_arr");
                let idx_tmp = self.local_idx_i32("__vn_idx");
                let count_tmp = self.local_idx_i32("__vn_count");
                let result_tmp = self.local_idx("__vn_result");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                if a.len() != 3 {
                    return Err("vec-set!: expected 3 args".into());
                }
                let arr_tmp = self.local_idx_i32("__vs_arr");
                let idx_tmp = self.local_idx_i32("__vs_idx");
                let val_tmp = self.local_idx("__vs_val");
                let count_tmp = self.local_idx_i32("__vs_count");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                if a.len() != 2 {
                    return Err("vec-push: expected 2 args".into());
                }
                let old_arr = self.local_idx("__vp_old");
                let new_arr = self.local_idx("__vp_new");
                let old_count = self.local_idx("__vp_oc");
                let word_idx = self.local_idx("__vp_wi");
                let val_tmp = self.local_idx("__vp_val");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                if a.len() != 1 {
                    return Err("vec?: expected 1 arg".into());
                }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7)); // tag mask
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Eq); // i32 result
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
                v.extend(offset_expr);
                v.push(Instruction::LocalSet(off_i));
                v.extend(size_expr);
                v.push(Instruction::LocalSet(sz_i));
                // Store length at offset-8
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(sz_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Zero-fill loop
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(sz_i));
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[offset + i*8] = 0
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_i));
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
                v.extend(idx);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                Ok(v)
            }
            "arr_set" => {
                let off = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let val = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(off);
                v.extend(idx);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "arr_len" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                Ok(v)
            }
            "arr_push" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__ap_off");
                let len_i = self.local_idx("__ap_len");
                let mut v = Vec::new();
                v.extend(off);
                v.push(Instruction::LocalSet(off_i));
                // Load current length from offset-8
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalSet(len_i));
                // Store val at offset + len*8
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Increment length
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
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
                v.extend(off);
                v.push(Instruction::LocalSet(off_i));
                // n = mem[(offset-8)]
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalSet(n_i));
                // Outer loop: i = 0..n-1
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= n-1: br 2 (exit)
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(n_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // j = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(j_i));
                // Inner loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if j >= n-i-1: br 2
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::LocalGet(n_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // tmp = arr[j], load arr[j+1]
                // Compare: if arr[j] > arr[j+1], swap
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalSet(tmp_i)); // tmp = arr[j]
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })); // arr[j+1]
                     // stack: arr[j+1]; tmp_i = arr[j]
                     // if arr[j] > arr[j+1] → swap
                v.push(Instruction::LocalGet(tmp_i)); // tmp, arr[j+1] on stack
                v.push(Instruction::I64LtS); // arr[j+1] < arr[j] i.e. arr[j] > arr[j+1]
                v.push(Instruction::If(BlockType::Empty));
                // arr[j] = arr[j+1]
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // arr[j+1] = tmp
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::End); // if swap
                v.push(Instruction::LocalGet(j_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(j_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // inner loop
                v.push(Instruction::End); // inner block
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // outer loop
                v.push(Instruction::End); // outer block
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "arr_find" => {
                let off = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let off_i = self.local_idx("__af_off");
                let val_i = self.local_idx("__af_val");
                let n_i = self.local_idx("__af_n");
                let i_i = self.local_idx("__af_i");
                let mut v = Vec::new();
                v.extend(off);
                v.push(Instruction::LocalSet(off_i));
                v.extend(val);
                v.push(Instruction::LocalSet(val_i));
                // Load length
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalSet(n_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(n_i));
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::Br(2)); // not found
                v.push(Instruction::End);
                // if arr[i] == val → return i
                v.push(Instruction::LocalGet(off_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::Br(2)); // found
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(-1)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }
            "list" => {
                // Two paths:
                // 1) ALL-literal list (e.g. constants like (c-p _d) (list ...)):
                //    a compile-time STATIC address is safe — the content never
                //    changes, so sharing one buffer across calls is fine and
                //    avoids heap churn (fe-mul calls (c-pp 0)/(c-p 0) ~186× per
                //    invocation; runtime alloc there would burn ~15KB/call).
                // 2) ANY dynamic element: must allocate at RUNTIME. A static
                //    address would make every execution of the same list site
                //    share ONE buffer — two live lists from the same call site
                //    would alias and clobber each other.
                let count = a.len() as u32;
                let slots_needed = 1 + count;
                let is_constant = a.iter().all(|x| matches!(x, LispVal::Num(_) | LispVal::Bool(_) | LispVal::Nil));
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                if is_constant {
                    let ptr = self.heap_bump(slots_needed * 8);
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
                } else {
                    let list_ptr_id = self.list_ptr_counter;
                    self.list_ptr_counter += 1;
                    let ptr_local = self.local_idx(&format!("__lst_ptr_{}", list_ptr_id));
                    // [old_ptr] on stack (guarded against mem_limit)
                    v.extend(self.emit_runtime_alloc((slots_needed * 8) as i64));
                    v.push(Instruction::LocalSet(ptr_local));
                    v.push(Instruction::LocalGet(ptr_local));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(count as i64));
                    v.push(Instruction::I64Store(ma));
                    for (i, elem) in a.iter().enumerate() {
                        v.push(Instruction::LocalGet(ptr_local));
                        v.push(Instruction::I64Const(((i as u32 + 1) * 8) as i64));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.extend(self.expr(elem)?);
                        v.push(Instruction::I64Store(ma));
                    }
                    v.push(Instruction::LocalGet(ptr_local));
                    v.push(Instruction::I64Const(TAG_BITS as i64));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(TAG_ARRAY));
                    v.push(Instruction::I64Or);
                }
                Ok(v)
            }
            "car" | "first" => {
                if a.len() != 1 {
                    return Err("car: expected 1 arg".into());
                }
                let arr_tmp = self.local_idx("__car_arr");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let result_tmp = self.local_idx("__car_res");
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Default result: nil
                v.push(Instruction::I64Const(4)); // TAG_NIL
                v.push(Instruction::LocalSet(result_tmp));
                // Only load if arr_tmp != 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(0)); // skip if nil
                // ptr + 8 (skip count word) → first element
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(result_tmp));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(result_tmp));
                Ok(v)
            }
            "map" => {
                if a.len() != 2 {
                    return Err("map: need (map fn lst)".into());
                }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "map")?;
                let arr_tmp = self.local_idx("__map_arr");
                let n_tmp = self.local_idx("__map_n");
                let i_tmp = self.local_idx("__map_i");
                let new_ptr = self.local_idx("__map_new");
                let res_tmp = self.local_idx("__map_res");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                // Alloc new array at heap (fixed max allocation since count is runtime)
                let alloc_size = 64 * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__map_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__map_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "filter" => {
                if a.len() != 2 {
                    return Err("filter: need (filter fn lst)".into());
                }
                let (param_name, body) = self.resolve_lambda_1(&a[0], "filter")?;
                let arr_tmp = self.local_idx("__fil_arr");
                let n_tmp = self.local_idx("__fil_n");
                let i_tmp = self.local_idx("__fil_i");
                let write_i = self.local_idx("__fil_w");
                let elem_tmp = self.local_idx("__fil_e");
                let _pred_tmp = self.local_idx("__fil_p");
                let new_ptr = self.local_idx("__fil_new");
                let p_idx = self.local_idx(&param_name);
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__fil_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__fil_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "cdr" | "rest" => {
                if a.len() != 1 {
                    return Err("cdr: expected 1 arg".into());
                }
                let arr_tmp = self.local_idx("__cdr_arr");
                let n_tmp = self.local_idx("__cdr_n");
                let new_ptr = self.local_idx("__cdr_new");
                let i_tmp = self.local_idx("__cdr_i");
                let val_tmp = self.local_idx("__cdr_v");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Guard: if arr_tmp == 0 (nil), return nil
                let cdr_res = self.local_idx("__cdr_res");
                v.push(Instruction::I64Const(4)); // TAG_NIL
                v.push(Instruction::LocalSet(cdr_res));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(0)); // skip if nil
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
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__cdr_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__cdr_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                v.push(Instruction::End);
                v.push(Instruction::LocalSet(cdr_res));
                v.push(Instruction::End); // end nil-guard block
                v.push(Instruction::LocalGet(cdr_res));
                Ok(v)
            }
            "cons" => {
                if a.len() != 2 {
                    return Err("cons: expected 2 args".into());
                }
                let item_tmp = self.local_idx("__cons_item");
                let arr_tmp = self.local_idx("__cons_arr");
                let n_tmp = self.local_idx("__cons_n");
                let new_ptr = self.local_idx("__cons_new");
                let i_tmp = self.local_idx("__cons_i");
                let val_tmp = self.local_idx("__cons_v");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                // Eval lst first (so item is evaluated after, but order doesn't matter for pure)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Eval item
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(item_tmp));
                // Load count (nil = 0 elements, not a heap pointer)
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0)); // nil → count 0
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::End);
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new: count + 1 elements
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__cons_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__cons_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "len" => {
                if a.len() != 1 {
                    return Err("len: expected 1 arg".into());
                }
                // len handles both TAG_STR (length from tagged value) and TAG_ARRAY (count from memory)
                let val_tmp = self.local_idx("__len_val");
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = self.expr(&a[0])?;
                // Save tagged value in local (evaluate ONCE)
                v.push(Instruction::LocalSet(val_tmp));
                // Check tag: TAG_STR=5 → extract len from value, TAG_ARRAY=6 → load count from memory
                v.push(Instruction::LocalGet(val_tmp));
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                // TAG_STR? (val & 7 == 5)
                v.push(Instruction::I64Const(5));
                v.push(Instruction::I64Eq); // i32
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // TAG_STR: untag then extract len (upper 32 bits)
                v.push(Instruction::LocalGet(val_tmp));
                v.extend(self.emit_untag()); // (len << 32 | ptr)
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // len
                v.push(Instruction::Else);
                // TAG_ARRAY: untag → ptr, load count from arr[0]
                v.push(Instruction::LocalGet(val_tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::End);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "length" => {
                if a.len() != 1 {
                    return Err("length: expected 1 arg".into());
                }
                let arr_tmp = self.local_idx("__len_arr");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Guard: if arr_tmp == 0 (nil/empty), return count 0
                let len_res = self.local_idx("__len_res");
                v.push(Instruction::I64Const(0)); // count = 0
                v.push(Instruction::LocalSet(len_res));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(0)); // skip if nil
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(len_res));
                v.push(Instruction::End); // end nil-guard block
                v.push(Instruction::LocalGet(len_res));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "nth" => {
                if a.len() != 2 {
                    return Err("nth: expected 2 args".into());
                }
                let arr_tmp = self.local_idx("__nth_arr");
                let idx_tmp = self.local_idx("__nth_i");
                let len_tmp = self.local_idx("__nth_len");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(idx_tmp));
                // Guard: if arr_tmp == 0 (nil), return nil
                let nth_res = self.local_idx("__nth_res");
                v.push(Instruction::I64Const(4)); // TAG_NIL
                v.push(Instruction::LocalSet(nth_res));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(0)); // skip if nil
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
                v.push(Instruction::LocalSet(nth_res));
                v.push(Instruction::End); // end nil-guard block
                v.push(Instruction::LocalGet(nth_res));
                Ok(v)
            }
            "range" => {
                if a.len() != 2 {
                    return Err("range: need (range start end)".into());
                }
                let start_tmp = self.local_idx("__rng_s");
                let end_tmp = self.local_idx("__rng_e");
                let i_tmp = self.local_idx("__rng_i");
                let write_i = self.local_idx("__rng_w");
                let new_ptr = self.local_idx("__rng_new");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(start_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(end_tmp));
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__rng_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__rng_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "reverse" => {
                if a.len() != 1 {
                    return Err("reverse: expected 1 arg".into());
                }
                let arr_tmp = self.local_idx("__rev_arr");
                let n_tmp = self.local_idx("__rev_n");
                let i_tmp = self.local_idx("__rev_i");
                let new_ptr = self.local_idx("__rev_new");
                let val_tmp = self.local_idx("__rev_v");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_tmp));
                // Load count
                v.push(Instruction::LocalGet(arr_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n_tmp));
                // Alloc new
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__rev_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__rev_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                Ok(v)
            }
            "reduce" => {
                if a.len() != 3 {
                    return Err("reduce: need (reduce fn init lst)".into());
                }
                let (acc_name, elem_name, body) = self.resolve_lambda_2(&a[0], "reduce")?;
                let arr_tmp = self.local_idx("__red_arr");
                let n_tmp = self.local_idx("__red_n");
                let i_tmp = self.local_idx("__red_i");
                let acc_local = self.local_idx(&acc_name);
                let elem_local = self.local_idx(&elem_name);
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
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
                if a.len() != 2 {
                    return Err("append: expected 2 args".into());
                }
                let a1_tmp = self.local_idx("__ap_a");
                let a2_tmp = self.local_idx("__ap_b");
                let n1_tmp = self.local_idx("__ap_n1");
                let n2_tmp = self.local_idx("__ap_n2");
                let i_tmp = self.local_idx("__ap_i");
                let val_tmp = self.local_idx("__ap_v");
                let new_ptr = self.local_idx("__ap_new");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a1_tmp));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(a2_tmp));
                // Guard: if either arg is nil (ptr==0), return the other re-tagged
                let ap_res = self.local_idx("__ap_res");
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I32Or);
                v.push(Instruction::If(BlockType::Empty));
                // Either is nil — store the non-nil one
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // a1 is nil → a2 re-tagged
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(ap_res));
                v.push(Instruction::Else);
                // a2 is nil → a1 re-tagged
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(6));
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(ap_res));
                v.push(Instruction::End);
                v.push(Instruction::Br(1)); // skip main body
                v.push(Instruction::End); // end nil guard
                // Load counts (main body: both non-nil)
                v.push(Instruction::LocalGet(a1_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n1_tmp));
                v.push(Instruction::LocalGet(a2_tmp));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(n2_tmp));
                // Alloc new
                let alloc_size = (1 + 64) * 8;
                if self.p2_mode || self.wasi_mode {
                    v.extend(self.heap_bump_runtime(alloc_size, "__ap_alloc"));
                    v.push(Instruction::LocalGet(self.local_idx("__ap_alloc")));
                } else {
                    let new_heap = self.heap_bump(alloc_size);
                    v.push(Instruction::I64Const(new_heap as i64));
                }
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
                v.push(Instruction::LocalGet(new_ptr));
                v.push(Instruction::I64Const(TAG_BITS as i64));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(ap_res));
                v.push(Instruction::End); // end outer block
                v.push(Instruction::LocalGet(ap_res));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}

impl WasmEmitter {
    /// __h_arr_to_str(arr_ptr i64) -> tagged str — renders [count, e0, e1..]
    /// arrays per interpreter LispVal::to_string(): "(e0 e1 ...)" with ' '
    /// separators; strings quoted, nested arrays recursive, nil/bools/nums.
    /// Self-recursive for TAG_ARRAY elements. Requires u128 str helpers first
    /// (for num rendering).
    pub(crate) fn ensure_arr_str_helper(&mut self) -> u32 {
        if let Some(idx) = self.arr_str_helper {
            return idx;
        }
        let h = self.ensure_u128_str_helpers();
        let mem_limit = (self.memory_pages as i64) * 65536;
        let idx = self.funcs.len();
        let v = Self::h_arr_to_str(idx as u32, h.i64_to_str, mem_limit);
        self.funcs.push(FuncDef {
            name: "__h_arr_to_str".into(),
            param_count: 1, local_count: 11,
            instrs: v,
            local_entries: None, custom_type: None,
        });
        self.arr_str_helper = Some(idx as u32);
        idx as u32
    }

    fn h_arr_to_str(self_idx: u32, i64_to_str: u32, mem_limit: i64) -> Vec<Instruction<'static>> {
        use Instruction as I;
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }; // 8-byte
        let ma4 = wasm_encoder::MemArg { offset: 0, align: 2, memory_index: 0 }; // 4-byte
        let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 }; // byte
        let mut v: Vec<Instruction<'static>> = Vec::new();
        // locals: 0=arr 1=n 2=dst 3=w 4=i 5=elem 6=slen 7=sptr 8=stag 9=j
        // n = load(arr)
        v.push(I::LocalGet(0)); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone())); v.push(I::LocalSet(1));
        // dst = heap bump: n*24 + 32 (24 bytes per element is generous)
        v.push(I::I64Const(56)); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone())); v.push(I::LocalSet(2));
        v.push(I::LocalGet(2)); v.push(I::LocalGet(1)); v.push(I::I64Const(24)); v.push(I::I64Mul); v.push(I::I64Add); v.push(I::I64Const(32)); v.push(I::I64Add); v.push(I::LocalSet(3)); // w = new heap top
        v.push(I::LocalGet(3)); v.push(I::I64Const(mem_limit)); v.push(I::I64LtU);
        v.push(I::If(BlockType::Empty));
        v.push(I::I64Const(56)); v.push(I::I32WrapI64); v.push(I::LocalGet(3)); v.push(I::I64Store(ma8.clone()));
        v.push(I::Else); v.push(I::Unreachable); v.push(I::End);
        // w = dst; write '('
        v.push(I::LocalGet(2)); v.push(I::LocalSet(3));
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x28)); v.push(I::I32Store8(ma1.clone())); v.push(I::LocalGet(3)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(3));
        // i = 0
        v.push(I::I64Const(0)); v.push(I::LocalSet(4));
        // loop
        v.push(I::Block(BlockType::Empty));
        v.push(I::Loop(BlockType::Empty));
        // if i >= n → break
        v.push(I::LocalGet(4)); v.push(I::LocalGet(1)); v.push(I::I64GeU); v.push(I::BrIf(1));
        // sep ' ' if i > 0
        v.push(I::LocalGet(4)); v.push(I::I64Const(0)); v.push(I::I64GtU);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x20)); v.push(I::I32Store8(ma1.clone())); v.push(I::LocalGet(3)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::End);
        // elem = load(arr + (i+1)*8)
        v.push(I::LocalGet(0)); v.push(I::LocalGet(4)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::I64Const(8)); v.push(I::I64Mul); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone())); v.push(I::LocalSet(5));
        // ── dispatch on tag ──
        // TAG_ARRAY (6)? recurse
        v.push(I::LocalGet(5)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::I64Const(6)); v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(5)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::Call(USER_BASE | self_idx)); v.push(I::LocalSet(8));
        v.push(I::LocalGet(8)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(32)); v.push(I::I64ShrU); v.push(I::LocalSet(6));
        v.push(I::LocalGet(8)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(0xFFFF_FFFF)); v.push(I::I64And); v.push(I::LocalSet(7));
        v.push(I::I64Const(0)); v.push(I::LocalSet(9));
        v.push(I::Block(BlockType::Empty)); v.push(I::Loop(BlockType::Empty));
        v.push(I::LocalGet(9)); v.push(I::LocalGet(6)); v.push(I::I64GeU); v.push(I::BrIf(1));
        v.push(I::LocalGet(3)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64);
        v.push(I::LocalGet(7)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I32Load8U(ma1.clone())); v.push(I::I32Store8(ma1.clone()));
        v.push(I::LocalGet(9)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(9));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End);
        v.push(I::LocalGet(3)); v.push(I::LocalGet(6)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::Else);
        // TAG_STR (5)?  "content"
        v.push(I::LocalGet(5)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::I64Const(5)); v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(5)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(32)); v.push(I::I64ShrU); v.push(I::LocalSet(6));
        v.push(I::LocalGet(5)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(0xFFFF_FFFF)); v.push(I::I64And); v.push(I::LocalSet(7));
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x22)); v.push(I::I32Store8(ma1.clone())); v.push(I::LocalGet(3)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::I64Const(0)); v.push(I::LocalSet(9));
        v.push(I::Block(BlockType::Empty)); v.push(I::Loop(BlockType::Empty));
        v.push(I::LocalGet(9)); v.push(I::LocalGet(6)); v.push(I::I64GeU); v.push(I::BrIf(1));
        v.push(I::LocalGet(3)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64);
        v.push(I::LocalGet(7)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I32Load8U(ma1.clone())); v.push(I::I32Store8(ma1.clone()));
        v.push(I::LocalGet(9)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(9));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End);
        // w = w + slen (past content); closing '"' AT w; then w++
        v.push(I::LocalGet(3)); v.push(I::LocalGet(6)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x22)); v.push(I::I32Store8(ma1.clone()));
        v.push(I::LocalGet(3)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::Else);
        // TAG_NUM (0)? i64_to_str then copy
        v.push(I::LocalGet(5)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::I64Const(0)); v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(5)); v.push(I::I64Const(3)); v.push(I::I64ShrS); v.push(I::Call(USER_BASE | i64_to_str)); v.push(I::LocalSet(8));
        v.push(I::LocalGet(8)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(32)); v.push(I::I64ShrU); v.push(I::LocalSet(6));
        v.push(I::LocalGet(8)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Const(0xFFFF_FFFF)); v.push(I::I64And); v.push(I::LocalSet(7));
        v.push(I::I64Const(0)); v.push(I::LocalSet(9));
        v.push(I::Block(BlockType::Empty)); v.push(I::Loop(BlockType::Empty));
        v.push(I::LocalGet(9)); v.push(I::LocalGet(6)); v.push(I::I64GeU); v.push(I::BrIf(1));
        v.push(I::LocalGet(3)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64);
        v.push(I::LocalGet(7)); v.push(I::LocalGet(9)); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I32Load8U(ma1.clone())); v.push(I::I32Store8(ma1.clone()));
        v.push(I::LocalGet(9)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(9));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End);
        v.push(I::LocalGet(3)); v.push(I::LocalGet(6)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::Else);
        // TAG_BOOL (1)? true/false
        v.push(I::LocalGet(5)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::I64Const(1)); v.push(I::I64Eq);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(5)); v.push(I::I64Const(3)); v.push(I::I64ShrU); v.push(I::I64Eqz);
        v.push(I::If(BlockType::Empty));
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x736c_6166)); v.push(I::I32Store(ma4.clone())); // "fals"
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(4)); v.push(I::I32Add); v.push(I::I32Const(0x65)); v.push(I::I32Store8(ma1.clone())); // 'e' at w+4
        v.push(I::LocalGet(3)); v.push(I::I64Const(5)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::Else);
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x6575_7274)); v.push(I::I32Store(ma4.clone())); // "true"
        v.push(I::LocalGet(3)); v.push(I::I64Const(4)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::End);
        v.push(I::Else);
        // TAG_NIL (4)? "nil"  (and FNREF/CLOSURE fallthrough → "nil" too)
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x006c_696e)); v.push(I::I32Store(ma4.clone())); // "nil"
        v.push(I::LocalGet(3)); v.push(I::I64Const(3)); v.push(I::I64Add); v.push(I::LocalSet(3));
        v.push(I::End); // bool
        v.push(I::End); // num
        v.push(I::End); // str
        v.push(I::End); // array
        // i += 1; br loop
        v.push(I::LocalGet(4)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(4));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End); // loop/block
        // ')'
        v.push(I::LocalGet(3)); v.push(I::I32WrapI64); v.push(I::I32Const(0x29)); v.push(I::I32Store8(ma1.clone())); v.push(I::LocalGet(3)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(3));
        // tagged = ((len<<32)|dst)<<TAG_BITS | TAG_STR ; len = w - dst
        v.push(I::LocalGet(3)); v.push(I::LocalGet(2)); v.push(I::I64Sub); v.push(I::I64Const(32)); v.push(I::I64Shl);
        v.push(I::LocalGet(2)); v.push(I::I64Or);
        v.push(I::I64Const(3)); v.push(I::I64Shl); v.push(I::I64Const(5)); v.push(I::I64Or);
        v
    }
}
