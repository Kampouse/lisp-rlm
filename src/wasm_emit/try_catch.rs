//! try/catch emission for the wasm target.
//!
//! `(try BODY (catch VAR HANDLER...))`
//!
//! The interpreter desugars try into `try-catch-impl` + lambdas; the wasm
//! surface has no closures-as-values yet, so try gets its OWN lowering:
//! guarded fallible operations. Fallible ops emitted inside a try body
//! (wrong-arity calls, arithmetic on non-numeric literals, u128 parse /
//! overflow / underflow / div-by-zero) emit a *catch jump* instead of a
//! trap: set the frame's error flag, bind the error message to the frame's
//! `e` local, and branch out of the body.
//!
//! Shape (single shared flag local per function, reset after consumption —
//! correct for nesting and loop re-entry):
//!
//! ```text
//!   i64.const 0            local.set flag        ; reset
//!   block                                        ; $err
//!     <BODY instrs — catch jumps are Br(depth→$err)>
//!     local.set res                               ; normal path result
//!   end                                          ; $err end
//!   local.get flag
//!   if (result i64)
//!     local.get e  → local.set v_local           ; bind catch var
//!     <HANDLER → i64>
//!   else
//!     local.get res
//!   end
//!   i64.const 0            local.set flag        ; consumed — reset
//! ```
//!
//! Catch jumps are emitted as `Br(u32::MAX)` sentinels and patched in a
//! post-pass (`patch_err_brs`) that walks the body vector counting
//! block/loop/if nesting — the guard sites don't need to know their own
//! depth. Errors raised INSIDE the handler are NOT caught (parity with the
//! interpreter's try-catch-impl).
//!
//! Coverage (round 4, 2026-08-27): static arity mismatches, arithmetic on
//! non-numeric literal operands, u128 string-op parse/overflow/underflow/
//! div-zero (via `__h_*_ck` checked helper variants). Fallible ops reached
//! without an enclosing try keep their existing loud behavior (compile
//! error or trap).

use super::*;

/// Sentinel branch depth used by catch jumps; patched by `patch_err_brs`.
pub(crate) const ERR_BR_SENTINEL: u32 = u32::MAX;

#[derive(Clone, Copy)]
pub(crate) struct TryFrame {
    pub(crate) flag_local: u32,
    pub(crate) e_local: u32,
}

/// Rewrite `Br(ERR_BR_SENTINEL)` → `Br(depth)` where depth = block/loop/if
/// nesting at the guard site, relative to the body vector root (= the $err
/// block). Br label indices count enclosing blocks from innermost (0), so a
/// guard nested n levels deep targets $err at index n. Existing numeric Brs
/// (loop back-edges etc.) are left untouched.
pub(crate) fn patch_err_brs(v: &mut Vec<Instruction<'static>>) {
    let mut depth: u32 = 0;
    for i in v.iter_mut() {
        match i {
            Instruction::Br(l) if *l == ERR_BR_SENTINEL => *l = depth,
            Instruction::Block(_) | Instruction::Loop(_) | Instruction::If(_) => depth += 1,
            Instruction::End => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
}

impl WasmEmitter {
    /// Entry from call_core: `(try BODY (catch VAR HANDLER...))`.
    pub(crate) fn emit_try_from(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        if a.len() != 2 {
            return Err("try: expected (try body (catch var handler...))".into());
        }
        let body = &a[0];
        let (var, handler) = match &a[1] {
            LispVal::List(cl) if cl.len() >= 3 && cl[0] == LispVal::Sym("catch".into()) => {
                match &cl[1] {
                    LispVal::Sym(s) => (s.clone(), cl[2..].to_vec()),
                    other => return Err(format!("try: catch variable must be symbol, got {}", other)),
                }
            }
            other => return Err(format!("try: expected (catch var handler...), got {}", other)),
        };
        self.emit_try(body, &var, &handler)
    }

    pub(crate) fn emit_try(
        &mut self,
        body: &LispVal,
        catch_var: &str,
        handler: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        // One flag local per function is enough: it is reset before the body
        // and reset again after the handler consumes it (nesting + loop safe).
        let flag = self.local_idx("__try_flag");
        let res = self.local_idx("__try_res");
        let e_local = self.local_idx("__try_e");
        let v_local = self.local_idx(&format!("__catch_{}", catch_var));

        let mut v = Vec::new();
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(flag));
        v.push(Instruction::Block(BlockType::Empty));
        self.try_stack.push(TryFrame { flag_local: flag, e_local });
        let r = self.expr(body);
        self.try_stack.pop();
        let mut body_v = r?;
        patch_err_brs(&mut body_v);
        v.extend(body_v);
        v.push(Instruction::LocalSet(res));
        v.push(Instruction::End); // $err

        // Bind catch var (scope save/restore so it can shadow)
        let saved = self.locals.insert(catch_var.to_string(), v_local);
        v.push(Instruction::LocalGet(flag));
        v.push(Instruction::I32WrapI64); // If requires i32 condition
        v.push(Instruction::If(BlockType::Result(ValType::I64)));
        v.push(Instruction::LocalGet(e_local));
        v.push(Instruction::LocalSet(v_local));
        let hv = if handler.len() == 1 {
            self.expr(&handler[0])?
        } else {
            let form = LispVal::List(
                std::iter::once(LispVal::Sym("begin".into()))
                    .chain(handler.iter().cloned())
                    .collect(),
            );
            self.expr(&form)?
        };
        v.extend(hv);
        v.push(Instruction::Else);
        v.push(Instruction::LocalGet(res));
        v.push(Instruction::End);
        v.push(Instruction::I64Const(0));
        v.push(Instruction::LocalSet(flag)); // consumed — reset

        match saved {
            Some(old) => { self.locals.insert(catch_var.to_string(), old); }
            None => { self.locals.remove(catch_var); }
        }
        Ok(v)
    }

    /// Emit a catch jump (flag=1, e=msg-string, Br→$err) when currently
    /// inside a try body. Returns false (emitting nothing) otherwise — call
    /// sites use it as: `if !self.try_guard(...) { return Err(...) }`.
    /// Post-pass for checked-op emitters: when a try is active, replace every
    /// `unreachable` in `v` with a catch jump (flag=1, e=msg, Br→sentinel).
    /// Without an active try the vector is returned unchanged (traps stay).
    /// Works for both If(Empty){trap} and If(Result){trap,Else} shapes —
    /// the branch is stack-polymorphic either way, and the enclosing try's
    /// patch_err_brs fixes the depth.
    pub(crate) fn guardify_traps(&mut self, v: &mut Vec<Instruction<'static>>, msg: &str) {
        if self.try_stack.is_empty() || !v.iter().any(|i| matches!(i, Instruction::Unreachable)) {
            return;
        }
        let Some(f) = self.try_stack.last().copied() else { return; };
        let msg_const = self
            .expr(&LispVal::Str(msg.to_string()))
            .unwrap_or_else(|_| vec![Instruction::I64Const(TAG_NIL)]);
        let mut out: Vec<Instruction<'static>> = Vec::with_capacity(v.len() + 8 * msg_const.len());
        for i in v.drain(..) {
            if matches!(i, Instruction::Unreachable) {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::LocalSet(f.flag_local));
                out.extend(msg_const.iter().cloned());
                out.push(Instruction::LocalSet(f.e_local));
                out.push(Instruction::Br(ERR_BR_SENTINEL));
            } else {
                out.push(i);
            }
        }
        *v = out;
    }

    pub(crate) fn try_guard(
        &mut self,
        v: &mut Vec<Instruction<'static>>,
        msg: &str,
    ) -> bool {
        let Some(frame) = self.try_stack.last().copied() else { return false; };
        v.push(Instruction::I64Const(1));
        v.push(Instruction::LocalSet(frame.flag_local));
        // e = message string (tagged). Literal emission may itself need the
        // emitter (data segments) — fall back to nil on failure.
        match self.expr(&LispVal::Str(msg.to_string())) {
            Ok(mut ec) => v.append(&mut ec),
            Err(_) => v.push(Instruction::I64Const(TAG_NIL)),
        }
        v.push(Instruction::LocalSet(frame.e_local));
        v.push(Instruction::Br(ERR_BR_SENTINEL));
        true
    }
}
