/// WASM linker: merges a library WASM module into the compiler's output.
///
/// Uses wasmparser for instruction-level parsing (handles all opcodes including SIMD)
/// and wasm-encoder for output encoding.

use wasm_encoder;

// ─── Minimal WASM binary reader ──────────────────────────────────────

struct WasmReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> WasmReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_byte(&mut self) -> u8 {
        let b = self.data[self.pos];
        self.pos += 1;
        b
    }

    fn read_u32_leb(&mut self) -> u32 {
        let mut result: u32 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_byte();
            result |= ((byte & 0x7F) as u32) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                break;
            }
        }
        result
    }

    fn read_i32_leb(&mut self) -> i32 {
        let mut result: i32 = 0;
        let mut shift = 0i32;
        loop {
            let byte = self.read_byte();
            result |= ((byte & 0x7F) as i32) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 32 && (byte & 0x40) != 0 {
                    result |= !0i32 << shift;
                }
                break;
            }
        }
        result
    }

    fn read_i64_leb(&mut self) -> i64 {
        let mut result: i64 = 0;
        let mut shift = 0i64;
        loop {
            let byte = self.read_byte();
            result |= ((byte & 0x7F) as i64) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && (byte & 0x40) != 0 {
                    result |= !0i64 << shift;
                }
                break;
            }
        }
        result
    }

    fn read_bytes(&mut self, n: usize) -> &'a [u8] {
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        slice
    }

    fn read_name(&mut self) -> String {
        let len = self.read_u32_leb() as usize;
        let bytes = self.read_bytes(len);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

// ─── Section IDs ─────────────────────────────────────────────────────

const SEC_TYPE: u8 = 1;
const SEC_IMPORT: u8 = 2;
const SEC_FUNCTION: u8 = 3;
const SEC_TABLE: u8 = 4;
const SEC_MEMORY: u8 = 5;
const SEC_GLOBAL: u8 = 6;
const SEC_EXPORT: u8 = 7;
const SEC_START: u8 = 8;
const SEC_ELEMENT: u8 = 9;
const SEC_CODE: u8 = 10;
const SEC_DATA: u8 = 11;

// ─── WASM section parsing ────────────────────────────────────────────

/// Iterate over top-level WASM sections, yielding (section_id, payload_bytes).
fn sections(data: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut r = WasmReader::new(data);
    // Skip magic + version (8 bytes)
    r.pos = 8;
    let mut result = Vec::new();
    while r.remaining() > 0 {
        let id = r.read_byte();
        let size = r.read_u32_leb() as usize;
        let payload = r.read_bytes(size).to_vec();
        result.push((id, payload));
    }
    result
}

/// ValType encoding: i32=0x7F, i64=0x7E, f32=0x7D, f64=0x7C, funcref=0x70, externref=0x6F
fn read_valtype(r: &mut WasmReader) -> u8 {
    r.read_byte()
}

fn valtype_to_reftype(vt: u8) -> wasm_encoder::RefType {
    match vt {
        0x70 => wasm_encoder::RefType::FUNCREF,
        0x6F => wasm_encoder::RefType::EXTERNREF,
        _ => wasm_encoder::RefType::FUNCREF,
    }
}

fn valtype_to_encoder(vt: u8) -> wasm_encoder::ValType {
    match vt {
        0x7F => wasm_encoder::ValType::I32,
        0x7E => wasm_encoder::ValType::I64,
        0x7D => wasm_encoder::ValType::F32,
        0x7C => wasm_encoder::ValType::F64,
        0x70 => wasm_encoder::ValType::Ref(wasm_encoder::RefType::FUNCREF),
        0x6F => wasm_encoder::ValType::Ref(wasm_encoder::RefType::EXTERNREF),
        _ => wasm_encoder::ValType::I32,
    }
}

struct FuncType {
    params: Vec<u8>,
    results: Vec<u8>,
}

struct ImportEntry {
    module: String,
    name: String,
    kind: u8, // 0=func, 1=table, 2=memory, 3=global
    type_idx: u32,
    val_type: u8,
    mutable: bool,
}

struct GlobalEntry {
    val_type: u8,
    mutable: bool,
    init_expr: Vec<u8>,
}

struct ExportEntry {
    name: String,
    kind: u8, // 0=func, 1=table, 2=memory, 3=global
    index: u32,
}

struct ParsedModule {
    types: Vec<FuncType>,
    imports: Vec<ImportEntry>,
    func_type_indices: Vec<u32>,
    func_bodies: Vec<Vec<u8>>,    // raw code entries
    table_type: Option<(u8, u32, Option<u32>)>, // (elem_type, min, max)
    memory_min: u64,
    memory_max: Option<u64>,
    memory_shared: bool,
    globals: Vec<GlobalEntry>,
    exports: Vec<ExportEntry>,
    start_func: Option<u32>,
    element_segments: Vec<(u32, Vec<u32>)>, // (table_idx, func_indices)
    data_segments: Vec<(i32, Vec<u8>)>,     // (offset, bytes)
}

fn parse_wasm(data: &[u8]) -> ParsedModule {
    let secs = sections(data);
    let mut m = ParsedModule {
        types: Vec::new(),
        imports: Vec::new(),
        func_type_indices: Vec::new(),
        func_bodies: Vec::new(),
        table_type: None,
        memory_min: 0,
        memory_max: None,
        memory_shared: false,
        globals: Vec::new(),
        exports: Vec::new(),
        start_func: None,
        element_segments: Vec::new(),
        data_segments: Vec::new(),
    };

    for (id, payload) in &secs {
        let mut r = WasmReader::new(payload);
        match *id {
            SEC_TYPE => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let tag = r.read_byte();
                    assert_eq!(tag, 0x60, "only functype supported");
                    let param_count = r.read_u32_leb() as usize;
                    let params: Vec<u8> = (0..param_count).map(|_| read_valtype(&mut r)).collect();
                    let result_count = r.read_u32_leb() as usize;
                    let results: Vec<u8> = (0..result_count).map(|_| read_valtype(&mut r)).collect();
                    m.types.push(FuncType { params, results });
                }
            }
            SEC_IMPORT => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let module = r.read_name();
                    let name = r.read_name();
                    let kind = r.read_byte();
                    let type_idx = r.read_u32_leb();
                    let mut val_type = 0u8;
                    let mut mutable = false;
                    if kind == 3 {
                        val_type = read_valtype(&mut r);
                        mutable = r.read_byte() != 0;
                    }
                    m.imports.push(ImportEntry {
                        module,
                        name,
                        kind,
                        type_idx,
                        val_type,
                        mutable,
                    });
                }
            }
            SEC_FUNCTION => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    m.func_type_indices.push(r.read_u32_leb());
                }
            }
            SEC_TABLE => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let elem_type = read_valtype(&mut r);
                    let has_max = r.read_byte();
                    let min = r.read_u32_leb();
                    let max = if has_max != 0 { Some(r.read_u32_leb()) } else { None };
                    m.table_type = Some((elem_type, min, max));
                }
            }
            SEC_MEMORY => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let flags = r.read_byte();
                    m.memory_shared = (flags & 0x02) != 0;
                    m.memory_min = r.read_u32_leb() as u64;
                    let has_max = (flags & 0x01) != 0;
                    if has_max {
                        m.memory_max = Some(r.read_u32_leb() as u64);
                    }
                }
            }
            SEC_GLOBAL => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let val_type = read_valtype(&mut r);
                    let mutable = r.read_byte() != 0;
                    let start = r.pos;
                    loop {
                        let b = r.read_byte();
                        if b == 0x0B { break; }
                    }
                    let mut init_expr = payload[start..r.pos].to_vec();
                    init_expr.pop();
                    m.globals.push(GlobalEntry { val_type, mutable, init_expr });
                }
            }
            SEC_EXPORT => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let name = r.read_name();
                    let kind = r.read_byte();
                    let index = r.read_u32_leb();
                    m.exports.push(ExportEntry { name, kind, index });
                }
            }
            SEC_START => {
                m.start_func = Some(r.read_u32_leb());
            }
            SEC_ELEMENT => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let flags = r.read_u32_leb();
                    let table_idx = if (flags & 0x02) != 0 { r.read_u32_leb() } else { 0 };
                    let _kind = r.read_byte();
                    loop {
                        let b = r.read_byte();
                        if b == 0x0B { break; }
                    }
                    let func_count = r.read_u32_leb();
                    let mut funcs = Vec::new();
                    for _ in 0..func_count {
                        funcs.push(r.read_u32_leb());
                    }
                    m.element_segments.push((table_idx, funcs));
                }
            }
            SEC_CODE => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let body_size = r.read_u32_leb() as usize;
                    let body = r.read_bytes(body_size).to_vec();
                    m.func_bodies.push(body);
                }
            }
            SEC_DATA => {
                let count = r.read_u32_leb();
                for _ in 0..count {
                    let flags = r.read_u32_leb();
                    let mem_idx = if (flags & 0x02) != 0 { r.read_u32_leb() } else { 0 };
                    assert_eq!(mem_idx, 0, "only memory[0] data supported");
                    let mut offset = 0i32;
                    loop {
                        let b = r.read_byte();
                        if b == 0x0B { break; }
                        if b == 0x41 { offset = r.read_i32_leb(); }
                    }
                    let data_len = r.read_u32_leb() as usize;
                    let data_bytes = r.read_bytes(data_len).to_vec();
                    m.data_segments.push((offset, data_bytes));
                }
            }
            _ => {}
        }
    }
    m
}

/// Call remapping in raw function body ─────────────────────────────

/// Write an unsigned LEB128.
fn write_uleb128(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            out.push(byte | 0x80);
        } else {
            out.push(byte);
            break;
        }
    }
}

/// Remap call/call_indirect/global.get/set in a raw function body using wasmparser.
///
/// wasmparser correctly handles ALL WASM opcodes (SIMD, GC, etc.) so we never
/// corrupt the instruction stream. We parse operators with byte offsets, copy raw
/// bytes for the 99% of instructions that need no remapping, and only re-encode
/// the few that reference remappable indices.
fn remap_calls(
    body: &[u8],
    remap_fn: impl Fn(u32) -> u32,
    global_remap: impl Fn(u32) -> u32,
    type_offset: u32,
) -> Vec<u8> {
    // Use wasmparser to iterate operators with their byte offsets.
    // FunctionBody::new expects the FULL body (locals header + code).
    // Offsets from into_iter_with_offsets() are relative to body start.
    let reader = wasmparser::BinaryReader::new(body, 0);
    let fb = wasmparser::FunctionBody::new(reader);
    let ops_reader = fb
        .get_operators_reader()
        .unwrap_or_else(|e| panic!("wasmparser operators: {}", e));
    let ops: Vec<_> = ops_reader
        .into_iter_with_offsets()
        .map(|r| r.unwrap())
        .collect();

    // Build output by copying raw bytes, remapping only the 4 instruction types
    let mut out = Vec::with_capacity(body.len() + 32);

    // Copy everything before the first operator (locals header)
    if let Some((_, first_offset)) = ops.first() {
        out.extend_from_slice(&body[..*first_offset]);
    }

    for (i, (op, offset)) in ops.iter().enumerate() {
        let op_start = *offset;
        let op_end = if i + 1 < ops.len() {
            ops[i + 1].1
        } else {
            body.len()
        };

        match op {
            wasmparser::Operator::Call { function_index } => {
                out.push(0x10);
                write_uleb128(&mut out, remap_fn(*function_index));
            }
            wasmparser::Operator::CallIndirect {
                type_index,
                table_index,
            } => {
                out.push(0x11);
                write_uleb128(&mut out, *type_index + type_offset);
                write_uleb128(&mut out, *table_index);
            }
            wasmparser::Operator::GlobalGet { global_index } => {
                out.push(0x23);
                write_uleb128(&mut out, global_remap(*global_index));
            }
            wasmparser::Operator::GlobalSet { global_index } => {
                out.push(0x24);
                write_uleb128(&mut out, global_remap(*global_index));
            }
            _ => {
                // Copy raw bytes — handles all other opcodes correctly
                out.extend_from_slice(&body[op_start..op_end]);
            }
        }
    }

    out
}

// ─── Main merge function ─────────────────────────────────────────────

/// Embed schnorr from schnorr.wat (self-contained, no env var or build.rs).
pub fn link_schnorr_wat(contract_wasm: &[u8], import_export_pairs: &[(&str, &str)]) -> Vec<u8> {
    let lib_wasm = wat::parse_str(include_str!("schnorr.wat"))
        .expect("failed to parse schnorr.wat");
    match merge_lib_wasm_multi(contract_wasm, &lib_wasm, import_export_pairs) {
        Ok(bytes) => bytes,
        Err(e) => panic!("schnorr WASM linking failed: {}", e),
    }
}

fn merge_lib_wasm(
    contract_wasm: &[u8],
    lib_wasm: &[u8],
    import_name: &str,
) -> Result<Vec<u8>, String> {
    let contract = parse_wasm(contract_wasm);
    let lib = parse_wasm(lib_wasm);

    // Find the exported schnorr function in the library
    let mut schnorr_lib_idx: Option<u32> = None;
    for exp in lib.exports {
        if exp.kind == 0 && exp.name == import_name {
            schnorr_lib_idx = Some(exp.index);
            break;
        }
    }
    let schnorr_lib_idx =
        schnorr_lib_idx.ok_or_else(|| format!("export '{}' not found in library", import_name))?;

    // Find the schnorr import index
    let mut import_idx = None;
    let mut import_func_count = 0u32;
    for imp in contract.imports.iter() {
        if imp.kind == 0 {
            if imp.name == import_name {
                import_idx = Some(import_func_count);
            }
            import_func_count += 1;
        }
    }
    let import_idx = import_idx.ok_or_else(|| format!("import '{}' not found", import_name))?;

    let total_funcs = import_func_count + contract.func_type_indices.len() as u32;

    // In the merged module: [imports sans schnorr] [schnorr] [contract defs] [lib defs]
    let schnorr_idx = import_func_count - 1;

    // Remap contract bodies: schnorr import → schnorr_idx, other imports shifted
    let contract_remap = |target: u32| -> u32 {
        if target == import_idx {
            schnorr_idx
        } else if target > import_idx && target < import_func_count {
            target - 1
        } else if target >= import_func_count {
            target
        } else {
            target
        }
    };
    let contract_bodies: Vec<Vec<u8>> = contract
        .func_bodies
        .iter()
        .map(|body| remap_calls(body, contract_remap, |g| g, 0))
        .collect();

    // Remap library function bodies: lib func[i] → merged index
    let lib_bodies: Vec<Vec<u8>> = lib
        .func_bodies
        .iter()
        .map(|body| {
            remap_calls(
                body,
                |target| {
                    if target == schnorr_lib_idx {
                        schnorr_idx
                    } else {
                        let adj = if target > schnorr_lib_idx { target - 1 } else { target };
                        schnorr_idx + 1 + contract.func_type_indices.len() as u32 + adj
                    }
                },
                |g| g + contract.globals.len() as u32,
                contract.types.len() as u32,
            )
        })
        .collect();

    // Re-encode the merged module
    let mut m = wasm_encoder::Module::new();

    // Type section: contract types, then lib types
    let mut type_sec = wasm_encoder::TypeSection::new();
    for ft in &contract.types {
        type_sec.ty().function(
            ft.params.iter().copied().map(valtype_to_encoder),
            ft.results.iter().copied().map(valtype_to_encoder),
        );
    }
    let lib_type_offset = contract.types.len() as u32;
    for ft in &lib.types {
        type_sec.ty().function(
            ft.params.iter().copied().map(valtype_to_encoder),
            ft.results.iter().copied().map(valtype_to_encoder),
        );
    }
    m.section(&type_sec);

    // Import section: contract imports MINUS the schnorr one
    let mut imp_sec = wasm_encoder::ImportSection::new();
    let mut new_import_func_count = 0u32;
    for imp in &contract.imports {
        if imp.kind == 0 && imp.name == import_name {
            continue;
        }
        match imp.kind {
            0 => {
                imp_sec.import(
                    &imp.module,
                    &imp.name,
                    wasm_encoder::EntityType::Function(imp.type_idx),
                );
                new_import_func_count += 1;
            }
            3 => {
                imp_sec.import(
                    &imp.module,
                    &imp.name,
                    wasm_encoder::EntityType::Global(wasm_encoder::GlobalType {
                        val_type: valtype_to_encoder(imp.val_type),
                        mutable: imp.mutable,
                        shared: false,
                    }),
                );
            }
            _ => {}
        }
    }
    m.section(&imp_sec);

    // Function section: schnorr first, then contract defined, then lib[1..]
    let mut func_sec = wasm_encoder::FunctionSection::new();
    func_sec.function(lib_type_offset + lib.func_type_indices[schnorr_lib_idx as usize]);
    for &ti in &contract.func_type_indices {
        func_sec.function(ti);
    }
    for (i, &ti) in lib.func_type_indices.iter().enumerate() {
        if i == schnorr_lib_idx as usize {
            continue;
        }
        func_sec.function(lib_type_offset + ti);
    }
    m.section(&func_sec);

    // Table section
    if contract.table_type.is_some() || lib.table_type.is_some() {
        let mut table_sec = wasm_encoder::TableSection::new();
        if let Some((et, min, max)) = contract.table_type {
            table_sec.table(wasm_encoder::TableType {
                element_type: valtype_to_reftype(et),
                table64: false,
                shared: false,
                minimum: min as u64,
                maximum: max.map(|m| m as u64),
            });
        }
        if let Some((et, min, max)) = lib.table_type {
            table_sec.table(wasm_encoder::TableType {
                element_type: valtype_to_reftype(et),
                table64: false,
                shared: false,
                minimum: min as u64,
                maximum: max.map(|m| m as u64),
            });
        }
        m.section(&table_sec);
    }

    // Memory section — ensure enough pages for lib's stack pointer and data segments.
    let highest_global_addr = lib
        .globals
        .iter()
        .filter_map(|g| {
            if g.init_expr.len() >= 2 && g.init_expr[0] == 0x41 {
                let mut r = WasmReader::new(&g.init_expr[1..]);
                let val = r.read_i32_leb();
                if val > 0 { Some(val as u64) } else { None }
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    let highest_data_addr = lib
        .data_segments
        .iter()
        .map(|(off, data)| (*off as u64).saturating_add(data.len() as u64))
        .max()
        .unwrap_or(0);
    let highest = highest_global_addr.max(highest_data_addr);
    let pages_needed = if highest > 0 { (highest / 65536) + 32 } else { 0 };
    let mem_min = std::cmp::max(contract.memory_min, std::cmp::max(lib.memory_min, pages_needed));
    let mut mem_sec = wasm_encoder::MemorySection::new();
    mem_sec.memory(wasm_encoder::MemoryType {
        minimum: mem_min,
        maximum: contract.memory_max,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mem_sec);

    // Global section: contract + lib
    let mut glob_sec = wasm_encoder::GlobalSection::new();
    for g in &contract.globals {
        glob_sec.global(
            wasm_encoder::GlobalType {
                val_type: valtype_to_encoder(g.val_type),
                mutable: g.mutable,
                shared: false,
            },
            &wasm_encoder::ConstExpr::raw(g.init_expr.iter().copied()),
        );
    }
    for g in &lib.globals {
        glob_sec.global(
            wasm_encoder::GlobalType {
                val_type: valtype_to_encoder(g.val_type),
                mutable: g.mutable,
                shared: false,
            },
            &wasm_encoder::ConstExpr::raw(g.init_expr.iter().copied()),
        );
    }
    m.section(&glob_sec);

    // Export section
    let mut exp_sec = wasm_encoder::ExportSection::new();
    for exp in &contract.exports {
        let new_index = if exp.kind == 0 {
            if exp.index == import_idx {
                schnorr_idx
            } else if exp.index > import_idx && exp.index < import_func_count {
                exp.index - 1
            } else if exp.index >= import_func_count {
                exp.index
            } else {
                exp.index
            }
        } else {
            exp.index
        };
        exp_sec.export(
            &exp.name,
            match exp.kind {
                0 => wasm_encoder::ExportKind::Func,
                1 => wasm_encoder::ExportKind::Table,
                2 => wasm_encoder::ExportKind::Memory,
                3 => wasm_encoder::ExportKind::Global,
                _ => wasm_encoder::ExportKind::Func,
            },
            new_index,
        );
    }
    m.section(&exp_sec);

    // Start section
    if let Some(start) = contract.start_func {
        let new_start = if start == import_idx {
            schnorr_idx
        } else if start > import_idx && start < import_func_count {
            start - 1
        } else if start >= import_func_count {
            start
        } else {
            start
        };
        m.section(&wasm_encoder::StartSection {
            function_index: new_start,
        });
    }

    // Element section: contract + library element segments
    if !contract.element_segments.is_empty() || !lib.element_segments.is_empty() {
        let mut elem_sec = wasm_encoder::ElementSection::new();
        for &(table_idx, ref funcs) in &contract.element_segments {
            let remapped: Vec<u32> = funcs
                .iter()
                .map(|&f| {
                    if f == import_idx { schnorr_idx }
                    else if f > import_idx && f < import_func_count { f - 1 }
                    else { f }
                })
                .collect();
            elem_sec.active(
                if table_idx == 0 { None } else { Some(table_idx) },
                &wasm_encoder::ConstExpr::i32_const(0),
                wasm_encoder::Elements::Functions(remapped.into()),
            );
        }
        // Library element segments: remap func indices to merged module space
        let contract_table_count = if contract.table_type.is_some() { 1u32 } else { 0 };
        for &(table_idx, ref funcs) in &lib.element_segments {
            let remapped: Vec<u32> = funcs
                .iter()
                .map(|&f| {
                    if f == schnorr_lib_idx { schnorr_idx }
                    else {
                        let adj = if f > schnorr_lib_idx { f - 1 } else { f };
                        schnorr_idx + 1 + contract.func_type_indices.len() as u32 + adj
                    }
                })
                .collect();
            elem_sec.active(
                if table_idx == 0 { None } else { Some(table_idx + contract_table_count) },
                &wasm_encoder::ConstExpr::i32_const(0),
                wasm_encoder::Elements::Functions(remapped.into()),
            );
        }
        m.section(&elem_sec);
    }

    // Code section: schnorr first, then contract, then lib[1..]
    let mut code_sec = wasm_encoder::CodeSection::new();
    code_sec.raw(&lib_bodies[schnorr_lib_idx as usize]);
    for body in &contract_bodies {
        code_sec.raw(body);
    }
    for (i, body) in lib_bodies.iter().enumerate() {
        if i == schnorr_lib_idx as usize {
            continue;
        }
        code_sec.raw(body);
    }
    m.section(&code_sec);

    // Data section
    let mut data_sec = wasm_encoder::DataSection::new();
    for &(offset, ref bytes) in &contract.data_segments {
        data_sec.active(
            0,
            &wasm_encoder::ConstExpr::i32_const(offset),
            bytes.iter().copied(),
        );
    }
    for &(offset, ref bytes) in &lib.data_segments {
        data_sec.active(
            0,
            &wasm_encoder::ConstExpr::i32_const(offset),
            bytes.iter().copied(),
        );
    }
    m.section(&data_sec);

    let result = m.finish();
    Ok(result)
}

/// Generalized merge supporting N import/export pairs.
///
/// For each (import_name, export_name) pair, the contract's import named
/// `import_name` is replaced by the library's exported function `export_name`.
///
/// Merged function layout:
///   [remaining imports] [resolved lib funcs in sorted-import-order]
///   [contract defined funcs] [remaining lib funcs]
fn merge_lib_wasm_multi(
    contract_wasm: &[u8],
    lib_wasm: &[u8],
    import_export_pairs: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let contract = parse_wasm(contract_wasm);
    let lib = parse_wasm(lib_wasm);
    let n = import_export_pairs.len() as u32;

    // --- 1. Find lib export indices for each pair ---
    let mut lib_export_indices: Vec<u32> = Vec::with_capacity(n as usize);
    for &(_import_name, export_name) in import_export_pairs {
        let mut found = None;
        for exp in &lib.exports {
            if exp.kind == 0 && exp.name == export_name {
                found = Some(exp.index);
                break;
            }
        }
        let idx = found.ok_or_else(|| format!("export '{}' not found in library", export_name))?;
        lib_export_indices.push(idx);
    }

    // --- 2. Find contract import indices (function-index space) for each pair ---
    let mut contract_import_indices: Vec<u32> = Vec::with_capacity(n as usize);
    let mut import_func_count = 0u32;
    for imp in &contract.imports {
        if imp.kind == 0 {
            // Check if this import matches any of our pairs
            for &(import_name, _export_name) in import_export_pairs {
                if imp.name == import_name {
                    contract_import_indices.push(import_func_count);
                    break;
                }
            }
            import_func_count += 1;
        }
    }
    if contract_import_indices.len() != n as usize {
        return Err(format!(
            "expected {} matching imports, found {}",
            n,
            contract_import_indices.len()
        ));
    }

    // --- 3. Sort by contract import index ascending ---
    // Build a list of (sorted_position, contract_import_idx, lib_export_idx, export_name)
    let mut sorted: Vec<(usize, u32, u32, &str)> = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        sorted.push((
            i, // original position in import_export_pairs
            contract_import_indices[i],
            lib_export_indices[i],
            import_export_pairs[i].1,
        ));
    }
    sorted.sort_by_key(|&(_, cidx, _, _)| cidx);

    let sorted_contract_indices: Vec<u32> = sorted.iter().map(|&(_, ci, _, _)| ci).collect();
    let sorted_lib_indices: Vec<u32> = sorted.iter().map(|&(_, _, li, _)| li).collect();
    let sorted_original_pos: Vec<usize> = sorted.iter().map(|&(op, _, _, _)| op).collect();

    // Map from lib export index to its position in sorted list (for lib remapping)
    // sorted_original_pos[k] = original position k, so for a given lib index we find its sorted position
    let mut lib_idx_to_sorted_pos: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (k, &li) in sorted_lib_indices.iter().enumerate() {
        lib_idx_to_sorted_pos.insert(li, k);
    }

    // New import count (after removing N)
    let new_import_count = import_func_count - n;
    // The resolved lib functions start at new_import_count in the merged index space
    // Position k (in sorted order) maps to index: new_import_count + k

    // --- 4. Remap contract function bodies ---
    // Merged index space:
    //   [0, new_import_count)                          remaining imports
    //   [new_import_count, new_import_count + n)       resolved lib funcs (sorted)
    //   [new_import_count + n, ...)                    contract defined funcs
    // Since new_import_count + n == import_func_count, a contract defined
    // function keeps its ORIGINAL index (identity). Only remaining imports
    // shift down by the removed ones before them.
    let contract_remap = |target: u32| -> u32 {
        // Check if target is one of the removed import indices
        if let Some(pos) = sorted_contract_indices.iter().position(|&ci| ci == target) {
            new_import_count + pos as u32
        } else if target < import_func_count {
            // Remaining import: shift down by removed imports before it
            let removed_before = sorted_contract_indices.iter().filter(|&&ci| ci < target).count() as u32;
            target - removed_before
        } else {
            // Contract defined function: identity
            target
        }
    };
    let contract_bodies: Vec<Vec<u8>> = contract
        .func_bodies
        .iter()
        .map(|body| remap_calls(body, contract_remap, |g| g, 0))
        .collect();

    // --- 5. Remap lib function bodies ---
    // Lib func index t in merged module:
    //   - If t is a resolved lib function (in sorted_lib_indices at position k):
    //       → new_import_count + k
    //   - Otherwise, standard offset: new_import_count + n + contract.func_type_indices.len() + adjusted_t
    //     where adjusted_t = t - (number of resolved lib funcs with index < t)
    let lib_bodies: Vec<Vec<u8>> = lib
        .func_bodies
        .iter()
        .map(|body| {
            remap_calls(
                body,
                |target: u32| {
                    if let Some(&k) = lib_idx_to_sorted_pos.get(&target) {
                        new_import_count + k as u32
                    } else {
                        let resolved_before = sorted_lib_indices.iter().filter(|&&li| li < target).count() as u32;
                        let adj = target - resolved_before;
                        new_import_count + n + contract.func_type_indices.len() as u32 + adj
                    }
                },
                |g| g + contract.globals.len() as u32,
                contract.types.len() as u32,
            )
        })
        .collect();

    // --- 6. Re-encode the merged module ---
    let mut m = wasm_encoder::Module::new();

    // Type section: contract types, then lib types
    let mut type_sec = wasm_encoder::TypeSection::new();
    for ft in &contract.types {
        type_sec.ty().function(
            ft.params.iter().copied().map(valtype_to_encoder),
            ft.results.iter().copied().map(valtype_to_encoder),
        );
    }
    let lib_type_offset = contract.types.len() as u32;
    for ft in &lib.types {
        type_sec.ty().function(
            ft.params.iter().copied().map(valtype_to_encoder),
            ft.results.iter().copied().map(valtype_to_encoder),
        );
    }
    m.section(&type_sec);

    // Import section: contract imports MINUS the resolved ones
    let mut imp_sec = wasm_encoder::ImportSection::new();
    let mut removed_import_names: std::collections::HashSet<&str> =
        import_export_pairs.iter().map(|&(name, _)| name).collect();
    let mut new_import_func_count = 0u32;
    for imp in &contract.imports {
        if imp.kind == 0 && removed_import_names.contains(imp.name.as_str()) {
            continue;
        }
        match imp.kind {
            0 => {
                imp_sec.import(
                    &imp.module,
                    &imp.name,
                    wasm_encoder::EntityType::Function(imp.type_idx),
                );
                new_import_func_count += 1;
            }
            3 => {
                imp_sec.import(
                    &imp.module,
                    &imp.name,
                    wasm_encoder::EntityType::Global(wasm_encoder::GlobalType {
                        val_type: valtype_to_encoder(imp.val_type),
                        mutable: imp.mutable,
                        shared: false,
                    }),
                );
            }
            _ => {}
        }
    }
    m.section(&imp_sec);

    // Function section: resolved lib funcs (in sorted order), then contract defined, then remaining lib
    let mut func_sec = wasm_encoder::FunctionSection::new();
    for &li in &sorted_lib_indices {
        func_sec.function(lib_type_offset + lib.func_type_indices[li as usize]);
    }
    for &ti in &contract.func_type_indices {
        func_sec.function(ti);
    }
    for (i, &ti) in lib.func_type_indices.iter().enumerate() {
        if lib_idx_to_sorted_pos.contains_key(&(i as u32)) {
            continue;
        }
        func_sec.function(lib_type_offset + ti);
    }
    m.section(&func_sec);

    // Table section
    if contract.table_type.is_some() || lib.table_type.is_some() {
        let mut table_sec = wasm_encoder::TableSection::new();
        if let Some((et, min, max)) = contract.table_type {
            table_sec.table(wasm_encoder::TableType {
                element_type: valtype_to_reftype(et),
                table64: false,
                shared: false,
                minimum: min as u64,
                maximum: max.map(|mx| mx as u64),
            });
        }
        if let Some((et, min, max)) = lib.table_type {
            table_sec.table(wasm_encoder::TableType {
                element_type: valtype_to_reftype(et),
                table64: false,
                shared: false,
                minimum: min as u64,
                maximum: max.map(|mx| mx as u64),
            });
        }
        m.section(&table_sec);
    }

    // Memory section
    let highest_global_addr = lib
        .globals
        .iter()
        .filter_map(|g| {
            if g.init_expr.len() >= 2 && g.init_expr[0] == 0x41 {
                let mut r = WasmReader::new(&g.init_expr[1..]);
                let val = r.read_i32_leb();
                if val > 0 {
                    Some(val as u64)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    let highest_data_addr = lib
        .data_segments
        .iter()
        .map(|(off, data)| (*off as u64).saturating_add(data.len() as u64))
        .max()
        .unwrap_or(0);
    let highest = highest_global_addr.max(highest_data_addr);
    let pages_needed = if highest > 0 {
        (highest / 65536) + 32
    } else {
        0
    };
    let mem_min = std::cmp::max(
        contract.memory_min,
        std::cmp::max(lib.memory_min, pages_needed),
    );
    let mut mem_sec = wasm_encoder::MemorySection::new();
    mem_sec.memory(wasm_encoder::MemoryType {
        minimum: mem_min,
        maximum: contract.memory_max,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mem_sec);

    // Global section: contract + lib
    let mut glob_sec = wasm_encoder::GlobalSection::new();
    for g in &contract.globals {
        glob_sec.global(
            wasm_encoder::GlobalType {
                val_type: valtype_to_encoder(g.val_type),
                mutable: g.mutable,
                shared: false,
            },
            &wasm_encoder::ConstExpr::raw(g.init_expr.iter().copied()),
        );
    }
    for g in &lib.globals {
        glob_sec.global(
            wasm_encoder::GlobalType {
                val_type: valtype_to_encoder(g.val_type),
                mutable: g.mutable,
                shared: false,
            },
            &wasm_encoder::ConstExpr::raw(g.init_expr.iter().copied()),
        );
    }
    m.section(&glob_sec);

    // Export section: remap contract exports
    let mut exp_sec = wasm_encoder::ExportSection::new();
    for exp in &contract.exports {
        let new_index = if exp.kind == 0 {
            contract_remap(exp.index)
        } else {
            exp.index
        };
        exp_sec.export(
            &exp.name,
            match exp.kind {
                0 => wasm_encoder::ExportKind::Func,
                1 => wasm_encoder::ExportKind::Table,
                2 => wasm_encoder::ExportKind::Memory,
                3 => wasm_encoder::ExportKind::Global,
                _ => wasm_encoder::ExportKind::Func,
            },
            new_index,
        );
    }
    m.section(&exp_sec);

    // Start section
    if let Some(start) = contract.start_func {
        m.section(&wasm_encoder::StartSection {
            function_index: contract_remap(start),
        });
    }

    // Element section: contract + library element segments
    if !contract.element_segments.is_empty() || !lib.element_segments.is_empty() {
        let mut elem_sec = wasm_encoder::ElementSection::new();
        for &(table_idx, ref funcs) in &contract.element_segments {
            let remapped: Vec<u32> = funcs.iter().map(|&f| contract_remap(f)).collect();
            elem_sec.active(
                if table_idx == 0 {
                    None
                } else {
                    Some(table_idx)
                },
                &wasm_encoder::ConstExpr::i32_const(0),
                wasm_encoder::Elements::Functions(remapped.into()),
            );
        }
        // Library element segments: remap func indices to merged module space
        let contract_table_count = if contract.table_type.is_some() {
            1u32
        } else {
            0
        };
        for &(table_idx, ref funcs) in &lib.element_segments {
            let remapped: Vec<u32> = funcs
                .iter()
                .map(|&f| {
                    if let Some(&k) = lib_idx_to_sorted_pos.get(&f) {
                        new_import_count + k as u32
                    } else {
                        let resolved_before =
                            sorted_lib_indices.iter().filter(|&&li| li < f).count() as u32;
                        let adj = f - resolved_before;
                        new_import_count + n + contract.func_type_indices.len() as u32 + adj
                    }
                })
                .collect();
            elem_sec.active(
                if table_idx == 0 {
                    None
                } else {
                    Some(table_idx + contract_table_count)
                },
                &wasm_encoder::ConstExpr::i32_const(0),
                wasm_encoder::Elements::Functions(remapped.into()),
            );
        }
        m.section(&elem_sec);
    }

    // Code section: resolved lib funcs (sorted order), then contract, then remaining lib
    let mut code_sec = wasm_encoder::CodeSection::new();
    for &li in &sorted_lib_indices {
        code_sec.raw(&lib_bodies[li as usize]);
    }
    for body in &contract_bodies {
        code_sec.raw(body);
    }
    for (i, body) in lib_bodies.iter().enumerate() {
        if lib_idx_to_sorted_pos.contains_key(&(i as u32)) {
            continue;
        }
        code_sec.raw(body);
    }
    m.section(&code_sec);

    // Data section
    let mut data_sec = wasm_encoder::DataSection::new();
    for &(offset, ref bytes) in &contract.data_segments {
        data_sec.active(
            0,
            &wasm_encoder::ConstExpr::i32_const(offset),
            bytes.iter().copied(),
        );
    }
    for &(offset, ref bytes) in &lib.data_segments {
        data_sec.active(
            0,
            &wasm_encoder::ConstExpr::i32_const(offset),
            bytes.iter().copied(),
        );
    }
    m.section(&data_sec);

    // Name custom section: remap contract names into merged index space.
    // A resolved import keeps its import-derived name but moves to its new
    // lib slot; remaining imports shift down via contract_remap; contract
    // defined funcs keep their index. Lib-internal functions have no names
    // in the contract map and are dropped.
    if let Some(entries) = crate::wasm_emit::name_map::decode_function_names(contract_wasm) {
        let mut resolved_pos: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();
        for (k, &ci) in contract_import_indices.iter().enumerate() {
            resolved_pos.insert(ci, k);
        }
        let total_funcs = import_func_count + contract.func_type_indices.len() as u32;
        let mapped: Vec<(u32, String)> = entries
            .into_iter()
            .filter_map(|(idx, name)| {
                let new_idx = if idx < import_func_count {
                    match resolved_pos.get(&idx) {
                        Some(&k) => Some(new_import_count + k as u32),
                        None => Some(contract_remap(idx)),
                    }
                } else if idx < total_funcs {
                    Some(idx)
                } else {
                    None
                };
                new_idx.map(|n| (n, name))
            })
            .collect();
        let sec = crate::wasm_emit::name_map::name_section(&mapped);
        m.section(&sec);
    }

    let result = m.finish();
    Ok(result)
}
