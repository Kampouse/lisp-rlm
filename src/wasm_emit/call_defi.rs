use super::*;

impl WasmEmitter {
    pub(crate) fn call_defi(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "tick_to_price64" => {
                let addr_expr = self.expr(&a[0])?;
                let tick = self.expr(&a[1])?;
                let addr_i = self.local_idx("__tp64_a");
                let t_i = self.local_idx("__tp64_t");
                let neg_i = self.local_idx("__tp64_neg");
                let r_i = self.local_idx("__tp64_r");
                let b_i = self.local_idx("__tp64_b");
                let mut v = Vec::new();
                v.extend(addr_expr); v.push(Instruction::LocalSet(addr_i));
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // result = 1.0 in Q32.32 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Binary exponentiation loop (same proven Q32.32 mul with 16-bit split)
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // Q32.32 mul: r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                // Convert Q32.32 → Q64.64: shift left by 32
                // Store lo = (r << 32) at addr
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // Store hi = (r >> 32) at addr+8
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "sqrt" => {
                let x = self.expr(&a[0])?;
                let x_i = self.local_idx("__sq_x");
                let r_i = self.local_idx("__sq_r");
                let prev_i = self.local_idx("__sq_p");
                let mut v = Vec::new();
                v.extend(x); v.push(Instruction::LocalSet(x_i));
                // if x == 0: return 0
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::I64Eqz); // → i32
                v.push(Instruction::I32Eqz); // invert: x != 0 → enter then branch
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // Initial guess: x >> 1 (rough sqrt)
                // Better: r = 1 << ((64 - clz(x)) / 2)
                // Simple: r = x, iterate r = (r + x/r) / 2
                v.push(Instruction::LocalGet(x_i)); v.push(Instruction::LocalSet(r_i));
                // Loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::LocalSet(prev_i));
                // r = (r + x/r) / 2
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(x_i));
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::I64DivU); // x / r
                v.push(Instruction::I64Add); // r + x/r
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); // / 2
                v.push(Instruction::LocalSet(r_i));
                // if r >= prev: converged, break
                v.push(Instruction::LocalGet(r_i));
                v.push(Instruction::LocalGet(prev_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Br(2)); // exit outer block
                v.push(Instruction::End);
                v.push(Instruction::Br(0)); // loop
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                v.push(Instruction::LocalGet(r_i)); // return prev (last decreasing value)
                // Actually if r >= prev, prev is the answer (converged from above)
                // But we want the one that stopped decreasing
                v.pop(); // remove the LocalGet r_i
                v.push(Instruction::LocalGet(prev_i)); // prev was last decreasing
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0)); // x == 0 case
                v.push(Instruction::End);
                Ok(v)
            }
            "tick_to_price" => {
                // Binary exponentiation: 1.0001^tick in Q32.32
                let tick = self.expr(&a[0])?;
                let t_i = self.local_idx("__ttp_t");
                let r_i = self.local_idx("__ttp_r");
                let b_i = self.local_idx("__ttp_b");
                let neg_i = self.local_idx("__ttp_neg");
                let _c_i = self.local_idx("__ttp_c");
                let mut v = Vec::new();
                v.extend(tick); v.push(Instruction::LocalSet(t_i));
                // Handle negative
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64LtS); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(neg_i));
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(-1i64)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::End);
                // result = 1.0 = 1 << 32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl); v.push(Instruction::LocalSet(r_i));
                // base = 1.0001 in Q32.32 = 0x100068DB8
                v.push(Instruction::I64Const(0x100068DB8)); v.push(Instruction::LocalSet(b_i));
                // Loop: while tick > 0
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(0)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty)); v.push(Instruction::Br(2)); v.push(Instruction::End);
                // if tick & 1: r *= b (Q32.32 mul with 16-bit split)
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64And);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // r = (r_hi * b_hi) + ((r_hi * b_lo) >> 16) + ((r_lo * b_hi) >> 16)
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // r_hi * b_hi
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End); // if
                // b *= b (Q32.32 square)
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(b_i));
                // tick >>= 1
                v.push(Instruction::LocalGet(t_i)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(t_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // Invert if negative: r = (1<<48) / r << ... actually just (1<<32) * (1<<16) / r
                v.push(Instruction::LocalGet(neg_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::If(BlockType::Empty));
                // 1/r ≈ (1 << 48) / r, then >> 16 to get back to Q32.32
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(48)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(r_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r_i));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(r_i));
                Ok(v)
            }
            "price_to_tick" => {
                let p = self.expr(&a[0])?;
                let p_i = self.local_idx("__ptp_p");
                let mut v = Vec::new();
                v.extend(p); v.push(Instruction::LocalSet(p_i));
                // First order: tick ≈ (p - 1.0) / log(1.0001) ≈ (p - 1<<64) * 10000
                // More precisely: (p - (1<<64)) >> 64 * 10000 gives integer approximation
                v.push(Instruction::LocalGet(p_i));
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(64)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU); // to integer
                v.push(Instruction::I64Const(10000));
                v.push(Instruction::I64Mul);
                Ok(v)
            }
            "liq_amount0" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la0_a"); let spb_i = self.local_idx("__la0_b"); let liq_i = self.local_idx("__la0_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // numerator = liq * (spb - spa) — Q64.64 mul
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(liq_i)); // reuse as numerator
                // denominator = spa * spb — Q64.64 mul
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                // result = numerator / denominator
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64DivU);
                Ok(v)
            }
            "liq_amount1" => {
                let spa = self.expr(&a[0])?; let spb = self.expr(&a[1])?; let liq = self.expr(&a[2])?;
                let spa_i = self.local_idx("__la1_a"); let spb_i = self.local_idx("__la1_b"); let liq_i = self.local_idx("__la1_l");
                let mut v = Vec::new();
                v.extend(spa); v.push(Instruction::LocalSet(spa_i));
                v.extend(spb); v.push(Instruction::LocalSet(spb_i));
                v.extend(liq); v.push(Instruction::LocalSet(liq_i));
                // liq * (spb - spa) — Q64.64 multiply
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(liq_i)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(spb_i)); v.push(Instruction::LocalGet(spa_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(64)); v.push(Instruction::I64ShrU);
                Ok(v)
            }
            "liq_amount0_64" => {
                // (liq_amount0_64 dst spa_addr spb_addr liq_addr)
                // amount0 = L * (sqrtPb - sqrtPa) / (sqrtPa * sqrtPb)
                // All Q64.64 memory. Uses high-word arithmetic for CLMM (prices ≈ 1.0)
                let dst = self.expr(&a[0])?;
                let spa_a = self.expr(&a[1])?;
                let spb_a = self.expr(&a[2])?;
                let liq_a = self.expr(&a[3])?;
                let dst_i = self.local_idx("__la0_d");
                let spa_lo = self.local_idx("__la0_sl");
                let spa_hi = self.local_idx("__la0_sh");
                let spb_lo = self.local_idx("__la0_bl");
                let spb_hi = self.local_idx("__la0_bh");
                let liq_hi = self.local_idx("__la0_lh");
                let diff_lo = self.local_idx("__la0_dl");
                let diff_hi = self.local_idx("__la0_dh");
                let num_hi = self.local_idx("__la0_nh");
                let den_hi = self.local_idx("__la0_dnh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                // Load all values upfront into locals
                v.extend(spa_a); v.push(Instruction::LocalSet(spa_lo)); // spa addr
                v.extend(spb_a); v.push(Instruction::LocalSet(spb_lo)); // spb addr
                v.extend(liq_a); v.push(Instruction::LocalSet(liq_hi)); // liq addr
                // Load spa Q64.64
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(diff_lo)); // spa low temporarily
                v.push(Instruction::LocalGet(spa_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                v.push(Instruction::LocalGet(diff_lo)); v.push(Instruction::LocalSet(spa_lo)); // proper spa_lo
                // Load spb Q64.64
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_lo)); // spb low
                v.push(Instruction::LocalGet(spb_lo)); v.push(Instruction::I32WrapI64); // need spb addr for hi
                // Wait, spb_lo is now the spb value, not addr. Need separate addr local.
                // Let me restructure with addr locals
                v.clear();
                // Redo with proper addr locals
                let dst2 = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la0_as");
                let addr_spb = self.local_idx("__la0_ab");
                let addr_liq = self.local_idx("__la0_al");
                v.extend(dst2); v.push(Instruction::LocalSet(dst_i));
                // Store addresses in locals
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load spa
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_hi));
                // Load spb
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_hi));
                // Load liq
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_hi));
                // diff_hi = spb_hi - spa_hi
                v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff_hi));
                // numerator = liq_hi * diff_hi
                v.push(Instruction::LocalGet(liq_hi)); v.push(Instruction::LocalGet(diff_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(num_hi));
                // denominator = spa_hi * spb_hi (both ≈ 1, so ≈ 1)
                v.push(Instruction::LocalGet(spa_hi)); v.push(Instruction::LocalGet(spb_hi)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(den_hi));
                // result = numerator / denominator
                v.push(Instruction::LocalGet(num_hi)); v.push(Instruction::LocalGet(den_hi)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(num_hi));
                // Store: lo=0, hi=result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(num_hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "liq_amount1_64" => {
                // (liq_amount1_64 dst spa_addr spb_addr liq_addr)
                // amount1 = L * (sqrtPb - sqrtPa)
                let dst = self.expr(&a[0])?;
                let addr_spa = self.local_idx("__la1_as");
                let addr_spb = self.local_idx("__la1_ab");
                let addr_liq = self.local_idx("__la1_al");
                let dst_i = self.local_idx("__la1_d");
                let spa_h = self.local_idx("__la1_sh");
                let spb_h = self.local_idx("__la1_bh");
                let liq_h = self.local_idx("__la1_lh");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                let spa_e = self.expr(&a[1])?;
                v.extend(spa_e); v.push(Instruction::LocalSet(addr_spa));
                let spb_e = self.expr(&a[2])?;
                v.extend(spb_e); v.push(Instruction::LocalSet(addr_spb));
                let liq_e = self.expr(&a[3])?;
                v.extend(liq_e); v.push(Instruction::LocalSet(addr_liq));
                // Load high words
                v.push(Instruction::LocalGet(addr_spa)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spa_h));
                v.push(Instruction::LocalGet(addr_spb)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(spb_h));
                v.push(Instruction::LocalGet(addr_liq)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(liq_h));
                // result_hi = liq_h * (spb_h - spa_h)
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::LocalGet(spb_h)); v.push(Instruction::LocalGet(spa_h)); v.push(Instruction::I64Sub);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalSet(liq_h));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(liq_h));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "price64_to_tick" => {
                // (price64_to_tick addr) → i64
                // Linear approximation: tick ≈ (price-1) * 10001
                // Good for ±500 ticks (< 0.5% error), acceptable for CLMM range queries
                // For wider range: iterate with tick_to_price64 refinement
                let pa = self.expr(&a[0])?;
                let addr_i = self.local_idx("__p2t_a");
                let ph = self.local_idx("__p2t_ph");
                let pl = self.local_idx("__p2t_pl");
                let diff = self.local_idx("__p2t_d");
                let tick = self.local_idx("__p2t_t");
                let mut v = Vec::new();
                v.extend(pa); v.push(Instruction::LocalSet(addr_i));
                // Load Q64.64 and convert to Q32.32
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ph));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(pl));
                // q32 = (ph << 32) | (pl >> 32)
                v.push(Instruction::LocalGet(ph)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(pl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Or);
                // diff = q32 - (1<<32)
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(diff));
                // tick = diff * 10001 >> 32
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Const(10001)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                // Quadratic correction for larger range: subtract diff^2 * 5002 >> 64
                v.push(Instruction::LocalGet(diff)); v.push(Instruction::LocalGet(diff)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(5002)); v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(tick));
                v.push(Instruction::LocalGet(tick)); Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
