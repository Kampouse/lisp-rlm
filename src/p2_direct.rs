//! Direct P2 component — minimal bridge, no full WASI adapter.
//!
//! Instead of the 53KB wasmtime adapter that handles ALL WASI P1 functions,
//! we build a tiny component that directly uses 5 P2 stream operations:
//!   1. get-stdin → input-stream handle
//!   2. blocking-read(handle, len) → list<u8>  
//!   3. get-stdout → output-stream handle
//!   4. blocking-write-and-flush(handle, data)
//!   5. exit (clean)
//!
//! The core module does computation on linear memory. The component wrapper
//! handles all P2 stream I/O via lowered component functions.

use wasm_encoder::*;

pub fn build_direct_p2(core_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut b = ComponentBuilder::default();

    // ═══════════════════════════════════════════
    // 1. Define component types
    // ═══════════════════════════════════════════

    // result<> (no ok, no err)
    let (result_void, mut rv_enc) = b.ty(None);
    { rv_enc.defined_type().result(None, None); }

    // result<_, stream-error> — used by write methods
    let (result_void_err, mut rve_enc) = b.ty(None);
    { rve_enc.defined_type().result(None, Some(ComponentValType::Primitive(PrimitiveValType::S32))); }

    // stream-error variant type (just need the discriminant as i32)
    // In canonical ABI, result<_, stream-error> = i32 where 0=ok, >0=error variant
    
    // input-stream resource type — opaque handle, represented as i32
    let (input_stream_type, mut is_enc) = b.ty(None);
    { is_enc.defined_type().resource(); }

    // output-stream resource type
    let (output_stream_type, mut os_enc) = b.ty(None);
    { os_enc.defined_type().resource(); }

    // () -> result<input-stream>  (get-stdin)
    // resource returns are result<handle> in canonical ABI
    let (get_stdin_type, mut gsi_enc) = b.ty(None);
    { gsi_enc.function()
        .params([] as [(&str, ComponentValType); 0])
        .result(Some(ComponentValType::Type(result_void.clone_type()))); // TODO: result<handle>
    }

    // Actually, let me reconsider the approach.
    //
    // The problem with building component types manually is that resources
    // in the component model have complex canonical ABI rules:
    // - Handles are managed via handle tables
    // - Resource methods use `canon resource.method` with special options
    // - list<u8> returns need a return pointer option
    //
    // All of this is handled by wit-bindgen when compiling Rust to P2.
    // Building it by hand with wasm-encoder is error-prone and fragile.
    //
    // BETTER APPROACH: Instead of component-level I/O, use a tiny core module
    // as a "bridge" that gets the lowered P2 functions as imports.
    // The bridge core module:
    //   1. On startup: calls lowered get_stdin (an i32-returning core func)
    //   2. Calls lowered blocking_read (handle, len) → writes data to memory
    //   3. Calls our main _start
    //   4. Calls lowered get_stdout
    //   5. Calls lowered blocking_write_and_flush (handle, ptr, len)
    //
    // The lowered functions are core WASM functions created by `canon lower`.
    // But here's the catch: `blocking-read` returns `result<list<u8>, error>`.
    // In the canonical ABI, `list<u8>` return is:
    //   - Written to memory at a "return area" pointer
    //   - Format: i32 ptr, i32 len (at return_area)
    //   - The `canon lower` needs `CanonicalOption::Return(return_area_ptr)`
    //
    // We CAN do this with wasm-encoder. Let me build it.

    // Actually, let me check if we can use a much simpler trick:
    // The executor reads stdout AFTER the component finishes.
    // What if our component writes output to a well-known memory location
    // and the executor reads it from the component's exported memory?
    
    // Looking at the executor code... it reads stdout from the WASI pipe.
    // So we MUST use the stream API to get output.
    
    // OK let me just build the minimal bridge properly.

    // Plan B: Build a minimal adapter core module that:
    // - Exports fd_read, fd_write, proc_exit (P1 interface)
    // - Internally calls lowered P2 stream functions
    // - Is much smaller than the full wasmtime adapter
    // 
    // This is basically building a minimal version of wasmtime's adapter.
    // The full adapter handles ALL edge cases (scatter/gather, errors, etc).
    // Our version only handles the happy path for fd=0 and fd=1.

    // For fd_read(fd=0, iov, iovcnt, nwritten):
    //   - First call: get_stdin() → save handle
    //   - Call blocking_read(handle, total_iov_len) → get data
    //   - Copy data from return buffer to iov buffers
    //   - Write total bytes to *nwritten
    //
    // For fd_write(fd=1, iov, iovcnt, nwritten):
    //   - First call: get_stdout() → save handle
    //   - Copy iov data to a contiguous buffer
    //   - Call blocking_write_and_flush(handle, data)
    //
    // For proc_exit(code):
    //   - Call exit(Ok(()))

    // The problem: blocking_read returns list<u8> which in canonical ABI
    // requires a return area. The bridge module needs memory to:
    //   - Store the return area (8 bytes: ptr + len)
    //   - Buffer the read data before copying to iovs
    //
    // And the lowered functions need to be created with proper options.

    // Let me try building this step by step.

    // Step 1: Define the component imports (P2 interfaces)
    // We need: wasi:cli/stdin, wasi:cli/stdout, wasi:io/streams, wasi:cli/exit

    // Actually — I just realized we can do something even simpler.
    // 
    // Our core module's _start already reads stdin and writes stdout using
    // fd_read/fd_write on linear memory. The P2 adapter translates these calls.
    //
    // What if instead of an adapter module, we REPLACE fd_read/fd_write
    // in the core module with direct calls to lowered P2 functions?
    //
    // The core module currently:
    //   - fd_read(0, iov, 1, nwritten) → reads stdin to STDIN_BUF
    //   - [runs main()]
    //   - fd_write(1, iov, 1, nwritten) → writes from STDOUT_BUF
    //
    // If we change the core module to NOT import fd_read/fd_write,
    // but instead have the component wrapper inject the data directly
    // into memory BEFORE calling _start, and read it AFTER...
    //
    // The component wrapper would:
    //   1. lower get_stdin → core func that returns i32 handle
    //   2. lower blocking_read(handle, len) → core func that returns (ptr, len) via return area
    //   3. Copy returned data to core memory at STDIN_BUF via i32.store
    //   4. Call core _start
    //   5. Read result from core memory at STDOUT_BUF via i32.load
    //   6. lower get_stdout → core func that returns i32 handle
    //   7. lower blocking_write_and_flush(handle, ptr, len) → core func
    //   8. Call it with the output data
    //
    // But the component can't do memory operations directly — it can only
    // lift/lower functions and instantiate core modules. The "component wrapper"
    // is just declarations, not executable code.
    //
    // To execute logic, we need core modules. The bridge module IS a core module
    // that does exactly steps 1-8 above.
    //
    // So we're back to building a bridge core module. Let me just do it.

    // The bridge module will be a core WASM module that:
    // - Imports lowered P2 functions as core functions
    // - Does the stream I/O
    // - Calls our main core module's _start
    // - Exports its own _start (which is what the component lifts as run)
    //
    // The bridge + component wrapper together replace the 53KB adapter.

    // Bridge module imports:
    //   "p2" "get_stdin"         () -> i32 (handle, 0=ok, trap on error)
    //   "p2" "blocking_read"     (i32, i64, i32) -> () (handle, len, ret_ptr → writes ptr+len at ret_ptr)
    //   "p2" "get_stdout"        () -> i32
    //   "p2" "blocking_write"    (i32, i32, i32) -> () (handle, ptr, len)
    //   "core" "_start"          () -> ()
    //
    // Wait — the lowered functions have different signatures than I think.
    // `canon lower` on a component function that returns result<list<u8>, error>
    // produces a core function with signature:
    //   (arg1, arg2, ..., ret_ptr: i32) -> i32
    // where ret_ptr points to the return area, and the i32 return is the error discriminant.
    //
    // For blocking_read(input-stream, u64) -> result<list<u8>, stream-error>:
    //   Lowered: (i32 handle, i64 len, i32 ret_ptr) -> i32
    //   ret_ptr gets: [i32 ptr, i32 len] (the list<u8>)
    //   return: 0 = ok, >0 = error variant
    //
    // For blocking_write_and_flush(output-stream, list<u8>) -> result<_, error>:
    //   Lowered: (i32 handle, i32 ptr, i32 len) -> i32
    //   return: 0 = ok, >0 = error variant

    // OK let me actually build this. The bridge module:

    let bridge = build_bridge_module();
    let bridge_idx = b.core_module_raw(None, &bridge);
    let core_idx = b.core_module_raw(None, core_bytes);

    // Now I need to:
    // 1. Import the P2 component interfaces
    // 2. Lower the functions
    // 3. Instantiate bridge with lowered functions + core _start
    // 4. Instantiate core with... nothing? It needs no WASI imports if we handle I/O in the bridge
    //    Wait — the core module STILL imports wasi_snapshot_preview1 (fd_read, fd_write, proc_exit)
    //    We'd need to modify the core module to NOT import those if we're using the bridge approach
    //
    // Hmm, this creates a chicken-and-egg problem. The core module currently uses fd_read/fd_write.
    // If we use a bridge, the core module should have a DIFFERENT _start that doesn't do I/O.
    // 
    // Alternative: the core module's _start still reads/writes via fd_read/fd_write,
    // but the bridge module PROVIDES fd_read/fd_write implementations that call P2 streams.
    // The bridge is instantiated first, providing fd_read/fd_write as exports.
    // Then the core module is instantiated with the bridge's fd_read/fd_write as imports.
    //
    // This is exactly what the adapter does! We're building a minimal adapter.
    // The difference is ours is ~2KB instead of 53KB because we only handle:
    //   fd_read(fd=0, ...) and fd_write(fd=1, ...) and proc_exit(0)

    // Architecture:
    // Component {
    //   import wasi:cli/stdin, wasi:cli/stdout, wasi:io/streams, wasi:cli/exit
    //   
    //   core module "bridge" {
    //     imports: lowered P2 functions
    //     exports: fd_read, fd_write, proc_exit
    //   }
    //   
    //   core module "main" (our lisp program) {
    //     imports: wasi_snapshot_preview1 { fd_read, fd_write, proc_exit }
    //   }
    //   
    //   instantiate bridge with lowered P2 functions
    //   instantiate main with bridge exports as wasi_snapshot_preview1 instance
    //   
    //   alias main._start → lift → export as wasi:cli/run
    // }

    // This is the right architecture. Let me build it.

    // First: define component types for the P2 interfaces
    // wasi:cli/stdin instance type: { get-stdin: () -> result<input-stream> }
    // wasi:cli/stdout instance type: { get-stdout: () -> result<output-stream> }
    // wasi:io/streams instance type: { [method]input-stream.blocking-read: (input-stream, u64) -> result<list<u8>, stream-error>, ... }
    // wasi:cli/exit instance type: { exit: (result<()>) -> () }

    // The resource types (input-stream, output-stream) are complex.
    // In the component model, resources have type indices.
    // Canonical ABI uses handle indices (i32).

    // Let me check: can I use PrimitiveValType::S32 for handles in lowered functions?
    // When `canon lower` is applied to a function that takes/returns a resource,
    // the core function signature uses i32 for the handle.
    // So in the bridge module, handles are just i32 values.

    // The real complexity is in the list<u8> return from blocking-read.
    // In the canonical ABI, when lowering a function that returns list<u8>:
    //   - The core function takes an extra i32 parameter (return pointer)
    //   - On success, it writes (i32 ptr, i32 len) at the return pointer
    //   - The function returns i32 (0=ok, >0=error discriminant)
    //   - The ptr points to data in the callee's memory (or the caller's memory?)

    // Actually, for lowered functions, the memory is specified by CanonicalOption::Memory.
    // The return area is also specified by CanonicalOption::Return(ptr).
    // So the bridge module's memory is used for all data transfer.

    // This means:
    // - blocking_read lowered: (handle: i32, len: i64, ret_ptr: i32) -> i32
    //   On success: memory[ret_ptr] = data_ptr, memory[ret_ptr+4] = data_len
    //   The data is written to the bridge's memory
    // - blocking_write lowered: (handle: i32, data_ptr: i32, data_len: i32) -> i32
    //   Reads data from bridge's memory

    // The bridge module then copies data between its memory and the main module's memory.
    // But wait — core modules in a component don't share memory by default.
    // Each core module has its own memory.
    //
    // Unless... the main module exports its memory and the bridge imports it.
    // Or the component shares memory between instances.
    //
    // In the adapter approach, the adapter and main module share memory.
    // The adapter imports "memory" from the main module.
    //
    // For our minimal bridge, we can do the same: the bridge imports the main module's memory.

    // OK this is getting really involved. Let me just code it up properly.

    Err("Direct P2 bridge module is work in progress. The architecture is clear but implementation needs canonical ABI handling for resource types and list<u8> returns. Use wit-component adapter for now (outlayer-p2 target).".into())
}

fn build_bridge_module() -> Vec<u8> {
    // Minimal adapter core module
    // Exports: fd_read, fd_write, proc_exit
    // Imports: lowered P2 functions + main module's memory + main module's _start
    //
    // This is a WASI P1 "adapter" module that translates fd_read/fd_write
    // to P2 component stream calls.
    //
    // Memory layout (uses main module's memory via import):
    //   0x00000 - 0x07FFF: main module's data (heap, stack, etc.)
    //   0x08000 - 0x0FFFF: bridge temp area (return buffer, etc.)
    //   
    // Globals:
    //   0: stdin_handle (i32, initialized to -1 = not yet opened)
    //   1: stdout_handle (i32, initialized to -1 = not yet opened)

    let mut m = Module::new();

    // Types
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32, ValType::I32, ValType::I32, ValType::I32], [ValType::I32]); // 0: fd_read/fd_write
    types.ty().function([ValType::I32], []); // 1: proc_exit
    types.ty().function([], [ValType::I32]); // 2: get_stdin/get_stdout () -> handle
    types.ty().function([ValType::I32, ValType::I64, ValType::I32], [ValType::I32]); // 3: blocking_read(handle, len, ret_ptr) -> err
    types.ty().function([ValType::I32, ValType::I32, ValType::I32], [ValType::I32]); // 4: blocking_write(handle, ptr, len) -> err
    types.ty().function([], []); // 5: exit() — void exit (no args for success)
    types.ty().function([], []); // 6: core_start () -> ()
    m.section(&types);

    // Imports
    let mut imports = ImportSection::new();
    imports.import("p2", "get_stdin", EntityType::Function(2));
    imports.import("p2", "blocking_read", EntityType::Function(3));
    imports.import("p2", "get_stdout", EntityType::Function(2));
    imports.import("p2", "blocking_write", EntityType::Function(4));
    imports.import("p2", "exit", EntityType::Function(5));
    imports.import("", "memory", EntityType::Memory(MemoryType {
        minimum: 4, maximum: None, memory64: false, shared: false, page_size_log2: None
    }));
    imports.import("core", "_start", EntityType::Function(6));
    m.section(&imports);

    // Globals: stdin_handle (-1), stdout_handle (-1)
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType { val_type: ValType::I32, mutable: true, shared: false },
        &ConstExpr::i32_const(-1),
    );
    globals.global(
        GlobalType { val_type: ValType::I32, mutable: true, shared: false },
        &ConstExpr::i32_const(-1),
    );
    m.section(&globals);

    // Functions: fd_read(7), fd_write(8), proc_exit(9)
    let mut funcs = FunctionSection::new();
    funcs.function(0); // fd_read: type 0
    funcs.function(0); // fd_write: type 0
    funcs.function(1); // proc_exit: type 1
    m.section(&funcs);

    // Exports
    let mut exports = ExportSection::new();
    exports.export("fd_read", ExportKind::Func, 7);
    exports.export("fd_write", ExportKind::Func, 8);
    exports.export("proc_exit", ExportKind::Func, 9);
    m.section(&exports);

    // Code
    let mut code = CodeSection::new();

    // fd_read(fd, iovs_ptr, iovs_len, nwritten_ptr) -> i32
    // Only handles fd=0 (stdin)
    {
        let mut f = Function::new([
            (0, ValType::I32), // fd
            (1, ValType::I32), // iovs_ptr
            (2, ValType::I32), // iovs_len
            (3, ValType::I32), // nwritten_ptr
            (4, ValType::I32), // temp handle
            (5, ValType::I32), // temp ptr
            (6, ValType::I32), // temp len
            (7, ValType::I32), // total read
        ]);

        let ma = MemArg { offset: 0, align: 2, memory_index: 0 };
        let ma1 = MemArg { offset: 0, align: 0, memory_index: 0 };

        // Return EBADF (28) if fd != 0
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::result(ValType::I32)));
        f.instruction(&Instruction::I32Const(28)); // EBADF
        f.instruction(&Instruction::Return);

        f.instruction(&Instruction::Else);

        // Get stdin handle (lazy init)
        f.instruction(&Instruction::GlobalGet(0)); // stdin_handle
        f.instruction(&Instruction::I32Const(-1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::Call(0)); // get_stdin → handle
        f.instruction(&Instruction::GlobalSet(0));
        f.instruction(&Instruction::End);

        // Read first iov to get buffer ptr and len
        // iov[0] = {ptr: i32, len: i32} at iovs_ptr
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(ma.clone())); // iov[0].buf
        f.instruction(&Instruction::LocalSet(5)); // buf_ptr
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load8(0, ma.clone())); // iov[0].len
        // Wait, iov layout is: [ptr: u32, len: u32] = 8 bytes per iov
        // Actually iov for fd_read is [buf_ptr: i32, buf_len: i32]
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load(ma.clone())); // iov[0].len
        f.instruction(&Instruction::LocalSet(6)); // buf_len

        // Call blocking_read(handle, buf_len, return_area)
        // return_area is at a temp location (offset 0x8000 = 32768)
        f.instruction(&Instruction::GlobalGet(0)); // stdin_handle
        f.instruction(&Instruction::LocalGet(6));
        f.instruction(&Instruction::I64ExtendI32U); // len as u64
        f.instruction(&Instruction::I32Const(0x8000)); // return area
        f.instruction(&Instruction::Call(1)); // blocking_read

        // Check error (return != 0)
        f.instruction(&Instruction::If(BlockType::result(ValType::I32)));
        f.instruction(&Instruction::I32Const(5)); // EIO
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::Else);

        // Read return area: [ptr, len]
        f.instruction(&Instruction::I32Const(0x8000));
        f.instruction(&Instruction::I32Load(ma.clone())); // ptr
        f.instruction(&Instruction::LocalSet(5));
        f.instruction(&Instruction::I32Const(0x8004));
        f.instruction(&Instruction::I32Load(ma.clone())); // len
        f.instruction(&Instruction::LocalSet(6));

        // Copy data from return ptr to iov buffer
        // Simple memcpy: copy min(buf_len, returned_len) bytes
        // For simplicity, assume returned data fits in the iov buffer
        // and just copy returned_len bytes to iov[0].buf
        // 
        // Actually, blocking_read writes data to memory. The ptr in the return area
        // points to where the data is. We need to copy it to the iov buffer.
        //
        // But if both are in the same memory (shared via import), we can use
        // memory.copy.
        f.instruction(&Instruction::LocalGet(1)); // iovs_ptr → iov[0].buf = dest
        f.instruction(&Instruction::I32Load(ma.clone())); // dest = iov[0].buf
        f.instruction(&Instruction::LocalGet(5)); // src = data ptr from read
        f.instruction(&Instruction::LocalGet(6)); // len
        f.instruction(&Instruction::MemoryCopy { src_mem: 0, dst_mem: 0 });

        // Write nwritten
        f.instruction(&Instruction::LocalGet(3)); // nwritten_ptr
        f.instruction(&Instruction::LocalGet(6)); // bytes read
        f.instruction(&Instruction::I32Store(ma));

        f.instruction(&Instruction::LocalGet(6)); // return bytes read
        f.instruction(&Instruction::Return);

        f.instruction(&Instruction::End); // else (blocking_read error)
        f.instruction(&Instruction::End); // else (fd != 0)
        f.instruction(&Instruction::I32Const(0)); // shouldn't reach here
        f.instruction(&Instruction::End);
        code.function(&f);
    }

    // fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> i32
    {
        let mut f = Function::new([
            (0, ValType::I32), // fd
            (1, ValType::I32), // iovs_ptr
            (2, ValType::I32), // iovs_len
            (3, ValType::I32), // nwritten_ptr
            (4, ValType::I32), // temp
        ]);

        let ma = MemArg { offset: 0, align: 2, memory_index: 0 };

        // Only handle fd=1 (stdout)
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::I32Const(1));
        f.instruction(&Instruction::I32Ne);
        f.instruction(&Instruction::If(BlockType::result(ValType::I32)));
        f.instruction(&Instruction::I32Const(28)); // EBADF
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::Else);

        // Get stdout handle (lazy init)
        f.instruction(&Instruction::GlobalGet(1)); // stdout_handle
        f.instruction(&Instruction::I32Const(-1));
        f.instruction(&Instruction::I32Eq);
        f.instruction(&Instruction::If(BlockType::Empty));
        f.instruction(&Instruction::Call(2)); // get_stdout → handle
        f.instruction(&Instruction::GlobalSet(1));
        f.instruction(&Instruction::End);

        // Get iov[0] ptr and len
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Load(ma.clone())); // iov[0].buf = ptr
        f.instruction(&Instruction::LocalSet(4));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I32Const(4));
        f.instruction(&Instruction::I32Add);
        f.instruction(&Instruction::I32Load(ma.clone())); // iov[0].len
        f.instruction(&Instruction::LocalSet(2)); // reuse iovs_len local as buf_len

        // Call blocking_write(handle, ptr, len)
        f.instruction(&Instruction::GlobalGet(1)); // stdout_handle
        f.instruction(&Instruction::LocalGet(4)); // ptr
        f.instruction(&Instruction::LocalGet(2)); // len
        f.instruction(&Instruction::Call(3)); // blocking_write

        // Check error
        f.instruction(&Instruction::If(BlockType::result(ValType::I32)));
        f.instruction(&Instruction::I32Const(5)); // EIO
        f.instruction(&Instruction::Return);
        f.instruction(&Instruction::Else);

        // Write nwritten
        f.instruction(&Instruction::LocalGet(3));
        f.instruction(&Instruction::LocalGet(2));
        f.instruction(&Instruction::I32Store(ma));

        f.instruction(&Instruction::LocalGet(2)); // return bytes written
        f.instruction(&Instruction::Return);

        f.instruction(&Instruction::End); // else
        f.instruction(&Instruction::End); // else fd
        f.instruction(&Instruction::I32Const(0));
        f.instruction(&Instruction::End);
        code.function(&f);
    }

    // proc_exit(code) — call P2 exit
    {
        let mut f = Function::new([(0, ValType::I32)]); // code
        // For success (code=0), call exit with no error
        // For failure, we'd need to encode the error but let's keep it simple
        f.instruction(&Instruction::Call(4)); // p2.exit
        f.instruction(&Instruction::Unreachable); // exit doesn't return
        f.instruction(&Instruction::End);
        code.function(&f);
    }

    m.section(&code);
    m.finish()
}
