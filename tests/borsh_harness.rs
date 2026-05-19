//! WasmRunner: compile Lisp → WASM, instantiate with wasmtime, run, inspect memory.
//!
//! Reusable test harness for Borsh round-trip and property-based tests.
//! Include this from a test file with:
//!
//! ```
//! #[path = "borsh_harness.rs"]
//! mod harness;
//! ```

use lisp_rlm_wasm::wasm_emit::compile_fuzz;
use wasmtime::*;

// Re-export tagged_value constants and functions
pub use lisp_rlm_wasm::tagged_value::*;

// ── Memory layout (must match wasm_emit.rs) ──

/// Where compile_fuzz stores the tagged result value.
pub const TEMP_MEM_USIZE: usize = 64;

/// Pre-write Borsh bytes here for deserialize tests.
pub const BORSH_BUF_USIZE: usize = 36864;

// ── WasmRunner ──

pub struct WasmRunner {
    memory: Memory,
    store: Store<()>,
    instance: Option<Instance>,
}

impl WasmRunner {
    /// Compile Lisp source with compile_fuzz, instantiate WASM.
    pub fn new(source: &str) -> Result<Self, String> {
        let wasm = compile_fuzz(source).map_err(|e| format!("compile: {}", e))?;
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {}", e))?;
        let mut store = Store::new(&engine, ());
        let mut linker = Linker::new(&engine);

        let needs_imported_memory = module
            .imports()
            .any(|i| i.module() == "env" && i.name() == "memory");

        let fallback_memory = Memory::new(&mut store, MemoryType::new(4, None))
            .map_err(|e| format!("memory: {}", e))?;

        if needs_imported_memory {
            linker
                .define(&store, "env", "memory", fallback_memory)
                .map_err(|e| format!("link memory: {}", e))?;
        }

        // Stub all env imports (except memory)
        for import in module.imports() {
            if import.module() == "env" && import.name() != "memory" {
                if let ExternType::Func(func_ty) = import.ty() {
                    let params: Vec<ValType> = func_ty.params().collect();
                    let results: Vec<ValType> = func_ty.results().collect();
                    let result_count = results.len();
                    let ft = FuncType::new(&engine, params, results);
                    let stub = Func::new(&mut store, ft, move |_, _, ret| {
                        for i in 0..result_count {
                            ret[i] = Val::I64(0);
                        }
                        Ok(())
                    });
                    linker
                        .define(&store, "env", import.name(), stub)
                        .map_err(|e| format!("link {}: {}", import.name(), e))?;
                }
            }
        }

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("instantiate: {}", e))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .unwrap_or(fallback_memory);

        Ok(Self {
            memory,
            store,
            instance: Some(instance),
        })
    }

    /// Pre-write bytes at `offset` in WASM memory.
    pub fn write_bytes(&mut self, offset: usize, bytes: &[u8]) {
        self.memory.data_mut(&mut self.store)[offset..offset + bytes.len()]
            .copy_from_slice(bytes);
    }

    /// Write a Borsh-encoded i64 at `offset` (8 bytes LE).
    pub fn write_i64(&mut self, offset: usize, val: i64) {
        self.write_bytes(offset, &val.to_le_bytes());
    }

    /// Write a Borsh-encoded u32 at `offset` (4 bytes LE).
    pub fn write_u32(&mut self, offset: usize, val: u32) {
        self.write_bytes(offset, &val.to_le_bytes());
    }

    /// Write a Borsh-encoded u8 at `offset`.
    pub fn write_u8(&mut self, offset: usize, val: u8) {
        self.write_bytes(offset, &[val]);
    }

    /// Run the "run" export.
    pub fn run(&mut self) -> Result<(), String> {
        let inst = self.instance.as_ref().ok_or("no instance")?;
        let run_fn = inst
            .get_typed_func::<(), ()>(&mut self.store, "run")
            .map_err(|e| format!("no 'run' export: {}", e))?;
        run_fn.call(&mut self.store, ())
            .map_err(|e| format!("trap: {}", e))?;
        Ok(())
    }

    /// Read an i64 from WASM memory (little-endian).
    pub fn read_i64(&self, offset: usize) -> i64 {
        let mem = self.memory.data(&self.store);
        i64::from_le_bytes(mem[offset..offset + 8].try_into().unwrap())
    }

    /// Read the tagged value at TEMP_MEM and decode it.
    /// Handles nil sentinel — returns TaggedValue::Nil for the sentinel.
    pub fn read_result(&self) -> TaggedValue {
        let tagged = self.read_raw_result();
        let mem = self.memory.data(&self.store);
        decode(mem, tagged)
    }

    /// Read the tagged value at TEMP_MEM as a raw i64.
    pub fn read_raw_result(&self) -> i64 {
        self.read_i64(TEMP_MEM_USIZE)
    }

    /// Get a copy of WASM memory contents.
    pub fn memory_data(&self) -> Vec<u8> {
        self.memory.data(&self.store).to_vec()
    }

    /// Read a runtime array from the heap.
    /// Returns the raw tagged elements (not untagged).
    pub fn read_array_raw(&self, tagged: i64) -> Vec<i64> {
        if is_nil_sentinel(tagged) {
            return vec![];
        }
        // Also handle normal TAG_NIL if we somehow get the short form
        if (tagged & TAG_MASK) == TAG_NIL && !is_nil_sentinel(tagged) {
            return vec![];
        }
        let mem = self.memory.data(&self.store);
        let ptr = (tagged >> TAG_BITS) as usize;
        let count = i64::from_le_bytes(mem[ptr..ptr + 8].try_into().unwrap()) as usize;
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let off = ptr + 8 + i * 8;
            let elem = i64::from_le_bytes(mem[off..off + 8].try_into().unwrap());
            result.push(elem);
        }
        result
    }

    /// Read a runtime array, untagging each element as a number.
    /// Panics if any element is not TAG_NUM.
    pub fn read_array_nums(&self, tagged: i64) -> Vec<i64> {
        self.read_array_raw(tagged)
            .into_iter()
            .map(|t| {
                assert_eq!(
                    t & TAG_MASK,
                    TAG_NUM,
                    "expected TAG_NUM, got tag {}",
                    t & TAG_MASK
                );
                t >> TAG_BITS
            })
            .collect()
    }

    /// Read Borsh-encoded bytes from BORSH_BUF.
    /// Returns `len` bytes starting at BORSH_BUF.
    pub fn read_borsh_bytes(&self, len: usize) -> Vec<u8> {
        let mem = self.memory.data(&self.store);
        mem[BORSH_BUF_USIZE..BORSH_BUF_USIZE + len].to_vec()
    }
}

// ── Borsh encoding helpers ──

/// Borsh-encoded i64 (8 bytes LE).
pub fn borsh_i64(val: i64) -> Vec<u8> {
    val.to_le_bytes().to_vec()
}

/// Borsh-encoded u32 (4 bytes LE).
pub fn borsh_u32(val: u32) -> Vec<u8> {
    val.to_le_bytes().to_vec()
}

/// Borsh-encoded u8 (1 byte).
pub fn borsh_u8(val: u8) -> Vec<u8> {
    vec![val]
}

/// Borsh-encoded bool (1 byte).
pub fn borsh_bool(val: bool) -> Vec<u8> {
    vec![if val { 1u8 } else { 0u8 }]
}

/// Build a Lisp program that serializes values with the given schema.
/// `args` is the argument list to `borsh-serialize`.
pub fn ser_program(schema: &str, args: &str) -> String {
    // Extract the top-level name from the schema, e.g. "(Counter (count i64))" -> "Counter"
    let name = schema
        .trim_start_matches('(')
        .split_whitespace()
        .next()
        .unwrap_or("S");
    format!(
        r#"
(borsh-schema {schema})
(define (run) (borsh-serialize "{name}" {args}))
(export "run" run)
"#
    )
}

/// Build a Lisp program that deserializes bytes from BORSH_BUF.
pub fn deser_program(schema: &str) -> String {
    // Extract the top-level name from the schema, e.g. "(Counter (count i64))" -> "Counter"
    let name = schema
        .trim_start_matches('(')
        .split_whitespace()
        .next()
        .unwrap_or("S");
    format!(
        r#"
(borsh-schema {schema})
(define (run)
  (let ((buf 36864))
    (borsh-deserialize "{name}" buf)))
(export "run" run)
"#
    )
}