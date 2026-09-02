use super::*;

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.as_bytes();
    if s.len() % 2 != 0 {
        return Err("hex string has odd length".into());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let hi = hex_nibble(s[i])?;
        let lo = hex_nibble(s[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("invalid hex char: {}", b as char)),
    }
}

impl WasmEmitter {
    /// Nibble (0..15) is on the wasm stack. Convert to an ASCII hex digit
    /// and store it at heap_base + i*2 + byte_off. Scratch local d_l is
    /// reused for the digit computation. Used by sha256-hash hex encoding.
    fn hex_digit_store(
        heap_base: u32,
        i_l: u32,
        d_l: u32,
        byte_off: i32,
        ma: &wasm_encoder::MemArg,
    ) -> Vec<Instruction<'static>> {
        let mut v = Vec::new();
        // d = nibble; d <= 9 ? '0'+d : 'a'+d-10  (48 + d + (d>9 ? 39 : 0))
        v.push(Instruction::LocalSet(d_l));
        v.push(Instruction::LocalGet(d_l));
        v.push(Instruction::I32Const(9));
        v.push(Instruction::I32GtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(d_l));
        v.push(Instruction::I32Const(48 + 39));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(d_l));
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(d_l));
        v.push(Instruction::I32Const(48));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(d_l));
        v.push(Instruction::End);
        // store8(heap_base + i*2 + off, d)
        v.push(Instruction::LocalGet(heap_base));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(i_l));
        v.push(Instruction::I32Const(2));
        v.push(Instruction::I32Mul);
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Const(byte_off));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalGet(d_l));
        v.push(Instruction::I32Store8(ma.clone()));
        v
    }

    pub(crate) fn call_near_crypto(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/sha256" => {
                if a.len() != 1 {
                    return Err("near/sha256: need 1 args (msg)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(21)); // sha256
                                             // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/keccak256" => {
                if a.len() != 1 {
                    return Err("near/keccak256: need 1 args (msg)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(22)); // keccak256
                                             // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ed25519_verify" => {
                if a.len() != 3 {
                    return Err("near/ed25519_verify: need 3 args (sig, msg, pk)".into());
                }
                // (near/ed25519_verify signature message public_key) → bool
                // All three args are byte strings (tagged Str)
                // NEAR host: ed25519_verify(sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) → u64 — idx 24
                let sig = self.expr(&a[0])?;
                let msg = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                // sig (param0, param1)
                v.extend(sig.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // sig_ptr
                                                    // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // msg_ptr
                                                    // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(24)); // ed25519_verify — returns u64 directly (1=valid, 0=invalid)
                                             // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "schnorr-verify" => {
                // (schnorr-verify pk_bytes sig_bytes msg_bytes) -> int (1/0)
                // BIP-340 via WASI-resolved import: schnorr_verify_bip340(pk_ptr, sig_ptr, msg_ptr, msg_len) -> i32
                // Local: near_mock.rs resolves to builtin_schnorr.rs. On-chain: linker stitches WASM.
                let pk = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let msg = self.expr(&a[2])?;
                let wasm_idx = self.need_wasm_import(
                    "schnorr_verify_bip340",
                    vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
                    vec![ValType::I32],
                );
                let mut v = Vec::new();
                // pk_ptr (untag Str -> raw pointer)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                // sig_ptr
                v.extend(sig.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                // msg_ptr
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                // msg_len (upper 32 bits of tagged Str = length)
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // Call stitched WASM schnorr
                v.push(Self::wasm_import_call(wasm_idx));
                // Result: i32 (1=valid, 0=invalid) -> tag as Num
                v.push(Instruction::I64ExtendI32S);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "sha256-hash" => {
                // (sha256-hash input_str) -> 32-byte hash string
                // WASM import: sha256_hash(input_ptr: i32, input_len: i32, output_ptr: i32)
                // Writes 32 bytes to output_ptr
                let input = self.expr(&a[0])?;
                let wasm_idx = self.need_wasm_import(
                    "sha256_hash",
                    vec![ValType::I32, ValType::I32, ValType::I32],
                    vec![],  // no return value
                );
                let mut v = Vec::new();
                // input_ptr
                v.extend(input.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                // input_len (upper 32 bits of tagged Str)
                v.extend(input.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                // output_ptr: use TEMP_MEM area
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I32WrapI64);
                // Call sha256_hash — 32 raw bytes now at TEMP_MEM
                v.push(Self::wasm_import_call(wasm_idx));

                // HEX-ENCODE into a fresh heap string (64 chars).
                // (2026-09-01, HTLC bug #12 root cause): the old path
                // returned a tagged Str aliasing the fixed TEMP_MEM
                // scratch — two live digests overwrote each other — and
                // RAW binary bytes embedded via json-set derailed
                // __json_set's structural scanner whenever the digest
                // contained { } " \ , : (silent fresh-object fallback =
                // total record loss on the next chained set). Hex is
                // scanner-safe, storage-safe, display-safe, and matches
                // the HTLC convention (hashlocks are hex digests).
                let hx_i = self.local_idx_i32("__hx_i");
                let hx_d = self.local_idx_i32("__hx_d");
                let hx_b = self.local_idx_i32("__hx_b");
                let hx_old = self.local_idx("__hx_old");
                let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                // hx_old = heap_bump(64)
                v.push(Instruction::I32Const(56)); // RUNTIME_HEAP_PTR addr
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(hx_old));
                v.push(Instruction::I32Const(56));
                v.push(Instruction::LocalGet(hx_old));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // i = 0
                v.push(Instruction::I32Const(0));
                v.push(Instruction::LocalSet(hx_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                // if i >= 32 → done
                v.push(Instruction::LocalGet(hx_i));
                v.push(Instruction::I32Const(32));
                v.push(Instruction::I32GeS);
                v.push(Instruction::I32Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // b = TEMP_MEM[i] (kept in its own local — the digit
                // scratch hx_d is clobbered by hex_digit_store itself)
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.push(Instruction::LocalGet(hx_i));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Load8U(ma1.clone()));
                v.push(Instruction::LocalSet(hx_b));
                // hi nibble → hex digit at old + i*2
                v.push(Instruction::LocalGet(hx_b));
                v.push(Instruction::I32Const(4));
                v.push(Instruction::I32ShrU);
                v.extend(Self::hex_digit_store(hx_old, hx_i, hx_d, 0, &ma1));
                // lo nibble → hex digit at old + i*2 + 1
                v.push(Instruction::LocalGet(hx_b));
                v.push(Instruction::I32Const(15));
                v.push(Instruction::I32And);
                v.extend(Self::hex_digit_store(hx_old, hx_i, hx_d, 1, &ma1));
                // i++; continue
                v.push(Instruction::LocalGet(hx_i));
                v.push(Instruction::I32Const(1));
                v.push(Instruction::I32Add);
                v.push(Instruction::LocalSet(hx_i));
                v.push(Instruction::Br(1));
                v.push(Instruction::End); // if
                v.push(Instruction::End); // loop
                v.push(Instruction::End); // block
                // tagged Str: (64 << 32) | hx_old
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(hx_old));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/p256_verify" => {
                if a.len() != 3 {
                    return Err("near/p256_verify: need 3 args (sig, msg, pk)".into());
                }
                // (near/p256_verify signature message public_key) → bool
                // NEAR host: p256_verify(sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) → u64 — idx 55
                // sig: 64 bytes (r||s), msg: prehashed digest, pk: 33 bytes (compressed SEC1)
                // ⚠ Requires protocol 85+ (p256_verify_host_fn). Fails with "unknown import" on older protocols.
                eprintln!("⚠️  near/p256_verify requires protocol 85+ (p256_verify_host_fn). Will fail on older protocols.");
                let sig = self.expr(&a[0])?;
                let msg = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                // sig (param0, param1)
                v.extend(sig.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // sig_ptr
                                                    // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // msg_ptr
                                                    // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(55)); // p256_verify — returns u64 directly (1=valid, 0=invalid)
                                             // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/random_seed" => self.read_to_register(23, a),
            "near/keccak512" => {
                if a.len() != 1 {
                    return Err("near/keccak512: need 1 args (msg)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(52));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ripemd160" => {
                if a.len() != 1 {
                    return Err("near/ripemd160: need 1 args (msg)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(53));
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ecrecover" => {
                if a.len() != 4 && a.len() != 5 {
                    return Err("near/ecrecover: need 4 args (hash, sig, v, malleability); 5th (s) ignored".into());
                }
                let hash = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let v_val = self.expr(&a[2])?;
                let malleability = self.expr(&a[3])?;
                let mut vv = Vec::new();
                vv.extend(hash.clone());
                vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32));
                vv.push(Instruction::I64ShrU);
                vv.extend(hash);
                vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64);
                vv.push(Instruction::I64ExtendI32U);
                vv.extend(sig.clone());
                vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32));
                vv.push(Instruction::I64ShrU);
                vv.extend(sig);
                vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64);
                vv.push(Instruction::I64ExtendI32U);
                vv.extend(v_val);
                vv.extend(malleability);
                vv.push(Instruction::I64Const(0)); // register_id
                vv.push(Self::host_call(54));
                vv.extend(self.emit_tag_num());
                Ok(vv)
            }
            "near/alt_bn128_g1_multiexp" => {
                if a.len() != 1 {
                    return Err("near/alt_bn128_g1_multiexp: need 1 args (pairs)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(56));
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/alt_bn128_g1_sum" => {
                if a.len() != 1 {
                    return Err("near/alt_bn128_g1_sum: need 1 args (data buffer)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(57));
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/alt_bn128_pairing_check" => {
                if a.len() != 1 {
                    return Err("near/alt_bn128_pairing_check: need 1 args (data buffer)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(58));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/bls12381_p1_sum" => {
                if a.len() != 1 {
                    return Err("near/bls12381_p1_sum: need 1 args (data buffer)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(59));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_p2_sum" => {
                if a.len() != 1 {
                    return Err("near/bls12381_p2_sum: need 1 args (data buffer)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(60));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_g1_multiexp" => {
                if a.len() != 1 {
                    return Err("near/bls12381_g1_multiexp: need 1 args (pairs)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(61));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_g2_multiexp" => {
                if a.len() != 1 {
                    return Err("near/bls12381_g2_multiexp: need 1 args (pairs)".into());
                }
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(62));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_map_fp_to_g1" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(63));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_map_fp2_to_g2" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(64));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_pairing_check" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(65));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/bls12381_p1_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(66));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_p2_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(67));
                v.push(Instruction::Drop); // drop status u64
                // ALIASING FIX (2026-09-02, found by bls_msig): reading the
                // register straight into TEMP_MEM returns a POINTER that the
                // next register-writing host call overwrites — two live
                // results alias (sigma showed apk's bytes). Copy to a fresh
                // runtime-heap buffer instead (call_near_iter.rs pattern).
                let bls_len = self.local_idx("__bls_rlen");
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(1)); // register_len(0)
                v.push(Instruction::LocalSet(bls_len));
                let bls_buf = self.local_idx("__bls_rbuf");
                v.extend(self.emit_rtheap_alloc(bls_buf, bls_len));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Self::host_call(0)); // read_register(0, buf)
                v.push(Instruction::LocalGet(bls_len));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(bls_buf));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/schnorr_verify" => {
                // (near/schnorr_verify pk_hex sig_hex msg_hex) -> int
                // Compile-time hex decode: hex string literals -> raw bytes in data section.
                // Returns tagged Num 1 (valid) or 0 (invalid).
                let wasm_idx = self.need_wasm_import(
                    "schnorr_verify_bip340",
                    vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
                    vec![ValType::I32],
                );
                // Hex-decode each arg at compile time, alloc in data section
                let mut bufs: [u32; 3] = [0; 3];
                let mut msg_len = 0u32;
                for (i, arg) in a.iter().enumerate().take(3) {
                    let hex_str = match arg {
                        LispVal::Str(s) => s.clone(),
                        _ => return Err(format!("near/schnorr_verify arg {} must be a string literal, got {:?}", i, arg)),
                    };
                    let bytes = hex_decode(&hex_str)
                        .map_err(|e| format!("near/schnorr_verify: invalid hex in arg {}: {}", i, e))?;
                    if i == 2 { msg_len = bytes.len() as u32; }
                    let offset = self.alloc_data(&bytes);
                    bufs[i] = offset;
                }
                let mut v = Vec::new();
                v.push(Instruction::I32Const(bufs[0] as i32));
                v.push(Instruction::I32Const(bufs[1] as i32));
                v.push(Instruction::I32Const(bufs[2] as i32));
                v.push(Instruction::I32Const(msg_len as i32));
                v.push(Self::wasm_import_call(wasm_idx));
                v.push(Instruction::I64ExtendI32S);
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
