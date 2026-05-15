//! Native P2 component emission — no WASI P1 adapter.
//!
//! Strategy: Extract the type/import/canon header from the adapter-based P2
//! component, replace the adapter+core with just our core module (pure computation),
//! and rewrite the instance/export sections to wire P2 streams directly.
//!
//! Result: A proper P2 component that uses wasi:io/streams natively,
//! with no adapter overhead.

use crate::wasm_emit::WasmEmitter;
use wasm_encoder::*;

/// Build a native P2 component from the emitter.
/// The core module will be pure computation (no WASI I/O).
/// I/O is handled by the component wrapper via P2 streams.
pub fn build_native_p2(em: &mut WasmEmitter) -> Result<Vec<u8>, String> {
    if em.funcs.is_empty() {
        return Err("no functions defined".into());
    }
    em.tree_shake();

    // 1. Build a pure core module that exports run_io
    //    run_io(input_ptr, input_len, output_ptr) -> output_len
    let core_bytes = build_pure_core(em)?;

    // 2. Build component using ComponentBuilder
    //    Import wasi P2 interfaces, embed core, wire with canon lift/lower
    let component_bytes = build_component(&core_bytes)?;

    Ok(component_bytes)
}

fn build_pure_core(em: &mut WasmEmitter) -> Result<Vec<u8>, String> {
    let W = ValType::I32;
    let I = ValType::I64;
    let ma1 = MemArg { offset: 0, align: 0, memory_index: 0 };
    let ma4 = MemArg { offset: 0, align: 2, memory_index: 0 };

    let mut m = Module::new();

    // ── Type section ──
    let mut types = TypeSection::new();
    // type 0: (i32, i32, i32) -> i32  — run_io
    types.ty().function(vec![W, W, W], vec![W]);
    // type 1-N: user function types
    let mut type_map: Vec<u32> = Vec::new();
    for f in &em.funcs {
        let nparams = f.params.len() as u32;
        types.ty().function(vec![I; nparams as usize], vec![I]);
        type_map.push(1 + type_map.len() as u32);
    }
    m.section(&types);

    let user_type_start = 1u32;

    // ── Import section ── (none — pure computation)
    // No imports at all!

    // ── Function section ──
    let mut funcs = FunctionSection::new();
    // Function 0: run_io (type 0)
    funcs.function(0);
    // Functions 1..N: user functions
    for i in 0..em.funcs.len() {
        funcs.function(user_type_start + i as u32);
    }
    m.section(&funcs);

    // ── Memory section ──
    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 4,
        maximum: Some(4),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    // ── Global section ── (same as P1)
    let mut globals = GlobalSection::new();
    let nglobals = em.globals.len() as u32;
    for g in &em.globals {
        globals.global(
            GlobalType { val_type: I, mutable: true },
            &ConstExpr::i64_const(g.initial_value),
        );
    }
    m.section(&globals);

    // ── Export section ──
    let mut exps = ExportSection::new();
    exps.export("memory", ExportKind::Memory, 0);
    // run_io is function 0 (no imports, so function 0)
    exps.export("run_io", ExportKind::Func, 0);
    m.section(&exps);

    // ── Code section ──
    let mut code = CodeSection::new();

    // ── run_io function ──
    // run_io(input_ptr: i32, input_len: i32, output_ptr: i32) -> output_len
    // Locals: temp vars
    let mut fb = Function::new(vec![
        (1, I), // local 3: input_str (tagged)
        (1, I), // local 4: result (tagged)
        (1, I), // local 5: temp / value
        (1, I), // local 6: digit_count
        (1, I), // local 7: negative flag
        (1, I), // local 8: write_ptr
        (1, I), // local 9: temp for output
    ]);

    // Tag the input as a string: tag = (payload << 3) | 5
    // payload = (input_len << 32) | input_ptr
    fb.instruction(&Instruction::LocalGet(1)); // input_len
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Const(32));
    fb.instruction(&Instruction::I64Shl);
    fb.instruction(&Instruction::LocalGet(0)); // input_ptr
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Or);
    fb.instruction(&Instruction::I64Const(3));
    fb.instruction(&Instruction::I64Shl);
    fb.instruction(&Instruction::I64Const(5));
    fb.instruction(&Instruction::I64Or);
    fb.instruction(&Instruction::LocalSet(3)); // input_str tagged

    // Call run function with input_str
    // run is function index 1 (first user function, no imports)
    fb.instruction(&Instruction::LocalGet(3));
    fb.instruction(&Instruction::Call(1)); // run function
    fb.instruction(&Instruction::LocalSet(4)); // result

    // Now convert result to string and write to output buffer
    // Result is tagged: type = result & 7
    // Type 1 = fixnum (integer), type 5 = string
    fb.instruction(&Instruction::LocalGet(4));
    fb.instruction(&Instruction::I64Const(7));
    fb.instruction(&Instruction::I64And);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::LocalSet(5)); // value = tag

    // If tag == 1 (fixnum): convert integer to string
    fb.instruction(&Instruction::Block(BlockType::Empty)); // block fixnum
    fb.instruction(&Instruction::Block(BlockType::Empty)); // block string
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I32Const(1));
    fb.instruction(&Instruction::I32Eq);
    fb.instruction(&Instruction::BrIf(1)); // if fixnum, skip to fixnum block

    // String case: tag == 5
    // Untag: payload = result >> 3
    // ptr = payload & 0xFFFFFFFF, len = payload >> 32
    fb.instruction(&Instruction::LocalGet(4));
    fb.instruction(&Instruction::I64Const(3));
    fb.instruction(&Instruction::I64ShrU);
    fb.instruction(&Instruction::LocalSet(5)); // payload
    // len = payload >> 32
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(32));
    fb.instruction(&Instruction::I64ShrU);
    fb.instruction(&Instruction::I64Const(32767)); // max output 32KB
    fb.instruction(&Instruction::I64GeU);
    fb.instruction(&Instruction::If(BlockType::Empty));
    fb.instruction(&Instruction::I64Const(0)); // len = 0 if too long
    fb.instruction(&Instruction::LocalSet(6));
    fb.instruction(&Instruction::Else);
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(32));
    fb.instruction(&Instruction::I64ShrU);
    fb.instruction(&Instruction::LocalSet(6)); // len
    fb.instruction(&Instruction::End);

    // Copy string to output_ptr
    // src_ptr = payload & 0xFFFFFFFF
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(0xFFFFFFFF));
    fb.instruction(&Instruction::I64And);
    fb.instruction(&Instruction::LocalSet(7)); // src_ptr

    // Copy loop: output_ptr[i] = src_ptr[i] for i in 0..len
    // Reuse local 8 as loop counter
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::LocalSet(8)); // i = 0
    fb.instruction(&Instruction::Block(BlockType::Empty));
    fb.instruction(&Instruction::Loop(BlockType::Empty));
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64GeU);
    fb.instruction(&Instruction::BrIf(1)); // if i >= len, break
    // output_ptr[i] = src_ptr[i]
    fb.instruction(&Instruction::LocalGet(2)); // output_ptr
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::LocalGet(7)); // src_ptr
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Load8U(ma1.clone()));
    fb.instruction(&Instruction::I32Store8(ma1.clone()));
    // i++
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::LocalSet(8));
    fb.instruction(&Instruction::Br(0));
    fb.instruction(&Instruction::End); // loop
    fb.instruction(&Instruction::End); // block

    // Return len as i32
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::Return);

    // Fixnum case: convert integer to string
    fb.instruction(&Instruction::End); // end string block
    // Fixnum: value = result >> 3 (arithmetic shift for sign)
    fb.instruction(&Instruction::LocalGet(4));
    fb.instruction(&Instruction::I64Const(3));
    fb.instruction(&Instruction::I64ShrS);
    fb.instruction(&Instruction::LocalSet(5)); // value

    // Check negative
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::LocalSet(7)); // negative = 0
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::I64LtS);
    fb.instruction(&Instruction::If(BlockType::Empty));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::LocalSet(7)); // negative = 1
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::I64Sub); // abs(value) via 0 - value... actually negate
    fb.instruction(&Instruction::LocalSet(5));
    fb.instruction(&Instruction::End);

    // Convert digits to string at output_ptr, writing backwards from end
    // Use a fixed buffer area: write digits at output_ptr + 20 backwards
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::LocalSet(6)); // digit_count = 0

    fb.instruction(&Instruction::Block(BlockType::Empty));
    fb.instruction(&Instruction::Loop(BlockType::Empty));
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::I64Eq);
    fb.instruction(&Instruction::BrIf(1)); // if value == 0, done

    // digit = value % 10
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(10));
    fb.instruction(&Instruction::I64RemU);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Const(48)); // '0'
    fb.instruction(&Instruction::I32Add);
    // Store at output_ptr + 19 - digit_count
    fb.instruction(&Instruction::LocalGet(2)); // output_ptr
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Const(19));
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Sub);
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Store8(ma1.clone()));

    // value /= 10
    fb.instruction(&Instruction::LocalGet(5));
    fb.instruction(&Instruction::I64Const(10));
    fb.instruction(&Instruction::I64DivU);
    fb.instruction(&Instruction::LocalSet(5));
    // digit_count++
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::LocalSet(6));
    fb.instruction(&Instruction::Br(0));
    fb.instruction(&Instruction::End); // loop
    fb.instruction(&Instruction::End); // block

    // Handle value == 0 initially (digit_count == 0)
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::I64Eq);
    fb.instruction(&Instruction::If(BlockType::Empty));
    fb.instruction(&Instruction::LocalGet(2));
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Const(19));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Const(48)); // '0'
    fb.instruction(&Instruction::I32Store8(ma1.clone()));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::LocalSet(6));
    fb.instruction(&Instruction::End);

    // If negative, add '-' prefix
    fb.instruction(&Instruction::LocalGet(7));
    fb.instruction(&Instruction::If(BlockType::Empty));
    fb.instruction(&Instruction::LocalGet(2));
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Const(19));
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Sub);
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::I64Sub);
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Const(45)); // '-'
    fb.instruction(&Instruction::I32Store8(ma1.clone()));
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::LocalSet(6));
    fb.instruction(&Instruction::End);

    // Copy digits from temp area to output_ptr start
    // src = output_ptr + 20 - digit_count, dst = output_ptr, len = digit_count
    fb.instruction(&Instruction::I64Const(0));
    fb.instruction(&Instruction::LocalSet(8)); // i = 0
    fb.instruction(&Instruction::Block(BlockType::Empty));
    fb.instruction(&Instruction::Loop(BlockType::Empty));
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64GeU);
    fb.instruction(&Instruction::BrIf(1));
    fb.instruction(&Instruction::LocalGet(2)); // output_ptr
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    // src = output_ptr + 20 - digit_count + i
    fb.instruction(&Instruction::LocalGet(2)); // output_ptr
    fb.instruction(&Instruction::I64ExtendI32U);
    fb.instruction(&Instruction::I64Const(20));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I64Sub);
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::I32Load8U(ma1.clone()));
    fb.instruction(&Instruction::I32Store8(ma1.clone()));
    fb.instruction(&Instruction::LocalGet(8));
    fb.instruction(&Instruction::I64Const(1));
    fb.instruction(&Instruction::I64Add);
    fb.instruction(&Instruction::LocalSet(8));
    fb.instruction(&Instruction::Br(0));
    fb.instruction(&Instruction::End);
    fb.instruction(&Instruction::End);

    // Return digit_count as i32
    fb.instruction(&Instruction::LocalGet(6));
    fb.instruction(&Instruction::I32WrapI64);
    fb.instruction(&Instruction::Return);

    fb.instruction(&Instruction::End); // end fixnum block

    // Fallback: return 0
    fb.instruction(&Instruction::I32Const(0));
    fb.instruction(&Instruction::Return);

    code.function(&fb);

    // ── User functions ──
    let internal_base = 1u32; // function 0 is run_io, users start at 1
    let name_map: std::collections::HashMap<&str, u32> = em.funcs.iter().enumerate()
        .map(|(i, f)| (f.name.as_str(), internal_base + i as u32))
        .collect();

    for f in &em.funcs {
        let nparams = f.params.len() as u32;
        let mut locals: Vec<(u32, ValType)> = Vec::new();
        // One i64 local per param (for register allocation)
        for _ in 0..nparams {
            locals.push((1, I));
        }
        // Extra locals for register allocator
        let nlocals = em.next_local;
        for _ in 0..nlocals {
            locals.push((1, I));
        }

        let mut ufb = Function::new(locals);
        // Copy params into register locals
        for i in 0..nparams {
            ufb.instruction(&Instruction::LocalGet(i));
            ufb.instruction(&Instruction::LocalSet(nparams + i));
        }

        // Emit body
        let body = &f.body;
        for instr in &body.code {
            // Fix up Call instructions: resolve function names to indices
            match instr {
                Instruction::Call(idx) => {
                    // Check if it's a user function
                    ufb.instruction(&Instruction::Call(*idx));
                }
                _ => ufb.instruction(instr),
            }
        }

        code.function(&ufb);
    }

    m.section(&code);

    // ── Data section ──
    let mut data = DataSection::new();
    for seg in &em.data_segments {
        data.active(
            seg.memory_index,
            &ConstExpr::i32_const(seg.offset as i32),
            seg.data.iter().copied(),
        );
    }
    if data.len() > 0 {
        m.section(&data);
    }

    Ok(m.finish())
}

/// Build the P2 component wrapping the pure core module.
/// Imports wasi:cli/stdin, wasi:cli/stdout, wasi:io/streams, etc.
/// Uses canonical lowering to connect P2 streams to core module.
fn build_component(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut b = ComponentBuilder::default();

    // For now, we'll use a hybrid approach:
    // Take the compiled core module with _start (P1 style),
    // and use wasm-tools to create the component with the adapter.
    // But first, let me try the direct component building.

    // The key realization: we can build a MUCH simpler component if
    // the core module exports run_io (pure computation) instead of _start (I/O).
    // The component wrapper handles all I/O via P2 streams.

    // However, building the full P2 component type system from scratch
    // requires defining ~20 types for wasi:io, wasi:cli, etc.
    // This is ~500 lines of wasm-encoder code.

    // Pragmatic approach: use the adapter-based component as a template.
    // Extract its type/import sections and reuse them.

    // Even MORE pragmatic: just build the component using raw bytes.
    // Compile the header separately and concatenate.

    // Simplest working approach: write a small Rust program that uses
    // wasmtime_wasi's types directly to create the component.
    // But we don't have wasmtime in the compiler dependencies.

    // FINAL APPROACH: Build the component binary manually.
    // We know the exact structure needed. Let's emit raw bytes.

    // Actually, the simplest thing that could work:
    // 1. Compile with _start (P1 WASI) + adapter (current approach)
    // 2. BUT use a custom minimal adapter that's much smaller
    // 3. Write the minimal adapter as raw WASM bytes

    // Let me build a ~200 byte adapter that only implements:
    //   fd_read(fd=0, iov_ptr, iov_len, result_ptr) → calls P2 blocking_read
    //   fd_write(fd=1, iov_ptr, iov_len, result_ptr) → calls P2 blocking_write
    //   proc_exit(code) → calls P2 exit
    //   (other functions → return 0 / nop)

    // This adapter would be a core WASM module that:
    //   - Imports wasi:io/streams (lowered to core functions)
    //   - Exports wasi_snapshot_preview1 functions
    //   - Maps fd_read → blocking_read, fd_write → blocking_write

    // But this adapter still needs to be wired via the component model...
    // which is the same complex type system problem.

    // OK. Let me just use wasm-tools with the full adapter for now,
    // and add an optimization pass that strips unused adapter functions.

    // Actually, the real answer is to use `wit-component` crate properly
    // by providing the correct WIT world. Let me go back to that approach
    // with the correct WIT files.

    drop(b);
    drop(core_bytes);
    Err("Component building not yet complete — using adapter approach as fallback".into())
}
