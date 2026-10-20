//! u128 native-chain phase 2 — param-provenance twins (TASK-u128-native-chain-2.md, lever 2A).
//!
//! Phase 1 kept values in limb form INSIDE a function, but every user-fn
//! param arrived as a tagged decimal string, so param-threaded hot paths
//! re-parsed at every crossing (live A/B 2026-10-13: only ~2–4% gas saved;
//! ~24 runtime parses per pool-a swap).
//!
//! The checker's `u128_param_provenance` solver classifies each param of
//! each user fn: Native (every direct call site provably feeds a u128
//! native value) or Stringly (ABI edge, tainted merge, first-class use).
//! For fns with a Native param this module compiles a `__u128_<name>`
//! TWIN: same body, but Native params are (lo, hi) limb-pair wasm params
//! and the result is returned as limbs in the static scratch cell
//! U128_W (a dummy i64 occupies the tagged return slot). Direct calls
//! route to the twin per-position:
//!
//! - Native arg, limb-borne at the call site → pair copied (zero parse);
//! - Native arg, generic → tagged parse ONCE at the call boundary;
//! - Stringly arg → tagged value passed through unchanged;
//! - result: limb contexts copy U128_W (let-bound chains stay limb-borne),
//!   generic contexts render once (canonical decimal — bit-identical to
//!   the tagged path's string).
//!
//! Exports, dynamic dispatch and HOF feeds keep the original tagged fn:
//! the solver forces Stringly params there (entry-ABI rule). A twin is
//! bailed (entry removed) if the body uses closures — the lambda
//! machinery allocates globals/lambdas at define time and would run
//! twice. Soundness of the whole scheme rests on the solver's rule that
//! Native ⇒ the value is a canonical u128 (op result / digit literal /
//! typed json-get), so every twin edge sees exactly the bits the tagged
//! path would have parsed (strict decimal, same trap timing).

use super::*;

/// One twin compile unit extracted from the program.
pub(crate) struct TwinUnit {
    pub name: String,
    /// per-original-param: true = limb pair, false = tagged passthrough
    pub kinds: Vec<bool>,
    /// true = solver vouches the ret is a u128 op result: the twin parses
    /// __ret once and returns limbs in U128_W. false = the twin keeps the
    /// tagged return ABI (params STILL hydrate as limbs — a str-ret fn
    /// like pool-a's `step` benefits just as much on the param boundary).
    pub ret_limb: bool,
    pub params: Vec<String>,
    pub body: LispVal,
}

/// Walk the (desugared) program and collect twin compile units from the
/// solver's verdict. Runs at emit-planning time; the emitter is not yet
/// involved, so this is a pure function of `exprs`.
pub(crate) fn plan_u128_param_twins(exprs: &[LispVal]) -> Vec<TwinUnit> {
    let prov = crate::typing::checker::u128_param_provenance(exprs);
    let mut units = Vec::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if items.len() >= 3 {
                if let (LispVal::Sym(s), LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s != "define" || sig.is_empty() {
                        continue;
                    }
                    let LispVal::Sym(name) = &sig[0] else {
                        continue;
                    };
                    let Some(fp) = prov.get(name.as_str()) else {
                        continue;
                    };
                    let ret_limb = fp.ret == crate::typing::checker::Provenance::Native;
                    let params: Vec<String> = match sig[1..]
                        .iter()
                        .map(|p| match p {
                            LispVal::Sym(ps) => Ok(ps.clone()),
                            _ => Err(()),
                        })
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let kinds: Vec<bool> = sig[1..]
                        .iter()
                        .map(|p| match p {
                            LispVal::Sym(pn) => {
                                fp.params.get(pn.as_str())
                                    == Some(&crate::typing::checker::Provenance::Native)
                            }
                            _ => false,
                        })
                        .collect();
                    if !kinds.iter().any(|&k| k) {
                        continue;
                    }
                    // body must match what emit_define compiles: same
                    // annotation split + multi-form begin wrap
                    let (_ann, body_items) = crate::helpers::split_define_annotation(&items[2..]);
                    let body = if body_items.len() > 1 {
                        LispVal::List(
                            std::iter::once(LispVal::Sym("begin".into()))
                                .chain(body_items.iter().cloned())
                                .collect(),
                        )
                    } else {
                        body_items.first().cloned().unwrap_or(LispVal::Nil)
                    };
                    units.push(TwinUnit {
                        name: name.clone(),
                        kinds,
                        ret_limb,
                        params,
                        body,
                    });
                }
            }
        }
    }
    units
}

impl WasmEmitter {
    /// Pre-register twin placeholders + the routing map BEFORE any fn body
    /// compiles, so intra-twin call routing sees the full set (mirrors the
    /// raw-twin pre-registration).
    pub(crate) fn declare_u128_twins(&mut self, units: &[TwinUnit]) {
        for u in units {
            if body_has_lambda(&u.body) {
                continue;
            }
            let extra: usize = u.kinds.iter().filter(|&&k| k).count();
            let idx = self.funcs.len() as u32;
            self.funcs.push(FuncDef {
                name: format!("__u128_{}", u.name),
                param_count: u.params.len() + extra,
                local_count: 0,
                instrs: Vec::new(),
                local_entries: None,
                custom_type: None,
            });
            self.u128_twins
                .insert(u.name.clone(), (idx, u.kinds.clone(), u.ret_limb));
        }
    }

    /// Second pass: compile every declared twin (after all fns exist, so
    /// bodies route into sibling twins; name-based resolution at finish()
    /// makes indices irrelevant).
    pub(crate) fn compile_u128_twins(&mut self, units: &[TwinUnit]) -> Result<(), String> {
        for u in units {
            let Some((idx, _, _)) = self.u128_twins.get(&u.name) else {
                continue; // bailed at declare (closures)
            };
            let idx = *idx as usize;
            let compiled = self.compile_one_twin(u);
            match compiled {
                Ok((instrs, total, ltm_len)) => {
                    let entries =
                        Self::group_local_entries(self.funcs[idx].param_count, total, &ltm_len);
                    self.funcs[idx] = FuncDef {
                        name: format!("__u128_{}", u.name),
                        param_count: self.funcs[idx].param_count,
                        local_count: total,
                        instrs,
                        local_entries: Some(entries),
                        custom_type: None,
                    };
                    eprintln!(
                        "⚡ u128 twin: {} ({} native params)",
                        u.name,
                        u.kinds.iter().filter(|&&k| k).count()
                    );
                }
                Err(e) => {
                    // kill the twin entirely: drop the routing entry so call
                    // sites route to the tagged fn (placeholder left
                    // unreferenced for tree_shake — raw-twin rule, 2026-08-30)
                    self.u128_twins.remove(&u.name);
                    eprintln!("ℹ️ {} not in u128-twin subset ({}): tagged only", u.name, e);
                }
            }
        }
        Ok(())
    }

    fn compile_one_twin(
        &mut self,
        u: &TwinUnit,
    ) -> Result<(Vec<Instruction<'static>>, usize, Vec<ValType>), String> {
        // Full scope save: the twin compiles with the GENERIC compiler
        // (same save set as the loop-lift in call_core.rs).
        let saved = (
            std::mem::take(&mut self.locals),
            self.next_local,
            std::mem::take(&mut self.free_locals),
            std::mem::take(&mut self.local_type_map),
            std::mem::take(&mut self.numeric_locals),
            std::mem::take(&mut self.raw_locals),
            std::mem::take(&mut self.limb_slots),
            std::mem::take(&mut self.parse_cache),
            std::mem::take(&mut self.limb_eligible),
            self.needs_frame,
            self.current_func.take(),
            self.current_param_count,
            std::mem::take(&mut self.current_param_names),
            self.while_id.get(),
            self.gas_local.take(),
            std::mem::take(&mut self.captured_map),
        );
        self.needs_frame = false;
        self.current_func = Some(format!("__u128_{}", u.name));
        self.current_param_count = 0; // elision never fires inside the twin
        self.while_id.set(0);

        let r = self.compile_twin_inner(u);

        // restore
        self.locals = saved.0;
        self.next_local = saved.1;
        self.free_locals = saved.2;
        self.local_type_map = saved.3;
        self.numeric_locals = saved.4;
        self.raw_locals = saved.5;
        self.limb_slots = saved.6;
        self.parse_cache = saved.7;
        self.limb_eligible = saved.8;
        self.needs_frame = saved.9;
        self.current_func = saved.10;
        self.current_param_count = saved.11;
        self.current_param_names = saved.12;
        self.while_id.set(saved.13);
        self.gas_local = saved.14;
        self.captured_map = saved.15;
        r
    }

    fn compile_twin_inner(
        &mut self,
        u: &TwinUnit,
    ) -> Result<(Vec<Instruction<'static>>, usize, Vec<ValType>), String> {
        let h = self.ensure_u128_str_helpers();
        // wasm param slots, in signature order: tagged params one i64
        // each; native params two (lo, hi) — the pairs live in
        // limb_slots, the tagged ones in locals.
        let mut prologue = Vec::new();
        for (p, &k) in u.params.iter().zip(&u.kinds) {
            if k {
                let lo = self.local_idx(&format!("__plimb_{}", p));
                let hi = self.local_idx(&format!("__phimb_{}", p));
                self.limb_slots.insert(p.clone(), (lo, hi));
            } else {
                self.local_idx(p);
            }
        }
        // frame save (same policy as emit_define: NEAR + frame-hosting body)
        self.scan_host(&u.body);
        let fp_save = if !self.p2_mode && !self.wasi_mode && self.needs_frame {
            let fp_save = self.local_idx("__fp_save");
            prologue.push(Instruction::GlobalGet(FP_GLOBAL));
            prologue.push(Instruction::LocalSet(fp_save));
            Some(fp_save)
        } else {
            None
        };
        let ret_local = self.local_idx("__ret");

        // body: params are seeded limb slots, so scan_limb_eligible's
        // fixpoint promotes the whole let-chain — pass NO params (nothing
        // is force-stringly inside the twin)
        self.limb_eligible = self.scan_limb_eligible(&u.body, &[]);
        for (n, _) in self.limb_slots.clone() {
            self.limb_eligible.insert(n);
        }
        let mut body_instrs = self.expr(&u.body)?;

        // epilogue: __ret (tagged) → strict parse → U128_W; return dummy.
        // Solver vouches ret=Native ⇒ __ret is TAG_STR canonical decimal;
        // the strict parse traps exactly where the tagged path would.
        let mut epilogue = Vec::new();
        epilogue.push(Instruction::LocalSet(ret_local));
        if let Some(fp_save) = fp_save {
            epilogue.push(Instruction::LocalGet(fp_save));
            epilogue.push(Instruction::GlobalSet(FP_GLOBAL));
        }
        epilogue.push(Instruction::LocalGet(ret_local));
        self.u128_parse_call(
            &mut epilogue,
            ret_local,
            crate::wasm_emit::call_u128_str::U128_W,
            &h,
        );
        epilogue.push(Instruction::I64Const(0));
        let _ = h;

        let mut instrs = prologue;
        instrs.append(&mut body_instrs);
        instrs.append(&mut epilogue);
        let instrs = Self::peephole(instrs);
        Ok((
            instrs,
            self.next_local as usize,
            self.local_type_map.clone(),
        ))
    }

    /// Is this expr limb-borne at this call site? (twin-call argument
    /// classification) — a seeded limb slot, a digits literal, or another
    /// twin call (result sits in U128_W, copied before any later clobber).
    pub(crate) fn sym_limb_arg(&self, e: &LispVal) -> bool {
        match e {
            LispVal::Sym(n) => self.limb_slots.contains_key(n),
            LispVal::Str(s) => s.parse::<u128>().is_ok(),
            LispVal::List(items) if !items.is_empty() => {
                if let Some(LispVal::Sym(h)) = items.first() {
                    if let Some((_, kinds, ret_limb)) = self.u128_twins.get(h.as_str()) {
                        return *ret_limb && kinds.len() + 1 == items.len();
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Emit one twin-call argument (per-position): returns the pushed
    /// values in signature order (1 i64 tagged / 2 i64 limbs).
    fn emit_twin_arg(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        e: &LispVal,
        native: bool,
    ) -> Result<(), String> {
        if !native {
            v.extend(self.expr(e)?);
            return Ok(());
        }
        if self.sym_limb_arg(e) {
            match e {
                LispVal::Sym(n) => {
                    let (lo, hi) = self.limb_slots[n];
                    v.push(Instruction::LocalGet(lo));
                    v.push(Instruction::LocalGet(hi));
                }
                LispVal::Str(_) => {
                    let val = match e {
                        LispVal::Str(s) => s.parse::<u128>().unwrap(),
                        _ => unreachable!(),
                    };
                    v.push(Instruction::I64Const((val as u64) as i64));
                    v.push(Instruction::I64Const((val >> 64) as u64 as i64));
                }
                LispVal::List(items) => {
                    // nested twin call: result lands in U128_W — call, then
                    // copy the pair
                    let name = match items.first() {
                        Some(LispVal::Sym(h)) => h.clone(),
                        _ => unreachable!(),
                    };
                    v.extend(self.emit_twin_call_raw(&name, &items[1..])?);
                    v.push(Instruction::I64Const(
                        crate::wasm_emit::call_u128_str::U128_W,
                    ));
                    v.push(Instruction::I64Load(crate::wasm_emit::call_u128_str::ma8()));
                    v.push(Instruction::I64Const(
                        crate::wasm_emit::call_u128_str::U128_W + 8,
                    ));
                    v.push(Instruction::I64Load(crate::wasm_emit::call_u128_str::ma8()));
                }
                _ => unreachable!("sym_limb_arg gated"),
            }
            return Ok(());
        }
        // generic: expr → temp → strict parse → U128_A → pair
        let h = self.ensure_u128_str_helpers();
        let t = self.local_idx(&format!("__twarg_{}", self.limb_call_count));
        let alo = self.local_idx(&format!("__twalo_{}", self.limb_call_count));
        let ahi = self.local_idx(&format!("__twahi_{}", self.limb_call_count));
        self.limb_call_count += 1;
        v.extend(self.expr(e)?);
        v.push(Instruction::LocalSet(t));
        self.u128_parse_call(v, t, crate::wasm_emit::call_u128_str::U128_A, &h);
        v.extend(self.limb_pair_load(crate::wasm_emit::call_u128_str::U128_A, alo, ahi));
        v.push(Instruction::LocalGet(alo));
        v.push(Instruction::LocalGet(ahi));
        Ok(())
    }

    /// Twin call, raw limb result in U128_W (dummy i64 left on stack).
    /// Validated entry used by the generic call router: asserts the site's
    /// arity against the twin's kinds (the router has already matched the
    /// tagged fn's arity).
    pub(crate) fn emit_twin_call_raw_checked(
        &mut self,
        name: &str,
        kinds: &[bool],
        a: &[LispVal],
        v: &mut Vec<Instruction<'static>>,
    ) -> Result<(), String> {
        if a.len() != kinds.len() {
            return Err(format!(
                "internal: twin '{}' arity {} != site {}",
                name,
                kinds.len(),
                a.len()
            ));
        }
        let inner = self.emit_twin_call_raw_inner(name, kinds, a)?;
        v.extend(inner);
        Ok(())
    }

    /// Twin call, raw limb result in U128_W (dummy i64 left on stack).
    fn emit_twin_call_raw(
        &mut self,
        name: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        let Some((idx, kinds, _)) = self.u128_twins.get(name).cloned() else {
            return Err(format!("internal: twin '{}' not declared", name));
        };
        self.emit_twin_call_raw_inner(name, &kinds, a)
    }

    fn emit_twin_call_raw_inner(
        &mut self,
        name: &str,
        kinds: &[bool],
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        let Some((idx, _, _)) = self.u128_twins.get(name).cloned() else {
            return Err(format!("internal: twin '{}' not declared", name));
        };
        let mut v = Vec::new();
        // Flat sequential pushes: each arg emits (evaluate →) its 1 tagged
        // or 2 limb values onto the wasm stack. Earlier args' pushed values
        // are stack slots — immune to the U128_A/U128_W scratch-memory
        // traffic later args may cause — so no stage locals are needed.
        for (x, &k) in a.iter().zip(kinds) {
            self.emit_twin_arg(&mut v, x, k)?;
        }
        v.push(Instruction::Call(USER_BASE | idx));
        Ok(v)
    }

    /// Twin call consumed in a LIMB context (let-init / op operand):
    /// args → call → copy U128_W into (lo, hi). Returns None when `e` is
    /// not a routed twin call (caller falls back to the generic path).
    pub(crate) fn emit_twin_call_limb(
        &mut self,
        e: &LispVal,
        lo: u32,
        hi: u32,
    ) -> Option<Result<Vec<Instruction<'static>>, String>> {
        let items = match e {
            LispVal::List(items) if items.len() >= 1 => items,
            _ => return None,
        };
        let LispVal::Sym(h) = &items[0] else {
            return None;
        };
        let Some((_, kinds, ret_limb)) = self.u128_twins.get(h.as_str()) else {
            return None;
        };
        if !*ret_limb || kinds.len() + 1 != items.len() {
            return None; // tagged-ret twins consume like the tagged fn
        }
        let kinds = kinds.clone();
        let h = h.clone();
        Some((|| {
            let mut v = self.emit_twin_call_raw(&h, &items[1..])?;
            v.push(Instruction::Drop); // dummy tagged-slot occupant
            let w = crate::wasm_emit::call_u128_str::U128_W;
            v.push(Instruction::I64Const(w));
            v.push(Instruction::I64Load(crate::wasm_emit::call_u128_str::ma8()));
            v.push(Instruction::LocalSet(lo));
            v.push(Instruction::I64Const(w + 8));
            v.push(Instruction::I64Load(crate::wasm_emit::call_u128_str::ma8()));
            v.push(Instruction::LocalSet(hi));
            Ok(v)
        })())
    }
}

/// Bail gate: closures allocate lambda machinery at define time; a twin
/// body would run it a second time. Keep such fns tagged-only (v1 scope).
fn body_has_lambda(e: &LispVal) -> bool {
    match e {
        LispVal::List(items) => items.iter().any(body_has_lambda),
        LispVal::Sym(n) => n == "lambda",
        _ => false,
    }
}
