use crate::emit::{WasmEmitter, FuncDef, HOST_BASE, HOST_FUNCS, TEMP_MEM, USER_BASE};
use std::collections::HashMap;
use wasm_encoder::{ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemorySection, MemoryType, Module, TypeSection, ValType};

impl WasmEmitter {
    pub(crate) fn tree_shake(&mut self) {
        if self.funcs.is_empty() { return; }

        // Build call graph: for each func index, which other func indices does it call?
        let func_names: Vec<&str> = self.funcs.iter().map(|f| f.name.as_str()).collect();
        let name_to_idx: HashMap<&str, usize> = func_names.iter().enumerate().map(|(i, &n)| (n, i)).collect();

        let mut calls: Vec<Vec<usize>> = vec![vec![]; self.funcs.len()];
        for (i, f) in self.funcs.iter().enumerate() {
            for instr in &f.instrs {
                if let Instruction::Call(idx) = instr {
                    if *idx >= USER_BASE {
                        let pos = (*idx - USER_BASE) as usize;
                        if pos < self.funcs.len() {
                            calls[i].push(pos);
                        }
                    }
                }
            }
        }

        // BFS from exported functions
        let mut reachable = vec![false; self.funcs.len()];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();

        // Seed with exported function names
        for (fn_name, _, _) in &self.exports {
            if let Some(&idx) = name_to_idx.get(fn_name.as_str()) {
                if !reachable[idx] { reachable[idx] = true; queue.push_back(idx); }
            }
        }
        // If no exports, keep last function (default export)
        if self.exports.is_empty() && !self.funcs.is_empty() {
            let last = self.funcs.len() - 1;
            if !reachable[last] { reachable[last] = true; queue.push_back(last); }
        }

        while let Some(idx) = queue.pop_front() {
            for &callee in &calls[idx] {
                if !reachable[callee] {
                    reachable[callee] = true;
                    queue.push_back(callee);
                }
            }
        }

        // Build old_idx -> new_idx mapping
        let mut old_to_new: Vec<Option<usize>> = vec![None; self.funcs.len()];
        let mut next = 0usize;
        for (i, r) in reachable.iter().enumerate() {
            if *r { old_to_new[i] = Some(next); next += 1; }
        }

        // Remap Call instructions
        for (i, f) in self.funcs.iter_mut().enumerate() {
            if !reachable[i] { continue; }
            for instr in &mut f.instrs {
                if let Instruction::Call(idx) = instr {
                    if *idx >= USER_BASE {
                        let pos = (*idx - USER_BASE) as usize;
                        if let Some(new_pos) = old_to_new[pos] {
                            *idx = USER_BASE | new_pos as u32;
                        }
                    }
                }
            }
        }

        // Remove unreachable functions
        let before = self.funcs.len();
        let mut new_funcs: Vec<FuncDef> = Vec::new();
        for (i, f) in std::mem::take(&mut self.funcs).into_iter().enumerate() {
            if reachable[i] { new_funcs.push(f); }
        }
        self.funcs = new_funcs;

        let removed = before - self.funcs.len();
        if removed > 0 {
            eprintln!("Tree-shake: removed {}/{} unused functions", removed, before);
        }
    }

    // ── Module assembly ──

    pub fn finish(&mut self, default_export: &str) -> Vec<u8> {
        // Tree-shake before emitting
        self.tree_shake();
        // Ensure host functions needed by export wrappers are included
        if !self.exports.is_empty() {
            self.need_host(7);  // input
            self.need_host(1);  // register_len
            self.need_host(0);  // read_register
            self.need_host(25); // value_return
        }
        let mut m = Module::new();
        let host_list: Vec<usize> = (0..HOST_FUNCS.len()).filter(|i| self.host_needed.contains(i)).collect();
        let host_count = host_list.len() as u32;

        // Type section
        let mut types = TypeSection::new();
        types.ty().function([], []); // type 0: () -> ()
        let max_p = self.funcs.iter().map(|f| f.param_count).max().unwrap_or(0);
        for p in 0..=max_p {
            let params: Vec<ValType> = (0..p).map(|_| ValType::I64).collect();
            types.ty().function(params, [ValType::I64]);
        }
        let host_type_base = (max_p + 2) as u32;
        for &hi in &host_list {
            types.ty().function(HOST_FUNCS[hi].1.iter().copied(), HOST_FUNCS[hi].2.iter().copied());
        }
        m.section(&types);

        // Import section (host functions only)
        let mut imports = ImportSection::new();
        let mut host_idx: HashMap<usize, u32> = HashMap::new();
        for (i, &hi) in host_list.iter().enumerate() {
            imports.import("env", HOST_FUNCS[hi].0, EntityType::Function(host_type_base + i as u32));
            host_idx.insert(hi, i as u32);
        }
        m.section(&imports);

        // Function section
        let mut funcs = FunctionSection::new();
        for f in &self.funcs { funcs.function(f.param_count as u32 + 1); }
        if self.exports.is_empty() {
            funcs.function(0); // default wrapper: () -> ()
        } else {
            for (fn_name, _, _) in &self.exports {
                let func = self.funcs.iter().find(|f| f.name.as_str() == fn_name.as_str());
                let param_count = func.map(|f| f.param_count).unwrap_or(0);
                // Wrapper type: (i64 × param_count) -> () — same as type param_count+1 but returns nothing
                // For simplicity, use type 0 for now (NEAR passes args via input() anyway)
                // TODO: create proper wrapper types
                let _ = param_count;
                funcs.function(0);
            }
        }
        m.section(&funcs);

        // Memory (internal, exported — same as near-sdk output)
        let mut mems = MemorySection::new();
        mems.memory(MemoryType { minimum: self.memory_pages.max(1) as u64, maximum: None, memory64: false, shared: false, page_size_log2: None });
        m.section(&mems);

        // Global section: mutable i64 for call depth tracking + return flag
        let mut globals = GlobalSection::new();
        globals.global(
            GlobalType { val_type: ValType::I64, mutable: true, shared: false },
            &ConstExpr::i64_const(0),
        );
        // Global 1: return flag (set by near/return to skip export wrapper's value_return)
        globals.global(
            GlobalType { val_type: ValType::I64, mutable: true, shared: false },
            &ConstExpr::i64_const(0),
        );
        m.section(&globals);

        // Exports
        let mut exps = ExportSection::new();
        exps.export("memory", ExportKind::Memory, 0);
        let internal_base = host_count;
        let wrapper_base = internal_base + self.funcs.len() as u32;
        if self.exports.is_empty() {
            if !self.funcs.is_empty() { exps.export(default_export, ExportKind::Func, wrapper_base); }
        } else {
            for (i, (_, en, _)) in self.exports.iter().enumerate() {
                exps.export(en, ExportKind::Func, wrapper_base + i as u32);
            }
        }
        m.section(&exps);

        // Code
        let name_map: HashMap<&str, u32> = self.funcs.iter().enumerate()
            .map(|(i, f)| (f.name.as_str(), internal_base + i as u32)).collect();
        let mut code = wasm_encoder::CodeSection::new();
        for f in &self.funcs {
            let extra = f.local_count.saturating_sub(f.param_count);
            let locals: Vec<(u32, ValType)> = if extra > 0 { vec![(extra as u32, ValType::I64)] } else { vec![] };
            let resolved = Self::resolve_static(&f.instrs, &host_idx, &name_map, &self.funcs);
            let mut fb = Function::new(locals);
            for instr in &resolved { fb.instruction(instr); }
            fb.instruction(&Instruction::End);
            code.function(&fb);
        }
        // Wrappers
        if self.exports.is_empty() {
            if let Some(f) = self.funcs.last() {
                let idx = internal_base + (self.funcs.len()-1) as u32;
                let mut fb = Function::new(vec![(1u32, ValType::I64)]); // local 0 for result swapping
                // Pass default args: for each param, push 100000 (for tight loop benchmarking)
                for _ in 0..f.param_count {
                    fb.instruction(&Instruction::I64Const(100000));
                }
                fb.instruction(&Instruction::Call(idx));
                fb.instruction(&Instruction::Drop);
                fb.instruction(&Instruction::End);
                code.function(&fb);
            }
        } else {
            for (fn_name, _, _) in &self.exports {
                if let Some(&idx) = name_map.get(fn_name.as_str()) {
                    let func = self.funcs.iter().find(|f| f.name.as_str() == fn_name.as_str());
                    let param_count = func.map(|f| f.param_count).unwrap_or(0);
                    let mut fb = Function::new(vec![(1u32, ValType::I64)]); // local 0 for result swapping
                    let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                    if param_count == 0 {
                        fb.instruction(&Instruction::Call(idx));
                        fb.instruction(&Instruction::LocalSet(0));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::I32WrapI64);
                        fb.instruction(&Instruction::LocalGet(0));
                        fb.instruction(&Instruction::I64Store(ma));
                        fb.instruction(&Instruction::I64Const(8));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&25]));
                    } else {
                        // input(0)
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::Call(host_idx[&7]));
                        // register_len(0) — drop
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::Call(host_idx[&1]));
                        fb.instruction(&Instruction::Drop);
                        // read_register(0, TEMP_MEM)
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&0]));
                        // Load args
                        for i in 0..param_count {
                            fb.instruction(&Instruction::I64Const(TEMP_MEM + (i as i64) * 8));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::I64Load(ma));
                        }
                        fb.instruction(&Instruction::Call(idx));
                        // Store result at TEMP_MEM: i64.store needs [i32 addr, i64 val]
                        // Stack: [i64 result]. Save to local 0, push addr, load local, store
                        fb.instruction(&Instruction::LocalSet(0)); // save result to local 0
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::I32WrapI64);   // addr as i32
                        fb.instruction(&Instruction::LocalGet(0));  // restore result
                        fb.instruction(&Instruction::I64Store(ma));
                        // value_return(8, TEMP_MEM)
                        fb.instruction(&Instruction::I64Const(8));
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::Call(host_idx[&25]));
                    }
                    fb.instruction(&Instruction::End);
                    code.function(&fb);
                }
            }
        }
        m.section(&code);

        // Data (section 11 — must come after code section 10)
        if !self.data_segments.is_empty() {
            let mut data = DataSection::new();
            for (off, bytes) in &self.data_segments {
                data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
            }
            m.section(&data);
        }

        m.finish()
    }

    pub(crate) fn resolve_static(
        instrs: &[Instruction<'static>],
        host_map: &HashMap<usize, u32>,
        name_map: &HashMap<&str, u32>,
        funcs: &[FuncDef],
    ) -> Vec<Instruction<'static>> {
        instrs.iter().map(|i| match i {
            Instruction::Call(idx) if *idx >= HOST_BASE && *idx < USER_BASE => {
                Instruction::Call(host_map[&((*idx - HOST_BASE) as usize)])
            }
            Instruction::Call(idx) if *idx >= USER_BASE => {
                let pos = (*idx - USER_BASE) as usize;
                Instruction::Call(name_map[funcs[pos].name.as_str()])
            }
            other => other.clone(),
        }).collect()
    }
}
