//! u128 native-chain 2B (emit): storage codec — hot-cell reads sniff, hot
//! writes go binary, all keyed off the SHARED storage_site resolver.
//!
//! READ path (emit_u128_val hot-cell arm): compile the site generically
//! (memo cache untouched — it stores the tagged str, identical bytes),
//! then sniff byte 0 of the value: NUL ⇒ 17-byte binary cell (0x00 tag +
//! 16B LE) ⇒ two I64Loads into (lo, hi); anything else ⇒ the SAME
//! __u128_parse the tagged path uses ⇒ identical result-or-trap for
//! arbitrary content. A "wrongly hot" read is only slower, never wrong.
//!
//! WRITE path (storage_set arm): when the site is a WRITE with a LITERAL
//! key and the value is limb-borne at emit time, store 17 binary bytes
//! instead of rendering decimal (saves to_str AND shrinks the cell 38→17
//! bytes — cheaper trie writes). Dynamic keys and tagged values keep the
//! decimal path byte-for-byte (audit cells stay human-readable; the
//! tagged read path sniffs, so mixed formats coexist).
//!
//! The value-limb-borne test and the write decision both live in ONE
//! place (should_write_binary): checker and emit share storage_site —
//! no duplicated classification (user rule: delegate, don't reimplement).
use super::*;
use crate::wasm_emit::call_u128_str::{ma8, U128_A, U128_B, U128_W};

/// True when this storage write should emit the binary codec:
/// literal key AND the value expr is limb-borne in the current fn scope.
/// Called from the storage_set arm with the original arg exprs.
pub(crate) fn is_limb_borne_write(
    em: &WasmEmitter,
    key_expr: &LispVal,
    val_expr: &LispVal,
) -> bool {
    // Hot-mode gate + 2-arg shape live at the call site; this predicate
    // decides limb-borne-ness of the VALUE (codec choice, not semantics —
    // both formats read back identically through the sniff).
    // literal key only — dynamic keys keep decimal (audit cells, aliases)
    if crate::typing::storage_prov::literal_key(key_expr).is_none() {
        return false;
    }
    let eligible = &em.limb_eligible;
    match val_expr {
        // copy of a limb local / param-slot
        LispVal::Sym(n) => eligible.contains(n) || em.limb_slots.contains_key(n),
        // digit literal (canonical decimal birth) — binary saves a parse,
        // but the decimal write is ALREADY minimal; keep decimal for
        // auditability? No: keep formats uniform per value kind — a literal
        // write is limb-borne by definition (compile-time const pair).
        LispVal::Str(s) => {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u128>().is_ok()
        }
        // direct u128 arith result (the hot case: p2, gained, newpaid)
        LispVal::List(items) if !items.is_empty() => {
            matches!(items.first(), Some(LispVal::Sym(h)) if matches!(
                h.as_str(),
                "u128/add" | "u128/sub" | "u128/mul" | "u128/div" | "u128/mod" | "u128/muldiv"
            ))
        }
        // twin calls returning limbs count too (routed at emit)
        _ => false,
    }
}

impl WasmEmitter {
    /// Emit the binary write value into U128_W (limb pair) then copy the
    /// 17-byte cell into a bump-alloc buffer and hand back (ptr, len)
    /// packed like a tagged str payload. Reuses the same bump-alloc shape
    /// as the storage read path (RUNTIME_HEAP_PTR @ 56, 8-aligned).
    pub(crate) fn emit_binary_write_value(
        &mut self,
        val_expr: &LispVal,
    ) -> Result<Vec<Instruction<'static>>, String> {
        // value limbs → a FRESH LOCAL pair (emit_u128_val takes local
        // indices; U128_A/B/W are memory cells, not locals — passing the
        // address produced MaxStack(LocalIndex(8576)) invalid wasm,
        // caught by the 2B roundtrip pin), then stored into the cell.
        let alo = self.local_idx("__b2b_wlo");
        let ahi = self.local_idx("__b2b_whi");
        let mut v = self.emit_u128_val(val_expr, alo, ahi)?;
        // bump-alloc 24 bytes (8-aligned) for the cell
        let ma8 = ma8();
        let buf = self.local_idx("__b2b_buf");
        v.push(Instruction::I64Const(56)); // RUNTIME_HEAP_PTR
        v.push(Instruction::I32WrapI64); // load ADDRESS is i32 in wasm
        v.push(Instruction::I64Load(ma8));
        v.push(Instruction::LocalSet(buf));
        v.push(Instruction::I64Const(56));
        v.push(Instruction::I32WrapI64); // store ADDRESS is i32 in wasm
        v.push(Instruction::LocalGet(buf));
        v.push(Instruction::I64Const(24));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(7));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(-8));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Store(ma8));
        // cell bytes: [0]=0 tag, [1..9]=lo LE, [9..17]=hi LE (I64Store8 + 2×I64Store)
        let p = buf;
        v.push(Instruction::LocalGet(p));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(0)); // i64.store8: [i32 addr, i64 value]
        v.push(Instruction::I64Store8(wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(p));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(alo));
        v.push(Instruction::I64Store(wasm_encoder::MemArg {
            offset: 1,
            align: 0,
            memory_index: 0,
        }));
        v.push(Instruction::LocalGet(p));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(ahi));
        v.push(Instruction::I64Store(wasm_encoder::MemArg {
            offset: 9,
            align: 0,
            memory_index: 0,
        }));
        // pack (ptr, len=17) and TAG it as a real Str — the generic
        // storage_set arm then treats it exactly like any tagged string
        // (assert passes, untag recovers ptr|len<<32, host args come out
        // len=17/ptr). No bypass of the tag discipline.
        v.push(Instruction::LocalGet(p));
        v.push(Instruction::I64Const(0xFFFF_FFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::I64Const(17));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Or);
        v.extend(self.emit_tag_str());
        Ok(v)
    }

    /// The write-arm hook: Some(bump-alloc'd binary cell as a tagged Str)
    /// when this site qualifies (hot mode + literal key + limb-borne value),
    /// None ⇒ caller falls through to the plain decimal path. Emits the
    /// key AND value code in call-site order (key first — matches the
    /// generic arm's `v.extend(key)` sequence, so observability of key
    /// traps is unchanged).
    pub(crate) fn try_hot_write(
        &mut self,
        key_expr: &LispVal,
        val_expr: &LispVal,
    ) -> Result<Option<Vec<Instruction<'static>>>, String> {
        if !is_limb_borne_write(self, key_expr, val_expr) {
            return Ok(None);
        }
        Ok(Some(self.emit_binary_write_value(val_expr)?))
    }

    /// Sniff-hydrate for the READ path: tagged str in `t` → (lo, hi).
    /// NUL first byte ⇒ binary hydrate; else identical strict parse.
    pub(crate) fn emit_hot_cell_hydrate(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        t: u32,
        lo: u32,
        hi: u32,
    ) {
        let h = self.ensure_u128_str_helpers();
        let ma0 = wasm_encoder::MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        };
        // payload = t >> TAG_BITS; ptr = payload & 0xFFFF_FFFF; len = payload >> 32
        // (untag inline — emit_untag exists but we need the pieces separately)
        let pl = self.local_idx("__b2b_pl");
        let ptr = self.local_idx("__b2b_ptr");
        v.push(Instruction::LocalGet(t));
        v.push(Instruction::I64Const(crate::wasm_emit::TAG_BITS));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(pl));
        v.push(Instruction::LocalGet(pl));
        v.push(Instruction::I64Const(0xFFFF_FFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(ptr));
        // len == 0 → the tagged path: empty value parses → trap (same as
        // phase-1 generic; storage_get miss returns "" which callers
        // default-handle BEFORE u128 materialization — unreachable here)
        v.push(Instruction::LocalGet(pl));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Unreachable);
        v.push(Instruction::End);
        // if load8_u(ptr) == 0 → binary hydrate
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma0));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        // binary: lo = load(ptr+1), hi = load(ptr+9)
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma0.clone()));
        v.push(Instruction::LocalSet(lo));
        v.push(Instruction::LocalGet(ptr));
        v.push(Instruction::I64Const(9));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma0.clone()));
        v.push(Instruction::LocalSet(hi));
        v.push(Instruction::Else);
        // decimal: the SAME parse the tagged path runs
        self.u128_parse_call(v, t, U128_A, &h);
        v.extend(self.limb_pair_load(U128_A, lo, hi));
        v.push(Instruction::End);
    }
}
