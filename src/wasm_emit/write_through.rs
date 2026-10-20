//! P3 write-through storage coalescing (2026-10-13).
//!
//! Measured motivation (seeded pool-b swap, fuel-exact mock): 1.700T total,
//! 1.434T host calls — 865G in 8 storage_write + 483G in 7 storage_read,
//! while the swap touches only FIVE distinct cells. Two wins, one structure:
//!
//! 1. Reads: a per-tx memo cache already exists (2B era) but every write
//!    nuked it (count=0), forcing the next read of the same key back to the
//!    host. Under write-through, writes no longer flush the table.
//! 2. Writes: storage_set upserts the (key, tagged-value) pair into the same
//!    table and marks the slot DIRTY instead of calling storage_write. One
//!    synthesized helper (`__h_wt_flush`) walks the table at the export
//!    boundary and replays exactly the dirty slots — last-write-wins, so
//!    N writes to one key collapse to 1 host call.
//!
//! Correctness contract:
//! - Read-your-own-writes: reads hit the same table the writes upsert.
//! - Program order with host-visible ops: `storage_remove` and every
//!   non-string-family write op (near/store, near/kv, u128/store_storage, …)
//!   replay pending dirty slots FIRST (`Call __h_wt_flush`), then do their
//!   own host call, then reset the table (count=0). The flush replay IS the
//!   program order — pending writes happened earlier in the program.
//! - removal semantics: remove = flush + host remove + count=0. A later
//!   re-put upserts a fresh slot; the boundary replays it. No tombstones
//!   needed because nothing shadows host state in between (reads go through
//!   the same table, and it is empty after count=0).
//! - Legacy-decimal values are read by the HOST at flush time exactly as
//!   they would be by any storage_write — the codec layer is untouched.
//! - Fallback: keys > CACHE_KEY_CAP or a full table (64 slots) skip the
//!   upsert and fall through to the immediate host write (no dirty bit, no
//!   buffering) — byte-identical to pre-P3 behavior for those sites.
//! - Documented deviation (mirrors the tagged-family split): value reads
//!   that bypass the memo table (near/json_get_* host scans) observe hosted
//!   state only; a pending dirty write is invisible to them until the
//!   boundary. The pool contracts never mix these paths on one key.
//!
//! Memory layout: dirty bitmap at [WT_DIRTY_BASE, WT_DIRTY_END) — 64 bytes,
//! one per slot, allocated immediately after the slot table (see mod.rs).

use super::*;
use wasm_encoder::{BlockType, Instruction, MemArg, ValType};

impl WasmEmitter {
    pub(crate) fn wt_active(&self) -> bool {
        self.wt_write_through && !self.fuzz_mode && !self.wasi_mode
    }

    /// Reserve the memo+bitmap region [CACHE_COUNT_ADDR, WT_DIRTY_END) BEFORE
    /// any function body compiles. Lazy reservation (at first upsert/lookup)
    /// races heap_bump: data literals allocated between the read-cache
    /// reserve (CACHE_END) and the wt reserve would land inside the bitmap.
    /// Called once at the hot-gate site; later reserves no-op.
    pub(crate) fn reserve_wt_region(&mut self) {
        if self.next_data_offset < WT_DIRTY_END {
            if (self.next_data_offset as i64) > CACHE_COUNT_ADDR {
                panic!(
                    "storage cache region overflow: data at {} already inside [{}, {})",
                    self.next_data_offset, CACHE_COUNT_ADDR, WT_DIRTY_END
                );
            }
            self.next_data_offset = WT_DIRTY_END;
        }
    }

    fn ma8() -> MemArg {
        MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        }
    }

    fn ma0() -> MemArg {
        MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }
    }

    /// Emit `Call(__h_wt_flush)` — replay all pending dirty writes now.
    /// Used before storage_remove and before non-string-family writes so the
    /// boundary never reorders them ahead of earlier program-order writes.
    pub(crate) fn emit_wt_flush_call(&mut self) -> Vec<Instruction<'static>> {
        if !self.wt_active() {
            return Vec::new();
        }
        let idx = self.ensure_wt_flush_helper();
        vec![Self::call_user(idx), Instruction::Drop]
    }

    /// Upsert the (key, value) pair into the memo table and mark the slot
    /// dirty. `k` is the tagged-Str key local, `v` the tagged value local —
    /// BOTH must already be live (the storage_set arm holds them in
    /// `__sst_k` / `__sst_v` before its host call).
    ///
    /// Scan is EXACT (length filter + full byte compare, same discipline as
    /// emit_storage_cache_lookup): adjacent-memory garbage can force a false
    /// miss (extra slot) but never a false match (stale overwrite).
    /// On update: slot+16 = value, dirty bit set, done (host write deferred
    /// to the boundary). On miss with room: append a new slot + dirty bit.
    /// On miss with no room (full table or key > 64B): emits nothing and
    /// returns flag = u32::MAX — the caller's immediate host write (generic
    /// path) remains the write and the host-call skip must NOT fire.
    pub(crate) fn emit_wt_upsert(
        &mut self,
        k: u32,
        val: u32,
    ) -> Result<(Vec<Instruction<'static>>, u32), String> {
        if !self.wt_active() {
            return Ok((Vec::new(), u32::MAX)); // flag u32::MAX = "not emitted"
        }
        // Region was reserved at the gate site (reserve_wt_region); keep a
        // lazy fallback for direct-call paths (tests/probes).
        self.reserve_wt_region();
        let cnt_l = self.local_idx("__wt_cnt");
        let idx_l = self.local_idx("__wt_idx");
        let slot_l = self.local_idx("__wt_slot");
        let klen_l = self.local_idx("__wt_klen");
        let kptr_l = self.local_idx("__wt_kptr");
        let j_l = self.local_idx("__wt_j");
        let eq_l = self.local_idx("__wt_eq");
        let found_l = self.local_idx("__wt_found");
        let ma8 = Self::ma8();
        let ma0 = Self::ma0();
        let mut v = Vec::new();
        // Block S wraps the whole upsert: the two fallbacks Br out of it and
        // the append path runs at its tail. Early exits: Br(1) (If → S).
        v.push(Instruction::Block(BlockType::Empty)); // S
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
        // (raw kptr: untag already yields (len<<32|ptr); low 32 = ptr — keep
        // the canonical wrap+extend so kptr_l is a clean i64 pointer.)
        // key too long for the table → fall back to the immediate host write
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Const(CACHE_KEY_CAP));
        v.push(Instruction::I64GtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(1)); // → out of S
        v.push(Instruction::End);
        // found = 0 ; idx = 0 ; cnt = mem[CACHE_COUNT_ADDR]
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(found_l));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(idx_l));
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::LocalSet(cnt_l));
        // full table → fall back to the immediate host write
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(CACHE_SLOTS));
        v.push(Instruction::I64GeU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(1)); // out of Block S
        v.push(Instruction::End);
        // ── scan: Block A { Loop B { … } } ──
        v.push(Instruction::Block(BlockType::Empty)); // A
        v.push(Instruction::Loop(BlockType::Empty)); // B
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1)); // idx >= cnt → exit scan (A)
        v.push(Instruction::I64Const(CACHE_SLOT_BASE));
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::I64Const(CACHE_STRIDE));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(slot_l));
        // if mem[slot+8] == klen → byte compare
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg {
            offset: 8,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); // C
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(eq_l));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(j_l));
        // Block D { Loop E { if key[j] != slot_key[j] { eq=0; br D }; j++ } }
        v.push(Instruction::Block(BlockType::Empty)); // D
        v.push(Instruction::Loop(BlockType::Empty)); // E
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1)); // j >= klen → compared everything
                                      // mem8[slot+24+j] vs mem8[kptr+j]
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load8U(MemArg {
            offset: 24,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load8U(ma0.clone()));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty)); // F
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(eq_l));
        // mismatch → leave the compare loop: If F=0, Loop E=1, Block D=2
        v.push(Instruction::Br(2));
        v.push(Instruction::End); // F
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(j_l));
        v.push(Instruction::Br(0)); // continue E
        v.push(Instruction::End); // E
        v.push(Instruction::End); // D
                                  // if eq → update in place, mark dirty, exit scan
        v.push(Instruction::LocalGet(eq_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); // G
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(val));
        v.push(Instruction::I64Store(MemArg {
            offset: 16,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::I32Const(WT_DIRTY_BASE as i32));
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Add);
        v.push(Instruction::I64Const(1)); // u64.store8 → low byte survives
        v.push(Instruction::I64Store8(ma0.clone()));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(found_l));
        // exit the scan: label stack inside G is G=0, C=1, B=2, A=3 —
        // Br(3) targets Block A; control lands after A, at the found? guard.
        // (Br(2) targets Loop B's START: idx unchanged → same slot re-matches
        // → infinite loop, the fuel bomb.)
        v.push(Instruction::Br(3));
        v.push(Instruction::End); // G
        v.push(Instruction::End); // C (length fast-path arm — close BEFORE
                                  // the scan tail so idx++/Br(0) sit directly in Loop B: a Br(0) from
                                  // inside C would retarget C itself → infinite loop, the 200Tgas bomb)
        v.push(Instruction::LocalGet(idx_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(idx_l));
        v.push(Instruction::Br(0)); // continue B (innermost now — targets B)
        v.push(Instruction::End); // B
        v.push(Instruction::End); // A
                                  // found → done (slot updated, dirty)
        v.push(Instruction::LocalGet(found_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); // H
        v.push(Instruction::Br(1)); // out of Block S (H=0, S=1 → br 1)
        v.push(Instruction::End); // H
                                  // ── append new slot at index cnt (table has room — checked above;
                                  // no concurrent growth inside one upsert) ──
        v.push(Instruction::I64Const(CACHE_SLOT_BASE));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(CACHE_STRIDE));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(slot_l));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(kptr_l));
        v.push(Instruction::I64Store(ma8.clone()));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(klen_l));
        v.push(Instruction::I64Store(MemArg {
            offset: 8,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(slot_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(val));
        v.push(Instruction::I64Store(MemArg {
            offset: 16,
            align: 3,
            memory_index: 0,
        }));
        // zero the key-copy tail then copy klen bytes (same as cache insert)
        for off in (24..88).step_by(8) {
            v.push(Instruction::LocalGet(slot_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Const(0));
            v.push(Instruction::I64Store(MemArg {
                offset: off,
                align: 3,
                memory_index: 0,
            }));
        }
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
        v.push(Instruction::I64Store8(MemArg {
            offset: 24,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(j_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(j_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // N
        v.push(Instruction::End); // M
                                  // dirty bit + count++
        v.push(Instruction::I32Const(WT_DIRTY_BASE as i32));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Add);
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Store8(ma0.clone()));
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::LocalGet(cnt_l));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Store(ma8.clone()));
        // table took the write — the caller's immediate host write is skipped
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(found_l));
        v.push(Instruction::End); // S (outer)
        self.wt_used = true;
        Ok((v, found_l))
    }

    /// Synthesize `__h_wt_flush` — walk all 64 slots, replay dirty ones via
    /// storage_write(key_len, key_ptr, val_len, val_ptr, register=0), clear
    /// the dirty bit. Signature: () -> () (called from wrappers AND inline
    /// pre-flush sites; leaves the operand stack untouched).
    pub(crate) fn ensure_wt_flush_helper(&mut self) -> u32 {
        if let Some(idx) = self.wt_flush_fn {
            return idx;
        }
        self.need_host(17); // storage_write
        let idx = self.funcs.len();
        let instrs = Self::h_wt_flush();
        self.funcs.push(FuncDef {
            name: "__h_wt_flush".into(),
            param_count: 0,
            local_count: 6,
            instrs,
            local_entries: None,
            custom_type: Some((vec![], vec![])),
        });
        self.wt_flush_fn = Some(idx as u32);
        self.wt_used = true;
        idx as u32
    }

    fn h_wt_flush() -> Vec<Instruction<'static>> {
        // locals: 0=i, 1=slot, 2=v(payload), 3=klen, 4=vlen, 5=count bound
        let ma8 = Self::ma8();
        let ma0 = Self::ma0();
        let mut v = Vec::new();
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(0));
        // bound = mem64[CACHE_COUNT_ADDR] into local 5 (locals 0-4 are
        // i/slot/payload/klen/vlen — vlen would clobber the bound mid-loop
        // and silently stop the walk past 17 slots)
        v.push(Instruction::I32Const(CACHE_COUNT_ADDR as i32));
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::LocalSet(5));
        v.push(Instruction::Block(BlockType::Empty)); // A
        v.push(Instruction::Loop(BlockType::Empty)); // B
        v.push(Instruction::LocalGet(0));
        v.push(Instruction::LocalGet(5));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1)); // done
                                      // d = mem8[WT_DIRTY_BASE + i]
        v.push(Instruction::I32Const(WT_DIRTY_BASE as i32));
        v.push(Instruction::LocalGet(0));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Add);
        v.push(Instruction::I64Load8U(ma0.clone()));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty)); // C: dirty
                                                   // slot = CACHE_SLOT_BASE + i*STRIDE
        v.push(Instruction::I64Const(CACHE_SLOT_BASE));
        v.push(Instruction::LocalGet(0));
        v.push(Instruction::I64Const(CACHE_STRIDE));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(1));
        // payload = mem[slot+16] ; klen = mem[slot+8]
        v.push(Instruction::LocalGet(1));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg {
            offset: 16,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::LocalSet(2));
        // UNTAG the payload first (tagged = (payload<<3)|5): splitting the
        // raw word without >>3 yields vlen = 17*8 = 136 and vptr = ptr*8.
        v.push(Instruction::LocalGet(2));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(2));
        v.push(Instruction::LocalGet(1));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg {
            offset: 8,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::LocalSet(3));
        // vlen = payload >> 32 ; vptr = (u32)payload
        v.push(Instruction::LocalGet(2));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(4));
        // storage_write(klen, kptr, vlen, vptr, 0)
        v.push(Instruction::LocalGet(3)); // klen
                                          // kptr = mem64[slot+0] low 32 — LOAD the word, never wrap the slot
                                          // address itself (that fed the host the slot-table bytes as the
                                          // "key": 136784 LE → "P\x16\x02\x00\x00").
        v.push(Instruction::LocalGet(1));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        }));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalGet(4)); // vlen
        v.push(Instruction::LocalGet(2));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64ExtendI32U); // vptr
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(17));
        v.push(Instruction::Drop);
        // clear dirty
        v.push(Instruction::I32Const(WT_DIRTY_BASE as i32));
        v.push(Instruction::LocalGet(0));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Add);
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Store8(ma0.clone()));
        v.push(Instruction::End); // C
        v.push(Instruction::LocalGet(0));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(0));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // B
        v.push(Instruction::End); // A
        v.push(Instruction::I64Const(0)); // (result i64) — mirror of set's Num(0)
        v
    }
}
