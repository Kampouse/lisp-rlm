use super::*;

impl WasmEmitter {
    pub(crate) fn call_dict(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "dict" => {
                if a.len() % 2 != 0 { return Err("dict: expected even number of args (key val pairs)".into()); }
                let n_pairs = a.len() / 2;
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let heap = self.heap_ptr;
                // count + 2*n_pairs elements
                let total_slots = 1 + 2 * n_pairs;
                self.heap_ptr = heap + (total_slots * 8) as u32;
                // But we need extra for alloc_data or strings — no, values are already tagged
                // We need enough space. Pad to 64 slots minimum for safety.
                if total_slots < 64 { self.heap_ptr = heap + 64 * 8; }
                let mut v = Vec::new();
                // Store n_pairs at ptr[0]: addr, value, store
                v.push(Instruction::I64Const(heap as i64));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(n_pairs as i64));
                v.push(Instruction::I64Store(ma));
                // Store key/val pairs
                for i in 0..n_pairs {
                    let off = (1 + 2 * i) as u64;
                    let tmp = self.local_idx("__dict_kv");
                    // key: emit value → save to local → push addr → push value → store
                    v.extend(self.expr(&a[2 * i])?);
                    v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::I64Const(heap as i64 + off as i64 * 8));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(tmp));
                    v.push(Instruction::I64Store(ma));
                    // val: same pattern
                    v.extend(self.expr(&a[2 * i + 1])?);
                    let off2 = (2 + 2 * i) as u64;
                    v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::I64Const(heap as i64 + off2 as i64 * 8));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(tmp));
                    v.push(Instruction::I64Store(ma));
                }
                // Return tagged array
                v.push(Instruction::I64Const(((heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "dict/get" => {
                if a.len() != 2 { return Err("dict/get: expected 2 args (dict key)".into()); }
                let d_ptr = self.local_idx("__dget_ptr");
                let n = self.local_idx("__dget_n");
                let key = self.local_idx("__dget_key");
                let idx = self.local_idx("__dget_idx");
                let k_raw = self.local_idx("__dget_kraw");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval key first
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(key));
                // Eval dict, untag → ptr
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(d_ptr));
                // Load n_pairs
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Loop
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64))); // $break
                v.push(Instruction::Loop(BlockType::Empty)); // $loop
                    // if idx >= n_pairs → not found, break with nil
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::If(BlockType::Empty));
                        v.push(Instruction::I64Const(TAG_NIL));
                        v.push(Instruction::Br(2)); // break out of Block with nil
                    v.push(Instruction::End);
                    // Load key at ptr[1 + 2*idx]
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                    // Compare with search key
                    v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                    v.extend(self.emit_str_eq());
                    v.push(Instruction::I64Const(8)); // tagged true
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Found! Load val at ptr[2 + 2*idx]
                        v.push(Instruction::LocalGet(d_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma));
                        v.push(Instruction::Br(2)); // break out of Block with val
                    v.push(Instruction::End);
                    // idx++, continue loop
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                    v.push(Instruction::Br(0)); // continue
                v.push(Instruction::End); // loop
                v.push(Instruction::Unreachable); // should never reach here
                v.push(Instruction::End); // block
                Ok(v)
            }
            "dict/set" => {
                if a.len() != 3 { return Err("dict/set: expected 3 args (dict key val)".into()); }
                let d_ptr = self.local_idx("__dset_ptr");
                let n = self.local_idx("__dset_n");
                let key = self.local_idx("__dset_key");
                let val = self.local_idx("__dset_val");
                let idx = self.local_idx("__dset_idx");
                let k_raw = self.local_idx("__dset_kraw");
                let found = self.local_idx("__dset_found");
                let new_ptr = self.local_idx("__dset_new");
                let i2 = self.local_idx("__dset_i2");
                let v_tmp = self.local_idx("__dset_vtmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Eval key and val first
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(key));
                v.extend(self.expr(&a[2])?); v.push(Instruction::LocalSet(val));
                // Eval dict
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(d_ptr));
                // Load n_pairs
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Scan for existing key
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(found)); // 0=not found
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load key at ptr[1 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                // Compare
                v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                v.extend(self.emit_str_eq());
                v.push(Instruction::I64Const(8)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalSet(found));
                    v.push(Instruction::Br(2)); // break
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block

                // Now: found != 0 means key exists at index (found-1)... actually found = idx
                // Determine new count: if found, same count; else count+1
                // Alloc new dict
                let new_heap = self.heap_ptr;
                // Max slots needed: 1 + 2*(n+1) — enough for either case
                let alloc_slots = 1 + 2 * 64; // generous allocation (max 64 pairs)
                self.heap_ptr = new_heap + alloc_slots * 8;
                v.push(Instruction::I64Const(new_heap as i64)); v.push(Instruction::LocalSet(new_ptr));

                // Branch: key found or not
                v.push(Instruction::LocalGet(found));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Ne);
                v.push(Instruction::If(BlockType::Empty));
                    // --- Key exists: same count, copy all, update val at found index ---
                    v.push(Instruction::LocalGet(new_ptr)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64Store(ma));
                    // Copy all pairs
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                    // Copy key at old[1+2*i]
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    // Copy val at old[2+2*i] — if i == found, use new val instead
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(found));
                    v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Use new val
                        v.push(Instruction::LocalGet(new_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(val));
                        v.push(Instruction::I64Store(ma));
                    v.push(Instruction::Else);
                        // Copy old val
                        v.push(Instruction::LocalGet(d_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                        v.push(Instruction::LocalGet(new_ptr));
                        v.push(Instruction::I64Const(16));
                        v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                        v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(v_tmp));
                        v.push(Instruction::I64Store(ma));
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                v.push(Instruction::Else);
                    // --- Key not found: count+1, copy all old pairs, append new pair ---
                    v.push(Instruction::LocalGet(new_ptr)); v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Add);
                    v.push(Instruction::I64Store(ma));
                    // Copy all old pairs
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                    // Copy key
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    // Copy val
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(v_tmp));
                    v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(i2)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(i2));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // Append new pair: key at [1 + 2*n], val at [2 + 2*n]
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(key));
                    v.push(Instruction::I64Store(ma));
                    v.push(Instruction::LocalGet(new_ptr));
                    v.push(Instruction::I64Const(16));
                    v.push(Instruction::LocalGet(n)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(val));
                    v.push(Instruction::I64Store(ma));
                v.push(Instruction::End); // if found / not found
                // Return tagged new dict
                v.push(Instruction::I64Const(((new_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "dict/has?" => {
                if a.len() != 2 { return Err("dict/has?: expected 2 args (dict key)".into()); }
                let d_ptr = self.local_idx("__dhas_ptr");
                let n = self.local_idx("__dhas_n");
                let key = self.local_idx("__dhas_key");
                let idx = self.local_idx("__dhas_idx");
                let k_raw = self.local_idx("__dhas_kraw");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?); v.push(Instruction::LocalSet(key));
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64))); // $break
                v.push(Instruction::Loop(BlockType::Empty)); // $loop
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::If(BlockType::Empty));
                        // Not found → tagged false
                        v.push(Instruction::I64Const(1));
                        v.push(Instruction::Br(2)); // break out of Block
                    v.push(Instruction::End);
                    // Load key
                    v.push(Instruction::LocalGet(d_ptr));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_raw));
                    v.push(Instruction::LocalGet(key)); v.push(Instruction::LocalGet(k_raw));
                    v.extend(self.emit_str_eq());
                    v.push(Instruction::I64Const(8)); v.push(Instruction::I64Eq);
                    v.push(Instruction::If(BlockType::Empty));
                        // Found → tagged true
                        v.push(Instruction::I64Const(8));
                        v.push(Instruction::Br(2)); // break out of Block
                    v.push(Instruction::End);
                    // i++, continue
                    v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                    v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::Unreachable);
                v.push(Instruction::End); // block
                Ok(v)
            }
            "dict/keys" => {
                if a.len() != 1 { return Err("dict/keys: expected 1 arg (dict)".into()); }
                let d_ptr = self.local_idx("__dk_ptr");
                let n = self.local_idx("__dk_n");
                let idx = self.local_idx("__dk_idx");
                let k_tmp = self.local_idx("__dk_tmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                // Alloc result list: [n, key0, key1, ...]
                let res_heap = self.heap_ptr;
                let alloc = std::cmp::max(1 + n as usize, 64);
                self.heap_ptr = res_heap + (alloc * 8) as u32;
                // Store count
                v.push(Instruction::I64Const(res_heap as i64)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64Store(ma));
                // Copy keys
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load key from dict at [1 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(k_tmp));
                // Store to result at [1 + i]
                v.push(Instruction::I64Const(res_heap as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(k_tmp));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((res_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            "dict/vals" => {
                if a.len() != 1 { return Err("dict/vals: expected 1 arg (dict)".into()); }
                let d_ptr = self.local_idx("__dv_ptr");
                let n = self.local_idx("__dv_n");
                let idx = self.local_idx("__dv_idx");
                let v_tmp2 = self.local_idx("__dv_tmp");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag()); v.push(Instruction::LocalSet(d_ptr));
                v.push(Instruction::LocalGet(d_ptr)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(n));
                let res_heap = self.heap_ptr;
                let alloc = std::cmp::max(1 + n as usize, 64);
                self.heap_ptr = res_heap + (alloc * 8) as u32;
                v.push(Instruction::I64Const(res_heap as i64)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::LocalGet(n));
                v.push(Instruction::I64GeU); v.push(Instruction::BrIf(1));
                // Load val from dict at [2 + 2*i]
                v.push(Instruction::LocalGet(d_ptr));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(v_tmp2));
                // Store to result at [1 + i]
                v.push(Instruction::I64Const(res_heap as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(v_tmp2));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(idx)); v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add); v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::I64Const(((res_heap as i64) << TAG_BITS) | TAG_ARRAY));
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
