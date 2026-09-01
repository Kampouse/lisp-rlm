use super::*;

impl WasmEmitter {
    pub(crate) fn read_to_register(
        &mut self,
        host_idx: usize,
        _a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        if !self.wasi_mode && !self.p2_mode {
            // NEAR mode: allocate from the runtime heap (U-fix 2, 2026-08-29 —
            // was FP_GLOBAL; register strings must survive later calls)
            let buf_i = self.local_idx("__rr_buf");
            let len_i = self.local_idx("__rr_len");
            let mut v = Vec::new();
            self.need_host(host_idx);
            // Call host function to write to register 0
            v.push(Instruction::I64Const(0)); // register_id=0
            v.push(Self::host_call(host_idx));
            // register_len(0) → save
            v.push(Instruction::I64Const(0));
            v.push(Self::host_call(1));
            v.push(Instruction::LocalSet(len_i));
            // Allocate buf from mem[56] runtime heap
            v.extend(self.emit_rtheap_alloc(buf_i, len_i));
            // read_register(0, buf)
            v.push(Instruction::I64Const(0));
            v.push(Instruction::LocalGet(buf_i));
            v.push(Self::host_call(0));
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
        // MUST register the host — the sentinel map only includes
        // host_needed; an unregistered call resolved to nothing and the
        // op silently evaluated to 0 (account_balance, 2026-09-01).
        self.need_host(host_idx);
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
        self.need_host(host_idx);
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

    /// (to-string <array>) — render "[e0, e1, ...]" (flat: num/bool/nil/str
    /// elements; nested arrays render as `<vec>`). Matches the interpreter's
    /// LispVal::Vec Display: strings quoted, ", " separators.
    /// Entry: tagged value on stack. Exit: tagged STR on stack.
    fn array_to_str_code(&mut self) -> Vec<Instruction<'static>> {
        let ma8 = |off: u64| wasm_encoder::MemArg { offset: off, align: 0, memory_index: 0 };
        let ma64 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let b = |i: i32| Instruction::I32Const(i);
        let n = |i: i64| Instruction::I64Const(i);
        let lg = |l: u32| Instruction::LocalGet(l);
        let ls = |l: u32| Instruction::LocalSet(l);

        let val = self.local_idx("__tsa_val");
        let base = self.local_idx("__tsa_base");
        let cnt = self.local_idx("__tsa_cnt");
        let sz = self.local_idx("__tsa_sz");
        let dst = self.local_idx("__tsa_dst");
        let cur = self.local_idx("__tsa_cur");
        let i = self.local_idx("__tsa_i");
        let elem = self.local_idx("__tsa_elem");
        let etag = self.local_idx("__tsa_etag");
        let nn = self.local_idx("__tsa_nn");
        let tmp = self.local_idx("__tsa_tmp");
        let nd = self.local_idx("__tsa_nd");
        let j = self.local_idx("__tsa_j");
        let sp = self.local_idx("__tsa_sp");
        let sl = self.local_idx("__tsa_sl");

        let mut v: Vec<Instruction<'static>> = Vec::new();
        let store8_at = |v: &mut Vec<_>, l: u32, off: i64, byte: i32| {
            v.push(lg(l));
            if off != 0 { v.push(n(off)); v.push(Instruction::I64Add); }
            v.push(Instruction::I32WrapI64);
            v.push(b(byte));
            v.push(Instruction::I32Store8(ma8(0)));
        };

        v.push(ls(val)); // val on stack → local
        // base = (val >> 3) & 0xFFFFFFFF
        v.push(lg(val)); v.push(n(3)); v.push(Instruction::I64ShrS);
        v.push(n(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(ls(base));
        // cnt = i64[base]
        v.push(lg(base)); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma64.clone())); v.push(ls(cnt));
        // sz = max(cnt*26 + 16, 64); dst = heap bump (aligned 8)
        v.push(lg(cnt)); v.push(n(26)); v.push(Instruction::I64Mul);
        v.push(n(16)); v.push(Instruction::I64Add); v.push(ls(sz));
        v.push(lg(sz)); v.push(n(64)); v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(n(64)); v.push(ls(sz));
        v.push(Instruction::End);
        v.push(b(56)); v.push(Instruction::I64Load(ma64.clone())); v.push(ls(dst));
        v.push(b(56));
        v.push(lg(dst)); v.push(lg(sz)); v.push(n(7)); v.push(Instruction::I64Add);
        v.push(n(-8)); v.push(Instruction::I64And); v.push(Instruction::I64Or);
        v.push(Instruction::I64Store(ma64.clone()));
        // cur = dst; mem[cur] = '['
        v.push(lg(dst)); v.push(ls(cur));
        store8_at(&mut v, cur, 0, 91); // '['
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        // for i in 0..cnt
        v.push(n(0)); v.push(ls(i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(lg(i)); v.push(lg(cnt)); v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // separator ", "
        v.push(lg(i)); v.push(n(0)); v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        store8_at(&mut v, cur, 0, 44); // ','
        store8_at(&mut v, cur, 1, 32); // ' '
        v.push(lg(cur)); v.push(n(2)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::End);
        // elem = i64[base + 8 + i*8]
        v.push(lg(base));
        v.push(lg(i)); v.push(n(8)); v.push(Instruction::I64Mul); v.push(Instruction::I64Add);
        v.push(n(8)); v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma64.clone())); v.push(ls(elem));
        // etag = elem & 7
        v.push(lg(elem)); v.push(n(7)); v.push(Instruction::I64And); v.push(ls(etag));

        // NUM branch
        v.push(lg(etag)); v.push(n(TAG_NUM)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(lg(elem)); v.push(n(3)); v.push(Instruction::I64ShrS); v.push(ls(nn));
        // negative: '-' then nn = -nn
        v.push(lg(nn)); v.push(n(0)); v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        store8_at(&mut v, cur, 0, 45); // '-'
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(n(0)); v.push(lg(nn)); v.push(Instruction::I64Sub); v.push(ls(nn));
        v.push(Instruction::End);
        v.push(lg(nn)); v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        store8_at(&mut v, cur, 0, 48); // '0'
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::Else);
        // digits backward from cur+19, then copy
        v.push(lg(cur)); v.push(n(19)); v.push(Instruction::I64Add); v.push(ls(tmp));
        v.push(n(0)); v.push(ls(nd));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(lg(nn)); v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(lg(tmp)); v.push(n(1)); v.push(Instruction::I64Sub); v.push(ls(tmp));
        store8_at(&mut v, tmp, 0, 48); // placeholder overwritten below
        // dig = nn % 10 + 48
        v.push(lg(tmp)); v.push(Instruction::I32WrapI64);
        v.push(lg(nn)); v.push(n(10)); v.push(Instruction::I64RemU);
        v.push(n(48)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8(0)));
        v.push(lg(nn)); v.push(n(10)); v.push(Instruction::I64DivU); v.push(ls(nn));
        v.push(lg(nd)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(nd));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        // copy tmp..tmp+nd-1 → cur
        v.push(n(0)); v.push(ls(j));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(lg(j)); v.push(lg(nd)); v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(lg(cur)); v.push(lg(j)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(lg(tmp)); v.push(lg(j)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8(0)));
        v.push(Instruction::I32Store8(ma8(0)));
        v.push(lg(j)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(j));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        v.push(lg(cur)); v.push(lg(nd)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::End); // if nn==0
        // tag dispatch: exclusive if/else-if chain (each element takes exactly one)
        // BOOL branch
        v.push(Instruction::Else);
        v.push(lg(etag)); v.push(n(TAG_BOOL)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(lg(elem)); v.push(n(3)); v.push(Instruction::I64ShrS);
        v.push(n(0)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        // "false"
        store8_at(&mut v, cur, 0, 102); store8_at(&mut v, cur, 1, 97);
        store8_at(&mut v, cur, 2, 108); store8_at(&mut v, cur, 3, 115);
        store8_at(&mut v, cur, 4, 101);
        v.push(lg(cur)); v.push(n(5)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::Else);
        // "true"
        store8_at(&mut v, cur, 0, 116); store8_at(&mut v, cur, 1, 114);
        store8_at(&mut v, cur, 2, 117); store8_at(&mut v, cur, 3, 101);
        v.push(lg(cur)); v.push(n(4)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::End);

        // NIL branch
        v.push(Instruction::Else);
        v.push(lg(etag)); v.push(n(TAG_NIL)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        store8_at(&mut v, cur, 0, 110); store8_at(&mut v, cur, 1, 105);
        store8_at(&mut v, cur, 2, 108); // "nil"
        v.push(lg(cur)); v.push(n(3)); v.push(Instruction::I64Add); v.push(ls(cur));
        // (no End — NIL's If stays open, chained into STR via Else)

        // STR branch: '"' bytes '"' (quoted, matches interp Vec Display)
        v.push(Instruction::Else);
        v.push(lg(etag)); v.push(n(TAG_STR)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(lg(elem)); v.push(n(3)); v.push(Instruction::I64ShrS); v.push(ls(nn));
        v.push(lg(nn)); v.push(n(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(ls(sp));
        v.push(lg(nn)); v.push(n(32)); v.push(Instruction::I64ShrU); v.push(ls(sl));
        store8_at(&mut v, cur, 0, 34); // '"'
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(n(0)); v.push(ls(j));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(lg(j)); v.push(lg(sl)); v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(lg(cur)); v.push(lg(j)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(lg(sp)); v.push(lg(j)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8(0)));
        v.push(Instruction::I32Store8(ma8(0)));
        v.push(lg(j)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(j));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(lg(cur)); v.push(lg(sl)); v.push(Instruction::I64Add); v.push(ls(cur));
        store8_at(&mut v, cur, 0, 34); // '"'
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        // (no End — STR's If stays open, chained into the default via Else)

        // default: ARRAY → "<vec>", other tags → "?"
        v.push(Instruction::Else);
        v.push(lg(etag)); v.push(n(TAG_ARRAY)); v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        store8_at(&mut v, cur, 0, 60); store8_at(&mut v, cur, 1, 118);
        store8_at(&mut v, cur, 2, 101); store8_at(&mut v, cur, 3, 99);
        store8_at(&mut v, cur, 4, 62); // "<vec>"
        v.push(lg(cur)); v.push(n(5)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::Else);
        store8_at(&mut v, cur, 0, 63); // '?'
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        v.push(Instruction::End);
        // close the 5-level tag dispatch chain (NUM/BOOL/NIL/STR/ARRAY-else)
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);

        // i++
        v.push(lg(i)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // loop
        v.push(Instruction::End); // block
        // ']'
        store8_at(&mut v, cur, 0, 93);
        v.push(lg(cur)); v.push(n(1)); v.push(Instruction::I64Add); v.push(ls(cur));
        // packed (len<<32)|dst, tagged STR
        v.push(lg(cur)); v.push(lg(dst)); v.push(Instruction::I64Sub); v.push(ls(nd));
        v.push(lg(nd)); v.push(n(32)); v.push(Instruction::I64Shl);
        v.push(lg(dst)); v.push(Instruction::I64Or);
        v.extend(self.emit_tag_str());
        v
    }

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
        let val_i = self.local_idx("__its2_val");
        let mut v = Vec::new();
        // Tag-aware: (to-string str) must be identity (matches the interpreter's
        // Display-based to-string). Only NUM takes the decimal path.
        v.extend(n);
        v.push(Instruction::LocalSet(val_i));
        v.push(Instruction::LocalGet(val_i));
        v.push(Instruction::I64Const(7));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Const(TAG_STR));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Result(wasm_encoder::ValType::I64)));
        // STR path: pass the tagged string through unchanged
        v.push(Instruction::LocalGet(val_i));
        v.push(Instruction::Else);
        // ARRAY path (2026-08-31): render "[e0, e1, ...]" — was falling into
        // the NUM path and logging the raw heap pointer as decimal.
        v.push(Instruction::LocalGet(val_i));
        v.push(Instruction::I64Const(7));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Const(TAG_ARRAY));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Result(wasm_encoder::ValType::I64)));
        v.push(Instruction::LocalGet(val_i));
        v.extend(self.array_to_str_code());
        v.push(Instruction::Else);
        // NIL path (2026-08-31): to-string(nil) → "nil" — was falling to
        // NUM and printing the tag bits as "0". `jsonGetStr` now returns
        // nil on missing keys; bare to-string of a miss must be visible.
        v.push(Instruction::LocalGet(val_i));
        v.push(Instruction::I64Const(7));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Const(TAG_NIL));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Result(wasm_encoder::ValType::I64)));
        let nil_off = self.alloc_data(b"nil") as i64;
        // (3 << 32 | nil_off) << 3 | TAG_STR — "nil" as a tagged str
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(nil_off));
        v.push(Instruction::I64Or);
        v.push(Instruction::I64Const(TAG_BITS));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(TAG_STR));
        v.push(Instruction::I64Or);
        v.push(Instruction::Else);
        // NUM path: untag the number before converting: val >> TAG_BITS
        // TAG_NUM uses signed values — must use arithmetic (signed) right shift
        // to preserve negative numbers. I64ShrU would mangle negatives.
        v.push(Instruction::LocalGet(val_i));
        v.push(Instruction::I64Const(TAG_BITS));
        v.push(Instruction::I64ShrS);
        v.push(Instruction::LocalSet(n_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(neg_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(len_i));
        // ALWAYS allocate from the runtime heap (2026-08-30): the old NEAR-mode
        // compile-time site (next_data_offset.max(3072)) collided with other
        // static-site allocators (str-concat sites at .max(3072), str-split
        // delimiter data at .max(4096)) — layout-dependent heap corruption:
        // a to-string result stored in an array came back as tagged-byte
        // garbage (run_g '10,21,8590263846'). Site reuse was also unsound for
        // any result that OUTLIVES the call (array slots, later joins).
        v.extend(self.heap_bump_runtime(64, "__its2_dst"));
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
        // Simpler: write '-' at dst-1, adjust dst and len
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(45)); // '-'
        v.push(Instruction::I32Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(dst_i)); // dst -= 1
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(len_i)); // len += 1
        v.push(Instruction::End);
        // Return packed: (len << 32) | dst, tagged as string
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Or);
        v.extend(self.emit_tag_str());
        v.push(Instruction::End); // close NIL/NUM if
        v.push(Instruction::End); // close ARRAY/else if
        v.push(Instruction::End); // close STR/else if
        Ok(v)
    }
}
