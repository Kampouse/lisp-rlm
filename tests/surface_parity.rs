//! Surface-parity inventory: builtin names dispatched by the wasm emitter
//! (src/wasm_emit/call*.rs) vs names the interpreter accepts
//! (helpers::BUILTIN_NAMES ∪ bytecode::eval_near_builtin_match).
//!
//! The wasm dispatch surface is extracted AT TEST TIME by scanning the
//! emitter sources, so adding a builtin to a call_* file without porting
//! it to the interpreter (or documenting it below) fails this test with a
//! printed diff — the T6 drift class must not silently regrow.
//! (GAPS.md ~line 217; str-cat precedent, commit dd0285d.)

use lisp_rlm_wasm::bytecode::eval_near_builtin_match;
use lisp_rlm_wasm::helpers::BUILTIN_NAMES;
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

/// Special forms handled by BOTH compilers before builtin dispatch — not
/// builtins, parity not applicable.
const SPECIAL_FORMS: &[&str] = &[
    "and", "begin", "cond", "default", "for", "if", "lambda", "let", "loop", "not", "or",
    "quote", "recur", "set!", "try", "while",
];

/// Names extracted from the emitter that are deliberately NOT ported to the
/// interpreter, each with a reason. Genuinely unimplementable-in-interp or
/// wasm-harness-only. Keep this table honest: new drift goes here ONLY with
/// a real reason, otherwise port the builtin.
const WASM_ONLY_DOCUMENTED: &[(&str, &str)] = &[
    // ── raw linear-memory / C-ABI internals: interp has no linear memory ──
    ("mem-get", "raw linear-memory read (wasm layout)"),
    ("mem-get8", "raw linear-memory read (wasm layout)"),
    ("mem-set!", "raw linear-memory write (wasm layout)"),
    ("mem-set8!", "raw linear-memory write (wasm layout)"),
    ("ptr-add", "raw pointer arithmetic (wasm layout)"),
    ("buf-alloc", "bump allocator control (wasm heap layout)"),
    ("buf-get", "byte-buffer access (wasm heap layout)"),
    ("buf-set!", "byte-buffer access (wasm heap layout)"),
    ("limb-add", "secp256k1 limb array ops (wasm memory)"),
    ("limb-sub", "secp256k1 limb array ops (wasm memory)"),
    ("limb-mul", "secp256k1 limb array ops (wasm memory)"),
    ("limb-cmp", "secp256k1 limb array ops (wasm memory)"),
    ("limb-get", "secp256k1 limb array ops (wasm memory)"),
    ("limb-set!", "secp256k1 limb array ops (wasm memory)"),
    ("bit_get", "bitfield word op over raw addresses (wasm memory)"),
    ("bit_set", "bitfield word op over raw addresses (wasm memory)"),
    ("bit_clr", "bitfield word op over raw addresses (wasm memory)"),
    ("fp64/set", "fixed-point slot op (wasm memory)"),
    ("fp64/get_frac", "fixed-point slot op (wasm memory)"),
    ("fp64/get_int", "fixed-point slot op (wasm memory)"),
    ("fp64/is_zero", "fixed-point slot op (wasm memory)"),
    ("str_len", "internal underscore twin of str-length (C-ABI style)"),
    ("str_cat", "internal underscore twin of str-cat (C-ABI style)"),
    ("str_eq", "internal underscore twin of str= (C-ABI style)"),
    ("str-ptr", "raw string pointer/len unpack (wasm layout)"),
    ("strlcat", "libc-style bounded concat on raw buffers (wasm memory)"),
    ("strlcpy", "libc-style bounded copy on raw buffers (wasm memory)"),
    ("clz", "integer intrinsic emitted inline (no interp slot op)"),
    ("ctz", "integer intrinsic emitted inline (no interp slot op)"),
    ("popcnt", "integer intrinsic emitted inline (no interp slot op)"),
    ("byte-at", "raw buffer byte read (wasm memory)"),
    ("bytes-to-u32", "raw buffer word read (wasm memory)"),
    ("u32-to-bytes", "raw buffer word write (wasm memory)"),
    ("sha256_hash", "C-ABI sha256 into raw buffer (wasm memory)"),
    ("sha256-hash", "C-ABI sha256 into raw buffer (wasm memory)"),
    ("schnorr_verify_bip340", "C-ABI verify over raw buffers (wasm memory)"),
    ("str_to_int", "C-ABI parse from raw buffer (wasm memory)"),
    ("str-slice", "C-ABI slice into raw buffer (wasm memory)"),
    ("arr_new", "array-in-linear-memory family (wasm layout)"),
    ("arr_get", "array-in-linear-memory family (wasm layout)"),
    ("arr_set", "array-in-linear-memory family (wasm layout)"),
    ("arr_len", "array-in-linear-memory family (wasm layout)"),
    ("arr_push", "array-in-linear-memory family (wasm layout)"),
    ("arr_find", "array-in-linear-memory family (wasm layout)"),
    ("arr_sort", "array-in-linear-memory family (wasm layout)"),
    ("array", "array-in-linear-memory family (wasm layout)"),
    ("vec-length", "array-in-linear-memory family (wasm layout)"),
    ("vec-push", "array-in-linear-memory family (wasm layout)"),
    ("vec-set!", "array-in-linear-memory family (wasm layout)"),
    ("map-into", "array-in-linear-memory family (wasm layout)"),
    ("hof/map", "fused HOF over wasm arrays (wasm layout)"),
    ("hof/filter", "fused HOF over wasm arrays (wasm layout)"),
    ("hof/reduce", "fused HOF over wasm arrays (wasm layout)"),
    ("filter-count", "fused HOF over wasm arrays (wasm layout)"),
    ("range-reduce", "fused HOF over wasm arrays (wasm layout)"),
    ("u128/load_storage", "raw-limb storage read (wasm memory)"),
    ("u128/store_storage", "raw-limb storage write (wasm memory)"),
    ("u128/is_zero", "raw-limb zero test (wasm memory)"),
    // ── Q64.64 fixed-point family: raw u64 bit patterns, no interp twin ──
    ("fp/div", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/from_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/mul", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/one", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/sqrt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/to_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/add", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/div", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/lt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/mul", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/set_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/sqrt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/sub", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    // ── outlayer / agent harness: host services that only exist under wasm runtimes ──
    ("outlayer/call", "outlayer host service (wasm harness only)"),
    ("outlayer/context", "outlayer host service (wasm harness only)"),
    ("outlayer/http-post", "outlayer host service (wasm harness only)"),
    ("outlayer/json-get", "outlayer host service (wasm harness only)"),
    ("outlayer/raw", "outlayer host service (wasm harness only)"),
    ("outlayer/rpc-call", "outlayer host service (wasm harness only)"),
    ("outlayer/send-telegram", "outlayer host service (wasm harness only)"),
    ("outlayer/sleep-ms", "outlayer host service (wasm harness only)"),
    ("outlayer/status", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-get", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-has", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-set", "outlayer host service (wasm harness only)"),
    ("outlayer/str-concat", "outlayer host service (wasm harness only)"),
    ("outlayer/transfer", "outlayer host service (wasm harness only)"),
    ("outlayer/view", "outlayer host service (wasm harness only)"),
    ("outlayer/web-search", "outlayer host service (wasm harness only)"),
    ("rpc-call", "outlayer host service (wasm harness only)"),
    ("web-search", "outlayer host service (wasm harness only)"),
    ("send-telegram", "outlayer host service (wasm harness only)"),
    ("sleep-ms", "outlayer host service (wasm harness only)"),
    ("ai-chat", "outlayer host service (wasm harness only)"),
    ("http-post-dynamic", "wasm http host import (harness only)"),
    ("env/predecessor", "wasm-run env probe (harness only)"),
    ("env/signer", "wasm-run env probe (harness only)"),
    // ── mock-storage harness builtins: test-harness state, not contract surface ──
    ("storage-get", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-set", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-has", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-delete", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-clear-all", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-increment", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-decrement", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-list-keys", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-set-if-absent", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-set-if-equals", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-get-worker", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-get-worker-from-project", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-set-worker", "mock-storage harness state (lisp-run keeps its own)"),
    ("storage-set-worker-public", "mock-storage harness state (lisp-run keeps its own)"),
    // ── defi/uniswap math over raw u64 slots: wasm-only domain for now ──
    ("liq_amount0", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount0_64", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount1", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount1_64", "defi raw-u64 math (wasm-only domain)"),
    ("price_to_tick", "defi raw-u64 math (wasm-only domain)"),
    ("price64_to_tick", "defi raw-u64 math (wasm-only domain)"),
    ("tick_to_price", "defi raw-u64 math (wasm-only domain)"),
    ("tick_to_price64", "defi raw-u64 math (wasm-only domain)"),
    ("tick_to_sqrtPrice64", "defi raw-u64 math (wasm-only domain)"),
    // ── bigint family: address-based limbs, shadowed legacy ──
    ("bigint-add", "address-based limb family (legacy; string-based u128/* is the spec)"),
    ("bigint-div", "address-based limb family (legacy; string-based u128/* is the spec)"),
    ("bigint-mul", "address-based limb family (legacy; string-based u128/* is the spec)"),
    ("bigint-from-str", "address-based limb family (legacy; string-based u128/* is the spec)"),
    ("bigint-to-str", "address-based limb family (legacy; string-based u128/* is the spec)"),
    // ── near/* wasm-only extras (host-register/register-index based) ──
    ("near/kload", "host-register based (interp near_storage keys differ)"),
    ("near/kstore", "host-register based (interp near_storage keys differ)"),
    ("near/load-amount", "u128 two-register read (wasm host ABI)"),
    ("near/store-deposit", "u128 two-register write (wasm host ABI)"),
    ("near/attached_deposit_u128", "u128 two-register read (wasm host ABI)"),
    ("near/call-signed", "wasm harness promise helper"),
    ("near/transfer-signed", "wasm harness promise helper"),
    ("near/batch-add-key", "wasm harness promise helper"),
    ("near/batch-call", "wasm harness promise helper"),
    ("near/batch-create-account", "wasm harness promise helper"),
    ("near/batch-deploy", "wasm harness promise helper"),
    ("near/batch-transfer", "wasm harness promise helper"),
    ("near/bls12381_g1_multiexp", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_g2_multiexp", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_map_fp2_to_g2", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_map_fp_to_g1", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_p1_decompress", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_p2_decompress", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_p2_sum", "alt_bn128/bls host ABI over raw buffers"),
    ("near/bls12381_pairing_check", "alt_bn128/bls host ABI over raw buffers"),
    ("near/json_get_arr", "wasm JSON register ABI"),
    ("near/json_get_u128", "wasm JSON register ABI"),
    // ── codec / encoding over raw byte buffers ──
    ("borsh-serialize", "borsh over raw byte buffers (wasm memory)"),
    ("borsh-deserialize", "borsh over raw byte buffers (wasm memory)"),
    ("base58-decode", "codec into raw buffer (wasm memory)"),
    ("base64-encode", "codec into raw buffer (wasm memory)"),
    ("base64url-decode", "codec into raw buffer (wasm memory)"),
    ("hex-decode", "codec into raw buffer (wasm memory)"),
    ("str", "internal stringify helper head (wasm)"),
    // ── wasm test-harness assertions (lisp-run has its own harness) ──
    ("assert-equal", "wasm test-harness assertion (harness only)"),
    ("assert-true", "wasm test-harness assertion (harness only)"),
    ("assert-raises", "wasm test-harness assertion (harness only)"),
    // ── C-ABI intrinsics over raw linear memory ──
    ("itoa", "C-ABI int-to-string into raw buffer (wasm memory)"),
    ("malloc", "linear-memory bump allocator (wasm layout)"),
    ("load_i64", "raw linear-memory tagged load (wasm layout)"),
    ("store_i64", "raw linear-memory tagged store (wasm layout)"),
    ("abort", "wasm harness abort (near/panic is the interp twin)"),
    ("assert", "wasm assert helper (interp: near/assert)"),
    ("json-bytes-to-str", "wasm JSON register ABI"),
    ("json-extract", "wasm JSON register ABI"),
    ("json-get-float", "wasm JSON register ABI"),
    ("json-get-str", "wasm JSON register ABI"),
    ("json-quote", "wasm JSON register ABI"),
    ("json-return", "wasm JSON register ABI"),
];

fn extract_wasm_ops() -> BTreeSet<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/wasm_emit");
    let arm = Regex::new(r#"^\s*(?:\|\s*)?"([A-Za-z0-9!$%&*+\-./:<=>?^_]+)"(\s+if\b|\s*(?:=>|\||,))"#).unwrap();
    let cmp = Regex::new(r#"op\s*(?:==|!=)\s*"([A-Za-z0-9!$%&*+\-./:<=>?^_]+)""#).unwrap();
    let mut names = BTreeSet::new();
    let entries = fs::read_dir(&dir).expect("read src/wasm_emit");
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("call"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    for path in files {
        let src = fs::read_to_string(&path).expect("read emitter file");
        for line in src.lines() {
            for m in arm.captures_iter(line) {
                names.insert(m[1].to_string());
            }
            for m in cmp.captures_iter(line) {
                names.insert(m[1].to_string());
            }
        }
    }
    names.retain(|n| !n.starts_with("__"));
    names
}

fn interp_accept_set() -> BTreeSet<String> {
    let mut s: BTreeSet<String> = BUILTIN_NAMES.iter().map(|n| n.to_string()).collect();
    // eval_near_builtin_match covers the near/* surface; probe it directly so
    // the two gates (compile gate + runtime dispatch) stay in lockstep.
    for probe in near_probe_list() {
        if eval_near_builtin_match(&probe) {
            s.insert(probe);
        }
    }
    s
}

/// All near/* and storage-ish literals we know the matcher might accept —
/// derived from the emitter's near names plus the legacy bare names.
fn near_probe_list() -> Vec<String> {
    let mut probes: Vec<String> = extract_wasm_ops()
        .into_iter()
        .filter(|n| n.contains('/') || n.contains('-') || n.contains('_'))
        .collect();
    // legacy bare storage/context names (interp compat surface)
    for p in [
        "storage-write", "storage_read", "storage-remove", "storage-has-key", "block-height",
        "block_timestamp", "signer-account-id", "predecessor_account_id", "current-account-id",
        "attached_deposit", "account-balance", "log-utf8", "log", "near-config", "near-reset",
        "near-promises", "near-batch-actions", "near-returned-promise", "near-register",
        "near-register-source", "near-contracts",
    ] {
        probes.push(p.to_string());
    }
    probes
}

#[test]
fn surface_parity() {
    let wasm_ops = extract_wasm_ops();
    let interp = interp_accept_set();
    let special: BTreeSet<&str> = SPECIAL_FORMS.iter().copied().collect();
    let documented: BTreeSet<&str> = WASM_ONLY_DOCUMENTED.iter().map(|(n, _)| *n).collect();

    // sanity: exclusion tables must not contain phantom entries
    for (name, _) in WASM_ONLY_DOCUMENTED {
        assert!(
            wasm_ops.contains(*name),
            "WASM_ONLY_DOCUMENTED entry '{name}' no longer exists in the emitter — prune it"
        );
    }

    let missing: Vec<&str> = wasm_ops
        .iter()
        .map(|n| n.as_str())
        .filter(|n| !interp.contains(*n) && !special.contains(n) && !documented.contains(n))
        .collect();

    assert!(
        missing.is_empty(),
        "builtin surface drift (T6 class): wasm emitter dispatches these ops but the \
         interpreter neither accepts nor documents them:\n  {}\n\
         Port them to eval_builtin/BUILTIN_NAMES with wasm-matching semantics, or add \
         them to WASM_ONLY_DOCUMENTED with a real reason.",
        missing.join("\n  ")
    );
}
