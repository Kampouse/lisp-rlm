//! P2 WASI bridge: core WASM module that translates P1 WASI (fd_read/fd_write)
//! calls into P2 stream calls (get-stdin/blocking-read, get-stdout/blocking-write-and-flush).

use wasm_encoder::*;

/// Return area offset for canonical ABI results.
/// Must not overlap: STDIN_BUF=32768, STDOUT_BUF=65536, STDIN_LEN=98304
const RET_AREA: i32 = 126976; // 0x1F000

pub fn build_p2_wasi_bridge() -> Vec<u8> {
    let mut m = Module::new();

    // ── Type section ──
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I32]); // 0: () -> i32 (get_stdin, get_stdout)
    types
        .ty()
        .function([ValType::I32, ValType::I64, ValType::I32], []); // 1: blocking_read(handle, len, ret_area)
    types.ty().function([ValType::I32], []); // 2: (i32) -> () (drop stream)
    types
        .ty()
        .function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], []); // 3: blocking_write_and_flush(handle, ptr, len, ret_ptr) -> () (result via ret_ptr)
    types.ty().function([ValType::I32; 4], [ValType::I32]); // 4: fd_read, fd_write signature
    types.ty().function([ValType::I32], []); // 5: proc_exit
    types.ty().function([ValType::I32; 2], [ValType::I32]); // 6: random_get, environ_sizes_get, environ_get
    types.ty().function(
        [ValType::I32, ValType::I64, ValType::I32, ValType::I32],
        [ValType::I32],
    ); // 7: fd_seek
    m.section(&types);

    // ── Import section ──
    let mut imports = ImportSection::new();
    // Import shared memory
    imports.import(
        "env",
        "memory",
        EntityType::Memory(MemoryType {
            minimum: 0,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        }),
    );
    // P2 stream functions (import indices 0-6 after memory doesn't get an index)
    // Actually, memory import doesn't create a function index. Function imports are:
    imports.import("p2", "get_stdin", EntityType::Function(0)); // func 0
    imports.import("p2", "blocking_read", EntityType::Function(1)); // func 1
    imports.import("p2", "drop_input_stream", EntityType::Function(2)); // func 2
    imports.import("p2", "get_stdout", EntityType::Function(0)); // func 3 (reuses type 0)
    imports.import("p2", "blocking_write_and_flush", EntityType::Function(3)); // func 4
    imports.import("p2", "drop_output_stream", EntityType::Function(2)); // func 5 (reuses type 2)
    m.section(&imports);

    // ── Function section (7 P1 WASI exports) ──
    let mut funcs = FunctionSection::new();
    funcs.function(4); // fd_read (type 4)
    funcs.function(4); // fd_write (type 4)
    funcs.function(5); // proc_exit (type 5)
    funcs.function(6); // random_get (type 6)
    funcs.function(6); // environ_sizes_get (type 6)
    funcs.function(6); // environ_get (type 6)
    funcs.function(7); // fd_seek (type 7)
    m.section(&funcs);
    // func indices: 6=fd_read, 7=fd_write, 8=proc_exit, 9=random_get, 10=environ_sizes_get, 11=environ_get, 12=fd_seek

    // ── Export section ──
    let mut exports = ExportSection::new();
    exports.export("fd_read", ExportKind::Func, 6);
    exports.export("fd_write", ExportKind::Func, 7);
    exports.export("proc_exit", ExportKind::Func, 8);
    exports.export("random_get", ExportKind::Func, 9);
    exports.export("environ_sizes_get", ExportKind::Func, 10);
    exports.export("environ_get", ExportKind::Func, 11);
    exports.export("fd_seek", ExportKind::Func, 12);
    m.section(&exports);

    // ── Code section ──
    let mut code = CodeSection::new();
    let ma4 = MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    };
    let ma4_4 = MemArg {
        offset: 4,
        align: 2,
        memory_index: 0,
    };
    let ma1 = MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    };

    // ── fd_read(fd=0, iovs_ptr, iovs_len, nread_ptr) -> errno ──
    // params: 0=fd, 1=iovs_ptr, 2=iovs_len, 3=nread_ptr
    // locals: 4=handle, 5=buf_ptr, 6=buf_len, 7=data_len, 8=i
    {
        let mut fb = Function::new([
            (1, ValType::I32), // 4: handle
            (1, ValType::I32), // 5: buf_ptr
            (1, ValType::I32), // 6: buf_len
            (1, ValType::I32), // 7: data_len
            (1, ValType::I32), // 8: copy counter
        ]);

        // if fd != 0: write 0 to nread, return 0
        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::If(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(3));
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::I32Store(ma4));
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::Return);
        fb.instruction(&Instruction::End);

        // Read iov[0]: buf_ptr = mem[iovs_ptr], buf_len = mem[iovs_ptr+4]
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::LocalSet(5));
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I32Load(ma4_4));
        fb.instruction(&Instruction::LocalSet(6));

        // handle = get_stdin()
        fb.instruction(&Instruction::Call(0));
        fb.instruction(&Instruction::LocalSet(4));

        // blocking_read(handle, buf_len as u64, RET_AREA)
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::LocalGet(6));
        fb.instruction(&Instruction::I64ExtendI32U);
        fb.instruction(&Instruction::I32Const(RET_AREA));
        fb.instruction(&Instruction::Call(1));

        // Check discriminant at RET_AREA
        fb.instruction(&Instruction::I32Const(RET_AREA));
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::If(BlockType::Empty));
        // Error: data_len = 0
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::LocalSet(7));
        fb.instruction(&Instruction::Else);
        // Ok: data_len = mem[RET_AREA + 8]
        fb.instruction(&Instruction::I32Const(RET_AREA + 8));
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::LocalSet(7));

        // Copy data from data_ptr to buf_ptr
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::LocalSet(8));
        fb.instruction(&Instruction::Block(BlockType::Empty));
        fb.instruction(&Instruction::Loop(BlockType::Empty));
        fb.instruction(&Instruction::LocalGet(8));
        fb.instruction(&Instruction::LocalGet(7));
        fb.instruction(&Instruction::I32GeU);
        fb.instruction(&Instruction::BrIf(1));
        // buf[5+i] = data[ptr+i]
        fb.instruction(&Instruction::LocalGet(5));
        fb.instruction(&Instruction::LocalGet(8));
        fb.instruction(&Instruction::I32Add);
        fb.instruction(&Instruction::I32Const(RET_AREA + 4));
        fb.instruction(&Instruction::I32Load(ma4)); // data_ptr
        fb.instruction(&Instruction::LocalGet(8));
        fb.instruction(&Instruction::I32Add);
        fb.instruction(&Instruction::I32Load8U(ma1));
        fb.instruction(&Instruction::I32Store8(ma1));
        // i++
        fb.instruction(&Instruction::LocalGet(8));
        fb.instruction(&Instruction::I32Const(1));
        fb.instruction(&Instruction::I32Add);
        fb.instruction(&Instruction::LocalSet(8));
        fb.instruction(&Instruction::Br(0));
        fb.instruction(&Instruction::End); // loop
        fb.instruction(&Instruction::End); // block
        fb.instruction(&Instruction::End); // if ok/err

        // drop_input_stream(handle)
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::Call(2));

        // mem[nread_ptr] = data_len
        fb.instruction(&Instruction::LocalGet(3));
        fb.instruction(&Instruction::LocalGet(7));
        fb.instruction(&Instruction::I32Store(ma4));

        fb.instruction(&Instruction::I32Const(0)); // return 0 (success)
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // ── fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno ──
    // params: 0=fd, 1=iovs_ptr, 2=iovs_len, 3=nwritten_ptr
    // locals: 4=handle, 5=buf_ptr, 6=buf_len
    {
        let mut fb = Function::new([
            (1, ValType::I32), // 4: handle
            (1, ValType::I32), // 5: buf_ptr
            (1, ValType::I32), // 6: buf_len
        ]);

        // Only handle fd=1 (stdout)
        fb.instruction(&Instruction::LocalGet(0));
        fb.instruction(&Instruction::I32Const(1));
        fb.instruction(&Instruction::I32Eq);
        fb.instruction(&Instruction::If(BlockType::Empty));

        // Read iov[0]
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I32Load(ma4));
        fb.instruction(&Instruction::LocalSet(5));
        fb.instruction(&Instruction::LocalGet(1));
        fb.instruction(&Instruction::I32Load(ma4_4));
        fb.instruction(&Instruction::LocalSet(6));

        // handle = get_stdout()
        fb.instruction(&Instruction::Call(3));
        fb.instruction(&Instruction::LocalSet(4));

        // blocking_write_and_flush(handle, buf_ptr, buf_len, ret_ptr) -> discard result discriminant
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::LocalGet(5));
        fb.instruction(&Instruction::LocalGet(6));
        fb.instruction(&Instruction::I32Const(RET_AREA + 8)); // ret_ptr for stream-error payload
        fb.instruction(&Instruction::Call(4));
        // No return value to drop — result written via ret_ptr

        // drop_output_stream(handle)
        fb.instruction(&Instruction::LocalGet(4));
        fb.instruction(&Instruction::Call(5));

        // mem[nwritten_ptr] = buf_len
        fb.instruction(&Instruction::LocalGet(3));
        fb.instruction(&Instruction::LocalGet(6));
        fb.instruction(&Instruction::I32Store(ma4));

        fb.instruction(&Instruction::End); // if

        fb.instruction(&Instruction::I32Const(0)); // return 0
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // proc_exit: unreachable
    {
        let mut fb = Function::new([]);
        fb.instruction(&Instruction::Unreachable);
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    // random_get, environ_sizes_get, environ_get, fd_seek: return 0
    for _ in 0..4 {
        let mut fb = Function::new([]);
        fb.instruction(&Instruction::I32Const(0));
        fb.instruction(&Instruction::End);
        code.function(&fb);
    }

    m.section(&code);
    m.finish()
}
