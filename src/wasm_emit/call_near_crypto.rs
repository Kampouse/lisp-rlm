use super::*;

impl WasmEmitter {
    pub(crate) fn call_near_crypto(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/sha256" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(21)); // sha256
                // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/keccak256" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                // Untag string: extract len and ptr
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(22)); // keccak256
                // read_register(0, TEMP_MEM)
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                // register_len(0)
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                // Pack: (len << 32) | TEMP_MEM — tag as Str
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ed25519_verify" => {
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
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // sig_ptr
                // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // msg_ptr
                // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(24)); // ed25519_verify — returns u64 directly (1=valid, 0=invalid)
                // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/p256_verify" => {
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
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // sig_len
                v.extend(sig);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // sig_ptr
                // msg (param2, param3)
                v.extend(msg.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // msg_len
                v.extend(msg);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // msg_ptr
                // pk (param4, param5)
                v.extend(pk.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // pk_len
                v.extend(pk);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // pk_ptr
                v.push(Self::host_call(55)); // p256_verify — returns u64 directly (1=valid, 0=invalid)
                // Tag result as Num
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/random_seed" => self.read_to_register(23, a),
            "near/keccak512" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0)); // register_id=0
                v.push(Self::host_call(52));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0)); // read_register
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1)); // register_len
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ripemd160" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone());
                v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); // data_len
                v.extend(data);
                v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U); // data_ptr
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(53));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/ecrecover" => {
                let hash = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let v_val = self.expr(&a[2])?;
                let malleability = self.expr(&a[3])?;
                let mut vv = Vec::new();
                vv.extend(hash.clone()); vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32)); vv.push(Instruction::I64ShrU);
                vv.extend(hash); vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64); vv.push(Instruction::I64ExtendI32U);
                vv.extend(sig.clone()); vv.extend(self.emit_untag());
                vv.push(Instruction::I64Const(32)); vv.push(Instruction::I64ShrU);
                vv.extend(sig); vv.extend(self.emit_untag());
                vv.push(Instruction::I32WrapI64); vv.push(Instruction::I64ExtendI32U);
                vv.extend(v_val);
                vv.extend(malleability);
                vv.push(Instruction::I64Const(0)); // register_id
                vv.push(Self::host_call(54));
                vv.extend(self.emit_tag_num());
                Ok(vv)
            }
            "near/p256_verify" => {
                let msg = self.expr(&a[0])?;
                let sig = self.expr(&a[1])?;
                let pk = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(msg.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(msg); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(sig.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(sig); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.extend(pk.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(pk); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(55));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/alt_bn128_g1_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0)); // register_id
                v.push(Self::host_call(56));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/alt_bn128_g1_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(57));
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/alt_bn128_pairing_check" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(58));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/bls12381_p1_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(59));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_p2_sum" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(60));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_g1_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(61));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_g2_multiexp" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(62));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_map_fp_to_g1" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(63));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_map_fp2_to_g2" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(64));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_pairing_check" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Self::host_call(65));
                v.extend(self.emit_tag_num());
                Ok(v)
            }
            "near/bls12381_p1_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(66));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            "near/bls12381_p2_decompress" => {
                let data = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(data.clone()); v.extend(self.emit_untag());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(data); v.extend(self.emit_untag());
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(67));
                v.push(Instruction::Drop); // drop status u64
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(0));
                v.push(Instruction::I64Const(0)); v.push(Self::host_call(1));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(TEMP_MEM)); v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
