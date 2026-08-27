//! List-returning string builtins: str-split / str-split-exact / str-chunk /
//! string->list (all → list of zero-copy views into the source string) and
//! str-join / list->string (list → new string via __to_string per element,
//! mirroring interp's stringify semantics).
//!
//! ASCII-bounded: wasm works in bytes; interp splits Rust chars (Unicode).
//! Divergence documented in corpus/COVERAGE.md §A.1. Algorithms machine-
//! checked against interp/Rust semantics (2026-08-27).

use super::*;

impl WasmEmitter {
    /// Domain entry (wired into call.rs try_domain! chain). Unknown ops must
    /// return "__not_handled__" so the dispatcher falls through.
    pub(crate) fn call_string_list(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "str-split" => {
                if a.len() != 2 {
                    return Err("str-split: expected 2 args (s, delimiter)".into());
                }
                self.str_split_emit(a, false)
            }
            "str-split-exact" => {
                if a.len() != 2 {
                    return Err("str-split-exact: expected 2 args (s, delimiter)".into());
                }
                self.str_split_emit(a, true)
            }
            "str-chunk" => {
                if a.len() != 2 {
                    return Err("str-chunk: expected 2 args (s, n)".into());
                }
                self.str_chunk(a)
            }
            "string->list" => {
                if a.len() != 1 {
                    return Err("string->list: expected 1 arg".into());
                }
                self.str_to_list(a)
            }
            "str-join" => {
                if a.len() != 2 {
                    return Err("str-join: expected 2 args (separator, list)".into());
                }
                self.str_join_emit(a, 1)
            }
            "list->string" => {
                if a.len() != 1 {
                    return Err("list->string: expected 1 arg".into());
                }
                self.str_join_emit(a, 0)
            }
            _ => Err("__not_handled__".into()),
        }
    }

    /// Bump alloc with RUNTIME byte count from local `count_i`. Pushes old ptr.
    fn emit_runtime_alloc_dyn(&mut self, count_i: u32) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx("__rad_tmp");
        let new_ptr = self.local_idx("__rad_new");
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let mem_limit = (self.memory_pages as i64) * 65536;
        vec![
            Instruction::I64Const(56),
            Instruction::I32WrapI64,
            Instruction::I64Load(ma8.clone()),
            Instruction::LocalSet(tmp),
            Instruction::LocalGet(tmp),
            Instruction::LocalGet(count_i),
            Instruction::I64Add,
            Instruction::LocalSet(new_ptr),
            Instruction::LocalGet(new_ptr),
            Instruction::I64Const(mem_limit),
            Instruction::I64LtU,
            Instruction::If(BlockType::Empty),
            Instruction::I64Const(56),
            Instruction::I32WrapI64,
            Instruction::LocalGet(new_ptr),
            Instruction::I64Store(ma8),
            Instruction::Else,
            Instruction::Unreachable,
            Instruction::End,
            Instruction::LocalGet(tmp),
        ]
    }

    /// Prologue: eval arg → raw string → (len, ptr) locals. Returns instructions.
    fn str_unwrap(&mut self, arg: &LispVal, pfx: &str) -> (Vec<Instruction<'static>>, u32, u32) {
        let raw_i = self.local_idx(&format!("__{}_raw", pfx));
        let len_i = self.local_idx(&format!("__{}_len", pfx));
        let ptr_i = self.local_idx(&format!("__{}_ptr", pfx));
        let mut v = Vec::new();
        v.extend(self.expr(arg).expect("str_unwrap: expr"));
        v.extend(self.emit_untag());
        v.push(Instruction::LocalSet(raw_i));
        v.push(Instruction::LocalGet(raw_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::LocalSet(len_i));
        v.push(Instruction::LocalGet(raw_i));
        v.push(Instruction::I64Const(0xFFFF_FFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalSet(ptr_i));
        (v, len_i, ptr_i)
    }

    /// (string->list s) → list of 1-char views. len 0 → empty list.
    fn str_to_list(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let (mut v, len_i, ptr_i) = self.str_unwrap(&a[0], "s2l");
        let lp_i = self.local_idx("__s2l_lp");
        let cnt_i = self.local_idx("__s2l_cnt");
        let i_i = self.local_idx("__s2l_i");
        // lp = alloc((1+len)*8)
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalSet(cnt_i));
        v.extend(self.emit_runtime_alloc_dyn(cnt_i));
        v.push(Instruction::LocalSet(lp_i));
        // [lp] = len
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Store(ma8.clone()));
        // [lp + 8 + 8i] = tag_str((1<<32) | (ptr+i))
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        // addr
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        // value
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl); // 1<<32
        v.push(Instruction::LocalGet(ptr_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Or); // (1<<32)|(ptr+i)
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(5));
        v.push(Instruction::I64Or); // tag STR
        v.push(Instruction::I64Store(ma8.clone()));
        // i++
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        // tag_array(lp)
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(6));
        v.push(Instruction::I64Or);
        Ok(v)
    }

    /// Emits one split-walk over s: on each delimiter match and at the tail,
    /// runs `action(v, seg_start_local, seg_end_local)` where the segment is
    /// [seg_start, seg_end). Caller re-initializes i/start locals before
    /// each walk. Locals i_i/start_i/j_i/ism_i/len_i/ptr_i must be from pfx.
    ///
    /// Walk invariant (matches the machine-checked port):
    ///   i=0,start=0; while i+dlen<=len: match@i ? {part[start,i); i+=dlen; start=i} : i++
    ///   tail: part [start,len)
    /// `filter_empties` ⇒ action runs only when seg_end > seg_start.
    fn str_split_walk(
        &mut self,
        pfx: &str,
        dlen: i64,
        d_base: u32,
        filter_empties: bool,
        action: SplitAction,
    ) -> Vec<Instruction<'static>> {
        let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let i_i = self.local_idx(&format!("__{}_i", pfx));
        let start_i = self.local_idx(&format!("__{}_start", pfx));
        let j_i = self.local_idx(&format!("__{}_j", pfx));
        let ism_i = self.local_idx(&format!("__{}_ism", pfx));
        let len_i = self.local_idx(&format!("__{}_len", pfx));
        let ptr_i = self.local_idx(&format!("__{}_ptr", pfx));
        let mut v = Vec::new();
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        // i + dlen > len → exit to tail
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(dlen));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64GtU);
        v.push(Instruction::BrIf(1));
        // match check at i
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(ism_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(j_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(j_i));
        v.push(Instruction::I64Const(dlen));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        v.push(Instruction::LocalGet(ptr_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(j_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma0.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Const(d_base as i64));
        v.push(Instruction::LocalGet(j_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma0.clone()));
        v.push(Instruction::I64ExtendI32U);
        v.push(Instruction::I64Ne);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(ism_i));
        v.push(Instruction::Br(2));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(j_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(j_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        // ism ? { action(start, i); i+=dlen; start=i } : i++
        v.push(Instruction::LocalGet(ism_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::If(BlockType::Empty));
        match action {
            SplitAction::Count => {
                // m += (filter_empties ? (i>start) : 1)
                if filter_empties {
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty));
                    let m_i = self.local_idx(&format!("__{}_m", pfx));
                    v.push(Instruction::LocalGet(m_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(m_i));
                    v.push(Instruction::End);
                } else {
                    let m_i = self.local_idx(&format!("__{}_m", pfx));
                    v.push(Instruction::LocalGet(m_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(m_i));
                }
            }
            SplitAction::Fill => {
                // if (!filter_empties || i>start): [lp+8+8w] = view(start,i); w++
                let emit_fill = |v: &mut Vec<Instruction<'static>>, emitter: &mut Self| {
                    let w_i = emitter.local_idx(&format!("__{}_w", pfx));
                    let lp_i = emitter.local_idx(&format!("__{}_lp", pfx));
                    v.push(Instruction::LocalGet(lp_i));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(w_i));
                    v.push(Instruction::I64Const(8));
                    v.push(Instruction::I64Mul);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I32WrapI64);
                    // value = tag_str(((i-start)<<32) | (ptr+start))
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::I64Sub);
                    v.push(Instruction::I64Const(32));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(ptr_i));
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64Const(3));
                    v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Const(5));
                    v.push(Instruction::I64Or);
                    v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                    v.push(Instruction::LocalGet(w_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(w_i));
                };
                if filter_empties {
                    v.push(Instruction::LocalGet(i_i));
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::I64GtU);
                    v.push(Instruction::If(BlockType::Empty));
                    emit_fill(&mut v, self);
                    v.push(Instruction::End);
                } else {
                    emit_fill(&mut v, self);
                }
            }
        }
        // i += dlen; start = i
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(dlen));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalSet(start_i));
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::End);
        v.push(Instruction::Br(0));
        v.push(Instruction::End); // Loop
        v.push(Instruction::End); // Block
        // tail: action(start, len)
        let tail_guard = filter_empties; // only emit part when non-empty
        match action {
            SplitAction::Count => {
                let m_i = self.local_idx(&format!("__{}_m", pfx));
                if tail_guard {
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64LtU);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::LocalGet(m_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(m_i));
                    v.push(Instruction::End);
                } else {
                    v.push(Instruction::LocalGet(m_i));
                    v.push(Instruction::I64Const(1));
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(m_i));
                }
            }
            SplitAction::Fill => {
                let mut fill = Vec::new();
                let w_i = self.local_idx(&format!("__{}_w", pfx));
                let lp_i = self.local_idx(&format!("__{}_lp", pfx));
                fill.push(Instruction::LocalGet(lp_i));
                fill.push(Instruction::I64Const(8));
                fill.push(Instruction::I64Add);
                fill.push(Instruction::LocalGet(w_i));
                fill.push(Instruction::I64Const(8));
                fill.push(Instruction::I64Mul);
                fill.push(Instruction::I64Add);
                fill.push(Instruction::I32WrapI64);
                fill.push(Instruction::LocalGet(len_i));
                fill.push(Instruction::LocalGet(start_i));
                fill.push(Instruction::I64Sub);
                fill.push(Instruction::I64Const(32));
                fill.push(Instruction::I64Shl);
                fill.push(Instruction::LocalGet(ptr_i));
                fill.push(Instruction::LocalGet(start_i));
                fill.push(Instruction::I64Add);
                fill.push(Instruction::I64Or);
                fill.push(Instruction::I64Const(3));
                fill.push(Instruction::I64Shl);
                fill.push(Instruction::I64Const(5));
                fill.push(Instruction::I64Or);
                fill.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                fill.push(Instruction::LocalGet(w_i));
                fill.push(Instruction::I64Const(1));
                fill.push(Instruction::I64Add);
                fill.push(Instruction::LocalSet(w_i));
                if tail_guard {
                    v.push(Instruction::LocalGet(start_i));
                    v.push(Instruction::LocalGet(len_i));
                    v.push(Instruction::I64LtU);
                    v.push(Instruction::If(BlockType::Empty));
                    v.extend(fill);
                    v.push(Instruction::End);
                } else {
                    v.extend(fill);
                }
            }
        }
        v
    }

    /// (str-split s delim) filter_empties=false → interp semantics (Rust
    /// split, empties dropped). (str-split-exact ...) keep all parts.
    fn str_split_emit(&mut self, a: &[LispVal], keep_empties: bool) -> Result<Vec<Instruction<'static>>, String> {
        let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let d_str = match &a[1] {
            LispVal::Str(s) => s.clone(),
            _ => return Err("str-split: delimiter must be a string literal".into()),
        };
        if d_str.is_empty() {
            if keep_empties {
                return Err("str-split-exact: empty delimiter unsupported in wasm (interp yields between-char empties)".into());
            }
            return self.str_to_list(a); // interp: per-char split
        }
        let d_bytes = d_str.as_bytes();
        let dlen = d_bytes.len() as i64;
        let d_base = self.next_data_offset.max(4096);
        self.next_data_offset = ((d_base as u64 + d_bytes.len() as u64 + 8) & !7) as u32;
        let mut v = Vec::new();
        for (j, &b) in d_bytes.iter().enumerate() {
            v.push(Instruction::I64Const(d_base as i64 + j as i64));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Const(b as i64));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I32Store8(ma0.clone()));
        }
        let (mut body, len_i, _ptr_i) = self.str_unwrap(&a[0], "ssl");
        v.append(&mut body);
        let i_i = self.local_idx("__ssl_i");
        let start_i = self.local_idx("__ssl_start");
        let m_i = self.local_idx("__ssl_m");
        let w_i = self.local_idx("__ssl_w");
        let lp_i = self.local_idx("__ssl_lp");
        let cnt_i = self.local_idx("__ssl_cnt");
        // pass 1
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::LocalSet(start_i));
        v.push(Instruction::LocalSet(m_i));
        v.extend(self.str_split_walk("ssl", dlen, d_base, !keep_empties, SplitAction::Count));
        // alloc + store count
        v.push(Instruction::LocalGet(m_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalSet(cnt_i));
        v.extend(self.emit_runtime_alloc_dyn(cnt_i));
        v.push(Instruction::LocalSet(lp_i));
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(m_i));
        v.push(Instruction::I64Store(ma8.clone()));
        // pass 2
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::LocalSet(start_i));
        v.push(Instruction::LocalSet(w_i));
        v.extend(self.str_split_walk("ssl", dlen, d_base, !keep_empties, SplitAction::Fill));
        // result tag_array(lp)
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(6));
        v.push(Instruction::I64Or);
        Ok(v)
    }

    /// (str-chunk s n) — interp port: n pieces, piece size ceil(total/n).
    /// n <= 0 traps (interp errors on n=0).
    fn str_chunk(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let (mut v, len_i, ptr_i) = self.str_unwrap(&a[0], "schk");
        let n_i = self.local_idx("__schk_n");
        let cs_i = self.local_idx("__schk_cs"); // chunk size
        let m_i = self.local_idx("__schk_m");
        let lp_i = self.local_idx("__schk_lp");
        let cnt_i = self.local_idx("__schk_cnt");
        let st_i = self.local_idx("__schk_st"); // seg start
        let en_i = self.local_idx("__schk_en"); // seg end
        let i_i = self.local_idx("__schk_i");
        // eval n
        v.extend(self.expr(&a[1])?);
        v.extend(self.emit_untag());
        v.push(Instruction::LocalSet(n_i));
        // n <= 0 → trap
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64LtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::Unreachable);
        v.push(Instruction::End);
        // cs = (len + n - 1) / n
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::I64DivU);
        v.push(Instruction::LocalSet(cs_i));
        // cs == 0 → m = min(n, len+1), all parts ""
        // else     → m = (len + cs - 1) / cs
        v.push(Instruction::LocalGet(cs_i));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        // then: m = min(n, len+1)
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64GtU); // n > len+1 ?
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(n_i));
        v.push(Instruction::End);
        v.push(Instruction::LocalSet(m_i));
        v.push(Instruction::Else);
        // else: m = (len + cs - 1) / cs
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::LocalGet(cs_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::LocalGet(cs_i));
        v.push(Instruction::I64DivU);
        v.push(Instruction::LocalSet(m_i));
        v.push(Instruction::End);
        // alloc (1+m)*8; [lp]=m
        v.push(Instruction::LocalGet(m_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalSet(cnt_i));
        v.extend(self.emit_runtime_alloc_dyn(cnt_i));
        v.push(Instruction::LocalSet(lp_i));
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(m_i));
        v.push(Instruction::I64Store(ma8.clone()));
        // fill: i=0; while i<m: st=i*cs; en=min(st+cs,len); view(st,en)
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(m_i));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        // st = i*cs
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(cs_i));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalSet(st_i));
        // en = st+cs; if en>len → en=len
        v.push(Instruction::LocalGet(st_i));
        v.push(Instruction::LocalGet(cs_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(en_i));
        v.push(Instruction::LocalGet(en_i));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::I64GtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(len_i));
        v.push(Instruction::LocalSet(en_i));
        v.push(Instruction::End);
        // store view tag_str(((en-st)<<32)|(ptr+st))
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(en_i));
        v.push(Instruction::LocalGet(st_i));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(ptr_i));
        v.push(Instruction::LocalGet(st_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Or);
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(5));
        v.push(Instruction::I64Or);
        v.push(Instruction::I64Store(ma8.clone()));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(lp_i));
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(6));
        v.push(Instruction::I64Or);
        Ok(v)
    }

    /// str-join(sep-literal, list) and list->string(list) — shared: elements
    /// passed through __to_string (matches interp's stringify), converted
    /// raws cached in a temp array so conversion runs once.
    fn str_join_emit(&mut self, a: &[LispVal], list_arg_idx: usize) -> Result<Vec<Instruction<'static>>, String> {
        let ma0 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        // sep literal (may be "" for list->string)
        let (sep_bytes, sep_base): (Vec<u8>, u32) = if list_arg_idx == 1 {
            match &a[0] {
                LispVal::Str(s) => {
                    let b = s.as_bytes().to_vec();
                    let base = self.next_data_offset.max(4096);
                    self.next_data_offset = ((base as u64 + b.len() as u64 + 8) & !7) as u32;
                    (b, base)
                }
                _ => return Err("str-join: separator must be a string literal".into()),
            }
        } else {
            (Vec::new(), 0)
        };
        let seplen = sep_bytes.len() as i64;
        let mut v = Vec::new();
        for (j, &b) in sep_bytes.iter().enumerate() {
            v.push(Instruction::I64Const(sep_base as i64 + j as i64));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I64Const(b as i64));
            v.push(Instruction::I32WrapI64);
            v.push(Instruction::I32Store8(ma0.clone()));
        }
        let arr_i = self.local_idx("__sjn_arr");
        let cnt_i = self.local_idx("__sjn_cnt");
        let tp_i = self.local_idx("__sjn_tp"); // temp raw array
        let i_i = self.local_idx("__sjn_i");
        let e_i = self.local_idx("__sjn_e");
        let tot_i = self.local_idx("__sjn_tot");
        let dst_i = self.local_idx("__sjn_dst");
        let o_i = self.local_idx("__sjn_o");
        let k_i = self.local_idx("__sjn_k");
        let b_i = self.local_idx("__sjn_b");
        // eval list → untag → arr (0 = nil)
        v.extend(self.expr(&a[list_arg_idx])?);
        v.extend(self.emit_untag());
        v.push(Instruction::LocalSet(arr_i));
        // count = arr==0 ? 0 : load[arr]
        v.push(Instruction::LocalGet(arr_i));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(cnt_i));
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(arr_i));
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::LocalSet(cnt_i));
        v.push(Instruction::End);
        // count == 0 → result "" (len 0, ptr 0)
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64Eqz);
        v.push(Instruction::If(BlockType::Empty));
        // (will fall through with tot=0; handled by generic path)
        v.push(Instruction::End);
        // temp raw array alloc: cnt*8
        let alloc_cnt = self.local_idx("__sjn_ac");
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::LocalSet(alloc_cnt));
        v.extend(self.emit_runtime_alloc_dyn(alloc_cnt));
        v.push(Instruction::LocalSet(tp_i));
        // pass 1: convert + sum lengths
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::LocalSet(tot_i));
        let ts_idx = self.ensure_to_string_func();
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        // e = to_string(load[arr + 8 + 8i]) → untag → raw
        v.push(Instruction::LocalGet(arr_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::Call(crate::wasm_emit::USER_BASE | ts_idx));
        v.extend(self.emit_untag());
        v.push(Instruction::LocalSet(e_i));
        // tp[i] = e
        v.push(Instruction::LocalGet(tp_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(e_i));
        v.push(Instruction::I64Store(ma8.clone()));
        // tot += e>>32
        v.push(Instruction::LocalGet(tot_i));
        v.push(Instruction::LocalGet(e_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(tot_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        // tot += seplen * (cnt-1) if cnt>0
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64GtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::LocalGet(tot_i));
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64Const(seplen));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(tot_i));
        v.push(Instruction::End);
        // dst = alloc(tot)
        v.extend(self.emit_runtime_alloc_dyn(tot_i));
        v.push(Instruction::LocalSet(dst_i));
        // pass 2: copy elements + seps
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::LocalSet(o_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        // e = tp[i]; copy e bytes to dst+o (byte loop k)
        v.push(Instruction::LocalGet(tp_i));
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(8));
        v.push(Instruction::I64Mul);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Load(ma8.clone()));
        v.push(Instruction::LocalSet(e_i));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(k_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::LocalGet(e_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::LocalGet(o_i));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::LocalGet(e_i));
        v.push(Instruction::I64Const(0xFFFF_FFFF));
        v.push(Instruction::I64And);
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma0.clone()));
        v.push(Instruction::I32Store8(ma0.clone()));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(k_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        // o += elen
        v.push(Instruction::LocalGet(o_i));
        v.push(Instruction::LocalGet(e_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64ShrU);
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(o_i));
        // if i < cnt-1: copy sep (k loop over seplen)
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::LocalGet(cnt_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Sub);
        v.push(Instruction::I64LtU);
        v.push(Instruction::If(BlockType::Empty));
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(k_i));
        v.push(Instruction::Block(BlockType::Empty));
        v.push(Instruction::Loop(BlockType::Empty));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Const(seplen));
        v.push(Instruction::I64GeU);
        v.push(Instruction::BrIf(1));
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::LocalGet(o_i));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I64Const(sep_base as i64));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Add);
        v.push(Instruction::I32WrapI64);
        v.push(Instruction::I32Load8U(ma0.clone()));
        v.push(Instruction::I32Store8(ma0.clone()));
        v.push(Instruction::LocalGet(k_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(k_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(o_i));
        v.push(Instruction::I64Const(seplen));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(o_i));
        v.push(Instruction::End);
        v.push(Instruction::LocalGet(i_i));
        v.push(Instruction::I64Const(1));
        v.push(Instruction::I64Add);
        v.push(Instruction::LocalSet(i_i));
        v.push(Instruction::Br(0));
        v.push(Instruction::End);
        v.push(Instruction::End);
        // result tag_str((o<<32)|dst)
        v.push(Instruction::LocalGet(o_i));
        v.push(Instruction::I64Const(32));
        v.push(Instruction::I64Shl);
        v.push(Instruction::LocalGet(dst_i));
        v.push(Instruction::I64Or);
        v.push(Instruction::I64Const(3));
        v.push(Instruction::I64Shl);
        v.push(Instruction::I64Const(5));
        v.push(Instruction::I64Or);
        let _ = b_i; // reserved
        Ok(v)
    }
}

enum SplitAction {
    Count,
    Fill,
}
