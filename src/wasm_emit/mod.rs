#![allow(dead_code)]
#![allow(unreachable_patterns)]
//! Binary WASM emitter via wasm-encoder. No string-based WAT.
//!
//! NEAR constraints baked in:
//! - All exports: () -> ()
//! - value_return (not return_value)
//! - Exact host function signatures
//! - Only import what's used
//! - Imports before memory

use crate::types::LispVal;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use wasm_encoder::{
    BlockType, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

// ─── Sub-modules ──────────────────────────────────────────────────────
pub mod helpers;
pub mod intrinsics;
pub mod lambda;
pub mod gas;
pub mod const_fold;
pub mod dynamic_call;
pub mod host_calls;
pub mod borsh;
pub mod json;
pub mod call;
pub mod call_core;
pub mod call_near_storage;
pub mod call_near_io;
pub mod call_hof;
pub mod call_json;
pub mod call_borsh;
pub mod call_list;
pub mod call_near_context;
pub mod call_near_crypto;
pub mod call_near_promise;
pub mod call_near_iter;
pub mod call_u128;
pub mod call_u128_str;
use call_u128_str::U128Helpers;
pub mod call_fp;
pub mod call_defi;
pub mod call_bitwise;
pub mod call_string;
mod call_string_list;
pub mod try_catch;
use try_catch::TryFrame;
pub mod call_outlayer;
pub mod call_predicate;
pub mod call_dict;
pub mod wasm_link;
pub mod compile;
pub mod int_emit;

// Re-exports: public API lives in compile.rs
pub use compile::{compile_pure, compile_standalone, compile_standalone_opts, compile_fuzz, compile_near, compile_near_untyped, compile_near_from_exprs, compile_near_to_wat_from_exprs, compile_pure_to_wat, compile_near_to_wat, resolve_modules};


// ── NEAR host functions (name, params, results) ──

// NEAR host function signatures — from nearcore imports.rs, verified on testnet Apr 30 2026
// Format: (wasm_name, params, results)
// [] = void return, [I64] = returns u64
pub(crate) const HOST_FUNCS: &[(&str, &[ValType], &[ValType])] = &[
    // Registers
    ("read_register",               &[ValType::I64, ValType::I64], &[]),           // 0
    ("register_len",                &[ValType::I64], &[ValType::I64]),             // 1
    ("write_register",              &[ValType::I64, ValType::I64, ValType::I64], &[]), // 2
    // Context
    ("current_account_id",          &[ValType::I64], &[]),                         // 3
    ("signer_account_id",           &[ValType::I64], &[]),                         // 4
    ("signer_account_pk",           &[ValType::I64], &[]),                         // 5
    ("predecessor_account_id",      &[ValType::I64], &[]),                         // 6
    ("input",                       &[ValType::I64], &[]),                         // 7
    ("block_index",                 &[], &[ValType::I64]),                         // 8
    ("block_timestamp",             &[], &[ValType::I64]),                         // 9
    ("epoch_height",                &[], &[ValType::I64]),                         // 10
    ("storage_usage",               &[], &[ValType::I64]),                         // 11
    // Economics
    ("account_balance",             &[ValType::I64], &[]),                         // 12
    ("account_locked_balance",      &[ValType::I64], &[]),                         // 13
    ("attached_deposit",            &[ValType::I64], &[]),                         // 14
    ("prepaid_gas",                 &[], &[ValType::I64]),                         // 15
    ("used_gas",                    &[], &[ValType::I64]),                         // 16
    // Storage
    ("storage_write",               &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 17
    ("storage_read",                &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),     // 18
    ("storage_remove",              &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),     // 19
    ("storage_has_key",             &[ValType::I64, ValType::I64], &[ValType::I64]),                   // 20
    // Math / Crypto
    ("sha256",                      &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 21
    ("keccak256",                   &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 22
    ("random_seed",                 &[ValType::I64], &[]),                                          // 23
    ("ed25519_verify",              &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 24
    // Misc
    ("value_return",                &[ValType::I64, ValType::I64], &[]),            // 25
    ("panic",                       &[], &[]),                                      // 26
    ("panic_utf8",                  &[ValType::I64, ValType::I64], &[]),            // 27
    ("log_utf8",                    &[ValType::I64, ValType::I64], &[]),            // 28
    ("log_utf16",                   &[ValType::I64, ValType::I64], &[]),            // 29
    // Promises (core)
    ("promise_create",              &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 30
    ("promise_then",                &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 31
    ("promise_and",                 &[ValType::I64, ValType::I64], &[ValType::I64]), // 32
    ("promise_results_count",       &[], &[ValType::I64]),                          // 33
    ("promise_result",              &[ValType::I64, ValType::I64], &[ValType::I64]),   // 34
    ("promise_return",              &[ValType::I64], &[]),                          // 35
    // Iterator
    ("storage_iter_prefix",         &[ValType::I64, ValType::I64], &[ValType::I64]), // 36
    ("storage_iter_range",          &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 37
    ("storage_iter_next",           &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 38
    // Promise batch
    ("promise_batch_create",        &[ValType::I64, ValType::I64], &[ValType::I64]),              // 39
    ("promise_batch_then",          &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 40
    ("promise_batch_action_create_account", &[ValType::I64], &[]),                                // 41
    ("promise_batch_action_deploy_contract", &[ValType::I64, ValType::I64, ValType::I64], &[]),    // 42
    ("promise_batch_action_function_call", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 43
    ("promise_batch_action_transfer", &[ValType::I64, ValType::I64], &[]),            // 44
    ("promise_batch_action_stake",  &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 45
    ("promise_batch_action_add_key_with_full_access", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 46
    ("promise_batch_action_add_key_with_function_call", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 47
    ("promise_batch_action_delete_key", &[ValType::I64, ValType::I64, ValType::I64], &[]),          // 48
    ("promise_batch_action_delete_account", &[ValType::I64, ValType::I64, ValType::I64], &[]),      // 49
    // Global contracts
    ("deploy_contract",             &[ValType::I64, ValType::I64], &[]),            // 50
    ("current_code_hash",           &[ValType::I64], &[]),                          // 51
    // Extra crypto
    ("keccak512",                   &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 52
    ("ripemd160",                   &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 53
    ("ecrecover",                   &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 54
    ("p256_verify",                 &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 55 — NEAR host: (sig_len, sig_ptr, msg_len, msg_ptr, pk_len, pk_ptr) → u64
    // Alt BN128
    ("alt_bn128_g1_multiexp",       &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 56
    ("alt_bn128_g1_sum",            &[ValType::I64, ValType::I64, ValType::I64], &[]),                // 57
    ("alt_bn128_pairing_check",     &[ValType::I64, ValType::I64], &[ValType::I64]),                   // 58
    // BLS12-381
    ("bls12381_p1_sum",             &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 59
    ("bls12381_p2_sum",             &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 60
    ("bls12381_g1_multiexp",        &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 61
    ("bls12381_g2_multiexp",        &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 62
    ("bls12381_map_fp_to_g1",       &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 63
    ("bls12381_map_fp2_to_g2",      &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 64
    ("bls12381_pairing_check",      &[ValType::I64, ValType::I64], &[ValType::I64]),                   // 65
    ("bls12381_p1_decompress",      &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 66
    ("bls12381_p2_decompress",      &[ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]),    // 67
    // Extra promises
    ("promise_set_refund_to",                    &[ValType::I64, ValType::I64, ValType::I64], &[]),               // 68
    ("promise_batch_action_state_init",          &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 69
    ("promise_batch_action_state_init_by_account_id", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 70
    ("set_state_init_data_entry",                &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 71
    ("current_contract_code",                    &[ValType::I64], &[ValType::I64]),                                // 72
    ("refund_to_account_id",                     &[ValType::I64], &[]),                                          // 73
    ("promise_batch_action_function_call_weight", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 74
    ("promise_batch_action_deploy_global_contract", &[ValType::I64, ValType::I64, ValType::I64], &[]),         // 75
    ("promise_batch_action_deploy_global_contract_by_account_id", &[ValType::I64, ValType::I64, ValType::I64], &[]), // 76
    ("promise_batch_action_use_global_contract", &[ValType::I64, ValType::I64, ValType::I64], &[]),             // 77
    ("promise_batch_action_use_global_contract_by_account_id", &[ValType::I64, ValType::I64, ValType::I64], &[]), // 78
    ("promise_batch_action_transfer_to_gas_key", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 79
    ("promise_batch_action_add_gas_key_with_full_access", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 80
    ("promise_batch_action_add_gas_key_with_function_call", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[]), // 81
    ("promise_yield_create",  &[ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 82
    ("promise_yield_resume", &[ValType::I64, ValType::I64, ValType::I64, ValType::I64], &[ValType::I64]), // 83
    // Validator
    ("validator_stake",       &[ValType::I64, ValType::I64, ValType::I64], &[]),  // 84
    ("validator_total_stake", &[ValType::I64], &[]),                              // 85
];

const HOST_BASE: u32 = 0xFF00_0000;
const USER_BASE: u32 = 0xFF01_0000;
pub(crate) const WASM_IMPORT_BASE: u32 = 0xFF02_0000; // sentinel for stitched WASM imports (e.g. schnorr)
pub const WASI_FD_WRITE: u32 = 90; // sentinel for WASI fd_write in outlayer mode
pub const MEMCPY_SENTINEL: u32 = 91; // sentinel for shared memcpy helper
const TEMP_MEM: i64 = 64;
const AMOUNT_MEM: i64 = 256; // u128 deposit buffer (16 bytes at 256..272)
const INPUT_BUF: i64 = 16384;  // 16KB for input JSON args
const RETURN_BUF: i64 = 32768;
const STORAGE_BUF: i64 = 8192;  // 8 bytes for storage read/write buffer
const STORAGE_U128_BUF: i64 = 8208;  // 16 bytes for u128 storage ops
pub(crate) const KEY_BUF: i64 = 8224; // 256 bytes for composite key building (near/kv, near/kv-get)
const KEY_BUF_MAX: usize = 256;       // max composite key length
pub(crate) const HEAP_START: i64 = 200_000; // heap starts above all data segments and buffers (STDOUT=65536, INPUT=16384, etc.)
const BORSH_BUF: i64 = 36864; // 4KB scratch buffer for Borsh serialize (after RETURN_BUF)

// ── Borsh schema types (compile-time only) ──
#[derive(Clone, Debug)]
pub(crate) enum BorshType {
    U8, U32, U64, I64, U128, F64, Bool, String, Bytes,
    Vec(Box<BorshType>),
    Option(Box<BorshType>),
    Struct { fields: Vec<(String, BorshType)> },
    Enum { variants: Vec<(String, Vec<(String, BorshType)>)> },
}
// ── Tagged value scheme (3-bit tag in bottom bits) ──
// Every value on the WASM stack is a tagged i64:
//   bits 2..0 = type tag, bits 63..3 = payload
// Falsy set: { Bool(false)=1, Nil=4, Num(0)=0 }  (GAPS.md round-3 fix 1 — numeric
// zero is FALSY, matching interpreter helpers::is_truthy; verified by nm_zero_is_falsy).
// "" (empty str) and '() (empty array) REMAIN truthy by decision.
// NOTE: Float(0.0) is falsy in the interpreter; wasm has no float runtime tag yet —
// float literals hard-error at compile. When floats land here, add Float(0.0) to the falsy set.
const TAG_NUM:     i64 = 0; // payload = integer value (61-bit signed)
const TAG_BOOL:    i64 = 1; // payload = 0 (false) or 1 (true)
const TAG_FNREF:   i64 = 2; // payload = function index
const TAG_CLOSURE: i64 = 3; // payload = heap pointer
pub const TAG_NIL:     i64 = 4;
pub const TAG_STR:     i64 = 5; // payload = (heap_off | (len << 32))
const TAG_ARRAY:   i64 = 6; // payload = ((heap_ptr << TAG_BITS) | TAG_ARRAY), heap layout: [count, elem0, elem1, ...]
pub(crate) const TAG_BITS: i64 = 3;
const RUNTIME_HEAP_PTR: i64 = 56; // 8-byte memory slot holding runtime bump-allocator ptr (initialized from heap_ptr)
// Sentinel falsy values (used for truthiness check)
const TAGGED_FALSE: i64 = TAG_BOOL;       // 1
const TAGGED_NIL:   i64 = TAG_NIL;        // 4
// ~300 Tgas on NEAR ≈ ~10B simple ops. Cap at 1B to be safe (stops runaway, still uses full NEAR runtime).
const GAS_LIMIT: i64 = 1_000_000_000;
const RETURN_FLAG: u32 = 0; // mutable i64 global for return flag
const FP_GLOBAL: u32 = 1;  // mutable i64 global for frame pointer (NEAR mode)

// ── Memory safety constants ──
const DEPTH_COUNTER: i64 = 40;  // 8-byte slot: recursion depth counter. MUST live below HEAP_START — the runtime heap grows upward unboundedly and would stomp a high address (e.g. old 999980) once allocation volume grows.
const MAX_DEPTH: i64 = 16384;   // max call depth before trap (2026-08-29: 512 was exhausted by meta-circular interpreters — every nested my-eval/eval-form call is a lisp call; ~11 wasm frames per interpreted recursion level left user code only ~45 levels)
// Protected memory regions: [start, end) — store_i64/load_i64/mem-set!/mem-get may NOT write here
// Covers: TEMP_MEM(64), AMOUNT_MEM(256), STORAGE_BUF(8192), STORAGE_U128_BUF(8208),
//         HANDLE_COUNT_ADDR(48), RUNTIME_HEAP_PTR(56), DEPTH_COUNTER(999980), BORSH_BUF(36864)
const PROTECTED_REGIONS: &[(i64, i64)] = &[
    (40, 48),    // DEPTH_COUNTER (below HANDLE_COUNT — 64..96 is the test-call marshalling area, do NOT use)
    (48, 56),    // HANDLE_COUNT_ADDR
    (56, 64),    // RUNTIME_HEAP_PTR
    (64, 72),    // TEMP_MEM
    (256, 272),  // AMOUNT_MEM
    (4096, 49152+4096), // HEAP..HANDLE_TABLE end
    (8192, 8480),// STORAGE_BUF + STORAGE_U128_BUF + KEY_BUF
    (16384, 32768+8192), // INPUT_BUF..RETURN_BUF end
    (36864, 40960), // BORSH_BUF
];
// Tag validation: valid low 3 bits are 0–6 (TAG_NUM..TAG_ARRAY). 7 is invalid.
const TAG_INVALID: i64 = 7;

// ── Handle table for memory-safe struct access ──
const HANDLE_COUNT_ADDR: i64 = 48;   // 8-byte slot: number of allocated handles
const HANDLE_TABLE_BASE: i64 = 49152; // base of handle table (1024 entries × 16 bytes = 16KB)
const MAX_HANDLES: i64 = 1024;        // max concurrent allocations
// 1024 entries occupy [49152, 65536). STDOUT_BASE is 65536 — table end exactly
// meets it, no overlap (2026-08-29: was 256 entries; recursive Lisp interpreters
// heap-allocating per eval node exhausted 256 handles and trapped 'unreachable').

pub(crate) struct FuncDef {
    pub name: String,
    pub param_count: usize,
    pub local_count: usize,
    pub instrs: Vec<Instruction<'static>>,
    /// If set, overrides the default (extra locals as I64) for this function's code section.
    pub local_entries: Option<Vec<(u32, ValType)>>,
    /// If set, overrides the type index in the function section (params: Vec<ValType>, results: Vec<ValType>).
    pub custom_type: Option<(Vec<ValType>, Vec<ValType>)>,
}

impl Default for FuncDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            param_count: 0,
            local_count: 0,
            instrs: Vec::new(),
            local_entries: None,
            custom_type: None,
        }
    }
}

/// Parse a URL string into (authority, path).
/// Strips `https://` or `http://` prefix, splits on first `/`.
/// E.g. `"https://api.open-meteo.com/v1/forecast?lat=45"` → `("api.open-meteo.com", "/v1/forecast?lat=45")`
pub(crate) fn parse_url(url: &str) -> (String, String) {
    let stripped = url.strip_prefix("https://").unwrap_or(url);
    let stripped = stripped.strip_prefix("http://").unwrap_or(stripped);
    match stripped.find('/') {
        Some(idx) => (stripped[..idx].to_string(), stripped[idx..].to_string()),
        None => (stripped.to_string(), "/".to_string()),
    }
}

pub struct WasmEmitter {
    pub(crate) locals: HashMap<String, u32>,
    pub(crate) next_local: u32,
    pub(crate) free_locals: Vec<u32>, // recyclable local slots
    pub(crate) local_type_map: Vec<ValType>, // per-local type (indexed by local idx)
    pub(crate) current_func: Option<String>,
    pub(crate) current_param_count: usize,
    /// Nesting depth inside tc() — 0 = directly in loop, 1 = inside tc_if's if, etc.
    pub(crate) tc_depth: u32,
    pub(crate) try_stack: Vec<TryFrame>, // active try bodies (guarded fallible ops)
    pub(crate) while_id: Cell<usize>,
    pub(crate) funcs: Vec<FuncDef>,
    pub(crate) memory_pages: u32,
    pub(crate) exports: Vec<(String, String, bool)>,
    pub(crate) data_segments: Vec<(u32, Vec<u8>)>,
    pub(crate) next_data_offset: u32,
    pub(crate) host_needed: HashSet<usize>,
    pub(crate) gas_local: Option<u32>, // index of the gas counter local (i64)
    pub(crate) needs_frame: bool, // function body allocates from FP_GLOBAL
    pub(crate) heap_ptr: u32, // bump allocator; 0 = not yet initialized (lazy snap to data section end)
    pub(crate) lambda_counter: u32, // unique lambda id
    pub(crate) list_ptr_counter: u32, // unique temp per (list ...) site — nested dynamic lists each need their OWN __lst_ptr local
    pub(crate) str_cat_depth: u32, // nesting depth for str-cat local isolation
    pub(crate) fuzz_mode: bool, // if true, export wrappers store tagged values (no untag, no value_return)
    pub(crate) u128h: Option<U128Helpers>, // string-based u128 helper func indices (lazily synthesized)
    pub(crate) fn_int_annotations: std::collections::HashMap<String, (usize, bool)>, // name → (n int params, ret int)
    pub(crate) raw_twins: std::collections::HashMap<String, u32>, // fn name → raw twin func idx
    pub(crate) need_outlayer: bool, // true if outlayer/* dispatch forms are used
    pub(crate) need_wasi_http: bool, // true if http-get is used (for P2 wasi:http path)
    pub(crate) http_urls: Vec<(String, String)>, // (authority, path) per http-get call in p2_mode
    pub(crate) http_post_urls: Vec<(String, String)>, // (authority, path) per http-post call in p2_mode
    pub(crate) wasi_mode: bool, // true when targeting WASI/OutLayer
    pub(crate) p2_mode: bool,   // true when targeting P2 component (return i32 from _start)
    pub(crate) no_proc_exit: bool, // true when wrapping with wit-component adapter (return cleanly, don't call proc_exit)
    pub(crate) storage_get_count: u32, // counter for unique ret_area per storage-get call
    pub(crate) http_post_call_count: u32, // counter for per-call sentinel offset in wasi:http POST path
    pub(crate) env_get_count: u32, // counter for unique ret_area per env/get call

    // Track which function each lambda maps to, and its captured var count
    // lambda_id -> (func_array_idx, captured_count)
    pub(crate) lambda_info: Vec<(usize, usize)>, 
    // When compiling a lambda, maps captured var names to their offset in the closure
    pub(crate) captured_map: HashMap<String, usize>,
    // Borsh schema registry: name → type layout (compile-time only)
    pub(crate) borsh_schemas: HashMap<String, BorshType>,
    // Named function definitions (for compile-time inlining in map/filter/reduce)
    // name → (param_names, body_ast)
    pub(crate) func_defs: HashMap<String, (Vec<String>, LispVal)>,
    // Top-level VALUE defines (define name value) — these compile to 0-param
    // fns but their interpreter semantics is a VALUE binding: a bare Sym
    // reference must CALL the fn (yield the value), not yield a TAG_FNREF
    // (which println renders as nil — the silent-nil gap, GAPS.md)
    pub(crate) value_defines: std::collections::HashSet<String>,
    // Index of the synthesized __h_arr_to_str helper (array → "(e0 e1)" str)
    pub(crate) arr_str_helper: Option<u32>,
    // Index of the synthesized __h_val_eq helper (structural equality)
    pub(crate) val_eq_helper: Option<u32>,
    // Stitched WASM imports (e.g. schnorr_verify_bip340)
    // Vec of (wasm_name, params: Vec<ValType>, results: Vec<ValType>)
    // These become env.* imports that wasm_stitch.py replaces with real implementations
    pub(crate) wasm_imports: Vec<(&'static str, Vec<ValType>, Vec<ValType>)>,
}

impl WasmEmitter {
    pub fn new() -> Self {
        Self {
            locals: HashMap::new(), next_local: 0, free_locals: Vec::new(), local_type_map: Vec::new(), current_func: None, current_param_count: 0, tc_depth: 0, try_stack: Vec::new(),
            while_id: Cell::new(0), funcs: Vec::new(), memory_pages: 64, exports: Vec::new(),
            data_segments: Vec::new(), next_data_offset: 256, host_needed: HashSet::new(),
            gas_local: None, needs_frame: false, heap_ptr: 0, lambda_counter: 0, str_cat_depth: 0, fuzz_mode: false, lambda_info: Vec::new(), captured_map: HashMap::new(), need_outlayer: false, need_wasi_http: false, http_urls: Vec::new(), http_post_urls: Vec::new(), wasi_mode: false, p2_mode: false, no_proc_exit: false, u128h: None, fn_int_annotations: std::collections::HashMap::new(), raw_twins: std::collections::HashMap::new(), borsh_schemas: HashMap::new(), storage_get_count: 0, http_post_call_count: 0, env_get_count: 0,
            func_defs: HashMap::new(),
            wasm_imports: Vec::new(),
            list_ptr_counter: 0,
            value_defines: std::collections::HashSet::new(),
            arr_str_helper: None,
            val_eq_helper: None,
        }
    }

    /// Ensure heap_ptr is initialized: snap to just past the data section end.
    /// Called lazily on first heap allocation so it accounts for all data segments.
    pub(crate) fn ensure_heap_init(&mut self) {
        if self.heap_ptr == 0 {
            let data_end = (self.next_data_offset as u64 + 7) & !7u64;
            // Floor: must be above fixed buffers (BORSH_BUF=36864 + 4096 scratch).
            // BUG FIX (2026-08-29, nostr-gov event-auth investigation): the heap
            // start was pinned at data_end at FIRST USE, which happens mid-compile
            // — literals allocated later in emission landed past the pin, INSIDE
            // the runtime heap region, and runtime allocations (json_get_str
            // buffers, FP bump) corrupted them. Symptoms were position-dependent
            // (function order changed which literals broke): json key patterns
            // matching garbage, str-contains needles misfiring, FP-global
            // corruption traps. Constant 1MiB floor keeps all literals below the
            // heap as long as total literal bytes stay under 1MiB (finish()
            // asserts the invariant; >1MiB of literals = compile error).
            // Constant 256KiB floor: clears every NEAR runtime buffer
            // (INPUT_BUF 16K, BORSH 36864, STDOUT 64K+) with margin, while
            // staying far BELOW the stitched module region ([1MiB, …) — its
            // data sits at 1MiB and its code panics on pointers past its
            // baked-in data_end). Together with heap_bump's unified bump
            // (U-fix 3), literals + slots + runtime heap all live in
            // [floor, 1MiB): ~768KB, allocations are KB-scale.
            // finish() asserts the literal invariant.
            let min_heap = 262_144u64;
            // +64K: the FP scratch region lives at [heap_ptr-64K, heap_ptr)
            // (see compile.rs FP_GLOBAL init). Reserving it BELOW the heap
            // start makes the monotonic runtime heap (and compile-time
            // slots) physically unable to grow into FP scratch — the old
            // layout had FP at heap_ptr+64K, directly in the growth path,
            // and any contract allocating >64K of strings corrupted every
            // FP user (bignum fib died at fib(600) exactly there).
            self.heap_ptr = (std::cmp::max(data_end, min_heap) + 65_536) as u32;
        }
    }

    /// Return current heap_ptr as i32, initializing if needed.
    pub(crate) fn heap_ptr_i32(&mut self) -> i32 {
        self.ensure_heap_init();
        // 2026-08-30 THE layout bug, root-caused: the mem[56] init used
        // self.heap_ptr, but literals/statics placed AFTER the last heap_bump()
        // only advance next_data_offset. Runtime heap started BELOW those
        // statics → the FIRST runtime alloc (str-join output, to-string,
        // storage_get copy, str-cat dst) clobbered literals/delimiter data.
        // Symptoms were position-dependent across modules (vote2 ',',,,,'
        // run_g '10,21,nil' ballot corruption, strCat-only views returning
        // data-segment bytes 'E('). Sync at read time — mem[56] init is now
        // always ≥ every static ever placed.
        if self.next_data_offset > self.heap_ptr {
            self.heap_ptr = self.next_data_offset;
        }
        self.heap_ptr as i32
    }

    /// Allocate (len_local + 7) & ~7 bytes from the runtime heap
    /// (RUNTIME_HEAP_PTR, mem addr 56), leaving the block start in dst_local.
    /// Monotonic — never restored — so results can safely escape the
    /// allocating function. (FP_GLOBAL is save/restored per function in
    /// emit_define, so FP-based results landed at the SAME address in every
    /// callee: sequential calls returning strings clobbered each other's
    /// buffers — the event-auth tag extractors all returned garbage from
    /// overlapping copies. 2026-08-29.) Traps if the bump passes mem_limit.
    /// NOTE: the FP region (heap_ptr + 64K) still exists for tiny scratch
    /// allocs; the runtime heap growing >64K would overlap it — fine for
    /// current contracts (allocations are tens of bytes).
    pub(crate) fn emit_rtheap_alloc(
        &mut self,
        dst_local: u32,
        len_local: u32,
    ) -> Vec<Instruction<'static>> {
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let mem_limit = (self.memory_pages as i64) * 65536;
        vec![
            // dst = mem[56]
            Instruction::I32Const(RUNTIME_HEAP_PTR as i32),
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(dst_local),
            // guard: dst + ((len+7)&~7) ≤ mem_limit
            Instruction::LocalGet(dst_local),
            Instruction::LocalGet(len_local),
            Instruction::I64Const(7),
            Instruction::I64Add,
            Instruction::I64Const(-8i64 as u64 as i64),
            Instruction::I64And,
            Instruction::I64Add,
            Instruction::I64Const(mem_limit),
            Instruction::I64GtU,
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
            // mem[56] = old + aligned(len)
            // (bottom i32 56 = store addr; second i32 56 = addr consumed by the load)
            Instruction::I32Const(RUNTIME_HEAP_PTR as i32),
            Instruction::I32Const(RUNTIME_HEAP_PTR as i32),
            Instruction::I64Load(ma.clone()),
            Instruction::LocalGet(len_local),
            Instruction::I64Const(7),
            Instruction::I64Add,
            Instruction::I64Const(-8i64 as u64 as i64),
            Instruction::I64And,
            Instruction::I64Add,
            Instruction::I64Store(ma),
        ]
    }

    /// Bump heap_ptr by `bytes`, return old position.
    /// In P2 mode, returns None (callers must use heap_bump_runtime for runtime allocation).
    /// In NEAR mode, returns compile-time address.
    pub(crate) fn heap_bump(&mut self, bytes: u32) -> u32 {
        self.ensure_heap_init();
        // U-fix 3b (2026-08-29): bidirectional sync. Slots start at
        // max(heap_ptr, next_data_offset) so they never land inside regions
        // reserved by the ~25 manual scratch sites (needles, str buffers —
        // they bump next_data_offset only). Observed failure: seg@284000
        // (64B literal) inside slot@[283904,284160) → the "pk" value's
        // unescape dst was covered by a literal written at instantiation;
        // every json read after that returned fragment garbage ("ewew=").
        let old = std::cmp::max(self.heap_ptr, self.next_data_offset);
        self.heap_ptr = old + bytes;
        self.next_data_offset = self.heap_ptr;
        if std::env::var("LISPLM_DEBUG_LAYOUT").is_ok() {
            eprintln!("   slot @{old} len {bytes}");
        }
        old
    }

    /// Emit instructions to allocate `bytes` from RUNTIME_HEAP_PTR (addr 56) at runtime.
    /// Returns the allocated address in a local variable.
    /// Used in P2/WASI mode for recursive-safe allocation.
    /// Generates: result = *(56); *(56) = result + bytes (aligned to 8)
    pub(crate) fn heap_bump_runtime(&mut self, bytes: u32, local_name: &str) -> Vec<Instruction<'static>> {
        let tmp = self.local_idx(local_name);
        let ma = wasm_encoder::MemArg {
            offset: 0,
            align: 3,
            memory_index: 0,
        };
        let aligned = ((bytes as i64 + 7) & !7) as i64;
        vec![
            // Load current heap ptr from RUNTIME_HEAP_PTR (addr 56)
            Instruction::I32Const(56),
            Instruction::I64Load(ma.clone()),
            Instruction::LocalSet(tmp),
            // Bump: *(56) = old + aligned_size
            Instruction::I32Const(56),
            Instruction::LocalGet(tmp),
            Instruction::I64Const(aligned),
            Instruction::I64Add,
            Instruction::I64Store(ma),
        ]
    }

    /// Split a URL string into (authority, path_with_query).
    /// e.g. "https://api.example.com/v1/data?q=1" → ("api.example.com", "/v1/data?q=1")
    pub fn split_url(url: &str) -> Option<(String, String)> {
        // Find "://"
        let after_scheme = url.find("://")?;
        let rest = &url[after_scheme + 3..];
        // Find first "/" after scheme — that separates authority from path
        if let Some(slash_pos) = rest.find('/') {
            Some((rest[..slash_pos].to_string(), rest[slash_pos..].to_string()))
        } else {
            Some((rest.to_string(), "/".to_string()))
        }
    }

    fn local_idx(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.locals.get(name) { return i; }
        // Take from the free list, but NEVER reuse an index that is still
        // live in the locals map or outside the allocated range. Guards
        // against free-list bookkeeping bugs (double-push / stale entries),
        // which previously aliased e.g. __sst_v with __str_a and corrupted
        // storage_set operands (host IntegerOverflow).
        let mut taken: Option<u32> = None;
        while let Some(i) = self.free_locals.pop() {
            if i < self.next_local && !self.locals.values().any(|&v| v == i) {
                taken = Some(i);
                break;
            }
        }
        let i = taken.unwrap_or(self.next_local);
        if i == self.next_local {
            self.next_local += 1;
            self.local_type_map.push(ValType::I64);
        }
        self.locals.insert(name.to_string(), i);
        i
    }

    /// Allocate an i32 local (for pointers, lengths, offsets)
    fn local_idx_i32(&mut self, name: &str) -> u32 {
        if let Some(&i) = self.locals.get(name) { return i; }
        let i = self.free_locals.pop().unwrap_or(self.next_local);
        if i == self.next_local {
            self.next_local += 1;
            self.local_type_map.push(ValType::I32);
        } else {
            // Reused slot — pad map to length, then overwrite type
            while self.local_type_map.len() <= i as usize {
                self.local_type_map.push(ValType::I64);
            }
            self.local_type_map[i as usize] = ValType::I32;
        }
        self.locals.insert(name.to_string(), i);
        i
    }

    /// Free a local slot for reuse (call after the local's last use)
    fn free_local(&mut self, name: &str) {
        if let Some(idx) = self.locals.remove(name) {
            self.free_locals.push(idx);
        }
    }

    // ── Tagged value helpers ──
    // Stack effect: [val] → [(val << TAG_BITS) | tag]

    // Stack effect: [val] → [val >> TAG_BITS] (arithmetic shift, preserves sign for Num)

    /// Coerce a tagged value to its numeric payload, or 0 if non-numeric.
    /// Matches F* spec: num_val Num(x) = x, num_val Float(f) = trunc, num_val _ = 0
    /// Stack: [val] → [numeric_value]

    // Stack effect: [val] → [(val << TAG_BITS) | TAG_NUM]

    /// Safe division: checks for zero divisor, traps instead of returning garbage.
    /// Stack: [a, b] → [a/b] or trap if b==0

    /// Safe remainder: checks for zero divisor, traps instead of returning garbage.
    /// Uses euclidean remainder (always non-negative) to match ClosureVM's rem_euclid.
    /// Stack: [a, b] → [euclidean a%b] or trap if b==0

    /// Checked i64 addition: traps on overflow.
    /// Stack: [a, b] → [a+b] or trap

    /// Checked i64 subtraction: traps on overflow.
    /// Stack: [a, b] → [a-b] or trap

    /// Checked i64 multiplication: traps on overflow.
    /// Stack: [a, b] → [a*b] or trap

    // Stack effect: [val] → [(val << TAG_BITS) | TAG_BOOL]

    // Stack effect: [val] → [(val << TAG_BITS) | TAG_STR]

    // Stack effect: [heap_ptr] → [(heap_ptr << TAG_BITS) | TAG_ARRAY]

    // Emit a tagged constant

    /// String equality: compares two tagged strings by content.
    /// Stack: [tagged_str_a, tagged_str_b] → [tagged bool]
    /// Handles: same pointer (fast path), different pointers with byte-by-byte comparison.

    // ─── NEAR-safe byte operation helpers ───
    // I32Load8U / I32Store8 are broken on NEAR (return/store zeros).
    // These helpers use I64Load/I64Store word operations instead.

    /// Store a single byte at a given address.
    /// Stack: [i64 addr, i32 byte_value]  →  []
    /// Reads the 8-byte word at addr, masks out the target byte, ORs in the new byte, stores back.
    /// Uses I64Load to read the 8-byte word, then extracts the target byte.

    /// Runtime word-by-word copy loop: copies `len_local` bytes from `src_local` to `dst_local`.
    /// Uses I64Load/I64Store for full words, then masked tail for remainder.
    /// Locals must already be allocated; `len_local` holds byte count (i64).

    // Stack effect: [val] → [1] if truthy, [0] if falsy
    /// Check if a tagged i64 is truthy. Expects [i64 tagged_val] on stack,
    /// leaves [i64] (0 = falsy, 1 = truthy) on stack.
    /// Uses a local to save the value since i64.eq is binary (consumes the value).

    // Stack effect: [cond_i64] → consumed, then If block opened
    // Emits truthiness check + branch (for if/while/and/or)

    /// Runtime string concatenation: stack has [tagged_a, tagged_b], returns tagged string.
    /// Both args must be tagged values. Converts numbers to their string representation.
    /// Uses runtime heap allocation for the result string.


    /// Process `\xNN` hex escape sequences in a byte slice, returning raw bytes.
    /// Used for binary data in string literals (e.g., ed25519 signatures).
    /// Other escapes (`\n`, `\t`, `\\`, `\"`) are also handled.

    /// Emit WASM instructions for runtime heap allocation.
    /// Reads runtime heap ptr from RUNTIME_HEAP_PTR, bumps by `n_bytes`, writes back.
    /// Leaves the *old* ptr (start of allocated block) on the stack as i64.


    /// Extract (lambda (param) body) → (param_name, body)

    /// Extract (lambda (p1 p2) body) → (vec![p1, p2], body)


    /// Find free variables in an expression (symbols not in the given param set)


    /// Compile (lambda (params) body) → tagged closure value on stack


    // ── Public API ──

    /// Resolve a 1-param lambda arg: inline (fn [x] body) or named function symbol.
    /// Returns (param_name, body_ast).
    pub fn set_fuzz_mode(&mut self, enabled: bool) -> &mut Self {
        self.fuzz_mode = enabled;
        self
    }

    pub fn emit_define(&mut self, name: &str, params: &[String], body: &LispVal) -> Result<(), String> {
        // Store AST for compile-time inlining
        self.func_defs.insert(name.to_string(), (params.to_vec(), body.clone()));
        self.locals.clear(); self.next_local = 0; self.free_locals.clear(); self.local_type_map.clear(); self.needs_frame = false;
        for p in params { self.local_idx(p); }
        self.current_func = Some(name.to_string());
        self.current_param_count = params.len();
        self.while_id.set(0);
        self.scan_host(body);

        // Allocate return-value local
        let ret_local = self.local_idx("__ret");

        // Check if already pre-registered (forward reference)
        let existing_idx = self.funcs.iter().position(|f| f.name == name);
        let placeholder_idx = if let Some(idx) = existing_idx {
            idx
        } else {
            let idx = self.funcs.len();
            self.funcs.push(FuncDef { name: name.into(), param_count: params.len(), local_count: 0, instrs: Vec::new(), local_entries: None,
            custom_type: None,
        });
            idx
        };

        let tc = self.has_tc(body);

        // Raw-i64 twin for fully-annotated int functions: `__raw_<name>` runs
        // the body on untagged payloads (zero tag shifts). The definition
        // below still emits as the generic tagged shim — exports and generic
        // call sites are unchanged; annotated callers + self-recursion hit
        // the twin. Bail on any form outside the int subset → tagged only.
        if let Some(&(np, ret_int)) = self.fn_int_annotations.get(name) {
            if np == params.len() && ret_int {
                // Pre-register the twin so self-recursion resolves during body compile
                let twin_idx = match self.raw_twins.get(name) {
                    Some(i) => *i,
                    None => {
                        let i = self.funcs.len() as u32;
                        self.funcs.push(FuncDef {
                            name: format!("__raw_{}", name),
                            param_count: params.len(),
                            local_count: 0,
                            instrs: Vec::new(),
                            local_entries: None,
                            custom_type: None,
                        });
                        self.raw_twins.insert(name.to_string(), i);
                        i
                    }
                };
                if let Ok((instrs, total)) = self.try_raw_int_fn(name, params, body) {
                    let twin_entries: Vec<(u32, ValType)> = if total > params.len() {
                        vec![((total - params.len()) as u32, ValType::I64)]
                    } else {
                        vec![]
                    };
                    self.funcs[twin_idx as usize] = FuncDef {
                        name: format!("__raw_{}", name),
                        param_count: params.len(),
                        local_count: total,
                        instrs,
                        local_entries: Some(twin_entries),
                        custom_type: None,
                    };
                    eprintln!("⚡ raw twin: {} ({} params, untagged)", name, params.len());
                } else {
                    // Not in the int subset — kill the twin ENTIRELY: drop the
                    // raw_twins entry so call sites route to the tagged fn (they
                    // are emitted after this point), leaving the placeholder
                    // unreferenced for tree_shake. (Leaving the entry live once
                    // routed real calls into an empty body — near-vm PrepareError,
                    // though wasmtime tolerated it. 2026-08-30.)
                    self.raw_twins.remove(name);
                    eprintln!("ℹ️ {} not in raw-int subset — tagged only", name);
                }
            }
        }

        // Build prologue: frame save (NEAR mode + function uses FP-allocating builtins)
        let mut prologue = Vec::new();
        // Depth guard: DISABLED on-chain (2026-08-29) — NEAR's runtime already bounds the
        // native wasm stack ("call stack exhausted" trap), and on-chain measurements show the
        // 4 mem-ops per call here burn ~8% of gas while never firing before the stack does.
        // Mock runs keep their higher ceiling via near-mock's 64MB stack. Re-enable only if a
        // deployment needs an explicit depth cap smaller than the native limit.
        // prologue.extend(self.emit_depth_inc());
        let fp_save = if !self.p2_mode && !self.wasi_mode && self.needs_frame {
            let fp_save = self.local_idx("__fp_save");
            prologue.push(Instruction::GlobalGet(FP_GLOBAL));
            prologue.push(Instruction::LocalSet(fp_save));
            Some(fp_save)
        } else {
            None
        };

        // Build body
        let mut body_instrs = if tc { self.tc_body(body)? } else { self.expr(body)? };

        // Epilogue: save return, restore FP if needed, restore return
        let mut epilogue = Vec::new();
        epilogue.push(Instruction::LocalSet(ret_local));
        // Recursion depth guard: decrement on exit — disabled with the inc (2026-08-29)
        // epilogue.extend(self.emit_depth_dec());
        if let Some(fp_save) = fp_save {
            epilogue.push(Instruction::LocalGet(fp_save));
            epilogue.push(Instruction::GlobalSet(FP_GLOBAL));
        }
        epilogue.push(Instruction::LocalGet(ret_local));

        // Combine: prologue + body + epilogue
        let mut instrs = prologue;
        instrs.append(&mut body_instrs);
        instrs.append(&mut epilogue);

        // Inject gas checks before every Br(0) back-edge and host_call (skip in P2 mode)
        // NEAR protocol meters gas natively — skip injected gas checks
        let instrs = Self::peephole(instrs);

        let total = self.next_local as usize;
        // Build per-local type entries for code section (params are declared in type, extras need declaring)
        let local_entries_vec: Vec<(u32, ValType)> = {
            let param_count = params.len();
            if total > param_count {
                // Group consecutive same-type locals
                let mut entries = Vec::new();
                let mut i = param_count;
                let ltm = &self.local_type_map;
                while i < total {
                    let ty = if i < ltm.len() { ltm[i] } else { ValType::I64 };
                    let mut count = 1u32;
                    while (i + count as usize) < total {
                        let next_ty = if (i + count as usize) < ltm.len() { ltm[i + count as usize] } else { ValType::I64 };
                        if next_ty != ty { break; }
                        count += 1;
                    }
                    entries.push((count, ty));
                    i += count as usize;
                }
                entries
            } else {
                vec![]
            }
        };
        self.current_func = None;
        self.gas_local = None;
        self.funcs[placeholder_idx] = FuncDef {
            name: name.into(),
            param_count: params.len(),
            local_count: total,
            instrs,
            local_entries: Some(local_entries_vec),
            custom_type: None,
        };
        Ok(())
    }

    /// Generate gas check instructions: gas -= 1; if gas <= 0: unreachable

    /// Post-process: inject gas check before every Br back-edge (any depth) and host_call

    /// Peephole optimizer: eliminate redundant instruction patterns
    /// - LocalSet(n) + LocalGet(n) → drop the LocalGet (value already on stack)
    /// - LocalGet(n) + LocalGet(n) → LocalGet(n) + Dup (not available in WASM MVP, so: LocalGet + LocalTee trick)
    /// - I64Const(0) + I64Or → noop (x | 0 = x)
    /// - I64Const(0) + I64Add → noop (x + 0 = x)

    pub fn set_memory(&mut self, p: u32) {
        // The runtime heap (RUNTIME_HEAP_PTR) starts at max(data_end, 256KiB)
        // = the page-4 boundary. A module with exactly 4 pages traps on the
        // FIRST runtime alloc (dst+len > mem_limit guard → Unreachable).
        // Floor at 6 pages: heap floor + 128KiB working headroom. (Corpus
        // safe.lisp hit this 2026-08-30: `(memory 4)` + str-join slots.)
        const MIN_PAGES: u32 = 6;
        if p < MIN_PAGES {
            eprintln!(
                "⚠ (memory {}) raised to {} pages — runtime heap starts at 256KiB (page 4)",
                p, MIN_PAGES
            );
            self.memory_pages = MIN_PAGES;
        } else {
            self.memory_pages = p;
        }
    }
    pub fn add_export(&mut self, fn_: &str, en: &str, is_view: bool) {
        // Replace existing export with same name to avoid duplicate export errors
        if let Some(pos) = self.exports.iter().position(|(_, e, _)| e == en) {
            self.exports[pos] = (fn_.into(), en.into(), is_view);
        } else {
            self.exports.push((fn_.into(), en.into(), is_view));
        }
    }

    // ── Tail-call ──

    fn tc_body(&mut self, body: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        self.tc_depth = 0;
        let inner = self.tc(body)?;
        let mut v = Vec::with_capacity(inner.len() + 5);
        v.push(Instruction::Block(BlockType::Result(ValType::I64)));
        v.push(Instruction::Loop(BlockType::Empty));
        v.extend(inner);
        v.push(Instruction::End);
        // Loop should never exit normally - inner must branch with Br(0) to loop or Br(1) to exit block
        v.push(Instruction::Unreachable);
        v.push(Instruction::End);
        Ok(v)
    }

    /// Replace (recur val...) inside loop body with (__loop_N val...) — direct self-call for TCO
    fn replace_recur(&mut self, expr: &mut LispVal, loop_name: &str, _var_names: &[String]) {
        match expr {
            LispVal::List(items) => {
                if let Some(LispVal::Sym(head)) = items.first() {
                    if head == "recur" {
                        // (recur val1 val2 ...) → (__loop_N val1 val2 ...)
                        let mut call_items = vec![LispVal::Sym(loop_name.into())];
                        for val in items[1..].iter() {
                            call_items.push(val.clone());
                        }
                        *expr = LispVal::List(call_items);
                        return;
                    }
                    if head == "loop" {
                        // Nested loop: its (recur ...) forms belong to the inner loop,
                        // which gets lifted separately during emit_define. Do not descend.
                        return;
                    }
                }
                // Recurse into children
                for item in items.iter_mut() {
                    self.replace_recur(item, loop_name, _var_names);
                }
            }
            _ => {}
        }
    }

    /// After replace_recur, patch (__loop_N val...) calls to also pass free vars through
    fn patch_recur_with_free_vars(&self, expr: &mut LispVal, loop_name: &str, free_vars: &[String]) {
        match expr {
            LispVal::List(items) => {
                if let Some(LispVal::Sym(head)) = items.first() {
                    if head == loop_name {
                        // This is a (__loop_N val...) call — append free var symbols
                        for fv in free_vars {
                            items.push(LispVal::Sym(fv.clone()));
                        }
                        return;
                    }
                }
                for item in items.iter_mut() {
                    self.patch_recur_with_free_vars(item, loop_name, free_vars);
                }
            }
            _ => {}
        }
    }

    fn tc(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let LispVal::List(items) = e else { return self.expr(e) };
        if items.is_empty() { return self.expr(e) }
        let LispVal::Sym(op) = &items[0] else { return self.expr(e) };
        let a = &items[1..];
        match op.as_str() {
            "if" => self.tc_if(a),
            "begin" | "progn" => {
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() { v.extend(self.expr(x)?); if i < a.len()-1 { v.push(Instruction::Drop); } }
                // Exit block: directly in TC loop, Br(1) exits block (result i64)
                v.push(Instruction::Br(1));
                Ok(v)
            }
            "let" => self.tc_let(a),
            "loop" => {
                // loop desugars to let + define + call — handled by expr path
                self.expr(e)
            }
            _ if Some(op.as_str()) == self.current_func.as_deref() && a.len() == self.current_param_count => {
                // Evaluate ALL args into distinct temps FIRST, then copy into param
                // locals — a later arg may reference a param local that an earlier
                // arg already overwrote (e.g. (recur (+ i 1) (+ s i)) must see the
                // OLD i when evaluating (+ s i)).
                let n = a.len();
                let tmps: Vec<u32> = (0..n).map(|i| self.local_idx(&format!("__tcarg{}", i))).collect();
                let mut v = Vec::new();
                for (i, x) in a.iter().enumerate() {
                    v.extend(self.expr(x)?);
                    v.push(Instruction::LocalSet(tmps[i]));
                }
                for (i, t) in tmps.iter().enumerate() {
                    v.push(Instruction::LocalGet(*t));
                    v.push(Instruction::LocalSet(i as u32));
                }
                v.push(Instruction::Br(0));
                Ok(v)
            }
            // Any other expression inside TC loop: evaluate and exit block via Br(1)
            _ => { let mut v = self.expr(e)?; v.push(Instruction::Br(1)); Ok(v) }
        }
    }

    fn tc_let(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = Vec::new();
        if let LispVal::List(bs) = &a[0] {
            for b in bs { if let LispVal::List(p) = b { if p.len()==2 { if let LispVal::Sym(n) = &p[0] {
                let idx = self.local_idx(n);
                v.extend(self.expr(&p[1])?); v.push(Instruction::LocalSet(idx));
            }}}}
        }
        // Implicit begin in let body
        for (i, x) in a[1..].iter().enumerate() {
            if i < a.len() - 2 { v.extend(self.expr(x)?); v.push(Instruction::Drop); }
            else { v.extend(self.tc(x)?); }
        }
        Ok(v)
    }

    fn tc_if(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.emit_cond_branch());
        let t = &a[1]; let default = LispVal::Num(0);
        let e = if a.len()>2 { &a[2] } else { &default };
        match (self.is_self(t), self.is_self(e)) {
            (true, true) => {
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.self_sets(t)?); v.push(Instruction::Br(0));
                v.push(Instruction::Else);
                v.extend(self.self_sets(e)?); v.push(Instruction::Br(0));
                v.push(Instruction::End);
            }
            (true, false) => {
                v.push(Instruction::I32Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.expr(e)?); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                v.extend(self.self_sets(t)?); v.push(Instruction::Br(0)); // loop
            }
            (false, true) => {
                v.push(Instruction::If(BlockType::Empty));
                v.extend(self.expr(t)?); v.push(Instruction::Br(2)); // exit block
                v.push(Instruction::End);
                v.extend(self.self_sets(e)?); v.push(Instruction::Br(0)); // loop
            }
            (false, false) => {
                // Inside TC loop: must either Br(0) loop or Br(2) exit — never fall through
                v.push(Instruction::If(BlockType::Empty));
                // Check if then-branch is a self-call
                if self.is_self(t) {
                    v.extend(self.self_sets(t)?); v.push(Instruction::Br(0));
                } else {
                    v.extend(self.expr(t)?); v.push(Instruction::Br(2)); // exit block with value
                }
                v.push(Instruction::Else);
                if self.is_self(e) {
                    v.extend(self.self_sets(e)?); v.push(Instruction::Br(0));
                } else {
                    v.extend(self.expr(e)?); v.push(Instruction::Br(2)); // exit block with value
                }
                v.push(Instruction::End);
            }
        }
        Ok(v)
    }

    fn self_sets(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        let LispVal::List(items) = e else { return Ok(Vec::new()) };
        // Args into temps first (see TC arm note) — avoid param-local aliasing.
        let n = items.len().saturating_sub(1).min(self.current_param_count);
        let tmps: Vec<u32> = (0..n).map(|i| self.local_idx(&format!("__tcarg{}", i))).collect();
        let mut v = Vec::new();
        for (i, a) in items[1..].iter().enumerate().take(n) {
            v.extend(self.expr(a)?);
            v.push(Instruction::LocalSet(tmps[i]));
        }
        for (i, t) in tmps.iter().enumerate() {
            v.push(Instruction::LocalGet(*t));
            v.push(Instruction::LocalSet(i as u32));
        }
        Ok(v)
    }

    // ── Expression ──

    fn expr(&mut self, e: &LispVal) -> Result<Vec<Instruction<'static>>, String> {
        match e {
            LispVal::Num(n) => self.emit_tagged_const(*n as i64, TAG_NUM),
            LispVal::Bool(true) => self.emit_tagged_const(1, TAG_BOOL),
            LispVal::Bool(false) => self.emit_tagged_const(0, TAG_BOOL),
            LispVal::Nil => Ok(vec![Instruction::I64Const(TAG_NIL)]),
            LispVal::Sym(n) => {
                // Check if it's a captured variable from enclosing lambda
                if let Some(&offset) = self.captured_map.get(n) {
                    // Load from closure: closure_ptr is local 0, load at offset*8
                    let ma = wasm_encoder::MemArg { offset: (offset as u64 * 8), align: 3, memory_index: 0 };
                    return Ok(vec![
                        Instruction::LocalGet(0), // closure_ptr
                        Instruction::I32WrapI64,
                        Instruction::I64Load(ma),
                    ]);
                }
                if let Some(&i) = self.locals.get(n) {
                    Ok(vec![Instruction::LocalGet(i)])
                } else if self.value_defines.contains(n) {
                    // Top-level value define: interpreter semantics is a VALUE
                    // binding — call the synthesized 0-param fn to get the value.
                    // (Was TAG_FNREF → println rendered it as nil, silently.)
                    let pos = self.funcs.iter().position(|f| &f.name == n)
                        .ok_or_else(|| format!("internal: value define '{}' not registered", n))?;
                    Ok(vec![Instruction::Call(USER_BASE | pos as u32)])
                } else if let Some(pos) = self.funcs.iter().position(|func| &func.name == n) {
                    self.emit_tagged_const(pos as i64, TAG_FNREF)
                } else {
                    // Check if it's a bare 0-arg host function call (e.g. near/block_index used as value)
                    let name = n.as_str();
                    let host_idx = match name {
                        "near/block_index" => Some(8),
                        "near/block_timestamp" => Some(9),
                        "near/epoch_height" => Some(10),
                        "near/storage_usage" => Some(11),
                        "near/prepaid_gas" => Some(15),
                        "near/used_gas" => Some(16),
                        "near/account_balance" => Some(12),
                        "near/account_balance_high" => Some(12),
                        "near/account_locked_balance" => Some(13),
                        "near/account_locked_balance_high" => Some(13),
                        "near/attached_deposit" => Some(14),
                        _ => None,
                    };
                    if let Some(idx) = host_idx {
                        // u128 functions: use read_u128_low, simple 0-arg: use host_call + tag
                        match idx {
                            12 | 13 | 14 => {
                                self.need_host(idx);
                                self.need_host(0); // read_register
                                self.need_host(1); // register_len
                                self.read_u128_low(idx)
                            }
                            _ => {
                                self.need_host(idx);
                                let mut v = vec![Self::host_call(idx)];
                                v.extend(self.emit_tag_num());
                                Ok(v)
                            }
                        }
                    } else {
                        Err(format!("undefined variable '{}' — not found in scope. Did you mean to define it?", n))
                    }
                }
            }
            LispVal::Str(s) => {
                // Process \xNN hex escapes to raw bytes (e.g., ed25519 signatures).
                // The tokenizer preserves \x as literal chars, so we decode them here.
                let raw = Self::process_hex_escapes(s.as_bytes());
                let off = self.alloc_data(&raw) as u64;
                let encoded = (off | ((raw.len() as u64) << 32)) as i64;
                let mut v = vec![Instruction::I64Const(encoded)];
                v.extend(self.emit_tag_str());
                Ok(v)
            }
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(op) = &items[0] { 
                    self.call(op, &items[1..]) 
                } else {
                    // Dynamic call: ((expr) args...) — callee is not a symbol
                    self.emit_dynamic_call(&items[0], &items[1..])
                }
            }
            _ => Err(format!("compile error: unsupported expression form: {:?}", e)),
        }
    }


}

// ── Compile helpers ──


/// Compile a Lisp program for fuzz testing.
/// Creates a "run" export that calls the last defined function and stores
/// the **tagged** return value at TEMP_MEM (offset 64) for the harness to read.
/// Does NOT call value_return (this is a test, not a NEAR contract).


// ── Borsh schema parsing helpers ──

/// Parse a BorshType from a Lisp symbol or list form.
/// Symbols: u8, u32, u64, i64, u128, f64, bool, string, bytes
/// Lists: (Vec T), (Option T), or struct-like ((field1 type1) (field2 type2) ...)

/// Parse struct fields from ((name type) ...) pairs
/// Parse implicit enum variant items: (VariantName (field1 type1) ...) or VariantName (unit)


/// Process (borsh-schema (Name ((field1 type1) ...)) ...) top-level forms.
/// Registers each type in the emitter's borsh_schemas map.

/// Compile pre-parsed LispVal expressions to NEAR WASM

/// Compile pre-parsed LispVal expressions to NEAR WAT


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_near_input_compiles() {
        let src = r#"(memory 1)
(define (query)
  (let ((p (near/promise_create "wrap.near" "ft_balance_of" (near/input) 0 0)))
    (near/promise_result 0)))
(export "query" query true)"#;
        let wat = compile_near_to_wat(src).expect("compile near/input");
        assert!(wat.contains("input"));
        assert!(wat.contains("promise_create"));
        eprintln!("WAT:\n{}", wat);
    }

    #[test]
    fn test_near_counter() {
        let src = r#"
(memory 1)
(define (get_counter) (near/load "c"))
(define (set_counter val) (near/store "c" val))
(define (new) (set_counter 0))
(define (increment) (set_counter (+ (get_counter) 1)))
(define (get) (near/return (get_counter)))
(export "new" new false)
(export "increment" increment false)
(export "get" get true)
"#;
        let wasm = compile_near(src).unwrap();
        assert!(!wasm.is_empty());
        // Verify it's valid WASM by roundtripping through wasmprinter
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Counter WAT:\n{}", wat);
    }

    #[test]
    fn test_pure_fib() {
        let src = "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))";
        let wasm = compile_pure(src).unwrap();
        assert!(!wasm.is_empty());
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Fib WAT:\n{}", wat);
    }

    #[test]
    fn test_tc_count() {
        // Tail-recursive count — must use loop, not recursive call
        let src = "(define (count n) (if (= n 0) 0 (count (- n 1))))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Count WAT:\n{}", wat);
        // The body should contain a loop (TC optimization) and no call to self
        assert!(wat.contains("loop"), "TC should generate a loop");
        // Count "call 0" occurrences — should be 0 inside the function body
        // (only the wrapper calls func 0)
        let lines: Vec<&str> = wat.lines().collect();
        let func_start = lines.iter().position(|l| l.contains("(func (;0;)")).unwrap();
        let func_end = lines.iter().rposition(|l| l.contains("(func (;1;)")).unwrap();
        let func_body = &lines[func_start..func_end];
        let call_count = func_body.iter().filter(|l| l.trim().starts_with("call")).count();
        assert_eq!(call_count, 0, "TC body should not contain any call instructions, found {}", call_count);
    }

    #[test]
    fn test_tc_nested_if() {
        // TC with nested if where neither branch is directly a self-call
        let src = "(define (f n) (if (= n 0) 0 (if (= n 1) 1 (f (- n 2)))))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Nested IF WAT:\n{}", wat);
        assert!(wat.contains("loop"), "TC should generate a loop");
    }

    #[test]
    fn test_implicit_begin_let() {
        let src = "(define (f x) (let ((y 1)) (set! y 2) y))";
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Implicit begin let WAT:\n{}", wat);
        // Should compile without error — multiple body exprs in let
    }

    #[test]
    fn test_implicit_begin_define() {
        let src = r#"(define (f x) (set! x (+ x 1)) x)"#;
        let wasm = compile_pure(src).unwrap();
        let wat = wasmprinter::print_bytes(&wasm).unwrap();
        println!("Implicit begin define WAT:\n{}", wat);
    }
}

#[cfg(test)]
mod ycomb_test {
    use super::*;
    #[test]
    fn test_self_call_non_tc() {
        // f calls itself in non-tail position via (me me (- x 1))
        let src = r#"
(define (f me x)
  (if (<= x 0) 1
    (* x (me me (- x 1)))))
(f f 5)
"#;
        let wasm = compile_pure(src).expect("should compile");
        assert!(wasm.len() > 0, "should produce WASM");
    }
}

#[cfg(test)]
mod ycomb_run_test {
    use super::*;
    #[test]
    fn test_ycomb_factorial() {
        // Factorial via Y-combinator: f(f,5) = 120
        let src = "(define (f me x) (if (<= x 0) 1 (* x (me me (- x 1))))) (define (main) (f f 5))";
        let _wat = compile_pure_to_wat(src).expect("compile to wat");
        // The key test: (me me args...) compiles without error
        // Runtime verification requires NEAR host stubs (<= uses read_register for i64 comparison)
    }
}

#[cfg(test)]
mod ycomb_wat_test {
    use super::*;
    #[test]
    fn print_ycomb_wat() {
        let src = "(define (f me x) (if (<= x 0) 1 (* x (me me (- x 1))))) (f f 5)";
        let wat = compile_pure_to_wat(src).expect("compile to wat");
        eprintln!("YCOMB WAT:\n{}", wat);
    }
}

#[cfg(test)]
mod ycomb_debug {
    use super::*;
    #[test]
    fn debug_ycomb() {
        let src = "(define (f me x) (if (<= x 0) 1 (* x (me me (- x 1))))) (define (main) (f f 5))";
        let result = compile_near_to_wat(src);
        eprintln!("RESULT: {:?}", result);
    }
}

#[cfg(test)]
mod near_ycomb_test {
    use super::*;
    #[test]
    fn test_near_ycomb() {
        let src = "(define (f me x) (if (<= x 0) 1 (* x (me me (- x 1))))) (define (main) (f f 5))";
        let r = compile_near(src);
        eprintln!("NEAR: {:?}", r.as_ref().map(|w| w.len()));
        let p = compile_pure(src);
        eprintln!("PURE: {:?}", p.as_ref().map(|w| w.len()));
        assert!(r.is_ok(), "near compile failed: {:?}", r);
    }
}

#[cfg(test)]
mod near_ycomb_wat {
    use super::*;
    #[test]
    fn print_wat() {
        let src = "(define (f me x) (if (<= x 0) 1 (* x (me me (- x 1))))) (define (main) (f f 5)) (export \"main\" main true)";
        let wat = compile_near_to_wat(src).expect("wat");
        eprintln!("{}", wat);
    }
}

#[cfg(test)]
mod lambda_test {
    use super::*;
    #[test]
    fn test_simple_lambda() {
        let src = "(define (test) ((lambda (x) (* x x)) 7))";
        let result = compile_pure(src);
        eprintln!("RESULT: {:?}", result.as_ref().map(|w| w.len()));
        if let Err(ref e) = result { eprintln!("ERROR: {}", e); }
    }
}

#[cfg(test)]
mod square_wat {
    use super::*;
    #[test]
    fn print_wat() {
        let src = r#"(define (square) ((lambda (x) (* x x)) 7)) (export "square" square true)"#;
        let wat = compile_near_to_wat(src).expect("wat");
        eprintln!("{}", wat);
    }
}

/// Resolve `(module name "path")` directives — text-level #include


#[cfg(test)]
mod json_auto_test {
    use super::*;

    #[test]
    fn test_json_get_auto_int() {
        let src = r#"(define (run) (near/json_get_int "price")) (export "run" run true)"#;
        let wat = compile_near_to_wat(src).expect("wat");
        assert!(wat.contains("call"), "should emit host calls");
        eprintln!("INT WAT:\n{}", wat);
    }

    #[test]
    fn test_json_get_auto_compiles() {
        let src = r#"(define (run) (json/get "price")) (export "run" run true)"#;
        let result = compile_near(src);
        assert!(result.is_ok(), "json/get should compile: {:?}", result);
        let wasm = result.unwrap();
        assert!(!wasm.is_empty());
    }

    #[test]
    fn test_json_get_auto_str() {
        let src = r#"(define (run) (json/get "name")) (export "run" run true)"#;
        let result = compile_near(src);
        assert!(result.is_ok(), "json/get str should compile: {:?}", result);
    }

    #[test]
    fn test_json_get_auto_missing_key() {
        // Should still compile — key not found returns 0 at runtime
        let src = r#"(define (run) (json/get "nonexistent")) (export "run" run true)"#;
        let result = compile_near(src);
        assert!(result.is_ok(), "json/get missing key should compile: {:?}", result);
    }
}
