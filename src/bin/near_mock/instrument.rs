//! Mainnet-parity gas + stack instrumentation (finite-wasm style).
//!
//! Ported from nearcore's `near-vm-runner/src/prepare/instrument_v3.rs`
//! (itself adapted from the finite-wasm project). Every contract compiled by
//! near-mock passes through here, exactly like every mainnet contract passes
//! through nearcore's prepare step. This replaces wasmtime fuel as the gas
//! meter: PV155 semantics — regular_op_cost per operator (control flow free),
//! linear_base + linear_unit × length for bulk memory/table ops and grow.
//!
//! Emission shape (identical to mainnet's prepared modules):
//! - 3 imports prepended under module "internal":
//!     finite_wasm_gas_exhausted() -> ()   — trap hook
//!     finite_wasm_stack_exhausted() -> () — trap hook
//!     finite_wasm_gas(i64) -> ()          — report hook on the failing charge
//! - 2 globals appended: remaining_gas (i64, mutable, EXPORTED "remaining_gas")
//!   and the stack budget (init = max_stack_height = 262_144).
//! - Constant fees: inline `global.get / i64.const / i64.lt_u / if / sub /
//!   global.set`; exhaustion path calls the hook then `unreachable`.
//! - Linear fees: runtime length operand × unit + constant via checked
//!   i64 math (nearcore's variant uses wide-arithmetic opcodes for the
//!   checked math; we use plain i64 — our gas magnitudes cannot overflow).
//! - Function-entry/exit charge/release of (frame + operand stack) against
//!   the stack budget global → mainnet's max_stack_height enforcement.

use crate::{
    LINEAR_OP_BASE_COST, LINEAR_OP_UNIT_COST, MAX_STACK_HEIGHT, REGULAR_OP_COST,
};
use finite_wasm::gas::InstrumentationKind;
use finite_wasm::{AnalysisOutcome, Fee};
use wasm_encoder::reencode::{Error as ReencodeError, Reencode};
use wasm_encoder::{self as we, InstructionSink};
use wasmparser as wp;

pub(crate) const REMAINING_GAS_EXPORT: &str = "remaining_gas";

const PLACEHOLDER_FOR_NAMES: u8 = !0;

const GAS_GLOBAL: u32 = 0;
const STACK_GLOBAL: u32 = GAS_GLOBAL + 1;
const G: u32 = STACK_GLOBAL + 1;

const GAS_EXHAUSTED_FN: u32 = 0;
const STACK_EXHAUSTED_FN: u32 = GAS_EXHAUSTED_FN + 1;
const GAS_INSTRUMENTATION_FN: u32 = STACK_EXHAUSTED_FN + 1;
const F: u32 = GAS_INSTRUMENTATION_FN + 1;

#[derive(Debug)]
pub(crate) enum InstrumentError {
    Parse(String),
    MissingAnalysis(&'static str, usize),
    InsufficientFunctionTypes,
    InvalidTypeIndex,
    TooManyGlobals,
    TooManyLocals,
    GasInstrumentationMisconfigured,
}

impl std::fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstrumentError::Parse(s) => write!(f, "instrument: parse failure: {s}"),
            InstrumentError::MissingAnalysis(k, i) => {
                write!(f, "instrument: analysis missing {k} entry for function {i}")
            }
            InstrumentError::InsufficientFunctionTypes => {
                write!(f, "instrument: function types exhausted")
            }
            InstrumentError::InvalidTypeIndex => write!(f, "instrument: invalid type index"),
            InstrumentError::TooManyGlobals => write!(f, "instrument: too many globals"),
            InstrumentError::TooManyLocals => write!(f, "instrument: too many locals"),
            InstrumentError::GasInstrumentationMisconfigured => {
                write!(f, "instrument: linear fee on non-aggregate operation")
            }
        }
    }
}
impl std::error::Error for InstrumentError {}

#[derive(Debug)]
struct ReencodeUserError;

impl std::fmt::Display for ReencodeUserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "function index remap overflow")
    }
}
impl std::error::Error for ReencodeUserError {}

struct InstrumentationReencoder;

impl Reencode for InstrumentationReencoder {
    type Error = ReencodeUserError;

    fn function_index(&mut self, func: u32) -> Result<u32, ReencodeError<Self::Error>> {
        func.checked_add(F)
            .ok_or(ReencodeError::UserError(ReencodeUserError))
    }
}

/// Analyze + instrument a validated module: mainnet prepare's gas/stack pass.
pub(crate) fn instrument(wasm: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Cost model = mainnet's SimpleGasCostCfg (prepare_v3.rs):
    //   block/end/else free; memory_init/copy/fill, table_init/copy/fill,
    //   memory_grow, table_grow linear; everything else regular.
    // (macro-generated exactly like nearcore's gas_cost! — every operator
    // gets a visitor so the analysis sees a complete cost table)
    use finite_wasm::wasmparser as fwp;
    struct GasCosts;

    macro_rules! gas_cost {
        ($( @$proposal:ident $op:ident $({ $($arg:ident: $argty:ty),* })? => $visit:ident ($($ann:tt)*))*) => {
            $(
                #[allow(unused_variables)]
                fn $visit(&mut self $($(, $arg: $argty)*)?) -> Fee {
                    gas_cost!(@@self $visit)
                }
            )*
        };

        (@@$self:ident visit_block) => { Fee::ZERO };
        (@@$self:ident visit_end) => { Fee::ZERO };
        (@@$self:ident visit_else) => { Fee::ZERO };
        (@@$self:ident visit_memory_init) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_memory_copy) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_memory_fill) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_table_init) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_table_copy) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_table_fill) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_memory_grow) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident visit_table_grow) => { Fee { linear: LINEAR_OP_UNIT_COST, constant: LINEAR_OP_BASE_COST } };
        (@@$self:ident $visit:ident) => { Fee::constant(REGULAR_OP_COST) };
    }

    impl<'a> fwp::VisitOperator<'a> for GasCosts {
        type Output = Fee;
        fwp::for_each_visit_operator!(gas_cost);
    }
    impl<'a> fwp::VisitSimdOperator<'a> for GasCosts {
        fwp::for_each_visit_simd_operator!(gas_cost);
    }

    // max-stack config = mainnet's SimpleMaxStackCfg
    struct MaxStackCfg;
    impl finite_wasm::max_stack::SizeConfig for MaxStackCfg {
        fn size_of_value(&self, ty: fwp::ValType) -> u8 {
            match ty {
                fwp::ValType::I32 | fwp::ValType::F32 => 4,
                fwp::ValType::I64 | fwp::ValType::F64 => 8,
                fwp::ValType::V128 => 16,
                fwp::ValType::Ref(_) => 8,
            }
        }
        fn size_of_function_activation(
            &self,
            locals: &finite_wasm::prefix_sum_vec::PrefixSumVec<fwp::ValType, u32>,
        ) -> u64 {
            let mut res = 64u64;
            let mut last = 0u64;
            for (idx, local) in locals {
                let idx = u64::from(*idx);
                res = res.saturating_add(
                    idx.checked_sub(last)
                        .expect("prefix-sum indices ascend")
                        .saturating_add(1)
                        .saturating_mul(u64::from(self.size_of_value(*local))),
                );
                last = idx.saturating_add(1);
            }
            res
        }
    }

    let analysis = finite_wasm::Analysis::new()
        .with_stack(MaxStackCfg)
        .with_gas(GasCosts)
        .analyze(wasm)?;
    InstrumentContext::new(wasm, "internal", &analysis).run()
}

struct InstrumentContext<'a> {
    analysis: &'a AnalysisOutcome,
    wasm: &'a [u8],
    import_env: &'a str,
    globals: u32,

    type_section: we::TypeSection,
    import_section: we::ImportSection,
    function_section: we::FunctionSection,
    table_section: Option<we::RawSection<'a>>,
    memory_section: Option<we::RawSection<'a>>,
    global_section: we::GlobalSection,
    export_section: we::ExportSection,
    element_section: we::ElementSection,
    datacount_section: Option<we::RawSection<'a>>,
    code_section: we::CodeSection,
    name_section: we::NameSection,
    raw_sections: Vec<we::RawSection<'a>>,

    types: Vec<we::FuncType>,
    function_types: std::vec::IntoIter<u32>,
}

impl<'a> InstrumentContext<'a> {
    fn new(wasm: &'a [u8], import_env: &'a str, analysis: &'a AnalysisOutcome) -> Self {
        Self {
            analysis,
            wasm,
            import_env,
            globals: 0,
            type_section: we::TypeSection::new(),
            import_section: we::ImportSection::new(),
            function_section: we::FunctionSection::new(),
            table_section: None,
            memory_section: None,
            global_section: we::GlobalSection::new(),
            export_section: we::ExportSection::new(),
            element_section: we::ElementSection::new(),
            datacount_section: None,
            code_section: we::CodeSection::new(),
            name_section: we::NameSection::new(),
            raw_sections: vec![],
            types: vec![],
            function_types: vec![].into_iter(),
        }
    }

    fn run(mut self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let parser = wp::Parser::new(0);
        let mut renc = InstrumentationReencoder;
        for payload in parser.parse_all(self.wasm) {
            let payload = payload.map_err(|e| InstrumentError::Parse(e.to_string()))?;
            match payload {
                wp::Payload::Version { .. } | wp::Payload::End(_) => {}
                wp::Payload::TypeSection(types) => {
                    for ty in types.into_iter_err_on_gc_types() {
                        let ty = ty.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        let ty = renc
                            .func_type(ty)
                            .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        self.type_section.ty().func_type(&ty);
                        self.types.push(ty);
                    }
                }
                wp::Payload::ImportSection(imports) => {
                    self.maybe_add_imports();
                    for import in imports.into_imports() {
                        let import = import.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        if let wp::TypeRef::Global(..) = import.ty {
                            self.globals = self
                                .globals
                                .checked_add(1)
                                .ok_or(InstrumentError::TooManyGlobals)?;
                        }
                        renc.parse_import(&mut self.import_section, import)
                            .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                    }
                }
                wp::Payload::StartSection { func, .. } => {
                    // Export the start fn under a well-known name; the runner
                    // calls it after setting the remaining_gas global (nearcore
                    // parity: start sections must not run at instantiation).
                    let idx = renc
                        .function_index(func)
                        .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                    self.export_section
                        .export("start", we::ExportKind::Func, idx);
                }
                wp::Payload::ElementSection(reader) => {
                    renc.parse_element_section(&mut self.element_section, reader)
                        .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                }
                wp::Payload::FunctionSection(reader) => {
                    let fn_types = reader
                        .into_iter()
                        .collect::<Result<Vec<u32>, _>>()
                        .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                    for fnty in &fn_types {
                        self.function_section.function(*fnty);
                    }
                    self.function_types = fn_types.into_iter();
                }
                wp::Payload::TableSection(..) | wp::Payload::MemorySection(..) => {
                    let (id, range) = payload.as_section().unwrap();
                    let raw = we::RawSection {
                        id,
                        data: self.wasm.get(range).unwrap(),
                    };
                    match payload {
                        wp::Payload::TableSection(..) => self.table_section = Some(raw),
                        _ => self.memory_section = Some(raw),
                    }
                }
                wp::Payload::CodeSectionStart { .. } => {}
                wp::Payload::CodeSectionEntry(reader) => {
                    self.maybe_add_imports();
                    if self.global_section.is_empty() {
                        self.add_globals();
                    }
                    let type_index = self
                        .function_types
                        .next()
                        .ok_or(InstrumentError::InsufficientFunctionTypes)?;
                    self.transform_code_section(&mut renc, reader, type_index)?;
                }
                wp::Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        let (kind, index) = match export.kind {
                            wp::ExternalKind::Func | wp::ExternalKind::FuncExact => {
                                let idx = renc
                                    .function_index(export.index)
                                    .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                                (we::ExportKind::Func, idx)
                            }
                            wp::ExternalKind::Table => (we::ExportKind::Table, export.index),
                            wp::ExternalKind::Memory => (we::ExportKind::Memory, export.index),
                            wp::ExternalKind::Global => (we::ExportKind::Global, export.index),
                            wp::ExternalKind::Tag => (we::ExportKind::Tag, export.index),
                        };
                        self.export_section.export(export.name, kind, index);
                    }
                }
                wp::Payload::GlobalSection(reader) => {
                    for global in reader {
                        let global = global.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        renc.parse_global(&mut self.global_section, global)
                            .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        self.globals = self
                            .globals
                            .checked_add(1)
                            .ok_or(InstrumentError::TooManyGlobals)?;
                    }
                    if self.globals.checked_add(G).is_none() {
                        return Err(Box::new(InstrumentError::TooManyGlobals));
                    }
                    self.add_globals();
                }
                wp::Payload::DataCountSection { .. } => {
                    let (id, range) = payload.as_section().unwrap();
                    self.datacount_section = Some(we::RawSection {
                        id,
                        data: self.wasm.get(range).unwrap(),
                    });
                }
                wp::Payload::CustomSection(reader) if reader.name() == "name" => {
                    let wp::KnownCustom::Name(names) = reader.as_known() else {
                        continue;
                    };
                    if let Ok(()) = self.transform_name_section(&mut renc, names) {
                        self.raw_sections.push(we::RawSection {
                            id: PLACEHOLDER_FOR_NAMES,
                            data: &[],
                        });
                    }
                }
                _ => {
                    let (id, range) = payload.as_section().unwrap();
                    self.raw_sections.push(we::RawSection {
                        id,
                        data: self.wasm.get(range).unwrap(),
                    });
                }
            }
        }
        let mut output = wasm_encoder::Module::new();
        if !self.type_section.is_empty() {
            output.section(&self.type_section);
        }
        if !self.import_section.is_empty() {
            output.section(&self.import_section);
        }
        if !self.function_section.is_empty() {
            output.section(&self.function_section);
        }
        if let Some(s) = self.table_section {
            output.section(&s);
        }
        if let Some(s) = self.memory_section {
            output.section(&s);
        }
        if !self.global_section.is_empty() {
            output.section(&self.global_section);
        }
        if !self.export_section.is_empty() {
            output.section(&self.export_section);
        }
        if !self.element_section.is_empty() {
            output.section(&self.element_section);
        }
        if let Some(s) = self.datacount_section {
            output.section(&s);
        }
        if !self.code_section.is_empty() {
            output.section(&self.code_section);
        }
        for s in self.raw_sections {
            match s.id {
                PLACEHOLDER_FOR_NAMES => output.section(&self.name_section),
                _ => output.section(&s),
            };
        }
        Ok(output.finish())
    }

    fn transform_code_section(
        &mut self,
        renc: &mut InstrumentationReencoder,
        reader: wp::FunctionBody,
        func_type_idx: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let func_type = self
            .types
            .get(func_type_idx as usize)
            .ok_or(InstrumentError::InvalidTypeIndex)?;

        let num_params: u32 = func_type.params().len().try_into().unwrap();
        let local_idx = num_params;
        let (mut locals, local_idx) = reader
            .get_locals_reader()
            .map_err(|e| InstrumentError::Parse(e.to_string()))?
            .into_iter()
            .try_fold((Vec::default(), local_idx), |(mut ls, li), v| {
                let (n, ty) = v.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                let ty = renc
                    .val_type(ty)
                    .map_err(|e| InstrumentError::Parse(e.to_string()))?;
                ls.push((n, ty));
                let li = li.checked_add(n).ok_or(InstrumentError::TooManyLocals)?;
                Ok::<_, InstrumentError>((ls, li))
            })?;

        let code_idx = self.code_section.len() as usize;
        macro_rules! get_idx {
            ($field: ident, $name: literal) => {{
                self.analysis
                    .$field
                    .get(code_idx)
                    .ok_or(InstrumentError::MissingAnalysis($name, code_idx))?
            }};
        }
        let gas_costs = get_idx!(gas_costs, "gas_costs");
        let gas_kinds = get_idx!(gas_kinds, "gas_kinds");
        let gas_offsets = get_idx!(gas_offsets, "gas_offsets");
        let stack_sz = *get_idx!(function_operand_stack_sizes, "operand_stack");
        let frame_sz = *get_idx!(function_frame_sizes, "frame_sizes");

        let mut instrumentation_points = gas_offsets
            .iter()
            .zip(gas_costs.iter())
            .zip(gas_kinds.iter())
            .peekable();
        let mut operators = reader
            .get_operators_reader()
            .map_err(|e| InstrumentError::Parse(e.to_string()))?;

        let (params, results) = (func_type.params(), func_type.results());
        let block_type = match (params, results) {
            (_, []) => we::BlockType::Empty,
            (_, [result]) => we::BlockType::Result(*result),
            ([], _) => we::BlockType::FunctionType(func_type_idx),
            (_, results) => {
                let idx = self.type_section.len();
                self.type_section
                    .ty()
                    .function(std::iter::empty(), results.iter().copied());
                we::BlockType::FunctionType(idx)
            }
        };

        locals.push((1, we::ValType::I64));
        locals.push((1, we::ValType::I32));
        let mut new_function = we::Function::new(locals);

        let stack_charge = stack_sz.checked_add(frame_sz).unwrap_or(0);
        if stack_charge > 0 {
            // gas charge for the function frame: ceil(frame/8) ops (nearcore
            // charges frame bytes as regular ops — see instrument_v3.rs).
            let gas_charge = frame_sz
                .checked_add(7)
                .map(|n| n / 8)
                .and_then(|n| n.checked_mul(REGULAR_OP_COST))
                .unwrap_or(u64::MAX);
            {
                let mut i = new_function.instructions();
                i.block(block_type)
                    .global_get(self.globals + STACK_GLOBAL)
                    .i64_const(stack_charge as i64)
                    .i64_lt_u()
                    .if_(we::BlockType::Empty)
                    .call(STACK_EXHAUSTED_FN)
                    .unreachable()
                    .else_()
                    .global_get(self.globals + STACK_GLOBAL)
                    .i64_const(stack_charge as i64)
                    .i64_sub()
                    .global_set(self.globals + STACK_GLOBAL)
                    .end();
                call_gas_instrumentation(
                    &mut i,
                    None,
                    Fee {
                        constant: gas_charge,
                        linear: 0,
                    },
                    self.globals,
                    local_idx,
                )?;
            }
        } else {
            new_function
                .instructions()
                .call(STACK_EXHAUSTED_FN)
                .unreachable()
                .end();
        }

        while !operators.eof() {
            let (op, offset) = operators
                .read_with_offset()
                .map_err(|e| InstrumentError::Parse(e.to_string()))?;
            let end_offset = operators.original_position();
            while instrumentation_points.peek().map(|((o, _), _)| **o) == Some(offset) {
                let ((_, g), k) = instrumentation_points.next().expect("peeked");
                if !matches!(k, InstrumentationKind::Unreachable) {
                    call_gas_instrumentation(
                        &mut new_function.instructions(),
                        Some(*k),
                        *g,
                        self.globals,
                        local_idx,
                    )?;
                }
            }
            match op {
                wp::Operator::RefFunc { function_index } => {
                    let idx = renc.function_index(function_index).map_err(boxed)?;
                    new_function.instructions().ref_func(idx);
                }
                wp::Operator::Call { function_index } => {
                    let idx = renc.function_index(function_index).map_err(boxed)?;
                    new_function.instructions().call(idx);
                }
                wp::Operator::ReturnCall { function_index } => {
                    let mut i = new_function.instructions();
                    call_unstack_instrumentation(&mut i, stack_charge, self.globals);
                    let idx = renc.function_index(function_index).map_err(boxed)?;
                    i.return_call(idx);
                }
                wp::Operator::ReturnCallIndirect { .. } => {
                    call_unstack_instrumentation(
                        &mut new_function.instructions(),
                        stack_charge,
                        self.globals,
                    );
                    new_function.raw(self.wasm[offset..end_offset].iter().copied());
                }
                wp::Operator::Return => {
                    let mut i = new_function.instructions();
                    call_unstack_instrumentation(&mut i, stack_charge, self.globals);
                    i.return_();
                }
                wp::Operator::End if operators.eof() => {
                    let mut i = new_function.instructions();
                    i.end();
                    call_unstack_instrumentation(&mut i, stack_charge, self.globals);
                    i.end();
                }
                _ => {
                    new_function.raw(self.wasm[offset..end_offset].to_vec());
                }
            };
        }
        self.code_section.function(&new_function);
        Ok(())
    }

    fn maybe_add_imports(&mut self) {
        if self.import_section.is_empty() {
            let exhausted_fnty = self.type_section.len();
            self.type_section.ty().function([], []);
            let gas_fnty = self.type_section.len();
            self.type_section.ty().function([we::ValType::I64], []);
            self.import_section.import(
                self.import_env,
                "finite_wasm_gas_exhausted",
                we::EntityType::Function(exhausted_fnty),
            );
            self.import_section.import(
                self.import_env,
                "finite_wasm_stack_exhausted",
                we::EntityType::Function(exhausted_fnty),
            );
            self.import_section.import(
                self.import_env,
                "finite_wasm_gas",
                we::EntityType::Function(gas_fnty),
            );
        }
    }

    fn add_globals(&mut self) {
        self.global_section.global(
            we::GlobalType {
                val_type: we::ValType::I64,
                mutable: true,
                shared: false,
            },
            &we::ConstExpr::i64_const(0),
        );
        self.global_section.global(
            we::GlobalType {
                val_type: we::ValType::I64,
                mutable: true,
                shared: false,
            },
            &we::ConstExpr::i64_const(MAX_STACK_HEIGHT as i64),
        );
        self.export_section.export(
            REMAINING_GAS_EXPORT,
            we::ExportKind::Global,
            self.globals + GAS_GLOBAL,
        );
    }

    fn transform_name_section(
        &mut self,
        renc: &mut InstrumentationReencoder,
        names: wp::NameSectionReader,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for name in names {
            let name = name.map_err(|e| InstrumentError::Parse(e.to_string()))?;
            match name {
                wp::Name::Module { name, .. } => self.name_section.module(name),
                wp::Name::Function(map) => {
                    let mut m = we::NameMap::new();
                    m.append(GAS_EXHAUSTED_FN, "finite_wasm_gas_exhausted");
                    m.append(STACK_EXHAUSTED_FN, "finite_wasm_stack_exhausted");
                    m.append(GAS_INSTRUMENTATION_FN, "finite_wasm_gas");
                    for naming in map {
                        let naming = naming.map_err(|e| InstrumentError::Parse(e.to_string()))?;
                        let idx = renc.function_index(naming.index).map_err(boxed)?;
                        m.append(idx, naming.name);
                    }
                    self.name_section.functions(&m)
                }
                wp::Name::Unknown { .. } => {}
                _ => {}
            }
        }
        Ok(())
    }
}

fn boxed(e: ReencodeError<ReencodeUserError>) -> Box<dyn std::error::Error> {
    Box::new(InstrumentError::Parse(e.to_string()))
}

fn call_unstack_instrumentation(func: &mut InstructionSink<'_>, charge: u64, globals: u32) {
    // Release a previously-reserved amount — plain add; the budget global
    // stays within [0, max_stack_height + max_charge], no overflow possible.
    func.global_get(globals + STACK_GLOBAL)
        .i64_const(charge as i64)
        .i64_add()
        .global_set(globals + STACK_GLOBAL);
}

fn call_gas_instrumentation(
    func: &mut InstructionSink<'_>,
    k: Option<InstrumentationKind>,
    gas: Fee,
    globals: u32,
    local_idx: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(gas, Fee::ZERO) {
        return Ok(());
    } else if gas.linear == 0 {
        func.global_get(globals + GAS_GLOBAL)
            .i64_const(gas.constant as i64)
            .i64_lt_u()
            .if_(we::BlockType::Empty)
            .i64_const(gas.constant as i64)
            .call(GAS_INSTRUMENTATION_FN)
            .unreachable()
            .else_()
            .global_get(globals + GAS_GLOBAL)
            .i64_const(gas.constant as i64)
            .i64_sub()
            .global_set(globals + GAS_GLOBAL)
            .end();
        return Ok(());
    }
    match k {
        Some(
            InstrumentationKind::TableInit
            | InstrumentationKind::TableFill
            | InstrumentationKind::TableCopy
            | InstrumentationKind::MemoryInit
            | InstrumentationKind::MemoryFill
            | InstrumentationKind::MemoryCopy
            | InstrumentationKind::MemoryGrow
            | InstrumentationKind::TableGrow,
        ) => {
            let count_idx = local_idx
                .checked_add(1)
                .ok_or(InstrumentError::TooManyLocals)?;
            // count × linear + constant — count ≤ 2^32, linear ≤ ~1e6,
            // constant ≤ ~3e7: the product cannot overflow i64.
            func.local_tee(count_idx)
                .i64_extend_i32_u()
                .i64_const(gas.linear as i64)
                .i64_mul()
                .i64_const(gas.constant as i64)
                .i64_add()
                .local_tee(local_idx)
                .global_get(globals + GAS_GLOBAL)
                .i64_gt_u()
                .if_(we::BlockType::Empty)
                .local_get(local_idx)
                .call(GAS_INSTRUMENTATION_FN)
                .unreachable()
                .else_()
                .global_get(globals + GAS_GLOBAL)
                .local_get(local_idx)
                .i64_sub()
                .global_set(globals + GAS_GLOBAL)
                .end()
                .local_get(count_idx);
            Ok(())
        }
        _ => Err(Box::new(InstrumentError::GasInstrumentationMisconfigured)),
    }
}
