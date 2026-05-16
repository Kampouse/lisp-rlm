//! Minimal test: read stdin, write to stdout. No HTTP.
//! Tests that native WASI P2 stdin/stdout works without the adapter.

use wasm_encoder::{
    BlockType, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, ImportSection, Instruction, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};

const I_GET_STDIN: u32 = 0;
const I_GET_STDOUT: u32 = 1;
const I_BLOCKING_READ: u32 = 2;
const I_BLOCKING_WRITE: u32 = 3;
const I_DROP_INSTREAM: u32 = 4;
const NUM_IMPORTS: u32 = 5;

const RET: i32 = 160;

pub fn build_echo() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function([], []);                                         // 0: () -> ()
    types.ty().function([ValType::I32], []);                             // 1: (i32) -> ()
    types.ty().function([], [ValType::I32]);                             // 2: () -> i32
    types.ty().function([ValType::I32, ValType::I32], []);               // 3: (2xi32) -> ()
    types.ty().function([ValType::I32; 4], []);                          // 4: (4xi32) -> ()
    types.ty().function([ValType::I32, ValType::I64, ValType::I32], []); // 5: stream read
    types.ty().function([], [ValType::I32]);                             // 6: () -> i32 (run export)
    types.ty().function([ValType::I32; 4], [ValType::I32]);              // 7: (4xi32) -> i32 (realloc)
    m.section(&types);

    let mut imp = ImportSection::new();
    imp.import("wasi:cli/stdin@0.2.2", "get-stdin", EntityType::Function(2));
    imp.import("wasi:cli/stdout@0.2.2", "get-stdout", EntityType::Function(2));
    imp.import("wasi:io/streams@0.2.2", "[method]input-stream.blocking-read", EntityType::Function(5));
    imp.import("wasi:io/streams@0.2.2", "[method]output-stream.blocking-write-and-flush", EntityType::Function(4));
    imp.import("wasi:io/streams@0.2.2", "[resource-drop]input-stream", EntityType::Function(1));
    m.section(&imp);

    let mut funcs = FunctionSection::new();
    funcs.function(0); // run: () -> ()
    funcs.function(0); // _start
    funcs.function(6); // run_export: () -> i32
    funcs.function(7); // realloc: (4xi32) -> i32
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType { minimum: 1, maximum: None, memory64: false, shared: false, page_size_log2: None });
    m.section(&mems);

    let mut exps = ExportSection::new();
    exps.export("memory", ExportKind::Memory, 0);
    exps.export("_start", ExportKind::Func, NUM_IMPORTS + 1);
    exps.export("wasi:cli/run@0.2.2#run", ExportKind::Func, NUM_IMPORTS + 2);
    exps.export("canonical_abi_realloc", ExportKind::Func, NUM_IMPORTS + 3);
    m.section(&exps);

    let mut code = wasm_encoder::CodeSection::new();
    
    // run: read stdin → write stdout
    let mut f = Function::new([]);
    // get stdin handle
    f.instruction(&Instruction::Call(I_GET_STDIN));
    // blocking-read(stdin_handle, 65536, ret_ptr)
    // Type 5: (i32, i64, i32) -> ()
    f.instruction(&Instruction::I64Const(65536));
    f.instruction(&Instruction::I32Const(RET));
    f.instruction(&Instruction::Call(I_BLOCKING_READ));
    // Read result from ret_ptr: [discrim, ptr, len]
    // ptr at RET+4, len at RET+8
    // Save ptr to mem[200], len to mem[204]
    f.instruction(&Instruction::I32Const(200)); // addr for ptr
    f.instruction(&Instruction::I32Const(RET + 4)); // addr to load from
    f.instruction(&Instruction::I32Load(mem_arg(0))); // ptr value
    f.instruction(&Instruction::I32Store(mem_arg(0))); // store ptr at 200
    f.instruction(&Instruction::I32Const(204)); // addr for len
    f.instruction(&Instruction::I32Const(RET + 8)); // addr to load from
    f.instruction(&Instruction::I32Load(mem_arg(0))); // len value
    f.instruction(&Instruction::I32Store(mem_arg(0))); // store len at 204
    
    // Drop stdin
    f.instruction(&Instruction::Call(I_GET_STDIN));
    f.instruction(&Instruction::Call(I_DROP_INSTREAM));
    
    // Get stdout handle
    f.instruction(&Instruction::Call(I_GET_STDOUT));
    // blocking-write-and-flush(stdout, ptr, len, ret_ptr)
    f.instruction(&Instruction::I32Const(200));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // body ptr
    f.instruction(&Instruction::I32Const(204));
    f.instruction(&Instruction::I32Load(mem_arg(0))); // body len
    f.instruction(&Instruction::I32Const(RET + 16)); // ret_ptr for result
    f.instruction(&Instruction::Call(I_BLOCKING_WRITE));
    
    f.instruction(&Instruction::End);
    code.function(&f);
    
    // _start
    let mut s = Function::new([]);
    s.instruction(&Instruction::Call(NUM_IMPORTS));
    s.instruction(&Instruction::End);
    code.function(&s);
    
    // run_export
    let mut r = Function::new([]);
    r.instruction(&Instruction::Call(NUM_IMPORTS));
    r.instruction(&Instruction::I32Const(0)); // Ok
    r.instruction(&Instruction::End);
    code.function(&r);
    
    // realloc: just return 256 always (simplest possible)
    let mut a = Function::new([]);
    a.instruction(&Instruction::I32Const(256)); // always return addr 256
    a.instruction(&Instruction::End);
    code.function(&a);
    
    m.section(&code);
    
    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(200), 256i32.to_le_bytes().to_vec());
    m.section(&data);
    
    m.finish()
}

fn mem_arg(offset: i32) -> wasm_encoder::MemArg {
    wasm_encoder::MemArg { offset: offset as u64, align: 2, memory_index: 0 }
}
