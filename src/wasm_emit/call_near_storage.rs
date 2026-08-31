use super::*;
use wasm_encoder::{MemArg, Instruction, BlockType, ValType};

impl WasmEmitter {
    /// Ops that mutate contract storage through the NEAR host (storage_write
    /// / storage_remove). Every one of them must invalidate the storage-read
    /// memo cache — the central hook in `call()` appends the flush.
    pub(crate) fn is_storage_write_op(op: &str) -> bool {
        matches!(
            op,
            "near/storage_set"
                | "near/storage_write"
                | "near/storage_remove"
                | "near/store"
                | "near/remove"
                | "near/store_num"
                | "near/kv"
                | "near/kstore"
                | "near/store-deposit"
                | "near/store-bytes"
                | "u128/store_storage"
                | "near/store_u128"
        )
    }

    /// Cache invalidation: slot i is valid iff i < count, so a single store
    /// of 0 to the count word empties the whole table. Appended after every
    /// storage-write op's emitted code (value on the stack is undisturbed).
    pub(crate) fn emit_storage_cache_flush(v: &mut Vec<Instruction<'static>>) {
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Store(MemArg { offset: 0, align: 3, memory_index: 0 }));
    }

    /// Emit the cache lookup for the key in local `k` (tagged Str).
    /// On hit: `res_l` = cached tagged value, `hit_l` = 1.
    /// On miss: `hit_l` = 0 (caller runs the host read + insert).
    /// Key compare is EXACT: length filter, 8-byte chunks over len>>3, then
    /// byte-wise tail — never reads past either key, so adjacent-memory
    /// garbage can only cause a false MISS (safe), never a false hit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn emit_storage_cache_lookup(
        &mut self,
        k: u32,
        res_l: u32,
        hit_l: u32,
        cnt_l: u32,
        idx_l: u32,
        slot_l: u32,
        j_l: u32,
        eq_l: u32,
        klen_l: u32,
        kptr_l: u32,
        tail_l: u32,
    ) -> Vec<Instruction<'static>> {
        let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
        let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
        let mut v = Vec::new();
        // klen = raw(k) >> 32 ; kptr = (u32)raw(k)
        v.push(Instruction::LocalGet(k));
        v.extend(self.emit_untag());
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(klen_l));
        v.push(Instruction::LocalGet(k));
        v.extend(self.emit_untag());
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(kptr_l));
        // hit = 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(hit_l));
        // cnt = mem64[CACHE_COUNT_ADDR]
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::LocalSet(cnt_l));
        // idx = 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(idx_l));
        // ── outer scan: Block A { Loop B { ... } } ──
        v.push(Instruction::Block(BlockType::Empty)); // A
        v.push(Instruction::Loop(BlockType::Empty)); // B
        // if idx >= cnt → br A (exit scan)
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        // slot = CACHE_SLOT_BASE + idx * CACHE_STRIDE
        v.push(Instruction::I64Const(CACHE_SLOT_BASE));
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::I64Const(CACHE_STRIDE));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(slot_l));
        // if mem64[slot+8] == klen:
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg { offset: 8, align: 3, memory_index: 0 }));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); // C
        //   eq = 1; j = 0
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(eq_l));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(j_l));
        //   Block D { Loop E { chunk compare } }
        v.push(Instruction::Block(BlockType::Empty)); // D
        v.push(Instruction::Loop(BlockType::Empty)); // E
        //   if j >= klen>>3 → br D
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        //   if mem64[slot+24+j*8] != mem64[kptr+j*8] → eq = 0
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg { offset: 24, align: 3, memory_index: 0 }));
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(eq_l));
        v.push(Instruction::End);
        //   if eq == 0 → br D (bail out of compare)
        v.push(Instruction::LocalGet(eq_l));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::BrIf(1));
        //   j++; br E
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(j_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // E
        v.push(Instruction::End); // D
        //   tail = klen & -8 (low 3 bits cleared; NOT (klen>>3)<<3 — the
        //   peephole optimizer deletes const-3 shr/shl pairs as tag round-trips)
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Const(-8));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(tail_l));
        //   Block G { Loop H { tail byte compare } }
        v.push(Instruction::Block(BlockType::Empty)); // G
        v.push(Instruction::Loop(BlockType::Empty)); // H
        //   if tail >= klen → br G
        v.push(Instruction::LocalGet(tail_l));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        //   if load8(slot+24+tail) != load8(kptr+tail) → eq = 0
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::LocalGet(tail_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load8U(MemArg { offset: 24, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::LocalGet(tail_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load8U(ma0.clone()));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(eq_l));
        v.push(Instruction::End);
        //   if eq == 0 → br G
        v.push(Instruction::LocalGet(eq_l));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::BrIf(1));
        //   tail++; br H
        v.push(Instruction::LocalGet(tail_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(tail_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // H
        v.push(Instruction::End); // G
        //   if eq → res = mem64[slot+16]; hit = 1; br A (from J: 0=J,1=C,2=B,3=A)
        v.push(Instruction::LocalGet(eq_l));
        v.push(Instruction::I32WrapI64); // if consumes an i32 condition
        v.push(Instruction::If(BlockType::Empty)); // J
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg { offset: 16, align: 3, memory_index: 0 }));
        v.push(Instruction::LocalSet(res_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(hit_l));
        v.push(Instruction::Br(3));
        v.push(Instruction::End); // J
        v.push(Instruction::End); // C
        // idx++; br B
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(idx_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // B
        v.push(Instruction::End); // A
        v
    }

    /// Emit the cache insert for the key in local `k` (tagged Str) with the
    /// freshly-read tagged result in `res_l`. Count is in `cnt_l` (loaded by
    /// the lookup — still current: single-threaded, nothing appended since).
    /// Skips (falls through uncached) when the table is full or the key is
    /// longer than CACHE_KEY_CAP. Key bytes are COPIED into the slot (with
    /// the tail zeroed), so later mutation of the source buffer (TEMP_MEM
    /// reuse, heap aliasing) can never poison a cached entry.
    pub(crate) fn emit_storage_cache_insert(
        &mut self,
        k: u32,
        res_l: u32,
        cnt_l: u32,
        slot_l: u32,
        j_l: u32,
        klen_l: u32,
        kptr_l: u32,
    ) -> Vec<Instruction<'static>> {
        let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
        let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
        let mut v = Vec::new();
        // guard: klen <= CAP && cnt < SLOTS
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Const(CACHE_KEY_CAP));
        v.push(Instruction::I64LeU);
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(CACHE_SLOTS));
        v.push(Instruction::I64LtU);
        v.push(Instruction::I32And); // i64 comparisons return i32 — combine as i32
        v.push(Instruction::If(BlockType::Empty)); // L
        // slot = SLOT_BASE + cnt*88
        v.push(Instruction::I64Const(CACHE_SLOT_BASE));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(CACHE_STRIDE));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(slot_l));
        // mem64[slot+0] = kptr ; [slot+8] = klen ; [slot+16] = res
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::I64Store(ma8.clone()));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Store(MemArg { offset: 8, align: 3, memory_index: 0 }));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(res_l));
        v.push(Instruction::I64Store(MemArg { offset: 16, align: 3, memory_index: 0 }));
        // zero the 64-byte key-copy tail (8 × i64 store at +24..+88)
        for off in (24..88).step_by(8) {
            v.push(Instruction::LocalGet(slot_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Const(0));
            v.push(Instruction::I64Store(MemArg { offset: off, align: 3, memory_index: 0 }));
        }
        // copy key bytes: j = 0; Block M { Loop N { if j>=klen br; slot[24+j] = key[j]; j++ } }
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(j_l));
        v.push(Instruction::Block(BlockType::Empty)); // M
        v.push(Instruction::Loop(BlockType::Empty)); // N
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load8U(ma0.clone()));
        v.push(Instruction::I64Store8(MemArg { offset: 24, align: 0, memory_index: 0 }));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(j_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // N
        v.push(Instruction::End); // M
        // count++
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Store(ma8.clone()));
        v.push(Instruction::End); // L
        v
    }

    pub(crate) fn call_near_storage(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
match op {
            // ═══════════════════════════════════════════════════════════════
            //  STRING-SAFE STORAGE FAMILY — near/storage_*
            //  Bytes-in-bytes-out over the raw NEAR host fns: strings are
            //  stored as their UTF-8 bytes (len from the register), so values
            //  survive fresh-memory transactions. The tagged-word API
            //  (near/store / near/load) keeps its 8-byte format for
            //  Num/Bool/Nil — do NOT mix families on the same key.
            //  Returns mirror the interpreter exactly: set→Num(0),
            //  get→Str ("" on miss), has→Num(1|0), remove→Num(0).
            // ═══════════════════════════════════════════════════════════════
            "near/storage_set" | "near/storage_write" => {
                if a.len() != 2 {
                    return Err("near/storage_set: need exactly 2 args (key, value)".into());
                }
                self.need_host(17);
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let k = self.local_idx("__sst_k");
                let val_l = self.local_idx("__sst_v");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(k));
                v.extend(val);
                v.push(Instruction::LocalSet(val_l));
                // Runtime tag guard: both operands must be Str. A Num value
                // would untag to garbage ptr/len and silently write binary
                // junk — the erc20 hazard class. Trap instead (interp
                // hard-errors with the same rule).
                Self::emit_assert_tag_str(&mut v, k);
                Self::emit_assert_tag_str(&mut v, val_l);
                // storage_write(key_len, key_ptr, val_len, val_ptr, register=0)
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalGet(val_l));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(val_l));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop); // evicted-length return is not the Lisp result
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/storage_get" | "near/storage_read" => {
                if a.len() != 1 {
                    return Err("near/storage_get: need exactly 1 arg (key)".into());
                }
                self.need_host(18);
                self.need_host(1);
                self.need_host(0);
                let key = self.expr(&a[0])?;
                let k = self.local_idx("__sg_k");
                let len_l = self.local_idx("__sg_len");
                let dst_l = self.local_idx("__sg_dst");
                let tmp_l = self.local_idx("__sg_tmp");
                let new_l = self.local_idx("__sg_new");
                // storage-read memo cache locals
                let res_l = self.local_idx("__sg_res");
                let hit_l = self.local_idx("__sg_hit");
                let cnt_l = self.local_idx("__sg_cnt");
                let idx_l = self.local_idx("__sg_idx");
                let slot_l = self.local_idx("__sg_slot");
                let j_l = self.local_idx("__sg_j");
                let eq_l = self.local_idx("__sg_eq");
                let klen_l = self.local_idx("__sg_klen");
                let kptr_l = self.local_idx("__sg_kptr");
                let tail_l = self.local_idx("__sg_tail");
                let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
                let mem_limit = (self.memory_pages as i64) * 65536;
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(k));
                Self::emit_assert_tag_str(&mut v, k);
                // ── memo cache lookup (per-tx storage-read cache) ──
                v.extend(self.emit_storage_cache_lookup(
                    k, res_l, hit_l, cnt_l, idx_l, slot_l, j_l, eq_l, klen_l, kptr_l, tail_l,
                ));
                // ── miss → host read path, then insert into the cache ──
                v.push(Instruction::LocalGet(hit_l));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // storage_read(key_len, key_ptr, register=0) → success flag
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18));
                // if flag == 0 → miss → Nil (typed (opt str): `??`/default handles
                // the miss; returning Str("") made fallbacks unreachable — FT bug)
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // len = register_len(0)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1));
                v.push(Instruction::LocalSet(len_l));
                // bump-allocate len bytes (8-aligned) from RUNTIME_HEAP_PTR (addr 56)
                v.push(Instruction::I64Const(56));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8.clone()));
                v.push(Instruction::LocalSet(tmp_l));
                v.push(Instruction::LocalGet(tmp_l));
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(-8));
                v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(new_l));
                v.push(Instruction::LocalGet(new_l));
                v.push(Instruction::I64Const(mem_limit));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(56));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(new_l));
                v.push(Instruction::I64Store(ma8.clone()));
                v.push(Instruction::Else);
                v.push(Instruction::Unreachable); // out of memory — hard error
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(tmp_l));
                v.push(Instruction::LocalSet(dst_l));
                // read_register(0, dst) — copies the value bytes (ptr is u64 in the host ABI)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(dst_l));
                v.push(Self::host_call(0));
                // packed = dst | len<<32, then tag as Str
                v.push(Instruction::LocalGet(len_l));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(dst_l));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                v.push(Instruction::End);
                // save result, then insert into the cache (skips when full /
                // key > 64 bytes — uncached fallback)
                v.push(Instruction::LocalSet(res_l));
                v.extend(self.emit_storage_cache_insert(k, res_l, cnt_l, slot_l, j_l, klen_l, kptr_l));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(res_l));
                Ok(v)
            }
            "near/storage_has" | "near/storage_has_key" => {
                if a.len() != 1 {
                    return Err("near/storage_has: need exactly 1 arg (key)".into());
                }
                let key = self.expr(&a[0])?;
                let k = self.local_idx("__ssh_k");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(k));
                Self::emit_assert_tag_str(&mut v, k);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/storage_remove" => {
                if a.len() != 1 {
                    return Err("near/storage_remove: need exactly 1 arg (key)".into());
                }
                let key = self.expr(&a[0])?;
                let k = self.local_idx("__ssr_k");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(k));
                Self::emit_assert_tag_str(&mut v, k);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(k));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            "near/store" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let __s = self.local_idx("__s");
                let __v = self.local_idx("__sv");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.extend(val);
                v.push(Instruction::LocalSet(__v));
                let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(__v));
                v.push(Instruction::I64Store(ma));
                // storage_write(key_len, key_ptr, val_len=8, val_ptr=STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load" => {
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__load_key");
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/remove" => {
                let key = self.expr(&a[0])?;
                let __s = self.local_idx("__s");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(19));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/store_num" => {
                let key = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let __s = self.local_idx("__s");
                let __n = self.local_idx("__n");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.extend(val);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(__n));
                let ma = MemArg { offset: 0, align: 3, memory_index: 0 };
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(__n));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load_num" => {
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__ln_key");
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/has_key" => {
                let key = self.expr(&a[0])?;
                let __s = self.local_idx("__s");
                let mut v = Vec::new();
                v.extend(key);
                v.push(Instruction::LocalSet(__s));
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(__s));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(20));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.extend(self.emit_tag_num());
                Ok(v)
            }

            // ── near/kv: composite key-value store ──
            // (near/kv "prefix" part1 part2 ... val)
            // Last arg = value (i64), all preceding = key parts (strings).
            // Builds composite key in KEY_BUF via byte-copy loop, then storage_write.
            "near/kv" => {
                self.need_host(17);
                let n_parts = a.len().saturating_sub(1);
                if n_parts < 1 {
                    return Err("near/kv needs >= 2 args: key parts + value".into());
                }
                let val_expr = self.expr(&a[a.len()-1])?;
                let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                let off = self.local_idx("__kv_off");
                let part = self.local_idx("__kv_part");
                let raw = self.local_idx("__kv_raw");
                let plen = self.local_idx("__kv_plen");
                let pptr = self.local_idx("__kv_pptr");
                let ci = self.local_idx("__kv_ci");
                // Evaluate value first, save to local
                v.extend(val_expr);
                let val_local = self.local_idx("__kv_val");
                v.push(Instruction::LocalSet(val_local));
                // offset = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(off));
                // Pre-evaluate all part expressions
                let mut part_exprs = Vec::new();
                for i in 0..n_parts {
                    part_exprs.push(self.expr(&a[i])?);
                }
                // For each part: untag, extract ptr/len, byte-copy to KEY_BUF, advance offset
                for part_e in part_exprs {
                    v.extend(part_e);
                    v.push(Instruction::LocalSet(part));
                    // Untag → raw = (len << 32) | ptr
                    v.push(Instruction::LocalGet(part));
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw));
                    // plen = raw >> 32
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(plen));
                    // pptr = raw & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(pptr));
                    // ci = 0
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(ci));
                    // byte-copy loop: while ci < plen { KEY_BUF[off+ci] = mem[pptr+ci]; ci++ }
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    // if ci >= plen: break
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::BrIf(1));
                    // dst = KEY_BUF + off + ci (i32)
                    v.push(Instruction::I64Const(KEY_BUF));
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // src = pptr + ci (i32)
                    v.push(Instruction::LocalGet(pptr));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // load 1 byte from src (zero-extend to i64), store 8 bytes to dst
                    v.push(Instruction::I64Load8U(ma0.clone()));
                    v.push(Instruction::I64Store(ma0.clone()));
                    // ci++
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(ci));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End); // loop
                    v.push(Instruction::End); // block
                    // off += plen
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off));
                }
                // Write tagged value to STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::LocalGet(val_local));
                v.push(Instruction::I64Store(ma8));
                // storage_write(key_len=off, key_ptr=KEY_BUF, val_len=8, val_ptr=STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(off));
                v.push(Instruction::I64Const(KEY_BUF));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }

            // ── near/kv-get: read from composite key ──
            // (near/kv-get "prefix" part1 part2 ...)
            // All args are key parts (strings). Returns tagged value or 0.
            "near/kv-get" => {
                self.need_host(18);
                self.need_host(0);
                let n_parts = a.len();
                if n_parts < 1 {
                    return Err("near/kv-get needs >= 1 key part".into());
                }
                let ma8 = MemArg { offset: 0, align: 3, memory_index: 0 };
                let ma0 = MemArg { offset: 0, align: 0, memory_index: 0 };
                let mut v = Vec::new();
                let off = self.local_idx("__kvg_off");
                let part = self.local_idx("__kvg_part");
                let raw = self.local_idx("__kvg_raw");
                let plen = self.local_idx("__kvg_plen");
                let pptr = self.local_idx("__kvg_pptr");
                let ci = self.local_idx("__kvg_ci");
                // offset = 0
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(off));
                // Pre-evaluate all part expressions
                let mut part_exprs = Vec::new();
                for i in 0..n_parts {
                    part_exprs.push(self.expr(&a[i])?);
                }
                // Build composite key in KEY_BUF
                for part_e in part_exprs {
                    v.extend(part_e);
                    v.push(Instruction::LocalSet(part));
                    // Untag → raw = (len << 32) | ptr
                    v.push(Instruction::LocalGet(part));
                    v.extend(self.emit_untag());
                    v.push(Instruction::LocalSet(raw));
                    // plen = raw >> 32
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(plen));
                    // pptr = raw & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(raw));
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64ExtendI32U);
                    v.push(Instruction::LocalSet(pptr));
                    // ci = 0
                    v.push(Instruction::I64Const(0));
                    v.push(Instruction::LocalSet(ci));
                    // byte-copy loop
                    v.push(Instruction::Block(BlockType::Empty));
                    v.push(Instruction::Loop(BlockType::Empty));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64GeU);
                    v.push(Instruction::BrIf(1));
                    v.push(Instruction::I64Const(KEY_BUF));
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::LocalGet(pptr));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    v.push(Instruction::I64Load8U(ma0.clone()));
                    v.push(Instruction::I64Store(ma0.clone()));
                    v.push(Instruction::LocalGet(ci));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(ci));
                    v.push(Instruction::Br(0));
                    v.push(Instruction::End);
                    v.push(Instruction::End);
                    v.push(Instruction::LocalGet(off));
                    v.push(Instruction::LocalGet(plen));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(off));
                }
                // storage_read(key_len=off, key_ptr=KEY_BUF, register=1)
                v.push(Instruction::LocalGet(off));
                v.push(Instruction::I64Const(KEY_BUF));
                v.push(Instruction::I64Const(1));
                v.push(Self::host_call(18));
                // 0 = not found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // register_read(1, STORAGE_BUF) — void
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0));
                // Load tagged value from STORAGE_BUF
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::End);
                Ok(v)
            }
            "near/storage_usage" => { let mut v = vec![Self::host_call(11)]; v.extend(self.emit_tag_num()); Ok(v) },
            // near/kstore: (near/kstore prefix account val) — FP_GLOBAL-safe storage via KEY_BUF
            // Concatenates prefix + account as key, stores value (tagged i64).
            "near/kstore" => {
                if a.len() != 3 { return Err("near/kstore requires 3 args: prefix account value".into()); }
                self.need_host(17); self.need_host(0); self.need_host(1);
                let prefix_expr = self.expr(&a[0])?;
                let acct_expr = self.expr(&a[1])?;
                let val_expr = self.expr(&a[2])?;
                let prefix_local = self.local_idx("__kstore_prefix");
                let acct_local = self.local_idx("__kstore_acct");
                let key_len_local = self.local_idx_i32("__kstore_keylen");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate prefix, account, value and save to locals
                v.extend(prefix_expr);
                v.push(Instruction::LocalSet(prefix_local));
                v.extend(acct_expr);
                v.push(Instruction::LocalSet(acct_local));
                // Store tagged val at STORAGE_BUF
                // I64Store: [addr (i32), value (i64)] - push addr FIRST, then value
                v.push(Instruction::I32Const(STORAGE_BUF as i32)); // addr (i32) - pushed FIRST
                v.extend(val_expr); // value (i64) - pushed SECOND
                v.push(Instruction::I64Store(ma));
                
                // Copy prefix to KEY_BUF
                // MemoryCopy: dst (i32), src (i32), len (i32)
                v.push(Instruction::I32Const(KEY_BUF as i32)); // dst
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = prefix ptr
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // prefix_len (i64)
                v.push(Instruction::I32WrapI64); // len = prefix_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // key_len = prefix_len (save for later)
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(key_len_local));
                
                // Copy account to KEY_BUF + prefix_len
                v.push(Instruction::I32Const(KEY_BUF as i32));
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::I32Add); // dst = KEY_BUF + prefix_len
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = account ptr
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64); // len = account_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // key_len = prefix_len + account_len
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalSet(key_len_local));
                
                // storage_write(key_len, KEY_BUF, val_len=8, STORAGE_BUF, register=0)
                v.push(Instruction::LocalGet(key_len_local));
                v.extend(self.emit_i32_to_i64());
                v.push(Instruction::I64Const(KEY_BUF as i64));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            // near/kload: (near/kload prefix account) — FP_GLOBAL-safe storage load
            // Returns tagged value or 0 (tagged as Num) if not found.
            "near/kload" => {
                if a.len() != 2 { return Err("near/kload requires 2 args: prefix account".into()); }
                let prefix_expr = self.expr(&a[0])?;
                let acct_expr = self.expr(&a[1])?;
                let prefix_local = self.local_idx("__kload_prefix");
                let acct_local = self.local_idx("__kload_acct");
                let key_len_local = self.local_idx_i32("__kload_keylen");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                // Evaluate prefix and account, save to locals
                v.extend(prefix_expr);
                v.push(Instruction::LocalSet(prefix_local));
                v.extend(acct_expr);
                v.push(Instruction::LocalSet(acct_local));
                
                // Copy prefix to KEY_BUF
                // MemoryCopy: dst (i32), src (i32), len (i32)
                v.push(Instruction::I32Const(KEY_BUF as i32)); // dst
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = prefix ptr
                v.push(Instruction::LocalGet(prefix_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // prefix_len (i64)
                v.push(Instruction::I32WrapI64); // len = prefix_len (i32)
                v.push(Instruction::LocalTee(key_len_local)); // save for later AND keep on stack
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // Copy account to KEY_BUF + prefix_len
                v.push(Instruction::I32Const(KEY_BUF as i32));
                v.push(Instruction::LocalGet(key_len_local));
                v.push(Instruction::I32Add); // dst = KEY_BUF + prefix_len
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); // src = account ptr
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64); // len = account_len (i32)
                v.push(Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });
                
                // Compute total key_len = prefix_len + account_len
                v.push(Instruction::LocalGet(key_len_local)); // prefix_len (i32)
                v.push(Instruction::LocalGet(acct_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // account_len (i64)
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalSet(key_len_local)); // key_len = prefix_len + account_len
                
                // storage_read(key_len, KEY_BUF, register=1) → returns 0 if not found, 1 if found
                v.push(Instruction::LocalGet(key_len_local));
                v.extend(self.emit_i32_to_i64()); // host expects i64
                v.push(Instruction::I64Const(KEY_BUF as i64));
                v.push(Instruction::I64Const(1)); // register 1
                v.push(Self::host_call(18)); // storage_read
                
                // Check return value: 0 = not found, 1 = found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return tagged 0
                v.push(Instruction::I64Const(0));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Else);
                // Found: read_register(1, STORAGE_BUF)
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Const(STORAGE_BUF));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I32Const(STORAGE_BUF as i32));
                v.push(Instruction::I64Load(ma));
                v.extend(self.emit_tag_validate());
                v.push(Instruction::End);
                Ok(v)
            }
            "near/store-deposit" => {
                // (near/store-deposit key) — stores attached_deposit (u128, 16 bytes)
                // directly from TEMP_MEM under the given storage key.
                // attached_deposit writes 16 bytes to TEMP_MEM (host idx 14).
                self.need_host(14); // attached_deposit
                self.need_host(17); // storage_write
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__sd_key");
                let mut v = Vec::new();
                // Call attached_deposit(TEMP_MEM) → writes 16 bytes at TEMP_MEM (addr 64).
                v.push(Instruction::I64Const(TEMP_MEM as i64));
                v.push(Self::host_call(14));
                // Save key for reuse.
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // storage_write(key_len, key_ptr, val_len=16, val_ptr=TEMP_MEM, register_id=0)
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // key_len
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(16)); // val_len
                v.push(Instruction::I64Const(TEMP_MEM)); // val_ptr
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(17));
                v.push(Instruction::Drop);
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "near/load-amount" => {
                // (near/load-amount key) — reads 16-byte u128 from storage into TEMP_MEM
                // and returns a tagged string (len=16 | ptr=TEMP_MEM) so it can be passed
                // to near/batch-transfer. Returns nil if the key is not found.
                self.need_host(18); // storage_read
                self.need_host(0);  // read_register
                let key_expr = self.expr(&a[0])?;
                let key_local = self.local_idx("__la_key");
                let mut v = Vec::new();
                v.extend(key_expr);
                v.push(Instruction::LocalSet(key_local));
                // storage_read(key_len, key_ptr, register_id=1)
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // key_len
                v.push(Instruction::LocalGet(key_local));
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // key_ptr
                v.push(Instruction::I64Const(1)); // register 1
                v.push(Self::host_call(18));
                // Check return: 0 = not found, 1 = found
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Not found: return nil
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::Else);
                // Found: read_register(1, TEMP_MEM) → writes 16 bytes at TEMP_MEM
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // Return tagged string: (16 << 32) | TEMP_MEM
                v.push(Instruction::I64Const(16));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
