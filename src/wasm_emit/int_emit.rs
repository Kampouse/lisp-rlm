//! Raw-i64 emission for fully-annotated int functions.
//!
//! A `(define (f x y) :: int int -> int BODY)` whose BODY lies entirely in
//! the provably-int subset compiles to a raw twin `__raw_f`: i64 locals with
//! NO tag shifts — arithmetic runs on payloads directly, checked exactly like
//! the interpreter (i64 overflow traps). The original tagged definition
//! remains as the public shim; generic call sites (and exports) are
//! unchanged. Raw callers (other annotated fns) call the twin directly, so
//! recursion and int-to-int call chains pay zero tagging.
//!
//! SAFETY CONTRACT: any form outside the subset returns Err(()) and the
//! function falls back to the generic tagged path — behavior can only stay
//! identical, never diverge.

use super::*;
use std::collections::HashSet;
use wasm_encoder::{BlockType, Instruction, ValType};

impl WasmEmitter {
    /// Compile `e` as a raw-i64 expression. `int_locals` = names bound to
    /// raw values in scope (params + int `let`s + loop bindings).
    pub(crate) fn int_expr(
        &mut self,
        e: &LispVal,
        int_locals: &mut HashSet<String>,
        depth: u32,
    ) -> Result<Vec<Instruction<'static>>, ()> {
        if depth > 200 {
            return Err(());
        }
        match e {
            LispVal::Num(n) => Ok(vec![Instruction::I64Const(*n)]),
            LispVal::Sym(s) => {
                if int_locals.contains(s) {
                    Ok(vec![Instruction::LocalGet(self.local_idx(s))])
                } else {
                    Err(())
                }
            }
            LispVal::List(l) if !l.is_empty() => {
                let (op, a) = (&l[0], &l[1..]);
                let LispVal::Sym(op) = op else {
                    return Err(());
                };
                match op.as_str() {
                    "+" | "-" | "*" => {
                        if a.is_empty() {
                            return Err(());
                        }
                        let mut v = self.int_expr(&a[0], int_locals, depth + 1)?;
                        for x in &a[1..] {
                            v.extend(self.int_expr(x, int_locals, depth + 1)?);
                            v.extend(match op.as_str() {
                                "+" => self.emit_checked_add(),
                                "-" => self.emit_checked_sub(),
                                _ => self.emit_checked_mul(),
                            });
                        }
                        Ok(v)
                    }
                    "<" | ">" | "<=" | ">=" => {
                        if a.len() != 2 {
                            return Err(());
                        }
                        let mut v = self.int_expr(&a[0], int_locals, depth + 1)?;
                        v.extend(self.int_expr(&a[1], int_locals, depth + 1)?);
                        v.push(match op.as_str() {
                            "<" => Instruction::I64LtS,
                            ">" => Instruction::I64GtS,
                            "<=" => Instruction::I64LeS,
                            _ => Instruction::I64GeS,
                        });
                        v.push(Instruction::I64ExtendI32U);
                        v.extend(self.emit_tag_bool());
                        Ok(v)
                    }
                    "if" => {
                        if a.len() < 2 {
                            return Err(());
                        }
                        // Condition must be a raw comparison: cmp leaves an
                        // i32, and wasm `If` consumes i32 natively — no
                        // bool-tag round-trip needed at all.
                        let LispVal::List(cl) = &a[0] else {
                            return Err(());
                        };
                        if cl.len() != 3 {
                            return Err(());
                        }
                        let LispVal::Sym(cop) = &cl[0] else {
                            return Err(());
                        };
                        let cmp_op = match cop.as_str() {
                            "<" => Instruction::I64LtS,
                            ">" => Instruction::I64GtS,
                            "<=" => Instruction::I64LeS,
                            ">=" => Instruction::I64GeS,
                            _ => return Err(()),
                        };
                        let mut v = self.int_expr(&cl[1], int_locals, depth + 1)?;
                        v.extend(self.int_expr(&cl[2], int_locals, depth + 1)?);
                        v.push(cmp_op); // i32 condition
                        v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.extend(self.int_expr(&a[1], int_locals, depth + 1)?);
                        v.push(Instruction::Else);
                        if let Some(els) = a.get(2) {
                            v.extend(self.int_expr(els, int_locals, depth + 1)?);
                        } else {
                            v.push(Instruction::I64Const(0));
                        }
                        v.push(Instruction::End);
                        Ok(v)
                    }
                    "let" | "let*" => {
                        // (let ((x init)) body) — single int binding only.
                        if a.len() < 3 {
                            return Err(());
                        }
                        let LispVal::List(bindings) = &a[0] else {
                            return Err(());
                        };
                        if bindings.len() != 1 {
                            return Err(());
                        }
                        let LispVal::List(b) = &bindings[0] else {
                            return Err(());
                        };
                        if b.len() != 2 {
                            return Err(());
                        }
                        let LispVal::Sym(name) = &b[0] else {
                            return Err(());
                        };
                        let mut v = self.int_expr(&b[1], int_locals, depth + 1)?;
                        let li = self.local_idx(name);
                        v.push(Instruction::LocalSet(li));
                        let fresh = int_locals.insert(name.clone());
                        // body = LAST form only (single-expression lets)
                        let body = a.last().unwrap();
                        let r = self.int_expr(body, int_locals, depth + 1);
                        if fresh {
                            int_locals.remove(name);
                        }
                        v.extend(r?);
                        Ok(v)
                    }
                    "min" | "max" => {
                        if a.len() != 2 {
                            return Err(());
                        }
                        let x = self.int_expr(&a[0], int_locals, depth + 1)?;
                        let y = self.int_expr(&a[1], int_locals, depth + 1)?;
                        // select(x cmp y ? a : b) — evaluate both, pick
                        let mut v = Vec::new();
                        v.extend(x);
                        v.extend(y);
                        // [x, y] → tmp_y = y; tmp_x = x — via locals
                        let ty = self.local_idx("__intsel");
                        let tx = self.local_idx("__intsel2");
                        v.push(Instruction::LocalSet(ty));
                        v.push(Instruction::LocalSet(tx));
                        v.push(Instruction::LocalGet(tx));
                        v.push(Instruction::LocalGet(ty));
                        v.push(match op.as_str() {
                            "min" => Instruction::I64LtS,
                            _ => Instruction::I64GtS,
                        });
                        v.push(Instruction::If(BlockType::Result(ValType::I64)));
                        v.push(Instruction::LocalGet(tx));
                        v.push(Instruction::Else);
                        v.push(Instruction::LocalGet(ty));
                        v.push(Instruction::End);
                        Ok(v)
                    }
                    // Calls: raw twin of another annotated fn, or self-recursion.
                    _ => {
                        let twin = match self.raw_twins.get(op) {
                            Some(idx) => *idx,
                            None => return Err(()),
                        };
                        let expect = self
                            .fn_int_annotations
                            .get(op.as_str())
                            .map(|(n, _)| *n)
                            .unwrap_or(usize::MAX);
                        if a.len() != expect {
                            return Err(());
                        }
                        let mut v = Vec::new();
                        for x in a {
                            v.extend(self.int_expr(x, int_locals, depth + 1)?);
                        }
                        v.push(Instruction::Call(USER_BASE | twin as u32));
                        Ok(v)
                    }
                }
            }
            _ => Err(()),
        }
    }

    /// A condition usable by raw `if`: exactly a raw comparison (returns a
    /// TAGGED bool value, so emit_cond_branch works unchanged).
    fn int_cond(
        &mut self,
        e: &LispVal,
        int_locals: &mut HashSet<String>,
        depth: u32,
    ) -> Result<Vec<Instruction<'static>>, ()> {
        let LispVal::List(l) = e else {
            return Err(());
        };
        if l.len() != 3 {
            return Err(());
        }
        let LispVal::Sym(op) = &l[0] else {
            return Err(());
        };
        match op.as_str() {
            "<" | ">" | "<=" | ">=" => {
                let mut v = self.int_expr(&l[1], int_locals, depth + 1)?;
                v.extend(self.int_expr(&l[2], int_locals, depth + 1)?);
                v.push(match op.as_str() {
                    "<" => Instruction::I64LtS,
                    ">" => Instruction::I64GtS,
                    "<=" => Instruction::I64LeS,
                    _ => Instruction::I64GeS,
                });
                v.push(Instruction::I64ExtendI32U);
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            _ => Err(()),
        }
    }

    /// Attempt to compile a fully-annotated int function to a raw twin.
    /// Returns the twin's instruction stream + local count, or Err(()) when
    /// any form falls outside the provably-int subset.
    pub(crate) fn try_raw_int_fn(
        &mut self,
        name: &str,
        params: &[String],
        body: &LispVal,
    ) -> Result<(Vec<Instruction<'static>>, usize), ()> {
        // Fresh local scope for the twin (params first, mirroring emit_define)
        let saved = (
            std::mem::take(&mut self.locals),
            self.next_local,
            std::mem::take(&mut self.free_locals),
            std::mem::take(&mut self.local_type_map),
        );
        self.locals.clear();
        self.next_local = 0;
        self.free_locals.clear();
        self.local_type_map.clear();
        for p in params {
            self.local_idx(p);
        }
        let mut int_locals: HashSet<String> = params.iter().cloned().collect();
        let r = self.int_expr(body, &mut int_locals, 0);
        let total = self.next_local as usize;
        let _ = name;
        // Restore the emitter's scope for the generic (shim) compilation
        self.locals = saved.0;
        self.next_local = saved.1;
        self.free_locals = saved.2;
        self.local_type_map = saved.3;
        r.map(|instrs| (instrs, total))
    }
}
