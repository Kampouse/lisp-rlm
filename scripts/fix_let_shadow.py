#!/usr/bin/env python3
"""Fix let*/let init-before-bind in call_core.rs (shadowing regression).

Bug: the binder inserted name→slot BEFORE compiling the init expression, so
(let* ((x (+ x 1)) ...) inside a scope already binding x read the FRESH
UNINITIALIZED slot (WASM locals zero-init) → (+ 0 1) = 1 instead of 2.
Also: the i64-tagged range check rejects nothing here, but init-first is the
correct sequential-let* semantics regardless.
"""
PATH = "/Users/asil/dev/lisp-rlm/src/wasm_emit/call_core.rs"

old = """                let mut saved: Vec<(String, Option<u32>)> = Vec::new();
                if let LispVal::List(bs) = &a[0] {
                    for b in bs {
                        if let LispVal::List(p) = b {
                            if p.len() == 2 {
                                if let LispVal::Sym(n) = &p[0] {
                                    let old = self.locals.get(n).copied();
                                    let i = self.free_locals.pop().unwrap_or(self.next_local);
                                    if i == self.next_local {
                                        self.next_local += 1;
                                        self.local_type_map.push(ValType::I64);
                                    }
                                    self.locals.insert(n.clone(), i);
                                    saved.push((n.clone(), old));
                                    v.extend(self.expr(&p[1])?);
                                    v.push(Instruction::LocalSet(i));
                                }
                            }
                        }
                    }
                }
"""

new = """                let mut saved: Vec<(String, Option<u32>)> = Vec::new();
                if let LispVal::List(bs) = &a[0] {
                    for b in bs {
                        if let LispVal::List(p) = b {
                            if p.len() == 2 {
                                if let LispVal::Sym(n) = &p[0] {
                                    // INIT FIRST, then bind: let* inits evaluate in
                                    // the OUTER scope — binding the name before
                                    // compiling the init made (let* ((x (+ x 1))))
                                    // read its own fresh zero-initialized slot
                                    // (returned 1, not 2). Fix 2026-09-05.
                                    let init = self.expr(&p[1])?;
                                    let old = self.locals.get(n).copied();
                                    let i = self.free_locals.pop().unwrap_or(self.next_local);
                                    if i == self.next_local {
                                        self.next_local += 1;
                                        self.local_type_map.push(ValType::I64);
                                    }
                                    self.locals.insert(n.clone(), i);
                                    saved.push((n.clone(), old));
                                    v.extend(init);
                                    v.push(Instruction::LocalSet(i));
                                }
                            }
                        }
                    }
                }
"""

with open(PATH) as f:
    c = f.read()
n = c.count(old)
assert n == 1, f"anchor count {n}"
c = c.replace(old, new, 1)
with open(PATH, "w") as f:
    f.write(c)
print("call_core.rs let* init-before-bind fixed")
