use super::*;

impl WasmEmitter {
    pub(crate) fn emit_borsh_serialize(
        &mut self,
        schema_name: &str,
        val_args: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        let schema = self
            .borsh_schemas
            .get(schema_name)
            .ok_or_else(|| format!("borsh-serialize: unknown schema '{}'", schema_name))?
            .clone();
        let pos = self.local_idx("__borsh_pos");
        let mut v: Vec<Instruction<'static>> =
            vec![Instruction::I64Const(BORSH_BUF), Instruction::LocalSet(pos)];
        // Collect field types
        let field_types: Vec<&BorshType> = match &schema {
            BorshType::Struct { fields } => fields.iter().map(|(_, bt)| bt).collect(),
            BorshType::Enum { variants } => {
                // Enum serialize: first val_arg is variant index (i64)
                // Then emit: write discriminant byte, then switch on variant to write fields
                if val_args.is_empty() {
                    return Err("borsh-serialize: Enum requires variant index as first arg".into());
                }
                let var_idx_arg = &val_args[0];
                // Write discriminant byte
                v.extend(self.expr(var_idx_arg)?);
                let disc_tmp = self.local_idx("__borsh_disc");
                v.push(Instruction::LocalSet(disc_tmp));
                // Store discriminant as u8 at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(disc_tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // pos += 1
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
                // Switch on variant index to write each variant's fields
                // Generate: if vi==0 { ... } else { if vi==1 { ... } else { ... } }
                let var_idx_local = self.local_idx("__borsh_var_idx");
                v.push(Instruction::LocalGet(disc_tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(var_idx_local));
                for (vi, (_vname, vfields)) in variants.iter().enumerate() {
                    if vi == 0 {
                        // First variant: check vi == 0
                        v.push(Instruction::LocalGet(var_idx_local));
                        v.push(Instruction::I64Const(0i64));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::If(BlockType::Empty));
                    } else if vi < variants.len() - 1 {
                        // Middle variant: Else + nested if vi == vi
                        v.push(Instruction::Else);
                        v.push(Instruction::LocalGet(var_idx_local));
                        v.push(Instruction::I64Const(vi as i64));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::If(BlockType::Empty));
                    } else {
                        // Last variant: Else (default/fallthrough)
                        v.push(Instruction::Else);
                    }
                    // Write this variant's fields
                    for (fi, (_, ftype)) in vfields.iter().enumerate() {
                        if 1 + fi >= val_args.len() {
                            break;
                        } // safety: skip if not enough args
                        v.extend(self.expr(&val_args[1 + fi])?);
                        let ftmp = self.local_idx("__borsh_ftmp");
                        v.push(Instruction::LocalSet(ftmp));
                        v.extend(self.borsh_write_field(ftype, ftmp, pos)?);
                    }
                }
                // Close nested if/else blocks: need (variants.len() - 1) End instructions
                // (= one End per If block, since Else closes the If's alternative)
                for _ in 0..variants.len().saturating_sub(1) {
                    v.push(Instruction::End);
                }
                // Skip normal field iteration below
                // Call value_return and return
                self.need_host(25);
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(BORSH_BUF));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(BORSH_BUF));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL));
                return Ok(v);
            }
            other => vec![other],
        };
        if val_args.len() != field_types.len() {
            return Err(format!(
                "borsh-serialize: expected {} fields, got {} values — struct field count mismatch",
                field_types.len(),
                val_args.len()
            ));
        }
        for (i, btype) in field_types.iter().enumerate() {
            v.extend(self.expr(&val_args[i])?);
            let tmp = self.local_idx("__borsh_tmp");
            v.push(Instruction::LocalSet(tmp));
            v.extend(self.borsh_write_field(btype, tmp, pos)?);
        }
        // Call value_return(total_len, BORSH_BUF) directly to return Borsh bytes
        // This bypasses the export wrapper's generic value_return
        self.need_host(25); // value_return host function
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(BORSH_BUF));
        v.push(Instruction::I64Sub); // total_len = pos - BORSH_BUF
        v.push(Instruction::I64Const(BORSH_BUF));
        // value_return(len, ptr)
        v.push(Self::host_call(25));
        // Set return flag so export wrapper skips its value_return
        v.push(Instruction::I64Const(1));
        v.push(Instruction::GlobalSet(RETURN_FLAG));
        v.push(Instruction::I64Const(TAG_NIL));
        Ok(v)
    }

    pub(crate) fn borsh_write_field(
        &mut self,
        btype: &BorshType,
        tmp: u32,
        pos: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let mut v: Vec<Instruction<'static>> = Vec::new();
        match btype {
            BorshType::I64 | BorshType::U64 => {
                // I64Store at pos: [addr_i32, val_i64]
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                // Use unsigned right shift for U64 to avoid sign-extension on large values
                // (tagged values > 2^61 set bit 63, making shr_s produce wrong results)
                if matches!(btype, BorshType::U64) {
                    v.push(Instruction::I64Const(TAG_BITS));
                    v.push(Instruction::I64ShrU);
                } else {
                    v.extend(self.emit_untag());
                }
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // pos += 8
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
            }
            BorshType::U32 => {
                // I32Store at pos: [addr_i32, val_i32]
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // pos += 4
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
            }
            BorshType::U8 | BorshType::Bool => {
                // I32Store8 at pos: [addr_i32, val_i32]
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // pos += 1
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
            }
            BorshType::U128 => {
                // U128 is represented as array [lo, hi] with count=2
                // Write 16 bytes: lo at pos, hi at pos+8
                let arr_lo = self.local_idx("__arr_lo");
                let arr_hi = self.local_idx("__arr_hi");
                // Load arr[1] (lo) - count is at arr[0], so lo is at arr+8
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_lo));
                // Load arr[2] (hi) - at arr+16
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_hi));
                // Write lo at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(arr_lo));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Write hi at pos+8
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(arr_hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // pos += 16
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
            }
            BorshType::F64 => {
                return Err("borsh-serialize: F64 not yet supported".into());
            }
            BorshType::String | BorshType::Bytes => {
                // Untag tmp to get raw: (heap_off | (len << 32))
                let raw = self.local_idx("__borsh_raw");
                let len = self.local_idx("__borsh_len");
                let src = self.local_idx("__borsh_src");
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(raw));
                // len = raw >> 32
                v.push(Instruction::LocalGet(raw));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(len));
                // src = raw & 0xFFFFFFFF
                v.push(Instruction::LocalGet(raw));
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(src));
                // Write 4-byte LE length at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(len));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // pos += 4
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
                // Byte-by-byte memcpy loop from src to pos for len bytes
                let idx = self.local_idx("__borsh_idx");
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if idx < len
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::LocalGet(len));
                v.push(Instruction::I64LtU);
                // I64LtU returns i32 directly — no wrap needed
                v.push(Instruction::If(BlockType::Empty));
                // dst addr: pos + idx
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                // src addr: src + idx, load byte
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // store byte
                v.push(Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // idx += 1
                v.push(Instruction::LocalGet(idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(idx));
                // Br(1) targets the Loop, not the If — continue iterating
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // if
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                                          // pos += len
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::LocalGet(len));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
            }
            BorshType::Option(inner) => {
                // Check if tmp's tag == TAG_NIL (nil = None, anything else = Some)
                // tmp & 7 extracts the 3-bit tag
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // nil → write 0x00 discriminant at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // pos += 1
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
                v.push(Instruction::Else);
                // some → write 0x01 discriminant at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                // pos += 1
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));
                // Recursively serialize inner value (tmp local still holds the value)
                v.extend(self.borsh_write_field(inner, tmp, pos)?);
                v.push(Instruction::End);
            }
            BorshType::Vec(inner) => {
                // tmp holds a TAG_ARRAY: heap layout [count, elem0, elem1, ...]
                // Untag to get heap ptr
                let arr_ptr = self.local_idx("__borsh_arr_ptr");
                let arr_count = self.local_idx("__borsh_arr_count");
                let arr_idx = self.local_idx("__borsh_arr_idx");
                let elem_tmp = self.local_idx("__borsh_elem_tmp");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };

                // Untag tmp → raw heap ptr
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(arr_ptr));

                // Read count from arr_ptr[0]
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(arr_count));

                // Write u32 LE count at pos
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(arr_count));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                // pos += 4
                v.push(Instruction::LocalGet(pos));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos));

                // Loop: for idx in 0..count, read arr_ptr[1+idx] and serialize
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(arr_idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if idx < count
                v.push(Instruction::LocalGet(arr_idx));
                v.push(Instruction::LocalGet(arr_count));
                v.push(Instruction::I64LtU);
                // I64LtU returns i32 directly — no wrap needed
                v.push(Instruction::If(BlockType::Empty));
                // Load element: arr_ptr + (1 + idx) * 8
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I64Const(8)); // skip count slot
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(arr_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalSet(elem_tmp));
                // Serialize element
                v.extend(self.borsh_write_field(inner, elem_tmp, pos)?);
                // idx += 1
                v.push(Instruction::LocalGet(arr_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(arr_idx));
                // Br(1) targets the Loop, not the If — continue iterating
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // if
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
            }
            BorshType::Struct { .. } => {
                return Err("borsh-serialize: nested struct serialize not supported — serialize fields individually".into());
            }
            BorshType::Enum { .. } => {
                return Err("borsh-serialize: Enum not yet supported".into());
            }
        }
        Ok(v)
    }

    pub(crate) fn emit_borsh_deserialize(
        &mut self,
        schema_name: &str,
        bytes_expr: Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let schema = self
            .borsh_schemas
            .get(schema_name)
            .ok_or_else(|| format!("borsh-deserialize: unknown schema '{}'", schema_name))?
            .clone();
        let src = self.local_idx("__borsh_src");
        let mut v: Vec<Instruction<'static>> = bytes_expr;
        // Untag to get raw pointer
        v.extend(self.emit_untag());
        // Extract ptr: raw & 0xFFFFFFFF
        v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(src));
        // Determine single-field vs multi-field
        match &schema {
            BorshType::Struct { fields } if fields.len() == 1 => {
                v.extend(self.borsh_read_field(&fields[0].1, src)?);
            }
            BorshType::Struct { fields } if fields.is_empty() => {
                return Err("borsh-deserialize: empty struct has no fields".into());
            }
            BorshType::Struct { .. } => {
                // Multi-field struct: read each field, store in runtime TAG_ARRAY
                if let BorshType::Struct { fields } = &schema {
                    let field_src = self.local_idx("__borsh_fsrc");
                    v.push(Instruction::LocalGet(src));
                    v.push(Instruction::LocalSet(field_src));

                    // Allocate runtime array: [count, field0, field1, ...]
                    let arr_slots = fields.len() as i64;
                    let arr_bytes = (1 + arr_slots) * 8; // count + elements
                    let arr_ptr = self.local_idx("__borsh_struct_arr");
                    v.extend(self.emit_runtime_alloc(arr_bytes));
                    v.push(Instruction::LocalSet(arr_ptr));

                    // Store count
                    v.push(Instruction::LocalGet(arr_ptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(arr_slots));
                    let ma = wasm_encoder::MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    };
                    v.push(Instruction::I64Store(ma));

                    // Read each field and store into array
                    for (i, (_fname, ftype)) in fields.iter().enumerate() {
                        // Read field value → tagged i64 on stack
                        v.extend(self.borsh_read_field(ftype, field_src)?);
                        let val_tmp = self.local_idx("__borsh_struct_val");
                        v.push(Instruction::LocalSet(val_tmp)); // save value
                                                                // Store at arr_ptr[1+i]
                        let slot_off = (1 + i) as i64 * 8;
                        v.push(Instruction::LocalGet(arr_ptr));
                        v.push(Instruction::I64Const(slot_off));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(val_tmp));
                        v.push(Instruction::I64Store(ma));
                        // Advance field_src by field size
                        let sz = Self::borsh_type_size(ftype);
                        if sz > 0 {
                            v.push(Instruction::LocalGet(field_src));
                            v.push(Instruction::I64Const(sz as i64));
                            v.push(Instruction::I64Add);
                            v.push(Instruction::LocalSet(field_src));
                        } else {
                            return Err(format!(
                                "borsh-deserialize: variable-length field '{}' in struct not yet supported",
                                _fname
                            ));
                        }
                    }
                    // Return tagged array
                    v.push(Instruction::LocalGet(arr_ptr));
                    v.extend(self.emit_tag(TAG_ARRAY));
                }
            }
            other => {
                v.extend(self.borsh_read_field(other, src)?);
            }
        }
        Ok(v)
    }

    pub(crate) fn borsh_type_size(btype: &BorshType) -> usize {
        match btype {
            BorshType::U8 | BorshType::Bool => 1,
            BorshType::U32 => 4,
            BorshType::I64 | BorshType::U64 | BorshType::F64 => 8,
            BorshType::U128 => 16,
            BorshType::Option(inner) => 1 + Self::borsh_type_size(inner),
            BorshType::Struct { fields } => {
                fields.iter().map(|(_, ft)| Self::borsh_type_size(ft)).sum()
            }
            BorshType::String | BorshType::Bytes | BorshType::Vec(_) | BorshType::Enum { .. } => 0,
        }
    }

    pub(crate) fn borsh_read_field(
        &mut self,
        btype: &BorshType,
        src: u32,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let mut v: Vec<Instruction<'static>> = Vec::new();
        match btype {
            BorshType::I64 | BorshType::U64 => {
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.extend(self.emit_tag_num());
            }
            BorshType::U32 => {
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
            }
            BorshType::U8 => {
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_num());
            }
            BorshType::Bool => {
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag(TAG_BOOL));
            }
            BorshType::U128 => {
                // U128 is 16 bytes. Return array [lo, hi] (count=2 at arr[0])
                // Allocate runtime array: [count=2, lo, hi]
                let arr_ptr = self.local_idx("__u128_arr");
                v.extend(self.emit_runtime_alloc(24)); // 3 slots * 8 bytes
                v.push(Instruction::LocalSet(arr_ptr));
                // Store count=2 at arr[0]
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(2));
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Store lo at arr[1] (arr+8)
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Store hi at arr[2] (arr+16)
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.extend(self.emit_tag_num());
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // Return tagged array
                v.push(Instruction::LocalGet(arr_ptr));
                v.extend(self.emit_tag(TAG_ARRAY));
            }
            BorshType::F64 => {
                return Err("borsh-deserialize: F64 not yet supported".into());
            }
            BorshType::String | BorshType::Bytes => {
                let len = self.local_idx("__borsh_len");
                // Read 4-byte LE length
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(len));
                // Build tagged Str pointing at src+4 with len
                // ptr = src + 4
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                // ptr | (len << 32)
                v.push(Instruction::LocalGet(len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
            }
            BorshType::Option(inner) => {
                // Read 1-byte discriminant from src
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                v.push(Instruction::I32Const(0));
                v.push(Instruction::I32Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // discriminant == 0 → None → TAG_NIL
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // discriminant == 1 → Some → recursively read inner from src+1
                let inner_src = self.local_idx("__borsh_opt_src");
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(inner_src));
                v.extend(self.borsh_read_field(inner, inner_src)?);
                v.push(Instruction::End);
            }
            BorshType::Vec(inner) => {
                let elem_sz = Self::borsh_type_size(inner);
                if elem_sz == 0 {
                    return Err(
                        "borsh-deserialize: Vec of variable-length element types not yet supported"
                            .into(),
                    );
                }
                let count = self.local_idx("__borsh_vec_count");
                let arr_ptr = self.local_idx("__borsh_vec_arr");
                let elem_idx = self.local_idx("__borsh_vec_eidx");
                let elem_src = self.local_idx("__borsh_vec_esrc");
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };

                // Read u32 LE count from src
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(count));

                // Runtime alloc: (1 + count) * 8 bytes  [count slot + elements]
                // We emit the alloc inline since count is runtime
                // alloc size = 8 + count * 8, but count is a local so we compute at runtime
                {
                    let alloc_tmp = self.local_idx("__borsh_vec_alloc_sz");
                    // alloc_size = (1 + count) * 8
                    v.push(Instruction::LocalGet(count));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::I64Mul);
                    v.push(Instruction::LocalSet(alloc_tmp));
                    // Read runtime heap ptr
                    v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load(ma));
                    v.push(Instruction::LocalSet(arr_ptr));
                    // Overflow guard: new_ptr < mem_limit
                    let rha_new = self.local_idx("__borsh_vec_rha_new");
                    let mem_limit = (self.memory_pages as i64) * 65536;
                    v.push(Instruction::LocalGet(arr_ptr));
                    v.push(Instruction::LocalGet(alloc_tmp));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(rha_new));
                    v.push(Instruction::LocalGet(rha_new));
                    v.push(Instruction::I64Const(mem_limit));
                    v.push(Instruction::I64LtU);
                    v.push(Instruction::If(BlockType::Empty));
                    // OK: write back new ptr
                    v.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(rha_new));
                    v.push(Instruction::I64Store(ma));
                    v.push(Instruction::Else);
                    // Overflow: trap
                    v.push(Instruction::Unreachable);
                    v.push(Instruction::End);
                }

                // Store count at arr_ptr[0]
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(count));
                v.push(Instruction::I64Store(ma));

                // Element data starts at src + 4
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(elem_src));

                // Loop: for i in 0..count, deserialize elem from elem_src, store at arr_ptr[1+i]
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(elem_idx));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if elem_idx < count
                v.push(Instruction::LocalGet(elem_idx));
                v.push(Instruction::LocalGet(count));
                v.push(Instruction::I64LtU);
                // I64LtU returns i32 directly — no wrap needed
                v.push(Instruction::If(BlockType::Empty));
                // Deserialize element from elem_src → tagged value on stack
                v.extend(self.borsh_read_field(inner, elem_src)?);
                // Store tagged value at arr_ptr + (1 + elem_idx) * 8
                // I64Store expects [i32 addr, i64 val] — swap order: addr first, then val
                // Use a temp local to save the value, push addr, then push val
                let store_tmp = self.local_idx("__borsh_store_tmp");
                v.push(Instruction::LocalSet(store_tmp)); // save tagged value
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I64Const(8)); // skip count
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(elem_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64); // addr as i32
                v.push(Instruction::LocalGet(store_tmp)); // tagged value
                v.push(Instruction::I64Store(ma)); // [i32 addr, i64 val]
                                                   // Advance elem_src by elem_sz
                v.push(Instruction::LocalGet(elem_src));
                v.push(Instruction::I64Const(elem_sz as i64));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(elem_src));
                // elem_idx += 1
                v.push(Instruction::LocalGet(elem_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(elem_idx));
                // Br(1) targets the Loop, not the If — continue iterating
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // if
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block

                // Advance caller's src by 4 + count * elem_sz
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(4));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(count));
                v.push(Instruction::I64Const(elem_sz as i64));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(src));

                // Return tagged array: (arr_ptr << TAG_BITS) | TAG_ARRAY
                v.push(Instruction::LocalGet(arr_ptr));
                v.extend(self.emit_tag_array());
            }
            BorshType::Struct { fields } => {
                if fields.is_empty() {
                    return Err("borsh-deserialize: empty nested struct has no fields".into());
                }
                // Allocate runtime array: [count, field0, field1, ...]
                let arr_slots = fields.len() as i64;
                let arr_bytes = (1 + arr_slots) * 8;
                let arr_ptr = self.local_idx("__borsh_nested_arr");
                v.extend(self.emit_runtime_alloc(arr_bytes));
                v.push(Instruction::LocalSet(arr_ptr));
                // Store count
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(arr_slots));
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                v.push(Instruction::I64Store(ma));
                // Read each field and store into array
                let field_src = self.local_idx("__borch_nested_fsrc");
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::LocalSet(field_src));
                for (i, (_fname, ftype)) in fields.iter().enumerate() {
                    // Read field value → tagged i64 on stack
                    v.extend(self.borsh_read_field(ftype, field_src)?);
                    let val_tmp = self.local_idx("__borsh_nested_val");
                    v.push(Instruction::LocalSet(val_tmp));
                    // Store at arr_ptr[1+i]
                    let slot_off = (1 + i) as i64 * 8;
                    v.push(Instruction::LocalGet(arr_ptr));
                    v.push(Instruction::I64Const(slot_off));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(val_tmp));
                    v.push(Instruction::I64Store(ma));
                    // Advance field_src by field size
                    let sz = Self::borsh_type_size(ftype);
                    if sz > 0 {
                        v.push(Instruction::LocalGet(field_src));
                        v.push(Instruction::I64Const(sz as i64));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::LocalSet(field_src));
                    } else {
                        return Err(format!(
                            "borsh-deserialize: variable-length field '{}' in nested struct not yet supported",
                            _fname
                        ));
                    }
                }
                // Return tagged array
                v.push(Instruction::LocalGet(arr_ptr));
                v.extend(self.emit_tag(TAG_ARRAY));
            }
            BorshType::Enum { variants } => {
                // Read 1-byte discriminant
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                v.push(Instruction::I64ExtendI32U);
                let disc_local = self.local_idx("__borsh_enum_disc");
                v.push(Instruction::LocalSet(disc_local));
                // Advance src by 1
                v.push(Instruction::LocalGet(src));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(src));
                // Allocate runtime array for result: [variant_index, field_values...]
                // Use max fields across all variants for allocation size
                let max_fields = variants.iter().map(|(_, f)| f.len()).max().unwrap_or(0);
                let max_arr_slots = 1 + max_fields; // variant_idx + up to max_fields values
                let arr_bytes = (1 + max_arr_slots) as i64 * 8; // count slot + elements
                let arr_ptr = self.local_idx("__borsh_enum_arr");
                v.extend(self.emit_runtime_alloc(arr_bytes));
                v.push(Instruction::LocalSet(arr_ptr));
                // Store variant index at arr_ptr[1] (slot 0 = count)
                v.push(Instruction::LocalGet(arr_ptr));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(disc_local));
                v.extend(self.emit_tag_num());
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                v.push(Instruction::I64Store(ma));
                // Switch on discriminant to read variant fields
                for (vi, (_, vfields)) in variants.iter().enumerate() {
                    if vi == 0 {
                        v.push(Instruction::LocalGet(disc_local));
                        v.push(Instruction::I64Const(0i64));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::If(BlockType::Empty));
                    } else if vi < variants.len() - 1 {
                        v.push(Instruction::Else);
                        v.push(Instruction::LocalGet(disc_local));
                        v.push(Instruction::I64Const(vi as i64));
                        v.push(Instruction::I64Eq);
                        v.push(Instruction::If(BlockType::Empty));
                    } else {
                        v.push(Instruction::Else);
                    }
                    // Set count = 1 + num_fields for this variant
                    let count = 1 + vfields.len() as i64;
                    v.push(Instruction::LocalGet(arr_ptr));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Const(count));
                    v.push(Instruction::I64Store(ma));
                    // Read this variant's fields into array slots 2, 3, ...
                    for (fi, (_, ftype)) in vfields.iter().enumerate() {
                        v.extend(self.borsh_read_field(ftype, src)?);
                        let field_sz = Self::borsh_type_size(ftype);
                        let val_tmp = self.local_idx("__borsh_enum_val");
                        v.push(Instruction::LocalSet(val_tmp)); // save tagged value
                        let slot_off = (2 + fi) as i64 * 8;
                        v.push(Instruction::LocalGet(arr_ptr));
                        v.push(Instruction::I64Const(slot_off));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(val_tmp)); // load tagged value
                        v.push(Instruction::I64Store(ma));
                        if field_sz > 0 {
                            v.push(Instruction::LocalGet(src));
                            v.push(Instruction::I64Const(field_sz as i64));
                            v.push(Instruction::I64Add);
                            v.push(Instruction::LocalSet(src));
                        }
                    }
                }
                // Close nested if/else blocks
                for _ in 0..variants.len().saturating_sub(1) {
                    v.push(Instruction::End);
                }
                // Return tagged array
                v.push(Instruction::LocalGet(arr_ptr));
                v.extend(self.emit_tag(TAG_ARRAY));
            }
        }
        Ok(v)
    }
}

fn parse_borsh_type(val: &LispVal) -> Result<BorshType, String> {
    match val {
        LispVal::Sym(s) => match s.as_str() {
            "u8" => Ok(BorshType::U8),
            "u32" => Ok(BorshType::U32),
            "u64" => Ok(BorshType::U64),
            "i64" => Ok(BorshType::I64),
            "u128" => Ok(BorshType::U128),
            "f64" => Ok(BorshType::F64),
            "bool" => Ok(BorshType::Bool),
            "string" => Ok(BorshType::String),
            "bytes" => Ok(BorshType::Bytes),
            other => Err(format!("borsh: unknown type '{}'", other)),
        },
        LispVal::List(items) if !items.is_empty() => {
            match &items[0] {
                LispVal::Sym(s) if s == "Vec" => {
                    if items.len() != 2 {
                        return Err("borsh: Vec requires exactly one type arg".into());
                    }
                    let inner = parse_borsh_type(&items[1])?;
                    Ok(BorshType::Vec(Box::new(inner)))
                }
                LispVal::Sym(s) if s == "Option" => {
                    if items.len() != 2 {
                        return Err("borsh: Option requires exactly one type arg".into());
                    }
                    let inner = parse_borsh_type(&items[1])?;
                    Ok(BorshType::Option(Box::new(inner)))
                }
                LispVal::Sym(s) if s == "Enum" => {
                    // (Enum (VariantName (field1 type1) ...) ...)
                    let mut variants = Vec::new();
                    for v in &items[1..] {
                        match v {
                            LispVal::List(var_items) if !var_items.is_empty() => {
                                let var_name = match &var_items[0] {
                                    LispVal::Sym(n) => n.clone(),
                                    _ => {
                                        return Err("borsh Enum: variant name must be symbol".into())
                                    }
                                };
                                let fields = parse_borsh_fields(&var_items[1..])?;
                                variants.push((var_name, fields));
                            }
                            LispVal::Sym(n) => {
                                // Unit variant — no fields
                                variants.push((n.clone(), Vec::new()));
                            }
                            _ => return Err("borsh Enum: variant must be list or symbol".into()),
                        }
                    }
                    Ok(BorshType::Enum { variants })
                }
                _ => {
                    // Treat as struct: ((field1 type1) (field2 type2) ...)
                    let fields = parse_borsh_fields(items)?;
                    Ok(BorshType::Struct { fields })
                }
            }
        }
        _ => Err("borsh: type must be symbol or list".into()),
    }
}

fn parse_borsh_enum_variants(
    items: &[LispVal],
) -> Result<Vec<(String, Vec<(String, BorshType)>)>, String> {
    let mut variants = Vec::new();
    for v in items {
        match v {
            LispVal::List(var_items) if !var_items.is_empty() => {
                let var_name = match &var_items[0] {
                    LispVal::Sym(n) => n.clone(),
                    _ => return Err("borsh Enum: variant name must be symbol".into()),
                };
                let fields = parse_borsh_fields(&var_items[1..])?;
                variants.push((var_name, fields));
            }
            LispVal::Sym(n) => {
                // Unit variant — no fields
                variants.push((n.clone(), Vec::new()));
            }
            _ => return Err("borsh Enum: variant must be list or symbol".into()),
        }
    }
    Ok(variants)
}

fn parse_borsh_fields(items: &[LispVal]) -> Result<Vec<(String, BorshType)>, String> {
    let mut fields = Vec::new();
    for item in items {
        match item {
            LispVal::List(pair) if pair.len() == 2 => {
                let name = match &pair[0] {
                    LispVal::Sym(n) => n.clone(),
                    _ => return Err("borsh: field name must be symbol".into()),
                };
                let btype = parse_borsh_type(&pair[1])?;
                fields.push((name, btype));
            }
            _ => return Err("borsh: field must be (name type) pair".into()),
        }
    }
    Ok(fields)
}

pub(crate) fn process_borsh_schema(em: &mut WasmEmitter, items: &[LispVal]) -> Result<(), String> {
    // items[0] = "borsh-schema", items[1..] = type definitions
    for def in &items[1..] {
        match def {
            LispVal::List(type_def) if type_def.len() >= 2 => {
                let name = match &type_def[0] {
                    LispVal::Sym(n) => n.clone(),
                    LispVal::Str(n) => n.clone(),
                    _ => return Err("borsh-schema: type name must be symbol or string".into()),
                };
                // If type_def[1..] are all bare symbols (no sub-lists), treat as Enum unit variants
                // e.g. (Color Red Green Blue) → (Enum Red Green Blue)
                let rest = &type_def[1..];
                let all_syms = rest.iter().all(|v| matches!(v, LispVal::Sym(_)));
                let any_list = rest.iter().any(|v| matches!(v, LispVal::List(_)));
                let btype = if all_syms && !any_list && rest.len() > 1 {
                    // All bare symbols → unit enum variants
                    let variants: Vec<(String, Vec<(String, BorshType)>)> = rest
                        .iter()
                        .map(|v| {
                            if let LispVal::Sym(n) = v {
                                (n.clone(), Vec::new())
                            } else {
                                unreachable!()
                            }
                        })
                        .collect();
                    BorshType::Enum { variants }
                } else if any_list && !all_syms {
                    // List items present: determine struct vs enum
                    // Enum variant: (VariantName (field1 type1) ...) — sub-items are also (name type) pairs, OR variant has inner lists
                    // Struct field: (name type) — exactly 2 elements, second is NOT a list
                    let is_struct = rest.iter().all(|v| {
                        if let LispVal::List(l) = v {
                            l.len() == 2 && match &l[1] {
                                LispVal::Sym(_) => true, // simple type like i64
                                LispVal::List(inner) if !inner.is_empty() => {
                                    // Compound type: (Option i64), (Vec i64), (Enum ...), (Struct ...)
                                    // These are type constructors, not field pairs
                                    matches!(&inner[0], LispVal::Sym(s) if matches!(s.as_str(), "Option" | "Vec" | "Enum" | "Struct"))
                                }
                                _ => false,
                            }
                        } else { false }
                    });
                    if is_struct {
                        // All items are (name type) pairs → struct fields
                        parse_borsh_type(&LispVal::List(rest.to_vec()))?
                    } else if let Some(LispVal::List(l)) = rest.first() {
                        if !l.is_empty() && matches!(&l[0], LispVal::Sym(s) if s == "Enum") {
                            // Explicit (Enum ...) form
                            parse_borsh_type(&LispVal::List(rest.to_vec()))?
                        } else {
                            // Implicit enum: (VariantName (field1 type1) ...) items
                            let variants = parse_borsh_enum_variants(rest)?;
                            BorshType::Enum { variants }
                        }
                    } else {
                        parse_borsh_type(&LispVal::List(rest.to_vec()))?
                    }
                } else {
                    // Single item or struct: ((field1 type1) (field2 type2) ...)
                    parse_borsh_type(&LispVal::List(rest.to_vec()))?
                };
                em.borsh_schemas.insert(name, btype);
            }
            _ => return Err("borsh-schema: each type def must be (Name fields...)".into()),
        }
    }
    Ok(())
}
