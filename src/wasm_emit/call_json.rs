use super::*;

impl WasmEmitter {
    pub(crate) fn call_json(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "near/json_get_int" => {
                if a.is_empty() {
                    return Err("near/json_get_int requires a string key argument".into());
                }
                // 2-arg form (2026-09-17): jsonGetInt(key, json) scans the
                // GIVEN JSON string, not the tx input — mirror of the
                // jsonGetStr fix (2026-09-14). It used to compile fine and
                // silently ignore the second arg (json_lit_lookup_str reads
                // tx input only): a silent-wrong-answer footgun in money
                // code. Composed as the nil-on-miss buffer scan
                // (`json-get-str?`, dot-paths supported) + the shared
                // int-parse tail: miss → nil (?? fallback fires), hit →
                // first-byte gate + __str_to_num — same semantics as the
                // unified 1-arg path (found-non-numeric → nil).
                if a.len() >= 2 {
                    if !matches!(&a[0], LispVal::Str(_)) {
                        return Err("jsonGetInt(key, json): key must be a string literal".into());
                    }
                    let args2: Vec<LispVal> = a[..2].to_vec();
                    let lookup = self.call_json("json-get-str?", &args2)?;
                    return self.emit_int_parse_gated(lookup);
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        // Shared-function route (2026-09-14, code size): the
                        // literal key builds a data-section pattern and goes
                        // through __json_get + __str_to_num — SAME path as
                        // dynamic keys. The old inline ~550-instr scanner per
                        // call site made a 6-getter entrypoint ~3.8k instrs;
                        // now it's ~15 per getter + one shared helper.
                        // Semantics UNIFY with the dynamic path: miss → nil,
                        // found-non-numeric → nil (first-byte gate),
                        // found → parse (2026-08-31 + 2026-09-14 rules).
                        let lookup = self.json_lit_lookup_str(key);
                        self.emit_int_parse_gated(lookup)
                    }
                    // Dynamic key (2026-09-13): same __json_get lookup, then
                    // parse the raw value bytes with the shared __str_to_num.
                    // Miss → TAG_NIL (?? fallback fires). Found-but-non-
                    // numeric ("n": "abc", true, {…}) → TAG_NIL too (2026-09-14).
                    _ => {
                        let lookup = self.json_dyn_lookup_str(&a[0])?;
                        self.emit_int_parse_gated(lookup)
                    }
                }
            }
            "near/json_get_u128" => {
                // 1-arg form (2026-10-12): near.jsonGetU128("key") — reads the
                // tx INPUT (same contract as 1-arg jsonGetStr) into the
                // dedicated 16-byte u128 scratch slot (STORAGE_U128_BUF), the
                // same destination the Lisp typed form (json-get :u128 "k")
                // uses. The read itself hardens-parses, so the result is a
                // provable u128 SOURCE. Previously only the internal 2-arg
                // (key, offset-addr) form existed, so the documented 1-arg
                // TS surface failed with "requires a string key and offset
                // argument" — and passing a TS string as the raw offset
                // (the 2-arg form reached from TS) would use a tagged-string
                // payload pointer as a WRITE ADDRESS. 2-arg from TS is now
                // rejected: the offset ABI is emitter-internal.
                if a.len() == 1 {
                    return match &a[0] {
                        LispVal::Str(key) => {
                            let offset_expr = vec![Instruction::I64Const(STORAGE_U128_BUF as i64)];
                            self.json_get_u128(key, offset_expr)
                        }
                        _ => Err("near/json_get_u128 key must be a string literal".into()),
                    };
                }
                if a.len() != 2 {
                    return Err(
                        "near/json_get_u128 requires a string key (optionally + internal offset addr)".into(),
                    );
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        let offset_expr = self.expr(&a[1])?;
                        self.json_get_u128(key, offset_expr)
                    }
                    _ => Err("near/json_get_u128 key must be a string literal".into()),
                }
            }
            "near/json_get_arr" => {
                if a.is_empty() {
                    return Err("near/json_get_arr requires a string key argument".into());
                }
                match &a[0] {
                    LispVal::Str(key) => self.json_get_arr(key),
                    _ => Err("near/json_get_arr key must be a string literal".into()),
                }
            }
            "near/json_get_str_opt" => {
                // (2026-10-12) Optional param prop (TS `prop?: string`):
                // NIL-on-miss so callers can distinguish absent from "".
                // Same emission as near/json_get_str 1-arg + miss gate.
                if a.is_empty() {
                    return Err("near/json_get_str_opt requires a string key argument".into());
                }
                if a.len() >= 2 {
                    if !matches!(&a[0], LispVal::Str(_)) {
                        return Err("jsonGetStrOpt(key, json): key must be a string literal".into());
                    }
                    let args2: Vec<LispVal> = a[..2].to_vec();
                    return self.call_json("json-get-str?", &args2);
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = self.json_get_str(key)?;
                        let t = self.local_idx("__jgs_miss");
                        v.push(Instruction::LocalTee(t));
                        v.push(Instruction::I64Eqz);
                        v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::I64Const(TAG_NIL));
                        v.push(Instruction::Else);
                        v.push(Instruction::LocalGet(t));
                        // (2026-10-12) Hit branch must TAG the raw packed
                        // span — same as near/json_get_str below. It used to
                        // return the untagged (len<<32|ptr), so downstream
                        // tagged-string consumers (u128 parse, str ops) saw
                        // garbage tag bits and trapped at the tag check.
                        v.extend(self.emit_tag_str());
                        v.push(Instruction::End);
                        Ok(v)
                    }
                    _ => Err("near/json_get_str_opt: key must be a string literal".into()),
                }
            }
            "near/json_get_str" => {
                if a.is_empty() {
                    return Err("near/json_get_str requires a string key argument".into());
                }
                // 2-arg form (2026-09-14): jsonGetStr(key, json) scans the
                // GIVEN JSON string, not the tx input. It used to compile
                // fine and silently ignore the second arg — a silent-
                // wrong-answer footgun (probe dot-path chain returned MISS
                // on valid input). Dot-paths work in this form: the buffer
                // scanner supports "a.b" keys.
                if a.len() >= 2 {
                    if !matches!(&a[0], LispVal::Str(_)) {
                        return Err("jsonGetStr(key, json): key must be a string literal".into());
                    }
                    let args2: Vec<LispVal> = a[..2].to_vec();
                    return self.call_json("json-get-str", &args2);
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = self.json_get_str(key)?;
                        // (2026-08-31) d.ts says `string | null` and `??`
                        // lowers to (default x fb) — but a miss returned ""
                        // (TAG_STR), so the fallback NEVER fired. Miss now
                        // yields TAG_NIL; `(jsonGetStr("g") ?? "fb")` works
                        // exactly as TS promises. Bare use without ?? gets
                        // nil (to-string renders it "nil" — visible, not
                        // silent corruption).
                        let t = self.local_idx("__jgs_miss");
                        v.push(Instruction::LocalTee(t));
                        v.push(Instruction::I64Eqz);
                        v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::I64Const(TAG_NIL));
                        v.push(Instruction::Else);
                        v.push(Instruction::LocalGet(t));
                        v.extend(self.emit_tag_str());
                        v.push(Instruction::End);
                        Ok(v)
                    }
                    _ => self.json_dyn_lookup_str(&a[0]),
                }
            }
            "json/get" => {
                if a.is_empty() {
                    return Err("json/get requires a string key argument".into());
                }
                match &a[0] {
                    LispVal::Str(key) => self.json_get_auto(key),
                    _ => Err("json/get key must be a string literal".into()),
                }
            }
            "near/json_return_int" => {
                let val_expr = self.expr(&a[0])?;
                self.json_return_int(val_expr)
            }
            "near/json_return_str" => {
                // Wrap with tag-aware to-string: raw NUM args rendered empty
                // before (json_return_str assumes a STR tag). to-string is a
                // no-op on strings, decimal-converts numbers.
                let wrapped = LispVal::List(vec![LispVal::Sym("to-string".into()), a[0].clone()]);
                let packed_expr = self.expr(&wrapped)?;
                self.json_return_str(packed_expr)
            }
            "json-return" => {
                self.need_host(25);
                let val_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.extend(val_expr);
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "json-get" if !a.is_empty() && matches!(&a[0], LispVal::Sym(s) if s == ":u128") => {
                // Typed form (2026-10-12 rebuild). Emits the exact DECIMAL
                // STRING of the 128-bit value (tagged str): composes with the
                // str-typed u128/* builtin family; the checker types it U128,
                // which unifies with Str but never Num (tagged-i64 firewall).
                // Mechanics, proven parts only:
                //   1. eval the json expr, copy bytes to JSON_SCAN_BUF
                //      (fixed scratch — never INPUT_CACHE_BUF, the
                //      2026-09-14 clobber rule);
                //   2. scan THAT buffer (miss ⇒ 0) into a fresh
                //      runtime-bumped 16-byte cell (cdr-rule: never a static
                //      per-site buffer);
                //   3. render via __h_u128_to_str (bumps the runtime heap
                //      for its own scratch, returns a tagged decimal str).
                let (json_arg, key_arg) = match a.len() {
                    2 => (None, &a[1]),   // (json-get :u128 "key") — input scan
                    3 => (Some(&a[1]), &a[2]),
                    _ => {
                        return Err(
                            "json-get :u128 expects (json-get :u128 <json> \"key\") or (json-get :u128 \"key\")"
                                .into(),
                        )
                    }
                };
                let key = match key_arg {
                    LispVal::Str(k) => k.clone(),
                    _ => return Err("json-get :u128 key must be a string literal".into()),
                };
                let slot = self.local_idx("__jgu128_slot");
                let mut v = Vec::new();
                match json_arg {
                    Some(je) => {
                        // Copy the json text to the scan buffer (same
                        // copy-loop discipline as the 2-arg auto arm).
                        let tmp = self.local_idx("__jgu128_tmp");
                        v.extend(self.expr(je)?);
                        v.push(Instruction::I64Const(3));
                        v.push(Instruction::I64ShrU);
                        v.push(Instruction::LocalSet(tmp));
                        let src_ptr_l = self.local_idx("__jgu128_sp");
                        let copy_i = self.local_idx("__jgu128_ci");
                        let ma8b = wasm_encoder::MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        };
                        v.push(Instruction::LocalGet(tmp));
                        v.push(Instruction::I64Const(0xFFFFFFFF));
                        v.push(Instruction::I64And);
                        v.push(Instruction::LocalSet(src_ptr_l));
                        // counter init only — the loop condition re-derives
                        // len each iteration; nothing left on the stack
                        v.push(Instruction::I64Const(0));
                        v.push(Instruction::LocalSet(copy_i));
                        v.push(Instruction::Block(BlockType::Empty));
                        v.push(Instruction::Loop(BlockType::Empty));
                        v.push(Instruction::LocalGet(copy_i));
                        v.push(Instruction::LocalGet(tmp));
                        v.push(Instruction::I64Const(32));
                        v.push(Instruction::I64ShrU);
                        v.push(Instruction::I64GeU);
                        v.push(Instruction::BrIf(1));
                        v.push(Instruction::I64Const(JSON_SCAN_BUF));
                        v.push(Instruction::LocalGet(copy_i));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(src_ptr_l));
                        v.push(Instruction::LocalGet(copy_i));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::I32Load8U(ma8b.clone()));
                        v.push(Instruction::I32Store8(ma8b));
                        v.push(Instruction::LocalGet(copy_i));
                        v.push(Instruction::I64Const(1));
                        v.push(Instruction::I64Add);
                        v.push(Instruction::LocalSet(copy_i));
                        v.push(Instruction::Br(0));
                        v.push(Instruction::End);
                        v.push(Instruction::End);
                        v.extend(self.heap_bump_runtime(16, "__jgu128_res"));
                        // heap_bump_runtime leaves NOTHING on the stack — the
                        // result lands in the __jgu128_res local (cdr-site
                        // pattern, call_list.rs:1070). LocalSet directly after
                        // the bump pops an empty stack (validation error).
                        v.push(Instruction::LocalGet(self.local_idx("__jgu128_res")));
                        v.push(Instruction::LocalSet(slot));
                        // scanned length = the staged json string's len>>32
                        let ilen_v = vec![
                            Instruction::LocalGet(tmp),
                            Instruction::I64Const(32),
                            Instruction::I64ShrU,
                        ];
                        let call = vec![Instruction::LocalGet(slot)];
                        v.extend(self.json_get_u128_from(
                            &key,
                            call,
                            JSON_SCAN_BUF,
                            Some(ilen_v),
                        )?);
                        // json_get_u128_from pushes exactly one value on both
                        // paths (hit: re-pushed dst addr; miss: dst addr) —
                        // Drop it; to_str reads the slot local instead.
                        v.push(Instruction::Drop);
                    }
                    None => {
                        v.extend(self.heap_bump_runtime(16, "__jgu128_res"));
                        v.push(Instruction::LocalGet(self.local_idx("__jgu128_res")));
                        v.push(Instruction::LocalSet(slot));
                        let call = vec![Instruction::LocalGet(slot)];
                        v.extend(self.json_get_u128_from(&key, call, INPUT_CACHE_BUF, None)?);
                        // one value on both paths — Drop; to_str reads slot
                        v.push(Instruction::Drop);
                    }
                }
                let h = self.ensure_u128_str_helpers();
                v.push(Instruction::LocalGet(slot));
                v.push(Self::call_user(h.to_str));
                Ok(v)
            }
            "json-get" => {
                // Unified argument order (2026-10-07): (json-get <json> "key")
                // — the interpreter's documented contract. The emitter used
                // to read arg0 as the KEY; inverted here and every in-repo
                // caller bumped in the same change. The 1-arg form
                // (json-get "key") still scans the global input buffer
                // (NEAR input() / outlayer stdin — wasm-only idiom).
                match a.len() {
                    1 => match &a[0] {
                        LispVal::Str(key) => {
                            if self.wasi_mode {
                                self.json_get_wasi(key, "auto")
                            } else {
                                self.json_get_with_scanner(key, "auto")
                            }
                        }
                        _ => Err("json-get key must be a string literal".into()),
                    },
                    2 => match &a[1] {
                        LispVal::Str(key) => {
                            // (json-get <json-string> "key") — copy the
                            // tagged string to a fixed scratch buffer, then
                            // run the shared scanner in "auto" mode: TAG_NUM
                            // for numerics, heap TAG_STR for strings, NIL on
                            // miss — interp parity (the old path always
                            // returned Str, "" on miss, which broke every
                            // (nil? …) guard downstream).
                            let buf_expr = self.expr(&a[0])?;
                            let mut setup = Vec::new();
                            let tmp = self.local_idx("__jgs_tmp");
                            // Untag: payload = tagged >> 3
                            setup.extend(buf_expr);
                            setup.push(Instruction::I64Const(3));
                            setup.push(Instruction::I64ShrU);
                            setup.push(Instruction::LocalSet(tmp));
                            // Copy the bytes to a fixed scratch buffer — the
                            // scanner takes a constant buf address. NEAR uses
                            // JSON_SCAN_BUF (must NOT overlap STDIN_BUF at
                            // 32768 — input-cache clobber bug, 2026-09-14);
                            // WASI uses 65536.
                            let target_buf = if self.wasi_mode {
                                65536i64
                            } else {
                                JSON_SCAN_BUF
                            };
                            let src_ptr_l = self.local_idx("__jgs_sp");
                            let copy_i = self.local_idx("__jgs_ci");
                            let ma8 = wasm_encoder::MemArg {
                                offset: 0,
                                align: 0,
                                memory_index: 0,
                            };
                            // src_ptr = tmp & 0xFFFFFFFF
                            setup.push(Instruction::LocalGet(tmp));
                            setup.push(Instruction::I64Const(0xFFFFFFFF));
                            setup.push(Instruction::I64And);
                            setup.push(Instruction::LocalSet(src_ptr_l));
                            // copy_setup leaves the LENGTH on the stack
                            // (json_get_from_buf's buf_len_setup contract):
                            // len = tmp >> 32, then the copy loop.
                            let mut copy_setup = Vec::new();
                            copy_setup.push(Instruction::LocalGet(tmp));
                            copy_setup.push(Instruction::I64Const(32));
                            copy_setup.push(Instruction::I64ShrU);
                            copy_setup.push(Instruction::I64Const(0));
                            copy_setup.push(Instruction::LocalSet(copy_i));
                            copy_setup.push(Instruction::Block(BlockType::Empty));
                            copy_setup.push(Instruction::Loop(BlockType::Empty));
                            copy_setup.push(Instruction::LocalGet(copy_i));
                            copy_setup.push(Instruction::LocalGet(tmp));
                            copy_setup.push(Instruction::I64Const(32));
                            copy_setup.push(Instruction::I64ShrU);
                            copy_setup.push(Instruction::I64GeU);
                            copy_setup.push(Instruction::BrIf(1));
                            // target_buf[i] = src[i]
                            copy_setup.push(Instruction::I64Const(target_buf));
                            copy_setup.push(Instruction::LocalGet(copy_i));
                            copy_setup.push(Instruction::I64Add);
                            copy_setup.push(Instruction::I32WrapI64);
                            copy_setup.push(Instruction::LocalGet(src_ptr_l));
                            copy_setup.push(Instruction::LocalGet(copy_i));
                            copy_setup.push(Instruction::I64Add);
                            copy_setup.push(Instruction::I32WrapI64);
                            copy_setup.push(Instruction::I32Load8U(ma8.clone()));
                            copy_setup.push(Instruction::I32Store8(ma8));
                            copy_setup.push(Instruction::LocalGet(copy_i));
                            copy_setup.push(Instruction::I64Const(1));
                            copy_setup.push(Instruction::I64Add);
                            copy_setup.push(Instruction::LocalSet(copy_i));
                            copy_setup.push(Instruction::Br(0));
                            copy_setup.push(Instruction::End);
                            copy_setup.push(Instruction::End);
                            // "auto": numeric → TAG_NUM, string → heap
                            // TAG_STR, miss → TAG_NIL
                            let mut v =
                                self.json_get_from_buf(key, "auto", target_buf, &mut copy_setup)?;
                            v.splice(0..0, setup.iter().cloned());
                            Ok(v)
                        }
                        _ => Err(
                            "json-get: second argument must be the key string literal — (json-get <json> \"key\")"
                                .into(),
                        ),
                    },
                    _ => Err(
                        "json-get: expected (json-get <json> \"key\") or 1-arg (json-get \"key\")"
                            .into(),
                    ),
                }
            }
            "json-get-str" | "json-get-str?" => {
                // `json-get-str?` (2026-09-15, JSON v3): NIL-on-miss variant —
                // `o.a.b ?? fb` on input handles needs the fallback to fire;
                // the base op returns "" (legacy contract, conflates with a
                // legit empty-string value). Same emission + a raw-result
                // gate (packed 0 == miss, unambiguous BEFORE tagging).
                let nil_on_miss = op.contains("?");
                if a.len() < 2 {
                    return Err(
                        "json-get-str requires two arguments: (json-get-str \"key\" input-buffer)"
                            .into(),
                    );
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        // (json-get-str "key" buffer) — supports dot-path notation
                        let buf_expr = self.expr(&a[1])?;
                        let tmp = self.local_idx("__jgs_tmp");
                        let mut setup = Vec::new();
                        // Untag buffer to get payload (len<<32|ptr)
                        setup.extend(buf_expr.clone());
                        setup.push(Instruction::I64Const(3));
                        setup.push(Instruction::I64ShrU);
                        setup.push(Instruction::LocalSet(tmp));
                        // Copy string to fixed buffer at 65536 (JSON_FIXED_BUF, not STDIN_BUF 32768)
                        let target_buf = if self.wasi_mode {
                            65536i64
                        } else {
                            JSON_SCAN_BUF
                        }; // 2026-09-14: was INPUT_BUF — clobbered the input cache
                        let src_ptr_l = self.local_idx("__jgs_sp");
                        let copy_i = self.local_idx("__jgs_ci");
                        let ma8 = wasm_encoder::MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        };
                        // src_ptr = tmp & 0xFFFFFFFF
                        setup.push(Instruction::LocalGet(tmp));
                        setup.push(Instruction::I64Const(0xFFFFFFFF));
                        setup.push(Instruction::I64And);
                        setup.push(Instruction::LocalSet(src_ptr_l));
                        // len = tmp >> 32
                        let mut len_setup = Vec::new();
                        len_setup.push(Instruction::LocalGet(tmp));
                        len_setup.push(Instruction::I64Const(32));
                        len_setup.push(Instruction::I64ShrU);
                        // Copy loop
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        setup.push(Instruction::LocalGet(copy_i));
                        len_setup.iter().for_each(|i| setup.push(i.clone()));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::BrIf(1));
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(src_ptr_l));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Load8U(ma8.clone()));
                        setup.push(Instruction::I32Store8(ma8.clone()));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        // Now use json_get_from_buf with "str" value type (supports dot-path)
                        let mut len_for_jg = Vec::new();
                        len_for_jg.push(Instruction::LocalGet(tmp));
                        len_for_jg.push(Instruction::I64Const(32));
                        len_for_jg.push(Instruction::I64ShrU);
                        let mut v =
                            self.json_get_from_buf(key, "str", target_buf, &mut len_for_jg)?;
                        // Copy result from stdout_buf (204800) to heap for persistence
                        let jgs_tmp2 = self.local_idx("jgs_packed2");
                        let jgs_len = self.local_idx_i32("jgs_len2");
                        let jgs_ptr = self.local_idx_i32("jgs_ptr2");
                        v.push(Instruction::LocalSet(jgs_tmp2));
                        if nil_on_miss {
                            // packed == 0 is an UNAMBIGUOUS miss (real spans
                            // have nonzero ptrs) — gate BEFORE processing,
                            // which would conflate it with ""
                            v.push(Instruction::LocalGet(jgs_tmp2));
                            v.push(Instruction::I64Eqz);
                            v.push(Instruction::If(BlockType::Result(ValType::I64)));
                            v.push(Instruction::I64Const(TAG_NIL));
                            v.push(Instruction::Else);
                        }
                        v.push(Instruction::LocalGet(jgs_tmp2));
                        v.push(Instruction::I64Const(32));
                        v.push(Instruction::I64ShrU);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalSet(jgs_len));
                        v.push(Instruction::LocalGet(jgs_tmp2));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalSet(jgs_ptr));
                        // Runtime heap allocation (safe for recursive calls)
                        let jgs_heap2 = self.local_idx("jgs_heap2");
                        let jgs_aligned2 = self.local_idx_i32("jgs_aligned2");
                        let rhp2: i32 = 56;
                        let ma82 = wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        };
                        // Align len to 8
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::I32Const(7));
                        v.push(Instruction::I32Add);
                        v.push(Instruction::I32Const(-8));
                        v.push(Instruction::I32And);
                        v.push(Instruction::LocalSet(jgs_aligned2));
                        v.push(Instruction::I32Const(rhp2));
                        v.push(Instruction::I64Load(ma82.clone()));
                        v.push(Instruction::LocalSet(jgs_heap2));
                        v.push(Instruction::I32Const(rhp2));
                        v.push(Instruction::LocalGet(jgs_heap2));
                        v.push(Instruction::LocalGet(jgs_aligned2));
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I64Store(ma82));
                        v.push(Instruction::LocalGet(jgs_heap2));
                        v.push(Instruction::I32WrapI64); // dst as i32
                        v.push(Instruction::LocalGet(jgs_ptr));
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::MemoryCopy {
                            src_mem: 0,
                            dst_mem: 0,
                        });
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(32));
                        v.push(Instruction::I64Shl);
                        v.push(Instruction::LocalGet(jgs_heap2)); // already i64
                        v.push(Instruction::I64Or);
                        v.extend(self.emit_tag_str());
                        if nil_on_miss {
                            v.push(Instruction::End); // close the miss gate
                        }
                        v.splice(0..0, setup.iter().cloned());
                        Ok(v)
                    }
                    _ => Err("json-get-str key must be a string literal".into()),
                }
            }
            "json-get-str-esc" | "json-get-str-esc?" => {
                // (2026-10-14) jsonGetStrEsc(key, escapedJson) — the
                // promiseResult(0) shape: a JSON string delivered in its
                // JSON-ESCAPED form (`\"key\":\"value\"`), where the plain
                // scanner misses every key (its pattern is `"key"` with
                // REAL quotes). Emission: copy the escaped text into
                // JSON_SCAN_BUF (same setup as json-get-str?), then an
                // IN-PLACE unescape pass (\" → ", \\ → \, \/ → /, \n → LF,
                // \t → TAB, \r → CR, \b, \f; \uXXXX → ASCII low byte —
                // non-ASCII BMP collapses, documented limitation), then the
                // standard json_get_from_buf scan on the now-clean buffer.
                // In-place is safe: unescaped output is never longer than
                // the input. The write cursor becomes the scan length.
                let nil_on_miss = op.contains("?");
                if a.len() < 2 {
                    return Err(
                        "json-get-str-esc requires two arguments: (json-get-str-esc \"key\" escaped-json)".into(),
                    );
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        let buf_expr = self.expr(&a[1])?;
                        let tmp = self.local_idx("__jge_tmp");
                        let mut setup = Vec::new();
                        setup.extend(buf_expr.clone());
                        setup.push(Instruction::I64Const(3));
                        setup.push(Instruction::I64ShrU);
                        setup.push(Instruction::LocalSet(tmp));
                        let target_buf = JSON_SCAN_BUF;
                        let src_ptr_l = self.local_idx("__jge_sp");
                        let copy_i = self.local_idx("__jge_ci");
                        let ma8 = wasm_encoder::MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        };
                        setup.push(Instruction::LocalGet(tmp));
                        setup.push(Instruction::I64Const(0xFFFFFFFF));
                        setup.push(Instruction::I64And);
                        setup.push(Instruction::LocalSet(src_ptr_l));
                        let mut len_setup = Vec::new();
                        len_setup.push(Instruction::LocalGet(tmp));
                        len_setup.push(Instruction::I64Const(32));
                        len_setup.push(Instruction::I64ShrU);
                        // plain copy into JSON_SCAN_BUF
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        setup.push(Instruction::LocalGet(copy_i));
                        len_setup.iter().for_each(|i| setup.push(i.clone()));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::BrIf(1));
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(src_ptr_l));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Load8U(ma8.clone()));
                        setup.push(Instruction::I32Store8(ma8.clone()));
                        setup.push(Instruction::LocalGet(copy_i));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(copy_i));
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        // ── in-place unescape pass ──
                        // r = read cursor, w = write cursor, both start 0.
                        // loop: c = buf[r]; if c == '\\': d = buf[r+1];
                        //   map d (or \u + 2 hex) → out; r += step; else
                        //   buf[w++] = c; r++. Ends at r == len.
                        // Then len = w.
                        let r = self.local_idx("__jge_r");
                        let w = self.local_idx("__jge_w");
                        let c = self.local_idx("__jge_c");
                        let d = self.local_idx("__jge_d");
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(r));
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(w));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        // while r < len
                        setup.push(Instruction::LocalGet(r));
                        len_setup.iter().for_each(|i| setup.push(i.clone()));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::BrIf(1));
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Load8U(ma8.clone()));
                        setup.push(Instruction::I64ExtendI32U);
                        setup.push(Instruction::LocalSet(c));
                        // if c == '\\'
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x5C));
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Empty));
                        // d = buf[r+1] (escaped text is well-formed at this
                        // position by construction; a trailing bare '\' at
                        // end-of-buffer would read the NUL sentry — maps to
                        // a harmless byte, loop exits next iteration)
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Load8U(ma8.clone()));
                        setup.push(Instruction::I64ExtendI32U);
                        setup.push(Instruction::LocalSet(d));
                        // r += 2 (default; \u overrides below)
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::I64Const(2));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(r));
                        // dispatch on d — each arm writes buf[w++] and falls
                        // through to the loop tail. \uXXXX handled first:
                        // if d == 'u', parse 4 hex → byte (low 8 bits).
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x75)); // 'u'
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Empty));
                        // \u: h = hex4(buf[r..r+4]) — read 4 bytes, fold
                        let h = self.local_idx("__jge_h");
                        let k = self.local_idx("__jge_k");
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(h));
                        setup.push(Instruction::I64Const(0));
                        setup.push(Instruction::LocalSet(k));
                        setup.push(Instruction::Block(BlockType::Empty));
                        setup.push(Instruction::Loop(BlockType::Empty));
                        setup.push(Instruction::LocalGet(k));
                        setup.push(Instruction::I64Const(4));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::BrIf(1));
                        setup.push(Instruction::LocalGet(h));
                        setup.push(Instruction::I64Const(4));
                        setup.push(Instruction::I64Mul);
                        setup.push(Instruction::LocalSet(h));
                        // n = buf[r+k] → hex digit value
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::LocalGet(k));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::I32Load8U(ma8.clone()));
                        setup.push(Instruction::I64ExtendI32U);
                        setup.push(Instruction::LocalSet(c));
                        // '0'..'9' → -'0'; 'a'..'f' → -'a'+10; 'A'..'F' → -'A'+10
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x30));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x39));
                        setup.push(Instruction::I64LeU);
                        setup.push(Instruction::I32And);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x30));
                        setup.push(Instruction::I64Sub);
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x61));
                        setup.push(Instruction::I64GeU);
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x66));
                        setup.push(Instruction::I64LeU);
                        setup.push(Instruction::I32And);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x57)); // -'a'+10 = -87
                        setup.push(Instruction::I64Sub);
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Const(0x37)); // -'A'+10 = -55
                        setup.push(Instruction::I64Sub);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::LocalSet(c));
                        setup.push(Instruction::LocalGet(h));
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(h));
                        setup.push(Instruction::LocalGet(k));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(k));
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        // r += 4 (hex consumed); write low byte
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::I64Const(4));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(r));
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(h));
                        setup.push(Instruction::I64Const(0xFF));
                        setup.push(Instruction::I64And);
                        setup.push(Instruction::I64Store8(ma8.clone()));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(w));
                        setup.push(Instruction::Else);
                        // simple escapes: " \ / b f n r t
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x22));
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x22));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x5C));
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x5C));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x2F));
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x2F));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x6E)); // n
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x0A));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x74)); // t
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x09));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x72)); // r
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x0D));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x62)); // b
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x08));
                        setup.push(Instruction::Else);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Const(0x66)); // f
                        setup.push(Instruction::I64Eq);
                        setup.push(Instruction::If(BlockType::Result(ValType::I64)));
                        setup.push(Instruction::I64Const(0x0C));
                        setup.push(Instruction::Else);
                        // unknown escape: keep the escaped char as-is
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        setup.push(Instruction::End); // 8th: close the 'f' If
                                                      // buf[w++] = mapped
                        setup.push(Instruction::LocalSet(d)); // d now holds mapped byte
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(d));
                        setup.push(Instruction::I64Store8(ma8.clone()));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(w));
                        setup.push(Instruction::End); // close if-escape
                                                      // plain path: buf[w++] = c; r+=1 — the escape branch
                                                      // already advanced r by 2 (or +6 for \u), so the +1
                                                      // lives ONLY in this else arm.
                        setup.push(Instruction::Else);
                        setup.push(Instruction::I64Const(target_buf));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::I32WrapI64);
                        setup.push(Instruction::LocalGet(c));
                        setup.push(Instruction::I64Store8(ma8.clone()));
                        setup.push(Instruction::LocalGet(w));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(w));
                        setup.push(Instruction::LocalGet(r));
                        setup.push(Instruction::I64Const(1));
                        setup.push(Instruction::I64Add);
                        setup.push(Instruction::LocalSet(r));
                        setup.push(Instruction::End);
                        setup.push(Instruction::Br(0));
                        setup.push(Instruction::End);
                        setup.push(Instruction::End);
                        // scan length = write cursor w
                        let mut len_for_jg = Vec::new();
                        len_for_jg.push(Instruction::LocalGet(w));
                        let mut v =
                            self.json_get_from_buf_any(key, "str", target_buf, &mut len_for_jg)?;
                        // ── result handling: identical tail to json-get-str? ──
                        let jgs_tmp2 = self.local_idx("jgs_packed2");
                        let jgs_len = self.local_idx_i32("jgs_len2");
                        let jgs_ptr = self.local_idx_i32("jgs_ptr2");
                        v.push(Instruction::LocalSet(jgs_tmp2));
                        if nil_on_miss {
                            v.push(Instruction::LocalGet(jgs_tmp2));
                            v.push(Instruction::I64Eqz);
                            v.push(Instruction::If(BlockType::Result(ValType::I64)));
                            v.push(Instruction::I64Const(TAG_NIL));
                            v.push(Instruction::Else);
                        }
                        v.push(Instruction::LocalGet(jgs_tmp2));
                        v.push(Instruction::I64Const(32));
                        v.push(Instruction::I64ShrU);
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalSet(jgs_len));
                        v.push(Instruction::LocalGet(jgs_tmp2));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalSet(jgs_ptr));
                        let jgs_heap2 = self.local_idx("jgs_heap2");
                        let jgs_aligned2 = self.local_idx_i32("jgs_aligned2");
                        let rhp2: i32 = 56;
                        let ma82 = wasm_encoder::MemArg {
                            offset: 0,
                            align: 3,
                            memory_index: 0,
                        };
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::I32Const(7));
                        v.push(Instruction::I32Add);
                        v.push(Instruction::I32Const(-8));
                        v.push(Instruction::I32And);
                        v.push(Instruction::LocalSet(jgs_aligned2));
                        v.push(Instruction::I32Const(rhp2));
                        v.push(Instruction::I64Load(ma82.clone()));
                        v.push(Instruction::LocalSet(jgs_heap2));
                        v.push(Instruction::I32Const(rhp2));
                        v.push(Instruction::LocalGet(jgs_heap2));
                        v.push(Instruction::LocalGet(jgs_aligned2));
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Add);
                        v.push(Instruction::I64Store(ma82));
                        v.push(Instruction::LocalGet(jgs_heap2));
                        v.push(Instruction::I32WrapI64);
                        v.push(Instruction::LocalGet(jgs_ptr));
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::MemoryCopy {
                            src_mem: 0,
                            dst_mem: 0,
                        });
                        v.push(Instruction::LocalGet(jgs_len));
                        v.push(Instruction::I64ExtendI32U);
                        v.push(Instruction::I64Const(32));
                        v.push(Instruction::I64Shl);
                        v.push(Instruction::LocalGet(jgs_heap2));
                        v.push(Instruction::I64Or);
                        v.extend(self.emit_tag_str());
                        if nil_on_miss {
                            v.push(Instruction::End);
                        }
                        v.splice(0..0, setup.iter().cloned());
                        Ok(v)
                    }
                    _ => Err("json-get-str-esc key must be a string literal".into()),
                }
            }
            "json-get-float" => {
                if a.is_empty() {
                    return Err("json-get-float requires a string key argument".into());
                }
                match &a[0] {
                    LispVal::Str(key) => {
                        let mut v = if self.wasi_mode {
                            self.json_get_wasi(key, "float")?
                        } else {
                            self.json_get_with_scanner(key, "float")?
                        };
                        v.extend(self.emit_tag_num());
                        Ok(v)
                    }
                    _ => Err("json-get-float key must be a string literal".into()),
                }
            }
            "json-return" => {
                self.need_host(25);
                let val_expr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.push(Instruction::I32Const(TEMP_MEM as i32));
                v.extend(val_expr);
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Const(TEMP_MEM));
                v.push(Self::host_call(25));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::GlobalSet(RETURN_FLAG));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            "json-extract" => {
                // (json-extract json_str "key0" "key1" ...) → tagged array of values
                if a.len() < 3 {
                    return Err("json-extract requires a JSON string and at least 2 keys".into());
                }
                let n_keys = a.len() - 1;
                if n_keys > 8 {
                    return Err("json-extract supports at most 8 keys".into());
                }
                // Build pattern data for each key: "key":
                let mut v = Vec::new();
                // Evaluate the JSON string argument
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag string
                                              // Push each key pattern as packed (len << 32) | ptr
                for key_arg in &a[1..] {
                    match key_arg {
                        LispVal::Str(key) => {
                            let pat = {
                                // I1-parity (2026-09-14): bare quoted key —
                                // __json_extract_N skips ws after the key
                                // and requires ':' at match time
                                let mut p = vec![b'"'];
                                p.extend(key.as_bytes());
                                p.push(b'"');
                                p
                            };
                            let pat_off = self.alloc_data(&pat) as i64;
                            let pat_len = pat.len() as i64;
                            let pat_packed = (pat_off as u64) | ((pat_len as u64) << 32);
                            v.push(Instruction::I64Const(pat_packed as i64));
                        }
                        _ => return Err("json-extract keys must be string literals".into()),
                    }
                }
                // Call __json_extract_N
                let idx = self.ensure_json_extract_func(n_keys);
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                // Result is already tagged as array (TAG_ARRAY)
                Ok(v)
            }
            "json-extract-input" => {
                // (json-extract-input "key0" "key1" ...) → single-pass
                // multi-key extraction from the TX INPUT (2026-09-14).
                // The lisp json-extract takes the json as arg 0; this
                // variant is what TS jsonExtract lowers to — reads input
                // once, one __json_extract_N pass over the buffer for all
                // keys (N full scans → 1). Returns a TAG_ARRAY of raw
                // span strings ("" for missing keys).
                if a.is_empty() {
                    return Err("json-extract-input requires at least 1 key".into());
                }
                let n_keys = a.len();
                if n_keys > 8 {
                    return Err("json-extract-input supports at most 8 keys".into());
                }
                self.need_host(7);
                self.need_host(0);
                self.need_host(1);
                let mut v = Vec::new();
                // cached input read → INPUT_CACHE_BUF; pack (ilen << 32) | buf
                let ilen_l = self.local_idx_i32("__jei_len");
                self.emit_input_read_cached(ilen_l, &mut v);
                v.push(Instruction::LocalGet(ilen_l));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(INPUT_CACHE_BUF));
                v.push(Instruction::I64Or);
                // key patterns (bare quoted keys — extract_N requires ':'
                // after ws at match time)
                for key_arg in a {
                    match key_arg {
                        LispVal::Str(key) => {
                            let pat = {
                                let mut p = vec![b'"'];
                                p.extend(key.as_bytes());
                                p.push(b'"');
                                p
                            };
                            let pat_off = self.alloc_data(&pat) as i64;
                            let pat_len = pat.len() as i64;
                            let pat_packed = (pat_off as u64) | ((pat_len as u64) << 32);
                            v.push(Instruction::I64Const(pat_packed as i64));
                        }
                        _ => return Err("json-extract-input keys must be string literals".into()),
                    }
                }
                let idx = self.ensure_json_extract_func(n_keys);
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                Ok(v)
            }
            "json-bytes-to-str" | "json-decode-bytes" => {
                if a.len() != 1 {
                    return Err("json-bytes-to-str: expected 1 arg".into());
                }
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag to get packed (len<<32|ptr)
                let idx = self.ensure_json_bytes_to_str_func();
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                // __json_bytes_to_str returns TAGGED str — no need to tag again
                Ok(v)
            }
            "json-array-get" => {
                if a.len() != 2 {
                    return Err("json-array-get: expected 2 args (array-str, index)".into());
                }
                let mut v = Vec::new();
                // Evaluate array string and index
                v.extend(self.expr(&a[0])?); // array string (tagged)
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag to packed (len<<32|ptr)
                v.extend(self.expr(&a[1])?); // index (tagged number)
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU); // untag to raw i64 number
                                              // Save packed and index to locals before any heap manipulation
                let jag_data = self.local_idx("jag_data"); // packed input (len<<32|ptr)
                let jag_idx = self.local_idx("jag_idx"); // raw index
                let jag_inlen = self.local_idx_i32("jag_inlen"); // input string len
                let jag_inalign = self.local_idx_i32("jag_inalign");
                v.push(Instruction::LocalSet(jag_idx)); // pop index
                v.push(Instruction::LocalSet(jag_data)); // pop packed
                                                         // Extract input length: packed >> 32
                v.push(Instruction::LocalGet(jag_data));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(jag_inlen));
                // Align input len to 8
                v.push(Instruction::LocalGet(jag_inlen));
                v.push(Instruction::I32Const(7));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(-8));
                v.push(Instruction::I32And);
                v.push(Instruction::LocalSet(jag_inalign));
                // CRITICAL: Bump the heap pointer PAST the input data BEFORE calling
                // __json_array_get. This ensures that when we later allocate space for
                // the result, it won't overlap with (and overwrite) the input string.
                let rhp: i32 = 56; // RUNTIME_HEAP_PTR
                let ma8 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let jag_oldheap = self.local_idx("jag_oldheap");
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::I64Load(ma8.clone()));
                v.push(Instruction::LocalSet(jag_oldheap));
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::LocalGet(jag_oldheap));
                v.push(Instruction::LocalGet(jag_inalign));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma8.clone()));
                // Now call __json_array_get with the original packed data + index
                let idx = self.ensure_json_array_get_func();
                v.push(Instruction::LocalGet(jag_data));
                v.push(Instruction::LocalGet(jag_idx));
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                // Post-processing: copy result from stdout_buf to heap
                let jag_tmp = self.local_idx("jag_packed");
                let jag_len = self.local_idx_i32("jag_len");
                let jag_ptr = self.local_idx_i32("jag_ptr");
                let jag_heap = self.local_idx("jag_heap");
                let jag_aligned = self.local_idx_i32("jag_aligned");
                v.push(Instruction::LocalSet(jag_tmp));
                v.push(Instruction::LocalGet(jag_tmp));
                v.extend(self.emit_untag()); // untag: >> 3
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // packed >> 32 = len
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(jag_len));
                v.push(Instruction::LocalGet(jag_tmp));
                v.extend(self.emit_untag()); // untag: >> 3
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(jag_ptr));
                // Align result len to 8
                v.push(Instruction::LocalGet(jag_len));
                v.push(Instruction::I32Const(7));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(-8));
                v.push(Instruction::I32And);
                v.push(Instruction::LocalSet(jag_aligned));
                // Read current heap ptr (already bumped past input)
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::I64Load(ma8.clone()));
                v.push(Instruction::LocalSet(jag_heap));
                // Bump: heap_ptr + aligned result len
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::LocalGet(jag_heap));
                v.push(Instruction::LocalGet(jag_aligned));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma8.clone()));
                // memory.copy dst=jag_heap, src=jag_ptr(stdout_buf), len=jag_len
                v.push(Instruction::LocalGet(jag_heap));
                v.push(Instruction::I32WrapI64); // dst as i32
                v.push(Instruction::LocalGet(jag_ptr));
                v.push(Instruction::LocalGet(jag_len));
                v.push(Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });
                // Repack: (len << 32) | jag_old
                v.push(Instruction::LocalGet(jag_len));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(jag_heap)); // already i64
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            // (json-set json key encoded-value) → new JSON object string.
            // json/key/value are runtime strings; encoded-value must already
            // be JSON value text (json-quote output, bare number/bool/null,
            // or raw object/array). __json_set (json.rs) scans the top-level
            // object, replaces the key's value in place (order preserved) or
            // inserts before the final '}'. Empty/invalid json arg → treated
            // as {}. Result is heap-allocated and tagged Str.
            "json-set" => {
                if a.len() != 3 {
                    return Err("json-set: expected 3 args (json key encoded-value)".into());
                }
                let mut v = Vec::new();
                let js_json = self.local_idx("jsv_json");
                let js_key = self.local_idx("jsv_key");
                let js_val = self.local_idx("jsv_val");
                // Eval all three args (tagged), untag each to packed (len<<32|ptr)
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                v.extend(self.expr(&a[2])?);
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64ShrU);
                // Stack: [json, key, val] → store in reverse
                v.push(Instruction::LocalSet(js_val));
                v.push(Instruction::LocalSet(js_key));
                v.push(Instruction::LocalSet(js_json));
                // CRITICAL: Bump the heap pointer PAST the input json BEFORE
                // calling __json_set, so the result allocation below cannot
                // overlap (and overwrite) the input string (same reason as
                // json-array-get).
                let js_inlen = self.local_idx_i32("jsv_inlen");
                let js_inalign = self.local_idx_i32("jsv_inalign");
                v.push(Instruction::LocalGet(js_json));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(js_inlen));
                v.push(Instruction::LocalGet(js_inlen));
                v.push(Instruction::I32Const(7));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(-8));
                v.push(Instruction::I32And);
                v.push(Instruction::LocalSet(js_inalign));
                let rhp: i32 = 56; // RUNTIME_HEAP_PTR
                let ma8 = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let js_oldheap = self.local_idx("jsv_oldheap");
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::I64Load(ma8.clone()));
                v.push(Instruction::LocalSet(js_oldheap));
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::LocalGet(js_oldheap));
                v.push(Instruction::LocalGet(js_inalign));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma8.clone()));
                // Call __json_set(json, key, val) — returns tagged Str in stdout_buf
                let idx = self.ensure_json_set_func();
                v.push(Instruction::LocalGet(js_json));
                v.push(Instruction::LocalGet(js_key));
                v.push(Instruction::LocalGet(js_val));
                v.push(Instruction::Call(crate::wasm_emit::USER_BASE | idx));
                // Post-processing: copy result from stdout_buf to runtime heap
                let js_tmp = self.local_idx("jsv_packed");
                let js_len = self.local_idx_i32("jsv_len");
                let js_ptr = self.local_idx_i32("jsv_ptr");
                let js_heap = self.local_idx("jsv_heap");
                let js_aligned = self.local_idx_i32("jsv_aligned");
                v.push(Instruction::LocalSet(js_tmp));
                v.push(Instruction::LocalGet(js_tmp));
                v.extend(self.emit_untag()); // untag: >> 3
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU); // packed >> 32 = len
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(js_len));
                v.push(Instruction::LocalGet(js_tmp));
                v.extend(self.emit_untag()); // untag: >> 3
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalSet(js_ptr));
                // Align result len to 8
                v.push(Instruction::LocalGet(js_len));
                v.push(Instruction::I32Const(7));
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Const(-8));
                v.push(Instruction::I32And);
                v.push(Instruction::LocalSet(js_aligned));
                // Read current heap ptr (already bumped past input)
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::I64Load(ma8.clone()));
                v.push(Instruction::LocalSet(js_heap));
                // Bump: heap_ptr + aligned result len
                v.push(Instruction::I32Const(rhp));
                v.push(Instruction::LocalGet(js_heap));
                v.push(Instruction::LocalGet(js_aligned));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Store(ma8.clone()));
                // memory.copy dst=js_heap, src=js_ptr(stdout_buf), len=js_len
                v.push(Instruction::LocalGet(js_heap));
                v.push(Instruction::I32WrapI64); // dst as i32
                v.push(Instruction::LocalGet(js_ptr));
                v.push(Instruction::LocalGet(js_len));
                v.push(Instruction::MemoryCopy {
                    src_mem: 0,
                    dst_mem: 0,
                });
                // Repack: (len << 32) | heap
                v.push(Instruction::LocalGet(js_len));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(js_heap)); // already i64
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }

    /// Dynamic-key JSON lookup → tagged STR (or TAG_NIL on miss).
    /// Evaluates the key expression at runtime, writes a `"key":` pattern to
    /// a runtime-heap scratch, calls the shared __json_get scanner, and
    /// heap-copies the result out of the shared stdout_buf scratch so
    /// consecutive dynamic reads don't clobber each other. Miss → TAG_NIL so
    /// `?? fallback` fires (2026-08-31 miss-gate semantics).
    /// Literal-key lookup via the SHARED __json_get (2026-09-14, code
    /// size): data-section pattern + cached-input pack + one Call. Replaces
    /// the ~550-instr inline scanner that used to be embedded at every
    /// literal jsonGetInt/jsonGetStr call site.
    pub(crate) fn json_lit_lookup_str(&mut self, key: &str) -> Vec<Instruction<'static>> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
        let mut pat = vec![b'"'];
        pat.extend(key.as_bytes());
        pat.push(b'"');
        let pat_off = self.alloc_data(&pat) as i64;
        let pat_packed = (pat_off as u64) | ((pat.len() as u64) << 32);
        let ilen_l = self.local_idx_i32("__jli_len");
        let mut v = Vec::new();
        self.emit_input_read_cached(ilen_l, &mut v);
        v.push(Instruction::LocalGet(ilen_l));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(INPUT_CACHE_BUF));
        v.push(Instruction::I64Or);
        v.push(Instruction::I64Const(pat_packed as i64));
        let f = self.ensure_json_get_func();
        v.push(Instruction::Call(crate::wasm_emit::USER_BASE | f));
        // __json_get returns UNtagged (ptr|len<<32) at stdout_buf — same
        // post-processing as the dynamic path: miss(0) → TAG_NIL, else
        // heap-copy + TAG_STR (consumers like emit_int_parse_gated expect
        // the tagged form; the heap copy survives later __json_get calls)
        let res_l = self.local_idx("__jli_res");
        v.push(Instruction::LocalSet(res_l));
        v.push(Instruction::LocalGet(res_l));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(TAG_NIL));
        v.push(Instruction::Else);
        {
            let res_len_l = self.local_idx_i32("__jli_rl");
            let res_ptr_l = self.local_idx_i32("__jli_rp");
            v.push(Instruction::LocalGet(res_l));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64ShrU);
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(res_len_l));
            v.push(Instruction::LocalGet(res_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(res_ptr_l));
            let dst_l = self.local_idx("__jli_dst");
            let alen_l = self.local_idx("__jli_al");
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::LocalSet(alen_l));
            v.extend(self.emit_rtheap_alloc(dst_l, alen_l));
            v.push(Instruction::LocalGet(dst_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalGet(res_ptr_l));
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64Shl);
            v.push(Instruction::LocalGet(dst_l));
            v.push(Instruction::I64Or);
            v.extend(self.emit_tag_str());
        }
        v.push(Instruction::End);
        v
    }

    /// Shared int-parse tail for jsonGetInt (literal + dynamic): span on
    /// stack → miss? nil : (first-byte digit/minus? __str_to_num : nil).
    pub(crate) fn emit_int_parse_gated(
        &mut self,
        lookup: Vec<Instruction<'static>>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let s2n = self.ensure_str_to_num_func();
        let t = self.local_idx("__jgi_miss");
        let fb = self.local_idx_i32("__jgi_fb");
        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        let mut v = lookup;
        v.push(Instruction::LocalSet(t));
        v.push(Instruction::LocalGet(t));
        v.push(Instruction::I64Const(TAG_NIL));
        v.push(Instruction::I64Eq);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(TAG_NIL));
        v.push(Instruction::Else);
        {
            // first byte of the span (untag: payload = t>>3,
            // ptr = payload & 0xFFFFFFFF)
            v.push(Instruction::LocalGet(t));
            v.push(Instruction::I64Const(3));
            v.push(Instruction::I64ShrU);
            v.push(Instruction::I64Const(0xFFFFFFFF));
            v.push(Instruction::I64And);
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I32Load8U(ma8));
            v.push(Instruction::LocalSet(fb));
            // fb in [0x30..=0x39] or == 0x2D ?
            v.push(Instruction::LocalGet(fb));
            v.push(Instruction::I32Const(0x30));
            v.push(Instruction::I32GeS);
            v.push(Instruction::LocalGet(fb));
            v.push(Instruction::I32Const(0x39));
            v.push(Instruction::I32LeS);
            v.push(Instruction::I32And);
            v.push(Instruction::LocalGet(fb));
            v.push(Instruction::I32Const(0x2D));
            v.push(Instruction::I32Eq);
            v.push(Instruction::I32Or);
            v.push(Instruction::If(BlockType::Result(ValType::I64)));
            v.push(Instruction::LocalGet(t));
            v.push(Instruction::Call(crate::wasm_emit::USER_BASE | s2n));
            v.push(Instruction::Else);
            v.push(Instruction::I64Const(TAG_NIL));
            v.push(Instruction::End);
        }
        v.push(Instruction::End);
        Ok(v)
    }

    pub(crate) fn json_dyn_lookup_str(
        &mut self,
        key_ast: &LispVal,
    ) -> Result<Vec<Instruction<'static>>, String> {
        self.need_host(7);
        self.need_host(0);
        self.need_host(1);
        let key_expr = self.expr(key_ast)?;
        let mut v = Vec::new();
        let ib = crate::wasm_emit::INPUT_CACHE_BUF as i64; // cached input (2026-09-14)

        // Store key in local, then extract ptr/len
        let key_l = self.local_idx("__jgd_key");
        v.extend(key_expr);
        v.push(Instruction::LocalSet(key_l));

        let key_ptr_l = self.local_idx_i32("__jgd_kp");
        v.push(Instruction::LocalGet(key_l));
        v.extend(self.emit_untag());
        v.push(Instruction::I64Const(0xFFFFFFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalSet(key_ptr_l));

        let key_len_l = self.local_idx_i32("__jgd_kl");
        v.push(Instruction::LocalGet(key_l));
        v.extend(self.emit_untag());
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalSet(key_len_l));

        // Cached input read; the packed buffer constant below matches
        let ilen_l = self.local_idx_i32("__jgd_il");
        self.emit_input_read_cached(ilen_l, &mut v);

        // Alloc scratch for pattern: quote + key + quote — the BARE
        // quoted key (I1-parity 2026-09-14): __json_get skips ws after
        // the key and requires ':' at match time. Scratch comes from the
        // monotonic runtime heap (mem[56]) via emit_rtheap_alloc.
        let pat_len_l = self.local_idx("__jgd_pl");
        v.push(Instruction::LocalGet(key_len_l));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(2));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(pat_len_l));

        let scr_l = self.local_idx("__jgd_sc");
        v.extend(self.emit_rtheap_alloc(scr_l, pat_len_l));

        let ma8 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };

        // scr[0] = '"'
        v.push(Instruction::LocalGet(scr_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Const(0x22));
        v.push(Instruction::I32Store8(ma8.clone()));

        // copy key bytes to scr[1..1+len]
        let ci_l = self.local_idx_i32("__jgd_ci");
        v.push(Instruction::I32Const(0));
        v.push(Instruction::LocalSet(ci_l));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(ci_l));
        v.push(Instruction::LocalGet(key_len_l));
        v.push(Instruction::I32GeS);
        v.push(Instruction::If(BlockType::Empty));
        // depth from inside this If: 0=If, 1=Loop, 2=Block —
        // Br(2) EXITS the block (Br(1) would restart the loop)
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(scr_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(ci_l));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalGet(key_ptr_l));
        v.push(Instruction::LocalGet(ci_l));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Load8U(ma8.clone()));
        v.push(Instruction::I32Store8(ma8.clone()));
        v.push(Instruction::LocalGet(ci_l));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::LocalSet(ci_l));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);

        // scr[1+len] = '"' — bare quoted key, no glued colon (I1-parity:
        // __json_get skips ws and requires ':' at match time)
        v.push(Instruction::LocalGet(scr_l));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(key_len_l));
        v.push(Instruction::I32Const(1));
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Add);
        v.push(Instruction::I32Const(0x22));
        v.push(Instruction::I32Store8(ma8.clone()));

        // Call __json_get(json=ib|ilen<<32, pat=scr|pat_len<<32)
        // (2 packed i64 params — matches ensure_json_get_func's ABI)
        let get_fn = self.ensure_json_get_func();
        v.push(Instruction::LocalGet(ilen_l));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(ib));
        v.push(Instruction::I64Or);
        v.push(Instruction::LocalGet(pat_len_l));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(scr_l));
        v.push(Instruction::I64Or);
        v.push(Instruction::Call(crate::wasm_emit::USER_BASE | get_fn));

        // __json_get returns UNtagged (ptr|len<<32) pointing at
        // stdout_buf — a SHARED scratch that the next __json_get
        // call overwrites (see json_get_wasi's heap-copy). Heap-
        // copy the result before tagging, so multiple dynamic
        // reads in one function don't clobber each other.
        // Miss (0) → TAG_NIL so `?? fallback` fires (2026-08-31
        // miss-gate semantics, same as the literal path).
        let res_l = self.local_idx("__jgd_res");
        v.push(Instruction::LocalSet(res_l));
        v.push(Instruction::LocalGet(res_l));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::I64Const(TAG_NIL));
        v.push(Instruction::Else);
        {
            let res_len_l = self.local_idx_i32("__jgd_rl");
            let res_ptr_l = self.local_idx_i32("__jgd_rp");
            v.push(Instruction::LocalGet(res_l));
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64ShrU);
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(res_len_l));
            v.push(Instruction::LocalGet(res_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalSet(res_ptr_l));
            let dst_l = self.local_idx("__jgd_dst");
            let alen_l = self.local_idx("__jgd_al");
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::LocalSet(alen_l));
            v.extend(self.emit_rtheap_alloc(dst_l, alen_l));
            v.push(Instruction::LocalGet(dst_l));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::LocalGet(res_ptr_l));
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            });
            // repack (res_len << 32 | dst) and tag as Str
            v.push(Instruction::LocalGet(res_len_l));
            v.push(Instruction::I64ExtendI32U);
            v.push(Instruction::I64Const(32));
            v.push(Instruction::I64Shl);
            v.push(Instruction::LocalGet(dst_l));
            v.push(Instruction::I64Or);
            v.extend(self.emit_tag_str());
        }
        v.push(Instruction::End);
        Ok(v)
    }
}
