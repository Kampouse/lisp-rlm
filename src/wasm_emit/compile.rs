
use super::*;

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
        // Also seed with lambda functions (called indirectly via CallIndirect)
        for &(func_idx, _) in &self.lambda_info {
            if func_idx < reachable.len() && !reachable[func_idx] {
                reachable[func_idx] = true;
                queue.push_back(func_idx);
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

    pub fn gas_estimate(&self) -> Vec<(String, usize, f64)> {
        let host_count = self.host_needed.len();
        let mut estimates = Vec::new();
        for (fn_name, export_name, _is_view) in &self.exports {
            if let Some(func) = self.funcs.iter().find(|f| f.name == *fn_name) {
                let instr_count = func.instrs.len();
                // Rough gas model: count host calls by checking Call instructions
                // that reference imported functions (indices < host_count)
                let host_calls = func.instrs.iter().filter(|i| {
                    matches!(i, Instruction::Call(idx) if (*idx as usize) < host_count)
                }).count();
                let regular = instr_count.saturating_sub(host_calls);
                // NEAR charges ~1 gas for simple ops, ~10 for host calls
                let estimated_gas = (regular + host_calls * 10) as f64;
                // 1 Tgas = 10^12 gas — but this is a rough estimate, so show in gas units
                estimates.push((export_name.clone(), instr_count, estimated_gas));
            }
        }
        estimates
    }

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
            if !self.funcs.is_empty() {
                funcs.function(0); // default wrapper: () -> ()
            }
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

        // Global section: mutable i64 globals
        let mut globals = GlobalSection::new();
        // Global 0: return flag (set by near/return to skip export wrapper's value_return)
        globals.global(
            GlobalType { val_type: ValType::I64, mutable: true, shared: false },
            &ConstExpr::i64_const(0),
        );
        // Global 1: frame pointer (bump allocator for string ops) — NEAR mode only
        if !self.wasi_mode && !self.p2_mode {
            globals.global(
                GlobalType { val_type: ValType::I64, mutable: true, shared: false },
                &ConstExpr::i64_const(self.heap_ptr as i64),
            );
        }
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
            let resolved = Self::resolve_static_pub(&f.instrs, &host_idx, &name_map, &self.funcs);
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
                        // Reset return flag before call
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::GlobalSet(RETURN_FLAG));
                        fb.instruction(&Instruction::Call(idx));
                        fb.instruction(&Instruction::LocalSet(0));
                        if self.fuzz_mode {
                            // Fuzz mode: store raw tagged i64 at TEMP_MEM, no value_return
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::LocalGet(0));
                            fb.instruction(&Instruction::I64Store(ma));
                        } else {
                            // NEAR mode: check return flag, handle TAG_NIL/TAG_ARRAY specially, call value_return
                            fb.instruction(&Instruction::GlobalGet(RETURN_FLAG));
                            fb.instruction(&Instruction::I64Const(0));
                            fb.instruction(&Instruction::I64Ne);
                            fb.instruction(&Instruction::If(BlockType::Empty));
                            // Return flag set — function already called value_return directly, nothing to do
                            fb.instruction(&Instruction::Else);
                            // Normal path: check for TAG_NIL first
                            fb.instruction(&Instruction::LocalGet(0));
                            fb.instruction(&Instruction::I64Const(7)); // tag mask
                            fb.instruction(&Instruction::I64And);
                            fb.instruction(&Instruction::I64Const(TAG_NIL));
                            fb.instruction(&Instruction::I64Eq);
                            fb.instruction(&Instruction::If(BlockType::Empty));
                            // TAG_NIL: write special nil marker at TEMP_MEM (0xFEFF sentinels — cannot be valid untagged i64)
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::I64Const(0x7FFE_FEFF_FEFF_FEFE_i64)); // nil sentinel
                            fb.instruction(&Instruction::I64Store(ma));
                            fb.instruction(&Instruction::I64Const(8));
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::Call(host_idx[&25])); // value_return
                            fb.instruction(&Instruction::Else);
                            // Non-nil: check for TAG_ARRAY — store tagged value for TS decoding
                            fb.instruction(&Instruction::LocalGet(0));
                            fb.instruction(&Instruction::I64Const(7)); // tag mask
                            fb.instruction(&Instruction::I64And);
                            fb.instruction(&Instruction::I64Const(TAG_ARRAY));
                            fb.instruction(&Instruction::I64Eq);
                            fb.instruction(&Instruction::If(BlockType::Empty));
                            // TAG_ARRAY: store full tagged value at TEMP_MEM so TS can decode the array
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::LocalGet(0)); // full tagged array value
                            fb.instruction(&Instruction::I64Store(ma));
                            fb.instruction(&Instruction::I64Const(8));
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::Call(host_idx[&25])); // value_return
                            fb.instruction(&Instruction::Else);
                            // TAG_STR/TAG_NUM/TAG_BOOL: store tagged value at TEMP_MEM and call value_return
                            // The JS runtime decodes the tag from the captured bytes.
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::LocalGet(0)); // full tagged value
                            fb.instruction(&Instruction::I64Store(ma));
                            fb.instruction(&Instruction::I64Const(8));
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::Call(host_idx[&25])); // value_return
                            fb.instruction(&Instruction::End); // if TAG_ARRAY
                            fb.instruction(&Instruction::End); // if TAG_NIL
                            fb.instruction(&Instruction::End); // if
                        }
                    } else {
                        // Reset return flag before call
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::GlobalSet(RETURN_FLAG));
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
                        // Load args — tag raw i64 from host as Num
                        for i in 0..param_count {
                            fb.instruction(&Instruction::I64Const(TEMP_MEM + (i as i64) * 8));
                            fb.instruction(&Instruction::I32WrapI64);
                            fb.instruction(&Instruction::I64Load(ma));
                            // Tag as Num: (val << 3) | 0
                            fb.instruction(&Instruction::I64Const(TAG_BITS));
                            fb.instruction(&Instruction::I64Shl);
                        }
                        fb.instruction(&Instruction::Call(idx));
                        // Store result at TEMP_MEM: i64.store needs [i32 addr, i64 val]
                        // Stack: [i64 result]. Save to local 0, push addr, load local, store
                        fb.instruction(&Instruction::LocalSet(0)); // save result to local 0
                        // Check return flag: if global[1] != 0, skip untag+store+value_return
                        fb.instruction(&Instruction::GlobalGet(RETURN_FLAG));
                        fb.instruction(&Instruction::I64Const(0));
                        fb.instruction(&Instruction::I64Ne);
                        fb.instruction(&Instruction::If(BlockType::Empty));
                        // Return flag set — nothing to do
                        fb.instruction(&Instruction::Else);
                        // Normal path: untag, store, value_return
                        fb.instruction(&Instruction::I64Const(TEMP_MEM));
                        fb.instruction(&Instruction::I32WrapI64);   // addr as i32
                        fb.instruction(&Instruction::LocalGet(0));  // restore result
                        if !self.fuzz_mode {
                            // Untag the return value before storing for host
                            fb.instruction(&Instruction::I64Const(TAG_BITS));
                            fb.instruction(&Instruction::I64ShrU);
                        }
                        fb.instruction(&Instruction::I64Store(ma));
                        if !self.fuzz_mode {
                            // value_return(8, TEMP_MEM)
                            fb.instruction(&Instruction::I64Const(8));
                            fb.instruction(&Instruction::I64Const(TEMP_MEM));
                            fb.instruction(&Instruction::Call(host_idx[&25]));
                        }
                        fb.instruction(&Instruction::End); // if return flag check
                    }
                    fb.instruction(&Instruction::End);
                    code.function(&fb);
                }
            }
        }
        m.section(&code);

        // Data (section 11 — must come after code section 10)
        // Always emit runtime heap pointer initialization at RUNTIME_HEAP_PTR
        {
            let mut data = DataSection::new();
            // Initialize runtime heap ptr with final compile-time heap_ptr
            let hp_bytes = self.heap_ptr.to_le_bytes();
            data.active(0, &ConstExpr::i32_const(RUNTIME_HEAP_PTR as i32), hp_bytes.iter().copied());
            for (off, bytes) in &self.data_segments {
                data.active(0, &ConstExpr::i32_const(*off as i32), bytes.iter().copied());
            }
            m.section(&data);
        }

        m.finish()
    }

    pub(crate) fn resolve_static_pub(
        instrs: &[Instruction<'static>],
        host_map: &HashMap<usize, u32>,
        name_map: &HashMap<&str, u32>,
        funcs: &[FuncDef],
    ) -> Vec<Instruction<'static>> {
        Self::resolve_static_pub_ex(instrs, host_map, name_map, funcs, &HashMap::new())
    }

    pub(crate) fn resolve_static_pub_ex(
        instrs: &[Instruction<'static>],
        host_map: &HashMap<usize, u32>,
        name_map: &HashMap<&str, u32>,
        funcs: &[FuncDef],
        outlayer_map: &HashMap<u32, u32>,
    ) -> Vec<Instruction<'static>> {
        instrs.iter().map(|i| match i {
            Instruction::Call(idx) if *idx >= HOST_BASE && *idx < USER_BASE => {
                Instruction::Call(host_map[&((*idx - HOST_BASE) as usize)])
            }
            Instruction::Call(idx) if *idx >= USER_BASE => {
                let pos = (*idx - USER_BASE) as usize;
                Instruction::Call(name_map[funcs[pos].name.as_str()])
            }
            Instruction::Call(idx) if outlayer_map.contains_key(idx) => {
                Instruction::Call(outlayer_map[idx])
            }
            other => other.clone(),
        }).collect()
    }

}

fn parse_and_compile(source: &str, near: bool) -> Result<WasmEmitter, String> {
    parse_and_compile_opts(source, near, true)
}

fn parse_and_compile_opts(source: &str, near: bool, typecheck: bool) -> Result<WasmEmitter, String> {
    let exprs = crate::parser::parse_all(source)?;
    let mut exprs = exprs;
    crate::clojure::desugar(&mut exprs);

    // Type check pass — catches undefined vars, arity mismatches, type errors
    if typecheck {
        crate::typing::type_check_program(&exprs, near)?;
    }

    // Storage schema validation — warns about reads without matching writes
    if near {
        crate::typing::check_storage_schema(&exprs);
    }

    let mut em = WasmEmitter::new();

    // Pre-scan: register all function names for forward references (mutual recursion)
    for e in &exprs {
        if let LispVal::List(items) = e {
            if items.len() >= 3 {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "define" {
                        // Function define: (define (name params...) body)
                        if let LispVal::List(sig) = &items[1] {
                            if !sig.is_empty() {
                                if let LispVal::Sym(name) = &sig[0] {
                                    if !em.funcs.iter().any(|f| &f.name == name) {
                                        em.funcs.push(FuncDef { name: name.clone(), param_count: sig.len()-1, local_count: 0, instrs: Vec::new() });
                                    }
                                }
                            }
                        }
                        // Value define: (define name value)
                        if let LispVal::Sym(name) = &items[1] {
                            if !em.funcs.iter().any(|f| &f.name == name) {
                                em.funcs.push(FuncDef { name: name.clone(), param_count: 0, local_count: 0, instrs: Vec::new() });
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect bare expressions (not define/export/borsh-schema/memory) for implicit toplevel
    let mut bare_exprs: Vec<LispVal> = Vec::new();
    for e in &exprs {
        if let LispVal::List(items) = e {
            if items.is_empty() { continue; }
            if let LispVal::Sym(s) = &items[0] {
                match s.as_str() {
                    "define" | "export" | "borsh-schema" => {
                        // Handle borsh-schema regardless of near mode
                        if s == "borsh-schema" {
                            super::borsh::process_borsh_schema(&mut em, items)?;
                        }
                        if items.len() >= 3 {
                            if let (LispVal::Sym(s2), LispVal::List(sig)) = (&items[0], &items[1]) {
                                if s2 == "define" && !sig.is_empty() {
                                    if let LispVal::Sym(name) = &sig[0] {
                                        let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                            LispVal::Sym(ps) => Ok(ps.clone()), _ => Err("param must be symbol".into()),
                                        }).collect::<Result<_, String>>()?;
                                        let body = if items.len() > 3 {
                                            LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                                .chain(items[2..].iter().cloned()).collect())
                                        } else {
                                            items[2].clone()
                                        };
                                        em.emit_define(name, &params, &body)?;
                                    }
                                }
                            }
                            // Value define: (define name value)
                            if let (LispVal::Sym(s2), LispVal::Sym(name)) = (&items[0], &items[1]) {
                                if s2 == "define" {
                                    let value = &items[2];
                                    em.emit_define(name, &[], value)?;
                                }
                            }
                            if let LispVal::Sym(s2) = &items[0] {
                                if s2 == "export" { if let (LispVal::Str(en), LispVal::Sym(fn_)) = (&items[1], &items[2]) {
                                    let view = items.len()>3 && matches!(&items[3], LispVal::Bool(true));
                                    em.add_export(fn_, en, view);
                                }}
                            }
                        }
                        if let (LispVal::Sym(s2), Some(LispVal::Num(n))) = (&items[0], items.get(1)) {
                            if s2 == "memory" { em.set_memory(*n as u32); }
                        }
                        continue;
                    }
                    "memory" => {
                        if let Some(LispVal::Num(n)) = items.get(1) { em.set_memory(*n as u32); }
                        continue;
                    }
                    _ => {}
                }
            }
            bare_exprs.push(e.clone());
        } else {
            bare_exprs.push(e.clone());
        }
    }
    // If there are bare expressions, wrap them in an implicit toplevel function
    if !bare_exprs.is_empty() {
        let body = if bare_exprs.len() == 1 {
            bare_exprs.into_iter().next().unwrap()
        } else {
            LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                .chain(bare_exprs.into_iter()).collect())
        };
        em.emit_define("__toplevel", &[], &body)?;
    }
    Ok(em)
}


pub fn compile_pure(source: &str) -> Result<Vec<u8>, String> {
    let mut em = parse_and_compile(source, false)?;
    // Add a "run" export for the last defined function (or the implicit top-level begin)
    // so that the export wrapper (which calls value_return) is included.
    if let Some(f) = em.funcs.last() {
        em.add_export(&f.name.clone(), "run", false);
    }
    Ok(em.finish("run"))
}


pub fn compile_fuzz(source: &str) -> Result<Vec<u8>, String> {
    let mut em = parse_and_compile(source, false)?;
    // Export the function named "run" (the test entry point), not funcs.last()
    // which may be a lambda added after the run function.
    if let Some(f) = em.funcs.iter().find(|f| f.name == "run") {
        em.add_export(&f.name.clone(), "run", false);
    } else if let Some(f) = em.funcs.last() {
        em.add_export(&f.name.clone(), "run", false);
    }
    em.set_fuzz_mode(true);
    Ok(em.finish("run"))
}


pub fn compile_near(source: &str) -> Result<Vec<u8>, String> {
    let resolved = resolve_modules(source, std::path::Path::new("."))?;
    let mut em = parse_and_compile(&resolved, true)?;
    // If no explicit exports, auto-export the "run" function as "_run"
    // so tree-shaking keeps it and all functions it calls.
    if em.exports.is_empty() {
        if let Some(f) = em.funcs.iter().find(|f| f.name == "run") {
            em.add_export(&f.name.clone(), "_run", false);
        } else if let Some(f) = em.funcs.last() {
            em.add_export(&f.name.clone(), "_run", false);
        }
    }
    // Emit gas estimates before finish consumes the emitter
    let estimates = em.gas_estimate();
    if !estimates.is_empty() {
        eprintln!("╔════════════════════════════════════════════════════╗");
        eprintln!("║  Gas Estimation (per export)                       ║");
        eprintln!("╠════════════════════════════════════════════════════╣");
        for (name, instrs, gas) in &estimates {
            eprintln!("║  {:<24} {:>4} instrs  ~{:.0} gas", name, instrs, gas);
        }
        eprintln!("╚════════════════════════════════════════════════════╝");
    }
    
   let wasm = em.finish("_run");
   Ok(wasm)
}

/// Compile NEAR WASM from source, skipping type checking.
/// Useful for dynamically-typed generated code (e.g. Solidity translation).
pub fn compile_near_untyped(source: &str) -> Result<Vec<u8>, String> {
   let resolved = resolve_modules(source, std::path::Path::new("."))?;
   let mut em = parse_and_compile_opts(&resolved, true, false)?;
   if em.exports.is_empty() {
       if let Some(f) = em.funcs.iter().find(|f| f.name == "run") {
           em.add_export(&f.name.clone(), "_run", false);
       } else if let Some(f) = em.funcs.last() {
           em.add_export(&f.name.clone(), "_run", false);
       }
   }
   let wasm = em.finish("_run");
   Ok(wasm)
}


pub fn compile_near_from_exprs(exprs: &[LispVal]) -> Result<Vec<u8>, String> {
    // Type check pass
    crate::typing::type_check_program(exprs, true)?;

    // Storage schema validation
    crate::typing::check_storage_schema(exprs);

    let mut em = WasmEmitter::new();
    for e in exprs {
        if let LispVal::List(items) = e {
            if items.is_empty() { continue; }
            // Handle (borsh-schema ...) — can have any number of args
            if let LispVal::Sym(s) = &items[0] {
                if s == "borsh-schema" {
                    super::borsh::process_borsh_schema(&mut em, items)?;
                }
            }
            if items.len() >= 3 {
                if let (LispVal::Sym(s), LispVal::List(sig)) = (&items[0], &items[1]) {
                    if s == "define" && !sig.is_empty() {
                        if let LispVal::Sym(name) = &sig[0] {
                            let params: Vec<String> = sig[1..].iter().map(|p| match p {
                                LispVal::Sym(s) => Ok(s.clone()), _ => Err("param must be symbol".into()),
                            }).collect::<Result<_, String>>()?;
                            let body = if items.len() > 3 {
                                let mut b = vec![LispVal::Sym("begin".into())];
                                b.extend(items[2..].iter().cloned());
                                LispVal::List(b)
                            } else {
                                items[2].clone()
                            };
                            em.emit_define(name, &params, &body)?;
                        }
                    }
                }
                // Handle (export "name" fn_name is_view)
                if let LispVal::Sym(s) = &items[0] {
                    if s == "export" {
                        if let (LispVal::Str(en), LispVal::Sym(fn_)) = (&items[1], &items[2]) {
                            let view = items.len() > 3 && matches!(&items[3], LispVal::Bool(true));
                            em.add_export(fn_, en, view);
                        }
                    }
                }
            }
        }
    }
    Ok(em.finish("_run"))
}


pub fn compile_near_to_wat_from_exprs(exprs: &[LispVal]) -> Result<String, String> {
    let b = compile_near_from_exprs(exprs)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}


pub fn compile_pure_to_wat(source: &str) -> Result<String, String> {
    let b = compile_pure(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}


pub fn compile_near_to_wat(source: &str) -> Result<String, String> {
    let b = compile_near(source)?;
    wasmprinter::print_bytes(&b).map_err(|e| e.to_string())
}


pub fn resolve_modules(source: &str, base_dir: &std::path::Path) -> Result<String, String> {
    resolve_modules_inner(source, base_dir, &mut Vec::new())
}


fn resolve_modules_inner(source: &str, base_dir: &std::path::Path, seen: &mut Vec<std::path::PathBuf>) -> Result<String, String> {
    let mut resolved = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("(module ") {
            let rest = rest.strip_suffix(")").unwrap_or(rest);
            if let Some(path_start) = rest.find('"') {
                let path_end = rest.rfind('"').unwrap_or(rest.len());
                if path_start + 1 < path_end {
                    let path_str = &rest[path_start + 1..path_end];
                    let module_path = base_dir.join(path_str).canonicalize()
                        .map_err(|e| format!("module not found: {} — {}", path_str, e))?;
                    if seen.contains(&module_path) {
                        return Err(format!("circular module dependency: {}", module_path.display()));
                    }
                    seen.push(module_path.clone());
                    let module_source = std::fs::read_to_string(&module_path)
                        .map_err(|e| format!("module not found: {} — {}", module_path.display(), e))?;
                    let module_dir = module_path.parent().unwrap_or(base_dir);
                    let resolved_module = resolve_modules_inner(&module_source, module_dir, seen)?;
                    resolved.push_str(&resolved_module);
                    resolved.push('\n');
                }
            }
        } else {
            resolved.push_str(line);
            resolved.push('\n');
        }
    }
    Ok(resolved)
}

