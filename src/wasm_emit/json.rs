use super::*;

impl WasmEmitter {
    pub(crate) fn json_get_with_scanner(
        &mut self,
        key: &str,
        value_type: &str,
    ) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
        let mut setup = Vec::new();
        setup.push(Instruction::I64Const(0));
        setup.push(Self::host_call(7));
        setup.push(Instruction::I64Const(0));
        setup.push(Self::host_call(1));
        setup.push(Instruction::I64Const(0));
        setup.push(Instruction::I64Const(INPUT_BUF));
        setup.push(Self::host_call(0));
        self.json_get_from_buf(key, value_type, INPUT_BUF, &mut setup)
    }

    pub fn json_get_wasi(
        &mut self,
        key: &str,
        value_type: &str,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let ma4 = wasm_encoder::MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        };
        let mut setup = Vec::new();
        setup.push(Instruction::I32Const(98304));
        setup.push(Instruction::I32Load(ma4));
        setup.push(Instruction::I64ExtendI32U);
        let mut v = self.json_get_from_buf(key, value_type, 32768, &mut setup)?;
        // Heap copy for "str" results: __json_get writes to stdout_buf (65536),
        // which gets overwritten by subsequent __json_get calls.
        if value_type == "str" {
            let rhp: i32 = 56; // RUNTIME_HEAP_PTR
            let jgs_tmp = self.local_idx("__jgw_tmp");
            let jgs_len = self.local_idx_i32("__jgw_len");
            let jgs_ptr = self.local_idx_i32("__jgw_ptr");
            let jgs_heap = self.local_idx_i32("__jgw_heap");
            v.push(Instruction::LocalSet(jgs_tmp));
            // Extract len and ptr from packed result
            v.push(Instruction::LocalGet(jgs_tmp));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64ShrU);
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(jgs_len));
            v.push(Instruction::LocalGet(jgs_tmp));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(jgs_ptr));
            // Use compile-time heap allocation (64KB) — large enough for any realistic JSON value
            let heap_dst = self.heap_bump(65536);
            v.push(Instruction::I32Const(heap_dst as i32));
            v.push(Instruction::LocalGet(jgs_ptr));
            v.push(Instruction::LocalGet(jgs_len));
            v.push(Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });
            // Repack: (len << 32) | heap
            v.push(Instruction::LocalGet(jgs_len));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64Shl);
            v.push(Instruction::I32Const(heap_dst as i32));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::I64Or);
            v.push(Instruction::I64Const(3));
            v.push(Instruction::I64Shl);
            v.push(Instruction::I64Const(crate::wasm_emit::TAG_STR));
            v.push(Instruction::I64Or);
        }
        Ok(v)
    }

    pub(crate) fn ensure_json_get_func(&mut self) -> u32 {
        if let Some(idx) = self.funcs.iter().position(|f| f.name == "__json_get") {
            return idx as u32;
        }
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let stdout_buf: i32 = 204800; // 200KB — must not collide with SENTINEL_BUF (65536)
        let json_ptr = 2u32;
        let json_len = 3u32;
        let pat_ptr = 4u32;
        let pat_len = 5u32;
        let scan_i = 6u32;
        let temp = 7u32;
        let _val_start = 8u32;
        let depth = 9u32;
        let cmp_j = 10u32;
        let esc = 11u32;
        let str_len = 12u32;
        let dst = 13u32;
        let ch = 14u32;
        let memchr_tmp = 15u32; // i64 for memchr/fast copy

        let mut ins: Vec<Instruction<'static>> = Vec::new();
        let mut d: u32 = 0;
        let mut ls: Vec<u32> = Vec::new();
        macro_rules! open_block {
            () => {
                ls.push(d);
                ins.push(Instruction::Block(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_loop {
            () => {
                ls.push(d);
                ins.push(Instruction::Loop(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_if {
            () => {
                ls.push(d);
                ins.push(Instruction::If(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_else {
            () => {
                ins.push(Instruction::Else);
            };
        }
        macro_rules! close {
            () => {
                ins.push(Instruction::End);
                ls.pop();
                d -= 1;
            };
        }
        macro_rules! br_to {
            ($idx:expr) => {
                let t = ls[$idx];
                ins.push(Instruction::Br(d - t - 1));
            };
        }

        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(json_len));
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(json_ptr));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(pat_len));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(pat_ptr));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(scan_i));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(depth));

        open_block!();
        let scan_block = ls.len() - 1;
        open_loop!();
        let scan_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(pat_len));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GtS);
        open_if!();
        br_to!(scan_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(temp));
        ins.push(Instruction::LocalGet(temp));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(depth));
        close!();
        ins.push(Instruction::LocalGet(temp));
        ins.push(Instruction::I32Const(0x7D));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(depth));
        close!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Ne);
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(scan_loop);
        close!();
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(temp));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(cmp_j));
        open_block!();
        let pat_block = ls.len() - 1;
        open_loop!();
        let pat_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(cmp_j));
        ins.push(Instruction::LocalGet(pat_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(pat_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(cmp_j));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalGet(pat_ptr));
        ins.push(Instruction::LocalGet(cmp_j));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::I32Eq);
        open_if!();
        open_else!();
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(temp));
        br_to!(pat_block);
        close!();
        ins.push(Instruction::LocalGet(cmp_j));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(cmp_j));
        br_to!(pat_loop);
        close!();
        close!();
        ins.push(Instruction::LocalGet(temp));
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32GtS);
        open_if!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x2C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x20));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x09));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x0A));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::I32Eqz);
        open_if!();
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(temp));
        close!();
        close!();
        close!();
        ins.push(Instruction::LocalGet(temp));
        open_if!();
        br_to!(scan_block);
        close!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(scan_loop);
        close!();
        close!();

        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::Return);
        close!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(pat_len));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        open_block!();
        let ws_block = ls.len() - 1;
        open_loop!();
        let ws_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(ws_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x20));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x09));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x0A));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(ws_loop);
        close!();
        close!();
        close!();

        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        open_if!();
        {
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            ins.push(Instruction::I32Const(stdout_buf));
            ins.push(Instruction::LocalSet(dst));
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(str_len));
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(esc));
            open_block!();
            let str_block = ls.len() - 1;

            // Fast 8-byte string copy loop
            open_block!();
            let fast_str_block = ls.len() - 1;
            open_loop!();
            let fast_str_loop = ls.len() - 1;
            // Need at least 8 bytes remaining AND not in escape state
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(json_len));
            ins.push(Instruction::I32GtS);
            open_if!();
            br_to!(fast_str_block);
            close!();
            ins.push(Instruction::LocalGet(esc));
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::I32Ne);
            open_if!();
            br_to!(fast_str_block);
            close!();
            // Check for '"' and '\' in 8-byte chunk
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I64Load(ma8.clone()));
            ins.push(Instruction::LocalTee(memchr_tmp));
            ins.push(Instruction::I64Const(0x2222222222222222_u64 as i64));
            ins.push(Instruction::I64Xor);
            ins.push(Instruction::LocalTee(memchr_tmp));
            ins.push(Instruction::I64Const(-1));
            ins.push(Instruction::I64Xor);
            ins.push(Instruction::LocalGet(memchr_tmp));
            ins.push(Instruction::I64Const(0x0101010101010101));
            ins.push(Instruction::I64Sub);
            ins.push(Instruction::I64And);
            ins.push(Instruction::I64Const(0x8080808080808080_u64 as i64));
            ins.push(Instruction::I64And);
            // has_zero for '"'
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I64Load(ma8.clone()));
            ins.push(Instruction::I64Const(0x5C5C5C5C5C5C5C5C_u64 as i64));
            ins.push(Instruction::I64Xor);
            ins.push(Instruction::LocalTee(memchr_tmp));
            ins.push(Instruction::I64Const(-1));
            ins.push(Instruction::I64Xor);
            ins.push(Instruction::LocalGet(memchr_tmp));
            ins.push(Instruction::I64Const(0x0101010101010101));
            ins.push(Instruction::I64Sub);
            ins.push(Instruction::I64And);
            ins.push(Instruction::I64Const(0x8080808080808080_u64 as i64));
            ins.push(Instruction::I64And);
            // has_zero for '\'
            ins.push(Instruction::I64Or);
            ins.push(Instruction::I64Eqz);
            open_if!();
            // No special char → copy 8 bytes
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I64Load(ma8.clone()));
            ins.push(Instruction::I64Store(ma8.clone()));
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(dst));
            ins.push(Instruction::LocalGet(str_len));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(str_len));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(fast_str_loop);
            close!(); // no special char
            close!();
            close!(); // fast_str_loop, fast_str_block

            // Slow byte-by-byte fallback
            open_loop!();
            let str_loop = ls.len() - 1;
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::LocalGet(json_len));
            ins.push(Instruction::I32GeS);
            open_if!();
            br_to!(str_block);
            close!();
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load8U(ma8.clone()));
            ins.push(Instruction::LocalSet(ch));
            ins.push(Instruction::LocalGet(esc));
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x22));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32And);
            open_if!();
            br_to!(str_block);
            close!();
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x5C));
            ins.push(Instruction::I32Eq);
            open_if!();
            ins.push(Instruction::LocalGet(esc));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Xor);
            ins.push(Instruction::LocalSet(esc));
            open_else!();
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(esc));
            close!();
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Store8(ma8.clone()));
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(dst));
            ins.push(Instruction::LocalGet(str_len));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(str_len));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(str_loop);
            close!();
            close!();
            ins.push(Instruction::LocalGet(str_len));
            ins.push(Instruction::I64ExtendI32U);
            ins.push(Instruction::I64Const(32));
            ins.push(Instruction::I64Shl);
            ins.push(Instruction::I64Const(stdout_buf as i64));
            ins.push(Instruction::I64Or);
            ins.push(Instruction::Return);
        }
        open_else!();
        {
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x7B));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x5B));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            open_if!();
            {
                ins.push(Instruction::I32Const(0));
                ins.push(Instruction::LocalSet(depth));
                ins.push(Instruction::I32Const(stdout_buf));
                ins.push(Instruction::LocalSet(dst));
                ins.push(Instruction::I32Const(0));
                ins.push(Instruction::LocalSet(str_len));
                open_block!();
                let brk_block = ls.len() - 1;
                open_loop!();
                let brk_loop = ls.len() - 1;
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::LocalGet(json_len));
                ins.push(Instruction::I32GeS);
                open_if!();
                br_to!(brk_block);
                close!();
                ins.push(Instruction::LocalGet(json_ptr));
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::I32Load8U(ma8.clone()));
                ins.push(Instruction::LocalSet(ch));
                ins.push(Instruction::LocalGet(dst));
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Store8(ma8.clone()));
                ins.push(Instruction::LocalGet(dst));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(dst));
                ins.push(Instruction::LocalGet(str_len));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(str_len));
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x7B));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x5B));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                open_if!();
                ins.push(Instruction::LocalGet(depth));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(depth));
                close!();
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x7D));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x5D));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                open_if!();
                ins.push(Instruction::LocalGet(depth));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Sub);
                ins.push(Instruction::LocalSet(depth));
                ins.push(Instruction::LocalGet(depth));
                ins.push(Instruction::I32Const(0));
                ins.push(Instruction::I32Eq);
                open_if!();
                br_to!(brk_block);
                close!();
                close!();
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(scan_i));
                br_to!(brk_loop);
                close!();
                close!();
                ins.push(Instruction::LocalGet(str_len));
                ins.push(Instruction::I64ExtendI32U);
                ins.push(Instruction::I64Const(32));
                ins.push(Instruction::I64Shl);
                ins.push(Instruction::I64Const(stdout_buf as i64));
                ins.push(Instruction::I64Or);
                ins.push(Instruction::Return);
            }
            open_else!();
            {
                ins.push(Instruction::I32Const(stdout_buf));
                ins.push(Instruction::LocalSet(dst));
                ins.push(Instruction::I32Const(0));
                ins.push(Instruction::LocalSet(str_len));
                open_block!();
                let raw_block = ls.len() - 1;
                open_loop!();
                let raw_loop = ls.len() - 1;
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::LocalGet(json_len));
                ins.push(Instruction::I32GeS);
                open_if!();
                br_to!(raw_block);
                close!();
                ins.push(Instruction::LocalGet(json_ptr));
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::I32Load8U(ma8.clone()));
                ins.push(Instruction::LocalSet(ch));
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x2C));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x7D));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x5D));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x20));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Const(0x0A));
                ins.push(Instruction::I32Eq);
                ins.push(Instruction::I32Or);
                open_if!();
                br_to!(raw_block);
                close!();
                ins.push(Instruction::LocalGet(dst));
                ins.push(Instruction::LocalGet(ch));
                ins.push(Instruction::I32Store8(ma8.clone()));
                ins.push(Instruction::LocalGet(dst));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(dst));
                ins.push(Instruction::LocalGet(str_len));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(str_len));
                ins.push(Instruction::LocalGet(scan_i));
                ins.push(Instruction::I32Const(1));
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalSet(scan_i));
                br_to!(raw_loop);
                close!();
                close!();
                ins.push(Instruction::LocalGet(str_len));
                ins.push(Instruction::I64ExtendI32U);
                ins.push(Instruction::I64Const(32));
                ins.push(Instruction::I64Shl);
                ins.push(Instruction::I64Const(stdout_buf as i64));
                ins.push(Instruction::I64Or);
                ins.push(Instruction::Return);
            }
            close!();
            close!();
        }
        ins.push(Instruction::I64Const(0));
        ins.push(Instruction::Return);
        self.funcs.push(FuncDef {
            name: "__json_get".to_string(),
            param_count: 2,
            local_count: 13,
            instrs: ins,
            local_entries: Some(vec![(13u32, ValType::I32), (1u32, ValType::I64)]),
        });
        (self.funcs.len() - 1) as u32
    }

    /// Generate a `__json_extract_N` function that scans a JSON buffer once and
    /// extracts N key values. Returns a tagged array pointer.
    /// Signature: (buf_packed: i64, key0_packed: i64, ..., keyN-1_packed: i64) -> i64
    pub(crate) fn ensure_json_extract_func(&mut self, n_keys: usize) -> u32 {
        let fname = format!("__json_extract_{}", n_keys);
        if let Some(idx) = self.funcs.iter().position(|f| f.name == fname) {
            return idx as u32;
        }
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let ma4 = wasm_encoder::MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        };
        let stdout_buf: i32 = 204800; // 200KB — must not collide with SENTINEL_BUF (65536)
        let slot_size: i32 = 4096; // bytes per result string area (large JSON responses need room)

        // Locals (all i32 — params are i64 at indices 0..n_keys, rest are i32)
        let mut next_local = (n_keys + 1) as u32; // params: buf + N keys
        let json_ptr = next_local;
        next_local += 1;
        let json_len = next_local;
        next_local += 1;
        // Key ptrs and lens
        let mut key_ptrs = Vec::new();
        let mut key_lens = Vec::new();
        for _ in 0..n_keys {
            key_ptrs.push(next_local);
            next_local += 1;
            key_lens.push(next_local);
            next_local += 1;
        }
        let scan_i = next_local;
        next_local += 1;
        let depth = next_local;
        next_local += 1;
        let temp = next_local;
        next_local += 1;
        let cmp_j = next_local;
        next_local += 1;
        let ch = next_local;
        next_local += 1;
        let dst = next_local;
        next_local += 1;
        let str_len = next_local;
        next_local += 1;
        let esc = next_local;
        next_local += 1;
        let found = next_local;
        next_local += 1;
        let cur_key = next_local;
        next_local += 1;
        let val_type = next_local;
        next_local += 1; // 0=string, 1=object/array, 2=raw
                         // memchr_tmp is i64 — index = all i32 locals (including any added below) + params
                         // We know exactly 1 extra i32 local (arr_ptr_local) is added later in the body
        let memchr_tmp = next_local + 1; // +1 for arr_ptr_local
        let n_i32_locals = next_local - (n_keys as u32 + 1);

        let mut ins: Vec<Instruction<'static>> = Vec::new();
        let mut d: u32 = 0;
        let mut ls: Vec<u32> = Vec::new();
        macro_rules! open_block {
            () => {
                ls.push(d);
                ins.push(Instruction::Block(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_loop {
            () => {
                ls.push(d);
                ins.push(Instruction::Loop(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_if {
            () => {
                ls.push(d);
                ins.push(Instruction::If(BlockType::Empty));
                d += 1;
            };
        }
        macro_rules! open_else {
            () => {
                ins.push(Instruction::Else);
            };
        }
        macro_rules! close {
            () => {
                ins.push(Instruction::End);
                ls.pop();
                d -= 1;
            };
        }
        macro_rules! br_to {
            ($idx:expr) => {
                let t = ls[$idx];
                ins.push(Instruction::Br(d - t - 1));
            };
        }

        // ── Extract buf ──
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(json_len));
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(json_ptr));

        // ── Extract keys ──
        for k in 0..n_keys {
            let param_idx = (k + 1) as u32;
            ins.push(Instruction::LocalGet(param_idx));
            ins.push(Instruction::I64Const(32));
            ins.push(Instruction::I64ShrU);
            ins.push(Instruction::I32WrapI64);
            ins.push(Instruction::LocalSet(key_lens[k]));
            ins.push(Instruction::LocalGet(param_idx));
            ins.push(Instruction::I32WrapI64);
            ins.push(Instruction::LocalSet(key_ptrs[k]));
        }

        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(found));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(scan_i));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(depth));

        // ── Outer scan loop ──
        open_block!();
        let scan_block = ls.len() - 1;
        open_loop!();
        let scan_loop = ls.len() - 1;

        // Early exit: all found
        ins.push(Instruction::LocalGet(found));
        ins.push(Instruction::I32Const(n_keys as i32));
        ins.push(Instruction::I32Eq);
        open_if!();
        br_to!(scan_block);
        close!();

        // Bounds check (need at least 2 bytes for shortest possible key match)
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(scan_block);
        close!();

        // Read byte
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(temp));

        // Track brace depth
        ins.push(Instruction::LocalGet(temp));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(depth));
        close!();
        ins.push(Instruction::LocalGet(temp));
        ins.push(Instruction::I32Const(0x7D));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(depth));
        close!();

        // Only match at depth 1
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Ne);
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(scan_loop);
        close!();

        // Memchr skip: at depth 1, keys start with '"' — use 8-byte I64Load chunks
        // to skip non-'"' bytes in bulk instead of one-by-one
        ins.push(Instruction::LocalGet(temp));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Ne);
        open_if!();
        {
            open_block!();
            let memchr_block = ls.len() - 1;
            open_loop!();
            let memchr_loop = ls.len() - 1;

            // Bounds check: need at least 8 bytes remaining
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(json_len));
            ins.push(Instruction::I32GtS);
            open_if!();
            // Less than 8 bytes — fall back to byte-by-byte
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(scan_loop);
            close!();

            // Load 8 bytes from current position
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I64Load(ma8.clone())); // byte-aligned i64 load

            // XOR with 0x2222222222222222 — zero bytes now indicate '"' positions
            ins.push(Instruction::I64Const(0x2222222222222222));
            ins.push(Instruction::I64Xor);

            // has_zero(v) = (v - 0x0101...) & ~v & 0x8080...
            // Save v to temp, compute ~v, then compute v - K, AND together
            ins.push(Instruction::LocalTee(memchr_tmp)); // temp = v (i64)
            ins.push(Instruction::I64Const(-1));
            ins.push(Instruction::I64Xor); // ~v
            ins.push(Instruction::LocalGet(memchr_tmp));
            ins.push(Instruction::I64Const(0x0101010101010101));
            ins.push(Instruction::I64Sub); // v - K
            ins.push(Instruction::I64And); // ~v & (v-K)
            ins.push(Instruction::I64Const(0x8080808080808080_u64 as i64));
            ins.push(Instruction::I64And);
            // Stack: [has_zero result] — non-zero if any byte was '"'

            // Save result, check if zero (no '"' found)
            ins.push(Instruction::LocalTee(memchr_tmp)); // reuse temp for result
            ins.push(Instruction::I64Eqz);
            open_if!();
            // No '"' in these 8 bytes — advance by 8 and continue
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(memchr_loop);
            close!();

            // Found '"' — extract byte position from CTZ of result
            ins.push(Instruction::LocalGet(memchr_tmp));
            ins.push(Instruction::I64Ctz);
            ins.push(Instruction::I64Const(3));
            ins.push(Instruction::I64ShrU); // ctz >> 3 = byte position
            ins.push(Instruction::I32WrapI64);
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(scan_loop); // main loop handles the '"' at this position

            close!();
            close!(); // memchr_loop, memchr_block
        }
        close!();

        // ── Try each key pattern ──
        // We chain: try key 0, if match goto extract; try key 1, if match goto extract; ...
        // After all keys tried, advance scan_i and continue
        open_block!();
        let try_block = ls.len() - 1; // break to here after trying all keys

        for k in 0..n_keys {
            // Pattern match: compare json_ptr[scan_i..] against key_ptrs[k] for key_lens[k] bytes
            // Multi-byte: use I32Load (4 bytes) when possible for 4x fewer iterations
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::LocalSet(temp)); // assume match
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(cmp_j));
            open_block!();
            let pat_block_k = ls.len() - 1;
            open_loop!();
            let pat_loop_k = ls.len() - 1;
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::LocalGet(key_lens[k]));
            ins.push(Instruction::I32GeS);
            open_if!();
            br_to!(pat_block_k);
            close!();
            // Check if we can do a 4-byte comparison
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Const(4));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(key_lens[k]));
            ins.push(Instruction::I32LeS);
            open_if!();
            // 4-byte comparison: I32Load from both JSON and pattern
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load(ma4.clone()));
            ins.push(Instruction::LocalGet(key_ptrs[k]));
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load(ma4.clone()));
            ins.push(Instruction::I32Eq);
            // If 4-byte match → advance by 4; else → mismatch
            open_if!();
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Const(4));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(cmp_j));
            open_else!();
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(temp));
            br_to!(pat_block_k);
            close!(); // end 4-byte match/mismatch if
                      // Else: single-byte comparison (remaining < 4 bytes)
            open_else!();
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load8U(ma8.clone()));
            ins.push(Instruction::LocalGet(key_ptrs[k]));
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load8U(ma8.clone()));
            ins.push(Instruction::I32Eq);
            // If byte match → advance by 1; else → mismatch
            open_if!();
            ins.push(Instruction::LocalGet(cmp_j));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(cmp_j));
            open_else!();
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(temp));
            br_to!(pat_block_k);
            close!(); // end byte match/mismatch if
            close!(); // end can-4-byte if/else
            br_to!(pat_loop_k);
            close!();
            close!(); // pat loop, pat block

            // If match (temp == 1), validate preceding char and extract
            ins.push(Instruction::LocalGet(temp));
            open_if!();
            // Validate preceding char
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::I32GtS);
            open_if!();
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Sub);
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load8U(ma8.clone()));
            ins.push(Instruction::LocalSet(ch));
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x7B));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x2C));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x20));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x09));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x0A));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::I32Eqz);
            open_if!();
            ins.push(Instruction::I32Const(0));
            ins.push(Instruction::LocalSet(temp));
            close!();
            close!(); // close invalid + prev_char blocks only

            // If still matching after validation
            ins.push(Instruction::LocalGet(temp));
            open_if!();
            // Set cur_key = k
            ins.push(Instruction::I32Const(k as i32));
            ins.push(Instruction::LocalSet(cur_key));
            // Advance scan_i past pattern
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::LocalGet(key_lens[k]));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(try_block); // break out of try-all-keys block → go to value extraction
            close!(); // match valid
            close!(); // match
        }

        // No key matched — advance and continue scan
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(scan_loop);
        close!(); // try_block

        // ── Value extraction (land here after a key match) ──
        // Skip whitespace
        open_block!();
        let ws_block = ls.len() - 1;
        open_loop!();
        let ws_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(ws_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x20));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x09));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x0A));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(ws_loop);
        close!();
        close!();
        close!(); // ws

        // Read first non-ws byte
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));

        // dst_base = stdout_buf + cur_key * slot_size
        ins.push(Instruction::I32Const(stdout_buf));
        ins.push(Instruction::LocalGet(cur_key));
        ins.push(Instruction::I32Const(slot_size));
        ins.push(Instruction::I32Mul);
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(dst));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(str_len));

        // ── STRING: '"' ──
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(esc));
        open_block!();
        let str_block = ls.len() - 1;

        // Fast 8-byte string copy loop
        open_block!();
        let fast_str_block = ls.len() - 1;
        open_loop!();
        let fast_str_loop = ls.len() - 1;
        // Need at least 8 bytes remaining AND not in escape state
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(8));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GtS);
        open_if!();
        br_to!(fast_str_block);
        close!(); // fall back to byte-by-byte
        ins.push(Instruction::LocalGet(esc));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Ne);
        open_if!();
        br_to!(fast_str_block);
        close!(); // in escape, fall back

        // Load 8 bytes from source
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I64Load(ma8.clone()));
        ins.push(Instruction::LocalTee(memchr_tmp)); // save for copy

        // Check for '"' (0x22) in chunk: has_zero(chunk XOR 0x2222...)
        ins.push(Instruction::I64Const(0x2222222222222222_u64 as i64));
        ins.push(Instruction::I64Xor);
        ins.push(Instruction::LocalTee(memchr_tmp)); // temp save XOR result for has_zero
        ins.push(Instruction::I64Const(-1));
        ins.push(Instruction::I64Xor); // ~v
        ins.push(Instruction::LocalGet(memchr_tmp));
        ins.push(Instruction::I64Const(0x0101010101010101));
        ins.push(Instruction::I64Sub);
        ins.push(Instruction::I64And);
        ins.push(Instruction::I64Const(0x8080808080808080_u64 as i64));
        ins.push(Instruction::I64And);
        // has_zero result on stack for '"'

        // Check for '\' (0x5C) in chunk: has_zero(chunk XOR 0x5C5C...)
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I64Load(ma8.clone()));
        ins.push(Instruction::I64Const(0x5C5C5C5C5C5C5C5C_u64 as i64));
        ins.push(Instruction::I64Xor);
        ins.push(Instruction::LocalTee(memchr_tmp));
        ins.push(Instruction::I64Const(-1));
        ins.push(Instruction::I64Xor);
        ins.push(Instruction::LocalGet(memchr_tmp));
        ins.push(Instruction::I64Const(0x0101010101010101));
        ins.push(Instruction::I64Sub);
        ins.push(Instruction::I64And);
        ins.push(Instruction::I64Const(0x8080808080808080_u64 as i64));
        ins.push(Instruction::I64And);
        // has_zero for '\'

        // If either has_zero is non-zero, special char found → fall back
        ins.push(Instruction::I64Or); // OR the two has_zero results
        ins.push(Instruction::I64Eqz);
        open_if!();
        // No '"' or '\' in this 8-byte chunk → copy all 8 bytes at once
        ins.push(Instruction::LocalGet(dst)); // dst addr
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I64Load(ma8.clone())); // reload source (tee was consumed by has_zero)
        ins.push(Instruction::I64Store(ma8.clone())); // 8-byte copy
        ins.push(Instruction::LocalGet(dst));
        ins.push(Instruction::I32Const(8));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(dst));
        ins.push(Instruction::LocalGet(str_len));
        ins.push(Instruction::I32Const(8));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(str_len));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(8));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(fast_str_loop);
        close!(); // no special char
        close!();
        close!(); // fast_str_loop, fast_str_block

        // Slow byte-by-byte path (for tail bytes or chunks with '"' / '\')
        open_loop!();
        let str_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(str_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(esc));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32And);
        open_if!();
        br_to!(str_block);
        close!();
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x5C));
        ins.push(Instruction::I32Eq);
        open_if!();
        ins.push(Instruction::LocalGet(esc));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Xor);
        ins.push(Instruction::LocalSet(esc));
        open_else!();
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(esc));
        close!();
        ins.push(Instruction::LocalGet(dst));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Store8(ma8.clone()));
        ins.push(Instruction::LocalGet(dst));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(dst));
        ins.push(Instruction::LocalGet(str_len));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(str_len));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(str_loop);
        close!();
        close!(); // str
                  // Store packed result: (str_len << 32) | dst_base
        ins.push(Instruction::I32Const(stdout_buf));
        ins.push(Instruction::LocalGet(cur_key));
        ins.push(Instruction::I32Const(slot_size));
        ins.push(Instruction::I32Mul);
        ins.push(Instruction::I32Add); // dst_base on stack as i32
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::LocalGet(str_len));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Or);
        // Stash packed value, compute addr, then store
        ins.push(Instruction::LocalSet(0)); // reuse param 0 (buf_packed, already consumed) as i64 temp
        {
            let result_base = stdout_buf + (n_keys as i32) * slot_size;
            ins.push(Instruction::I32Const(result_base));
            ins.push(Instruction::LocalGet(cur_key));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Mul);
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(0));
            ins.push(Instruction::I64Store(ma));
        }
        // found++; continue scan
        ins.push(Instruction::LocalGet(found));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(found));
        br_to!(scan_loop);
        close!(); // string branch

        // ── OBJECT/ARRAY: '{' or '[' ──
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        open_if!();
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(depth)); // reuse depth for brace tracking
        open_block!();
        let brk_block = ls.len() - 1;
        open_loop!();
        let brk_loop = ls.len() - 1;
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::LocalGet(json_len));
        ins.push(Instruction::I32GeS);
        open_if!();
        br_to!(brk_block);
        close!();
        ins.push(Instruction::LocalGet(json_ptr));
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma8.clone()));
        ins.push(Instruction::LocalSet(ch));
        ins.push(Instruction::LocalGet(dst));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Store8(ma8.clone()));
        ins.push(Instruction::LocalGet(dst));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(dst));
        ins.push(Instruction::LocalGet(str_len));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(str_len));
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(depth));
        close!();
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x7D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(ch));
        ins.push(Instruction::I32Const(0x5D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        open_if!();
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(depth));
        ins.push(Instruction::LocalGet(depth));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        open_if!();
        br_to!(brk_block);
        close!();
        close!();
        ins.push(Instruction::LocalGet(scan_i));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(scan_i));
        br_to!(brk_loop);
        close!();
        close!(); // brk
                  // Store packed result
        ins.push(Instruction::I32Const(stdout_buf));
        ins.push(Instruction::LocalGet(cur_key));
        ins.push(Instruction::I32Const(slot_size));
        ins.push(Instruction::I32Mul);
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::LocalGet(str_len));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Or);
        ins.push(Instruction::LocalSet(0)); // stash packed value
        {
            let result_base = stdout_buf + (n_keys as i32) * slot_size;
            ins.push(Instruction::I32Const(result_base));
            ins.push(Instruction::LocalGet(cur_key));
            ins.push(Instruction::I32Const(8));
            ins.push(Instruction::I32Mul);
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalGet(0));
            ins.push(Instruction::I64Store(ma));
        }
        ins.push(Instruction::LocalGet(found));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(found));
        br_to!(scan_loop);
        close!(); // object/array branch

        // ── RAW: number, null, bool, etc. ──
        {
            open_block!();
            let raw_block = ls.len() - 1;
            open_loop!();
            let raw_loop = ls.len() - 1;
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::LocalGet(json_len));
            ins.push(Instruction::I32GeS);
            open_if!();
            br_to!(raw_block);
            close!();
            ins.push(Instruction::LocalGet(json_ptr));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I32Load8U(ma8.clone()));
            ins.push(Instruction::LocalSet(ch));
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x2C));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x7D));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x5D));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x20));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Const(0x0A));
            ins.push(Instruction::I32Eq);
            ins.push(Instruction::I32Or);
            open_if!();
            br_to!(raw_block);
            close!();
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::LocalGet(ch));
            ins.push(Instruction::I32Store8(ma8.clone()));
            ins.push(Instruction::LocalGet(dst));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(dst));
            ins.push(Instruction::LocalGet(str_len));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(str_len));
            ins.push(Instruction::LocalGet(scan_i));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(scan_i));
            br_to!(raw_loop);
            close!();
            close!(); // raw
                      // Store packed result
            ins.push(Instruction::I32Const(stdout_buf));
            ins.push(Instruction::LocalGet(cur_key));
            ins.push(Instruction::I32Const(slot_size));
            ins.push(Instruction::I32Mul);
            ins.push(Instruction::I32Add);
            ins.push(Instruction::I64ExtendI32U);
            ins.push(Instruction::LocalGet(str_len));
            ins.push(Instruction::I64ExtendI32U);
            ins.push(Instruction::I64Const(32));
            ins.push(Instruction::I64Shl);
            ins.push(Instruction::I64Or);
            ins.push(Instruction::LocalSet(0));
            {
                let result_base = stdout_buf + (n_keys as i32) * slot_size;
                ins.push(Instruction::I32Const(result_base));
                ins.push(Instruction::LocalGet(cur_key));
                ins.push(Instruction::I32Const(8));
                ins.push(Instruction::I32Mul);
                ins.push(Instruction::I32Add);
                ins.push(Instruction::LocalGet(0));
                ins.push(Instruction::I64Store(ma));
            }
            ins.push(Instruction::LocalGet(found));
            ins.push(Instruction::I32Const(1));
            ins.push(Instruction::I32Add);
            ins.push(Instruction::LocalSet(found));
            br_to!(scan_loop);
        }

        close!();
        close!(); // scan_block, scan_loop

        // ── Build result array on heap ──
        // Layout: [count, val0_packed, val1_packed, ...]
        // Read from result slots at result_base
        let result_base = stdout_buf + (n_keys as i32) * slot_size;
        let slots = 1 + n_keys; // count + values
                                // Use runtime bump allocator
        ins.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::I64Load(ma));
        ins.push(Instruction::I32WrapI64);
        let arr_ptr_local = next_local;
        next_local += 1;
        ins.push(Instruction::LocalSet(arr_ptr_local));
        // Bump by slots * 8
        ins.push(Instruction::I64Const(RUNTIME_HEAP_PTR));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalGet(arr_ptr_local));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const((slots * 8) as i64));
        ins.push(Instruction::I64Add);
        ins.push(Instruction::I64Store(ma));
        // Store count
        ins.push(Instruction::LocalGet(arr_ptr_local));
        ins.push(Instruction::I64Const(n_keys as i64));
        ins.push(Instruction::I64Store(ma));
        // Store each value from result slot (tagged as string)
        for k in 0..n_keys {
            ins.push(Instruction::LocalGet(arr_ptr_local));
            ins.push(Instruction::I64ExtendI32U);
            ins.push(Instruction::I64Const(((k as i32 + 1) * 8) as i64));
            ins.push(Instruction::I64Add);
            ins.push(Instruction::I32WrapI64);
            // Load from result slot and tag as string: (packed << 3) | 5
            ins.push(Instruction::I32Const(result_base + (k as i32) * 8));
            ins.push(Instruction::I64Load(ma));
            ins.push(Instruction::I64Const(3));
            ins.push(Instruction::I64Shl);
            ins.push(Instruction::I64Const(5));
            ins.push(Instruction::I64Or);
            ins.push(Instruction::I64Store(ma));
        }
        // Return tagged array
        ins.push(Instruction::LocalGet(arr_ptr_local));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(6)); // TAG_ARRAY
        ins.push(Instruction::I64Or);

        let extra_i32 = next_local - (n_keys as u32 + 1) - n_i32_locals;
        let total_i32 = n_i32_locals + extra_i32;
        self.funcs.push(FuncDef {
            name: fname,
            param_count: (n_keys + 1) as usize,
            local_count: total_i32 as usize + 1,
            instrs: ins,
            local_entries: Some(vec![(total_i32 as u32, ValType::I32), (1u32, ValType::I64)]),
        });
        (self.funcs.len() - 1) as u32
    }

    pub(crate) fn json_get_from_buf(
        &mut self,
        key: &str,
        value_type: &str,
        buf: i64,
        buf_len_setup: &mut Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let func_idx = self.ensure_json_get_func();
        let keys: Vec<&str> = key.split('.').collect();
        let mut v: Vec<Instruction<'static>> = Vec::new();
        let tmp = self.local_idx("__jg_buf_tmp");

        // First iteration: use provided buf and buf_len_setup
        // Stack after each __json_get call: (len << 32 | ptr) as i64
        for (ki, subkey) in keys.iter().enumerate() {
            let mut pattern = vec![b'"'];
            pattern.extend(subkey.as_bytes());
            pattern.extend_from_slice(b"\":");
            let pat_off = self.alloc_data(&pattern) as i64;
            let pat_len = pattern.len() as i64;
            let pat_packed = (pat_off as u64) | ((pat_len as u64) << 32);

            if ki == 0 {
                v.extend(buf_len_setup.iter().cloned());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(buf));
                v.push(Instruction::I64Or);
            } else {
                // Use previous result as new buffer
                // Stack has (len << 32 | ptr) from previous call
                // Check for 0 first
                v.push(Instruction::LocalSet(tmp));
                v.push(Instruction::LocalGet(tmp));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Return);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(tmp));
            }
            v.push(Instruction::I64Const(pat_packed as i64));
            v.push(Instruction::Call(crate::wasm_emit::USER_BASE | func_idx));
        }

        // Result on stack: (len << 32 | ptr) as i64
        if value_type == "int" {
            // Parse ASCII digits at ptr into i64
            let i_local = self.local_idx("__jg_int_i");
            let result_local = self.local_idx("__jg_int_result");
            let ma1 = wasm_encoder::MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            };
            v.push(Instruction::LocalSet(tmp)); // save packed result
                                                // Check if result is 0 (not found)
            v.push(Instruction::LocalGet(tmp));
            v.push(Instruction::I64Const(0));
            v.push(Instruction::I64Eq);
            v.push(Instruction::If(BlockType::Empty));
            v.push(Instruction::I64Const(0));
            v.push(Instruction::Return);
            v.push(Instruction::End);
            v.push(Instruction::I64Const(0));
            v.push(Instruction::LocalSet(result_local));
            v.push(Instruction::I64Const(0));
            v.push(Instruction::LocalSet(i_local));
            v.push(Instruction::Block(BlockType::Empty));
            v.push(Instruction::Loop(BlockType::Empty));
            v.push(Instruction::LocalGet(i_local));
            v.push(Instruction::LocalGet(tmp));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64ShrU);
            v.push(Instruction::I64GeU);
            v.push(Instruction::BrIf(1));
            v.push(Instruction::LocalGet(tmp));
            v.push(Instruction::I64Const(0xFFFFFFFF));
            v.push(Instruction::I64And);
            v.push(Instruction::LocalGet(i_local));
            v.push(Instruction::I64Add);
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I32Load8U(ma1.clone()));
            v.push(Instruction::I32Const(48)); // '0'
            v.push(Instruction::I32Sub);
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::LocalGet(result_local));
            v.push(Instruction::I64Const(10));
            v.push(Instruction::I64Mul);
            v.push(Instruction::I64Add);
            v.push(Instruction::LocalSet(result_local));
            v.push(Instruction::LocalGet(i_local));
            v.push(Instruction::I64Const(1));
            v.push(Instruction::I64Add);
            v.push(Instruction::LocalSet(i_local));
            v.push(Instruction::Br(0));
            v.push(Instruction::End); // loop
            v.push(Instruction::End); // block
            v.push(Instruction::LocalGet(result_local));
        }
        // For "str" or "float", return (len << 32 | ptr) as-is
        Ok(v)
    }

    pub(crate) fn json_get_int(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
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
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(7)); // input(0)
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(1)); // register_len(0)
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(0)); // read_register(0, ib)

        // pos = 0, depth = 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(pos));
        let depth = self.local_idx("__js_depth");
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(depth));

        // Scan loop (block/loop)
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        // if pos + pat_len > ilen: break
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);

        // Track brace depth: load byte at INPUT_BUF+pos
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        let scan_byte = self.local_idx("__js_sb");
        v.push(Instruction::LocalSet(scan_byte));
        // if byte == '{': depth++
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        // if byte == '}': depth--
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I64Const(0x7D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);

        // Only try to match at depth == 1 (top level)
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        // depth != 1, skip comparison, just advance pos
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); // back to outer LOOP (skip label 0 = this if)
        v.push(Instruction::End);

        // Assume match (mi=1), compare bytes
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // Load input[ib+pos+j]
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        // Load pattern[pat_off+j]
        v.push(Instruction::I64Const(pat_off as i64));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        // Compare
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty)); // match continues
        v.push(Instruction::Else); // mismatch
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End); // inner loop/block

        // If mi==1: check preceding byte boundary
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        // pos > 0 → check preceding byte
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        // Load byte at INPUT_BUF[pos-1]
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(prev_byte));
        // Valid if prev_byte in {0x7B '{', 0x2C ',', 0x20 ' ', 0x09 '\t', 0x0A '\n'}
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x2C));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x20));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x09));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x0A));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        // If NOT valid boundary, reset mi
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End); // end pos > 0 check
        v.push(Instruction::End); // end mi==1 check
                                  // Now check mi again — if still 1, break outer
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // pos++
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End); // outer loop/block

        // Wrap parse section: if pos >= ilen (key not found), skip parsing; res stays 0
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty)); // if pos < ilen → parse

        // pos at match. Value at pos + pat_len
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));

        // Skip whitespace (space, tab, LF, CR)
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(ws_byte));
        // byte == ' ' || byte == '\t' || byte == '\n' || byte == '\r'
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x20));
        v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x09));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x0A));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x0D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Skip quote if present
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Check negative
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x2D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Parse digits
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(dg));
        // if dg < 0x30: break
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // if dg > 0x39: break
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // res = res*10 + (dg - 0x30)
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Apply negative → store to res
        v.push(Instruction::LocalGet(ng));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::End); // end if neg
        v.push(Instruction::End); // end if pos < ilen (parse section)
                                  // Return res (0 if key not found, parsed value otherwise)
        v.push(Instruction::LocalGet(res));
        Ok(v)
    }

    /// Emit WASM to read input JSON, scan for "key": pattern, parse decimal into u128 at offset.
    /// Returns offset (i64). u128 stored as lo 8 bytes at offset, hi 8 bytes at offset+8.
    pub(crate) fn json_get_u128(
        &mut self,
        key: &str,
        offset_expr: Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
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
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let ib = INPUT_BUF;
        let mut v = offset_expr;

        // Store offset to a temp local
        let off_local = self.local_idx("__ju_offset");
        v.push(Instruction::LocalSet(off_local));

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(1));
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(0));

        // pos = 0, depth = 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(depth));

        // ── Scan loop ──
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);

        // Track brace depth
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(scan_byte));
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I64Const(0x7D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);

        // Only match at depth == 1
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);

        // Compare bytes
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(pat_off as i64));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Check preceding byte boundary
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(prev_byte));
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x7B));
        v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x2C));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x20));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x09));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I64Const(0x0A));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // ── Parse section ──
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));

        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));

        // Skip whitespace
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(ws_byte));
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x20));
        v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x09));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x0A));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I64Const(0x0D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Skip quote if present
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Init lo = 0, hi = 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(lo));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(hi));

        // ── Digit parse loop: hi:lo = hi:lo * 10 + digit ──
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(dg));
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);

        // digit = dg - 0x30
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64Sub);
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
        v.push(Instruction::LocalGet(lo));
        v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(lo_lo));
        // lo_hi = lo >> 32
        v.push(Instruction::LocalGet(lo));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(lo_hi));
        // p0 = lo_lo * 10 + digit
        v.push(Instruction::LocalGet(lo_lo));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(p0));
        // r0 = p0 & 0xFFFFFFFF
        v.push(Instruction::LocalGet(p0));
        v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(r0));
        // c0 = p0 >> 32
        v.push(Instruction::LocalGet(p0));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(c0));
        // p1 = lo_hi * 10 + c0
        v.push(Instruction::LocalGet(lo_hi));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(c0));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(p1));
        // r1 = p1 & 0xFFFFFFFF
        v.push(Instruction::LocalGet(p1));
        v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(r1));
        // c1 = p1 >> 32
        v.push(Instruction::LocalGet(p1));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(c1));
        // lo = r0 | (r1 << 32)
        v.push(Instruction::LocalGet(r1));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(r0));
        v.push(Instruction::I64Or);
        v.push(Instruction::LocalSet(lo));
        // hi = hi * 10 + c1
        v.push(Instruction::LocalGet(hi));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(c1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(hi));

        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // ── Write lo/hi to memory at offset ──
        let ma64 = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        v.push(Instruction::LocalGet(off_local));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(lo));
        v.push(Instruction::I64Store(ma64.clone()));
        v.push(Instruction::LocalGet(off_local));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(hi));
        v.push(Instruction::I64Store(ma64));

        v.push(Instruction::End); // end if pos < ilen

        v.push(Instruction::LocalGet(off_local));
        Ok(v)
    }

    /// Emit WASM to read input JSON, scan for "key": "value", return packed string (ptr|len<<32).

    pub(crate) fn json_get_str(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
        let mut pattern = vec![b'"'];
        pattern.extend(key.as_bytes());
        pattern.extend_from_slice(b"\":");
        let pat_off = self.alloc_data(&pattern) as i32;
        let pat_len = pattern.len() as i32;
        let pos = self.local_idx_i32("__jss_pos");
        let ilen = self.local_idx_i32("__jss_ilen");
        let mi = self.local_idx_i32("__jss_mi");
        let jj = self.local_idx_i32("__jss_j");
        let slen = self.local_idx_i32("__jss_slen");
        let prev_byte = self.local_idx_i32("__jss_prev");
        let ws_byte = self.local_idx_i32("__jss_ws_byte");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let ib = INPUT_BUF as i32;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(1));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(ib as i64));
        v.push(Self::host_call(0));

        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(pos));
        let depth = self.local_idx_i32("__jss_depth");
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(depth));

        // Scan loop
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(pat_len));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I32GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);

        // Track brace depth
        let scan_byte = self.local_idx_i32("__jss_sb");
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::LocalSet(scan_byte));
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I32Const(0x7B));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(scan_byte));
        v.push(Instruction::I32Const(0x7D));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Sub);
        v.push(Instruction::LocalSet(depth));
        v.push(Instruction::End);
        // Only match at depth == 1
        v.push(Instruction::LocalGet(depth));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1)); // back to outer LOOP (skip label 0 = this if)
        v.push(Instruction::End);

        v.push(Instruction::I32Const(1));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I32Const(pat_len));
        v.push(Instruction::I32GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Const(pat_off));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // If mi==1: check preceding byte boundary
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(0));
        v.push(Instruction::I32GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Sub);
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::LocalSet(prev_byte));
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I32Const(0x7B));
        v.push(Instruction::I32Eq);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I32Const(0x2C));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I32Const(0x20));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I32Const(0x09));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(prev_byte));
        v.push(Instruction::I32Const(0x0A));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::I32Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // If pos >= ilen, key not found — return 0 (packed as 0)
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I32LtS);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));

        // Value at pos + pat_len
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(pat_len));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(pos));

        // Skip whitespace (space, tab, LF, CR)
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I32GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::LocalSet(ws_byte));
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I32Const(0x20));
        v.push(Instruction::I32Eq);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I32Const(0x09));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I32Const(0x0A));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(ws_byte));
        v.push(Instruction::I32Const(0x0D));
        v.push(Instruction::I32Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Skip opening quote (the quote before the string value)
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(pos));

        // Measure string length (scan until closing quote)
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I32GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Const(0x22));
        v.push(Instruction::I32Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Copy to compile-time heap area above all static buffers
        let heap_dst = self.heap_bump(256);
        v.push(Instruction::I32Const(heap_dst as i32)); // dst
        v.push(Instruction::I32Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I32Add); // src
        v.push(Instruction::LocalGet(slen)); // len
        v.push(Instruction::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
        // Return packed: (slen << 32) | heap_dst
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(heap_dst as i64));
        v.push(Instruction::I64Or);
        v.push(Instruction::Else);
        // Key not found: return 0
        v.push(Instruction::I64Const(0));
        v.push(Instruction::End); // end if pos < ilen
        Ok(v)
    }

    pub(crate) fn json_return_int(
        &mut self,
        val_expr: Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(25);
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
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
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // addr = ib + ci
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        // val = load from prefix_off + ci
        v.push(Instruction::I64Const(prefix_off as i64));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Write integer digits backwards from ib + prefix_len + 20
        v.extend(val_expr);
        v.push(Instruction::LocalSet(abs_val));

        // Check negative
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64LtS);
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(is_neg));
        v.push(Instruction::LocalGet(is_neg));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Sub);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::End);
        v.push(Instruction::LocalSet(abs_val));

        let digit_end = prefix_len + 21;
        v.push(Instruction::I64Const(digit_end));
        v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(dc));

        // Handle 0
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(dc));
        v.push(Instruction::Else);

        // Digit loop
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64RemS);
        v.push(Instruction::LocalSet(td));
        v.push(Instruction::LocalGet(abs_val));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64DivS);
        v.push(Instruction::LocalSet(abs_val));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(td));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(dc));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::End); // end else

        // Add minus sign
        v.push(Instruction::LocalGet(is_neg));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalSet(ptr));
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x2D));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(dc));
        v.push(Instruction::End);

        // Shift digits to position prefix_len
        let si = self.local_idx("__jri_si");
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(si));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(si));
        v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // Push dst addr first (deeper), then load byte (top) for I32Store8
        v.push(Instruction::I64Const(ib + prefix_len));
        v.push(Instruction::LocalGet(si));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        // Stack: [dst_addr]
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(si));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        // Stack: [dst_addr, loaded_byte] — I32Store8 pops value=byte, addr=dst_addr
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(si));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(si));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Write '}'
        v.push(Instruction::I64Const(ib + prefix_len));
        v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(b'}' as i64));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        // total_len = prefix_len + dc + 1
        let tl = self.local_idx("__jri_tl");
        v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::LocalGet(dc));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(tl));

        // value_return(total_len, ib)
        v.push(Instruction::LocalGet(tl));
        v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(25));

        v.push(Instruction::I64Const(1));
        v.push(Instruction::GlobalSet(RETURN_FLAG));
        v.push(Instruction::I64Const(0));
        Ok(v)
    }

    pub(crate) fn json_return_str(
        &mut self,
        packed_expr: Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(25);
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
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
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(prefix_off as i64));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Unpack string (untag first — expr() returns tagged values)
        v.extend(packed_expr);
        v.push(Instruction::LocalSet(packed));
        v.push(Instruction::LocalGet(packed));
        v.extend(self.emit_untag()); // >> 3 to get packed (len<<32|ptr)
        v.push(Instruction::LocalSet(packed)); // store untagged packed
        v.push(Instruction::LocalGet(packed));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(str_ptr));
        v.push(Instruction::LocalGet(packed));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(str_len));

        // Copy string bytes to ib + prefix_len
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        // dst
        v.push(Instruction::I64Const(ib + prefix_len));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        // src
        v.push(Instruction::LocalGet(str_ptr));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(ci));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Write '"}'
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0x22));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(prefix_len + 1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(b'}' as i64));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Store8(ma8.clone()));

        // value_return(prefix_len + str_len + 2, ib)
        v.push(Instruction::I64Const(prefix_len));
        v.push(Instruction::LocalGet(str_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(2));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(25));

        v.push(Instruction::I64Const(1));
        v.push(Instruction::GlobalSet(RETURN_FLAG));
        v.push(Instruction::I64Const(0));
        Ok(v)
    }

    pub(crate) fn json_get_auto(&mut self, key: &str) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
        let mut pattern = vec![b'"'];
        pattern.extend(key.as_bytes());
        pattern.extend_from_slice(b"\":");
        let pat_off = self.alloc_data(&pattern);
        let pat_len = pattern.len() as i64;

        let pos = self.local_idx("__ja_pos");
        let ilen = self.local_idx("__ja_ilen");
        let mi = self.local_idx("__ja_mi");
        let jj = self.local_idx("__ja_j");
        let first = self.local_idx("__ja_first");
        let res = self.local_idx("__ja_res");
        let ng = self.local_idx("__ja_ng");
        let dg = self.local_idx("__ja_dg");
        let slen = self.local_idx("__ja_slen");

        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let ib = INPUT_BUF;
        let mut v = Vec::new();

        // Read input to INPUT_BUF
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(7));
        v.push(Instruction::I64Const(0));
        v.push(Self::host_call(1));
        v.push(Instruction::LocalSet(ilen));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::I64Const(ib));
        v.push(Self::host_call(0));

        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(pos));

        // Scan for "key": pattern
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);

        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(pat_off as i64));
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Else);
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(mi));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(jj));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(jj));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);

        v.push(Instruction::LocalGet(mi));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Advance past pattern
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(pat_len));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));

        // Skip whitespace (space, \n, \r, \t)
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(first));
        // break if not ws
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x20));
        v.push(Instruction::I64Eq);
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x0A));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x0D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x09));
        v.push(Instruction::I64Eq);
        v.push(Instruction::I32Or);
        v.push(Instruction::BrIf(0));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // Re-read first non-ws byte
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(first));

        // NULL: 'n' -> -1
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x6E));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(-1));
        v.push(Instruction::Return);
        v.push(Instruction::End);

        // BOOL false: 'f' -> -2
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x66));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(-2));
        v.push(Instruction::Return);
        v.push(Instruction::End);

        // BOOL true: 't' -> -3
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x74));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(-3));
        v.push(Instruction::Return);
        v.push(Instruction::End);

        // STRING: '"' -> packed (len << 32) | ptr
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x22));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(0x22));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(slen));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(slen));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Or);
        v.push(Instruction::Return);
        v.push(Instruction::End);

        // NUMBER: parse int (digit or minus)
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(first));
        v.push(Instruction::I64Const(0x2D));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(ng));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::End);

        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::LocalGet(ilen));
        v.push(Instruction::I64GeS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::LocalSet(dg));
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64LtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x39));
        v.push(Instruction::I64GtS);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::I64Const(10));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalGet(dg));
        v.push(Instruction::I64Const(0x30));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::LocalGet(pos));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pos));
        v.push(Instruction::Br(1));
        v.push(Instruction::End);
        v.push(Instruction::End);

        v.push(Instruction::LocalGet(ng));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::I64Sub);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::End);
        Ok(v)
    }

    pub(crate) fn ensure_json_array_get_func(&mut self) -> u32 {
        if let Some(idx) = self.funcs.iter().position(|f| f.name == "__json_array_get") {
            return idx as u32;
        }
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        // Params: 0=i64 packed(len<<32|ptr), 1=i64 index
        // 13 i32 locals (indices 2..14):
        //   2=scan_i 3=ch 4=depth 5=in_str 6=esc 7=elem_i 8=target_idx
        //   9=val_start 10=val_end 11=str_len 12=dst 13=arr_len 14=arr_ptr
        let mut ins: Vec<Instruction<'static>> = Vec::new();

        // Unpack param 0 → arr_len, arr_ptr
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(13)); // arr_len
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(14)); // arr_ptr
                                             // Unpack param 1 → target_idx (i64 → i32)
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(8)); // target_idx

        // Phase 1: scan to find first '['
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(7)); // elem_i=0
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(2)); // scan_i=0
        ins.push(Instruction::Block(BlockType::Empty)); // @B1
        ins.push(Instruction::Loop(BlockType::Empty)); // @L2
                                                       // scan_i >= arr_len → not found, return nil
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalGet(13));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty)); // @I3
        ins.push(Instruction::I64Const(4));
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        // ch = arr[scan_i]
        ins.push(Instruction::LocalGet(14));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(3));
        // ch == '[' ? → advance and exit
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I3
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(2)); // exit L2 + B1
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0)); // back to L2
        ins.push(Instruction::End); // L2
        ins.push(Instruction::End); // B1

        // Phase 3: element scanning loop
        // Uses a flat approach: for each element, check if elem_i==target_idx
        // If yes, extract and return. If no, skip to next comma/bracket.
        ins.push(Instruction::Block(BlockType::Empty)); // @B1 elem_block
        ins.push(Instruction::Loop(BlockType::Empty)); // @L2 elem_loop

        // Check if this element is the target
        ins.push(Instruction::LocalGet(7));
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I3 IS target
                                                     // Record start position, init extract state
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalSet(9)); // val_start
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(4)); // depth
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(5)); // in_str
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(6)); // esc

        // Extract loop: scan until element ends
        ins.push(Instruction::Block(BlockType::Empty)); // @B4 extract_block
        ins.push(Instruction::Loop(BlockType::Empty)); // @L5 extract_loop
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalGet(13));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalSet(10)); // val_end
        ins.push(Instruction::Br(2)); // exit L5+B4
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(14));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(3)); // ch
                                            // escape
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(1)); // back to L5
        ins.push(Instruction::End);
        // in_str
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I7
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I7
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(5));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(1)); // back to L5
        ins.push(Instruction::End);
        // opening quote
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(5));
        ins.push(Instruction::End);
        // { [ → depth++
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        // }
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x7D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I7
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalSet(10)); // val_end
        ins.push(Instruction::Br(3)); // exit I7+I6+L5+B4
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        // ]
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty)); // @I7
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalSet(10)); // val_end
        ins.push(Instruction::Br(3)); // exit I7+I6+L5+B4
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        // , at depth 0
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x2C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32And);
        ins.push(Instruction::If(BlockType::Empty)); // @I6
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalSet(10)); // val_end
        ins.push(Instruction::Br(2)); // exit L5+B4
        ins.push(Instruction::End);
        // default: advance
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0)); // back to L5
        ins.push(Instruction::End); // L5
        ins.push(Instruction::End); // B4

        // Copy val_start..val_end to json_get output buf via memory.copy (byte-by-byte loop
        // writes null inside block+loop with WASI P1 runtimes — wasmtime bug)
        let stdout_buf: i32 = 204800; // 200KB — must not collide with SENTINEL_BUF (65536)
        ins.push(Instruction::LocalGet(10));
        ins.push(Instruction::LocalGet(9));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(11)); // str_len
        ins.push(Instruction::I32Const(stdout_buf));
        ins.push(Instruction::LocalGet(14));
        ins.push(Instruction::LocalGet(9));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(11));
        ins.push(Instruction::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });

        // Return tagged: ((str_len << 32 | stdout_buf) << 3) | 5
        ins.push(Instruction::LocalGet(11));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(stdout_buf as i64));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);

        ins.push(Instruction::End); // end IS target if

        // === NOT target: skip element ===
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(4)); // depth
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(5)); // in_str
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(6)); // esc
        ins.push(Instruction::Block(BlockType::Empty)); // skip_block
        ins.push(Instruction::Loop(BlockType::Empty)); // skip_loop
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalGet(13));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::Br(2)); // exit skip_loop+skip_block
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(14));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(3));
        // escape
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(1)); // back to skip_loop
        ins.push(Instruction::End);
        // in_str
        ins.push(Instruction::LocalGet(5));
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(5));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(1)); // back to skip_loop
        ins.push(Instruction::End);
        // structural chars
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x22));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(5));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x7B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32Or);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x7D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(2)); // exit skip_loop+skip_block
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x5D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(2)); // exit skip_loop+skip_block
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::LocalSet(4));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x2C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::I32And);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(2)); // exit skip_loop+skip_block
        ins.push(Instruction::End);
        // default
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0)); // back to skip_loop
        ins.push(Instruction::End); // skip_loop
        ins.push(Instruction::End); // skip_block

        // elem_i++, skip whitespace, continue elem_loop
        ins.push(Instruction::LocalGet(7));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(7));
        // Skip whitespace inline (no block/loop, just a flat if-chain)
        ins.push(Instruction::Block(BlockType::Empty)); // ws_block
        ins.push(Instruction::Loop(BlockType::Empty)); // ws_loop
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::LocalGet(13));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::Br(1)); // exit ws_loop → ws_block end
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(14));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(3));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x20));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0)); // ws_loop
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x09));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x0A));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32Const(0x0D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::Br(1)); // exit ws_loop → ws_block end
        ins.push(Instruction::End); // ws_loop
        ins.push(Instruction::End); // ws_block

        ins.push(Instruction::Br(0)); // back to elem_loop
        ins.push(Instruction::End); // L2 elem_loop
        ins.push(Instruction::End); // B1 elem_block

        // Fallback: not found
        ins.push(Instruction::I64Const(4));
        ins.push(Instruction::Return);

        self.funcs.push(FuncDef {
            name: "__json_array_get".to_string(),
            param_count: 2,
            local_count: 13,
            instrs: ins,
            local_entries: Some(vec![(13u32, ValType::I32)]),
        });
        (self.funcs.len() - 1) as u32
    }

    pub(crate) fn ensure_json_bytes_to_str_func(&mut self) -> u32 {
        if let Some(idx) = self
            .funcs
            .iter()
            .position(|f| f.name == "__json_bytes_to_str")
        {
            return idx as u32;
        }
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        // Use offset 131072 (128KB) — safely above:
        // - BORSH_BUF (36864 + 4096 = 40960)
        // - Above OL_RET_AREA_BASE+ to avoid HTTP response buffer collision
        let decode_buf: i32 = crate::wasi_http::OL_RET_AREA_BASE + 2048; // SCRATCH + 2048
                                                                         // Param 0 = i64 packed (len<<32|ptr)
                                                                         // Locals 1..8 = i32: scan_i=1 ch=2 arr_len=3 arr_ptr=4 byte_val=5 out_i=6 in_num=7 num_val=8
        let mut ins: Vec<Instruction<'static>> = Vec::new();

        // Unpack param 0
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64ShrU);
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(3)); // arr_len
        ins.push(Instruction::LocalGet(0));
        ins.push(Instruction::I32WrapI64);
        ins.push(Instruction::LocalSet(4)); // arr_ptr

        // Find '['
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Block(BlockType::Empty));
        ins.push(Instruction::Loop(BlockType::Empty));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I64Const(4));
        ins.push(Instruction::Return);
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(2));
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x5B));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Br(2));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Br(0));
        ins.push(Instruction::End);
        ins.push(Instruction::End);

        // Parse [91,123,...] byte values
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(6)); // out_i
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(7)); // in_num
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(8)); // num_val

        ins.push(Instruction::Block(BlockType::Empty)); // parse_block
        ins.push(Instruction::Loop(BlockType::Empty)); // parse_loop
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::LocalGet(3));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::Br(2));
        ins.push(Instruction::End);
        ins.push(Instruction::LocalGet(4));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::I32Load8U(ma.clone()));
        ins.push(Instruction::LocalSet(2));
        // ']' → done
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x5D));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::Br(2));
        ins.push(Instruction::End);
        // digit → accumulate
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x30));
        ins.push(Instruction::I32GeS);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x39));
        ins.push(Instruction::I32LeS);
        ins.push(Instruction::I32And);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::LocalSet(7));
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Const(10));
        ins.push(Instruction::I32Mul);
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x30));
        ins.push(Instruction::I32Sub);
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(8));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Br(1)); // Continue parse_loop (targets Loop at depth 1)
        ins.push(Instruction::End);
        // ',' → flush byte
        ins.push(Instruction::LocalGet(2));
        ins.push(Instruction::I32Const(0x2C));
        ins.push(Instruction::I32Eq);
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(decode_buf));
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Store8(ma.clone()));
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(8));
        ins.push(Instruction::I32Const(0));
        ins.push(Instruction::LocalSet(7));
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Br(1)); // Continue parse_loop (targets Loop at depth 1)
        ins.push(Instruction::End);
        // skip other (space, etc)
        ins.push(Instruction::LocalGet(1));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(1));
        ins.push(Instruction::Br(0)); // Continue parse_loop (targets Loop at depth 0)
        ins.push(Instruction::End); // parse_loop
        ins.push(Instruction::End); // parse_block

        // Flush final byte if in_num
        ins.push(Instruction::LocalGet(7));
        ins.push(Instruction::If(BlockType::Empty));
        ins.push(Instruction::I32Const(decode_buf));
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalGet(8));
        ins.push(Instruction::I32Store8(ma.clone()));
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I32Const(1));
        ins.push(Instruction::I32Add);
        ins.push(Instruction::LocalSet(6));
        ins.push(Instruction::End);

        // Return tagged
        ins.push(Instruction::LocalGet(6));
        ins.push(Instruction::I64ExtendI32U);
        ins.push(Instruction::I64Const(32));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(decode_buf as i64));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::I64Const(3));
        ins.push(Instruction::I64Shl);
        ins.push(Instruction::I64Const(5));
        ins.push(Instruction::I64Or);
        ins.push(Instruction::Return);

        self.funcs.push(FuncDef {
            name: "__json_bytes_to_str".to_string(),
            param_count: 1,
            local_count: 8,
            instrs: ins,
            local_entries: Some(vec![(8u32, ValType::I32)]),
        });
        (self.funcs.len() - 1) as u32
    }
}
