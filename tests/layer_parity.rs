//! FOUR-LAYER BUILTIN PARITY (2026-10-07, hardening item 1).
//!
//! The compiler grew four name inventories that must stay in lockstep:
//!   (a) the compile allowlist  — `helpers::BUILTIN_NAMES`
//!   (b) typing registrations    — `typing::types` `insert_mono("name", …)`
//!   (c) bytecode/interp runtime— `bytecode::eval_builtin` match arms +
//!                                 `eval_near_builtin_match` +
//!                                 `dispatch::*::handle` match arms
//!   (d) wasm emitter runtime   — `wasm_emit/call*.rs` dispatch arms
//!
//! Every layer is extracted AT TEST TIME by scanning the sources, so:
//!   * adding a builtin to ONE layer (or to BUILTIN_NAMES alone) without
//!     the runtime layers fails with a printed diff, and
//!   * allowlist entries whose name later disappears (or gets ported) fail
//!     as stale phantoms — the tables cannot rot.
//!
//! surface_parity.rs (T6, 2026-09) polices the wasm→interp direction for
//! the CALL surface only; this test adds the registry+typing layers, the
//! interp→wasm direction, and the reverse-phantom sweep. Deliberate
//! asymmetries live in the two tables below with per-family reasons —
//! new drift goes there ONLY with a real reason, otherwise port the op.
//!
//! NOTE on `==` / `/=`: comparison aliases the interp accepts and the
//! emitter does not — real drift, allowlisted as "documented, low value
//! to port" (wasm spells them `=`/`!=`). If you port them, delete the
//! entries (the phantom check will demand it).

use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Compile-time special forms handled by BOTH compilers before any builtin
/// dispatch — parity is not applicable (superset of surface_parity's list).
const SPECIAL_FORMS: &[&str] = &[
    "and", "begin", "case", "cond", "default", "defmacro", "define", "delay", "do", "fn", "for",
    "if", "lambda", "let", "let*", "letrec", "loop", "match", "or", "progn", "quasiquote", "quote",
    "recur", "set!", "try", "unquote", "unless", "when", "while",
];

/// Scanner over-captures: string literals that match the arm regex but are
/// not ops (schema field name, outlayer context keys). Documented, not ops.
const SCAN_NOISE: &[&str] = &["name", "predecessor", "signer", "predecessor_id", "signer_id"];

/// Interp-layer names with NO wasm emitter arm (direction: present in
/// registry/typing/bytecode/dispatch, absent from wasm_emit). Families:
/// host-harness services (rlm/llm/shell/file/storage-mock), the r7rs
/// char/string/math host surface, legacy bare-name near aliases, checker
/// forms consumed at compile time, and a few trivial aliases (== /= inc dec).
/// The wasm contract surface intentionally does not grow these.
const INTERP_ONLY: &[(&str, &str)] = &[
    ("account-balance", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("account_balance", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("acos", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("append-file", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("apply", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("asin", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("assoc", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("assq", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("assv", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("atan", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("attached-deposit", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("attached_deposit", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("block-height", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("block-timestamp", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("block_height", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("block_timestamp", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("boolean", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("boolean=?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("boolean?", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("butlast", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("cadr", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("call-with-values", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("case-lambda", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("char->integer", "r7rs char library (interp host surface)"),
    ("char-alphabetic?", "r7rs char library (interp host surface)"),
    ("char-ci<=?", "r7rs char library (interp host surface)"),
    ("char-ci<?", "r7rs char library (interp host surface)"),
    ("char-ci=?", "r7rs char library (interp host surface)"),
    ("char-ci>=?", "r7rs char library (interp host surface)"),
    ("char-ci>?", "r7rs char library (interp host surface)"),
    ("char-downcase", "r7rs char library (interp host surface)"),
    ("char-foldcase", "r7rs char library (interp host surface)"),
    ("char-lower-case?", "r7rs char library (interp host surface)"),
    ("char-numeric?", "r7rs char library (interp host surface)"),
    ("char-upcase", "r7rs char library (interp host surface)"),
    ("char-upper-case?", "r7rs char library (interp host surface)"),
    ("char-whitespace?", "r7rs char library (interp host surface)"),
    ("char<=?", "r7rs char library (interp host surface)"),
    ("char<?", "r7rs char library (interp host surface)"),
    ("char=?", "r7rs char library (interp host surface)"),
    ("char>=?", "r7rs char library (interp host surface)"),
    ("char>?", "r7rs char library (interp host surface)"),
    ("char?", "r7rs char library (interp host surface)"),
    ("check", "checker/annotation forms consumed at compile time"),
    ("check!", "checker/annotation forms consumed at compile time"),
    ("complex?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("cons*", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("contract-check-param", "checker/annotation forms consumed at compile time"),
    ("contract-check-return", "checker/annotation forms consumed at compile time"),
    ("contract-wrap", "checker/annotation forms consumed at compile time"),
    ("cos", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("current-account-id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("current_account_id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("debug", "interp meta/tooling (session surface, not contract surface)"),
    ("dec", "bytecode sugar; wasm lowers (+ x 1) — do NOT call before porting (see overflow_audit)"),
    ("define-values", "destructured binding form (compile-time in wasm)"),
    ("defschema", "checker/annotation forms consumed at compile time"),
    ("delete-file", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("denominator", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("dict-ref", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("dict-set", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("dict/merge", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("dict/remove", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("digit-value", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("display", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("doc", "interp meta/tooling (session surface, not contract surface)"),
    ("drop", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("elapsed", "interp meta/tooling (session surface, not contract surface)"),
    ("empty?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("eq?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("equal?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("eqv?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("error", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("eval", "interp meta/tooling (session surface, not contract surface)"),
    ("even?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("every", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("exact", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("exact->inexact", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("exact-integer-sqrt", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("exact-integer?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("exact?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("exp", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("fields", "checker/annotation forms consumed at compile time"),
    ("file-exists?", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("file/exists?", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("file/list", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("file/read", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("file/write", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("final", "checker/annotation forms consumed at compile time"),
    ("final-var", "checker/annotation forms consumed at compile time"),
    ("find", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("finite?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("float", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("float?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("floor/", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("fmt", "checker/annotation forms consumed at compile time"),
    ("fold-left", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("fold-right", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("for-each", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("force", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("fork", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("fork-exec", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("from-json", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("get-field", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("http-get-json", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("inc", "bytecode sugar; wasm lowers (+ x 1) — do NOT call before porting (see overflow_audit)"),
    ("inexact", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("inexact->exact", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("inexact?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("infer-type", "interp meta/tooling (session surface, not contract surface)"),
    ("infinite?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("inspect", "interp meta/tooling (session surface, not contract surface)"),
    ("int?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("integer", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("integer->char", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("integer?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("json-array-len", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("json-build", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("json-get-in", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("json-parse", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("last", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("let*-values", "destructured binding form (compile-time in wasm)"),
    ("let-values", "destructured binding form (compile-time in wasm)"),
    ("list-copy", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("list-ref", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("list-tail", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("llm", "LLM host-harness service (interp session)"),
    ("llm-batch", "LLM host-harness service (interp session)"),
    ("llm-code", "LLM host-harness service (interp session)"),
    ("load-file", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("load-state", "interp meta/tooling (session surface, not contract surface)"),
    ("log", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("log-utf8", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("log_utf8", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("macro?", "interp meta/tooling (session surface, not contract surface)"),
    ("macroexpand", "interp meta/tooling (session surface, not contract surface)"),
    ("make-case-lambda", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("make-list", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("make-promise", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("make-string", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("map?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("mark-pure", "interp meta/tooling (session surface, not contract surface)"),
    ("matches?", "checker/annotation forms consumed at compile time"),
    ("member", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("memoize", "interp meta/tooling (session surface, not contract surface)"),
    ("memq", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("memv", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("modulo", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("nan?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("near-batch-actions", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-config", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-contracts", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-promises", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-register", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-register-source", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-reset", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near-returned-promise", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near/account_id", "near host ABI interp alias (register model differs)"),
    ("near/assert", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_add_key", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_create", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_delete_account", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_delete_key", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_delete_key_full_access", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_delete_key_function_call", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_delete_key_with_access_key", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_deploy_contract", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_function_call", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_stake", "near host ABI interp alias (register model differs)"),
    ("near/batch_action_transfer", "near host ABI interp alias (register model differs)"),
    ("near/batch_commit", "near host ABI interp alias (register model differs)"),
    ("near/batch_create", "near host ABI interp alias (register model differs)"),
    ("near/config", "near host ABI interp alias (register model differs)"),
    ("near/debug", "near host ABI interp alias (register model differs)"),
    ("near/ecdsa_verify", "near host ABI interp alias (register model differs)"),
    ("near/has", "near host ABI interp alias (register model differs)"),
    ("near/hmac_sha256", "near host ABI interp alias (register model differs)"),
    ("near/log-debug", "near host ABI interp alias (register model differs)"),
    ("near/log_str", "near host ABI interp alias (register model differs)"),
    ("near/predecessor", "near host ABI interp alias (register model differs)"),
    ("near/predecessor_id", "near host ABI interp alias (register model differs)"),
    ("near/require", "near host ABI interp alias (register model differs)"),
    ("near/return_json", "near host ABI interp alias (register model differs)"),
    ("near/return_value", "near host ABI interp alias (register model differs)"),
    ("near/signer_id", "near host ABI interp alias (register model differs)"),
    ("near/signer_public_key", "near host ABI interp alias (register model differs)"),
    ("near/value_return", "near host ABI interp alias (register model differs)"),
    ("near_config", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("near_reset", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("neg?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("negative?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("newline", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("now", "interp meta/tooling (session surface, not contract surface)"),
    ("null?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("num->str", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("number->string", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("numerator", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("odd?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("pair?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("par-filter", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("par-map", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("partition", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("pos?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("positive?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("pow", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("predecessor-account-id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("predecessor_account_id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("procedure?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("promise", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("promise?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("pure-type", "interp meta/tooling (session surface, not contract surface)"),
    ("quotient", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("rational?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("read", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("read-all", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("read-file", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("real?", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("remainder", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("reset-eval-budget", "interp meta/tooling (session surface, not contract surface)"),
    ("rlm", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm-calls", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm-get", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm-set", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm-tokens", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm/config", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm/format-prompt", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm/signature", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm/trace", "RLM host-harness service (interp session state, no contract analog)"),
    ("rlm_set", "RLM host-harness service (interp session state, no contract analog)"),
    ("rollback", "interp meta/tooling (session surface, not contract surface)"),
    ("rollback-to", "interp meta/tooling (session surface, not contract surface)"),
    ("save-state", "interp meta/tooling (session surface, not contract surface)"),
    ("schema", "checker/annotation forms consumed at compile time"),
    ("sha256", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("shell", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("shell-bg", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("shell-kill", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("show-context", "interp meta/tooling (session surface, not contract surface)"),
    ("show-vars", "interp meta/tooling (session surface, not contract surface)"),
    ("signer-account-id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("signer_account_id", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("sin", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("sleep", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("snapshot", "interp meta/tooling (session surface, not contract surface)"),
    ("some", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("sort", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("storage-has-key", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage-read", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage-remove", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage-write", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage_has_key", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage_read", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage_remove", "legacy bare-name near alias (near/* is the wasm surface; surface_parity covers the probe set)"),
    ("storage_write", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("strict", "checker/annotation forms consumed at compile time"),
    ("string", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("string->symbol", "r7rs string library (interp host surface)"),
    ("string-ci<=?", "r7rs string library (interp host surface)"),
    ("string-ci<?", "r7rs string library (interp host surface)"),
    ("string-ci=?", "r7rs string library (interp host surface)"),
    ("string-ci>=?", "r7rs string library (interp host surface)"),
    ("string-ci>?", "r7rs string library (interp host surface)"),
    ("string-contains", "r7rs string library (interp host surface)"),
    ("string-copy", "r7rs string library (interp host surface)"),
    ("string-foldcase", "r7rs string library (interp host surface)"),
    ("string-index", "r7rs string library (interp host surface)"),
    ("string-ref", "r7rs string library (interp host surface)"),
    ("string-replace", "r7rs string library (interp host surface)"),
    ("string<=?", "r7rs string library (interp host surface)"),
    ("string<?", "r7rs string library (interp host surface)"),
    ("string=?", "r7rs string library (interp host surface)"),
    ("string>=?", "r7rs string library (interp host surface)"),
    ("string>?", "r7rs string library (interp host surface)"),
    ("sub-rlm", "RLM host-harness service (interp session state, no contract analog)"),
    ("symbol->string", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("symbol=?", "r7rs predicate alias (wasm has the =/!= core)"),
    ("symbol?", "interp host-surface builtin with no wasm emitter arm (see SURVEY notes in commit)"),
    ("tag-test", "checker/annotation forms consumed at compile time"),
    ("tagged-hash", "checker/annotation forms consumed at compile time"),
    ("take", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("tan", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("to-float", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("to-int", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("to-json", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("to-num", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("trace", "interp meta/tooling (session surface, not contract surface)"),
    ("truncate", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("truncate/", "float/rational host math (wasm surface is the f64 subset: floor/ceil/round/sqrt)"),
    ("try-catch-impl", "destructured binding form (compile-time in wasm)"),
    ("ts->lisp", "interp meta/tooling (session surface, not contract surface)"),
    ("type-of", "interp meta/tooling (session surface, not contract surface)"),
    ("type?", "interp meta/tooling (session surface, not contract surface)"),
    ("u64-and", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-mul-hi", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-not", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-or", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-shl", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-shr", "U64 field arithmetic (wasm twin is the limb family)"),
    ("u64-xor", "U64 field arithmetic (wasm twin is the limb family)"),
    ("valid-type?", "interp meta/tooling (session surface, not contract surface)"),
    ("validate", "checker/annotation forms consumed at compile time"),
    ("values", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec-assoc", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec-conj", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec-contains?", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec-len", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("vec-slice", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
    ("write", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("write-file", "host I/O (interp tooling; contracts cannot do I/O)"),
    ("zip", "r7rs/stdlib host surface (wasm has its own spellings: str-*/len/nth family)"),
];

const WASM_ONLY: &[(&str, &str)] = &[
    ("abort", "wasm test-harness assertion"),
    ("ai-chat", "wasm-run host import (harness only)"),
    ("arr_find", "array-in-linear-memory family (wasm layout)"),
    ("arr_get", "array-in-linear-memory family (wasm layout)"),
    ("arr_len", "array-in-linear-memory family (wasm layout)"),
    ("arr_new", "array-in-linear-memory family (wasm layout)"),
    ("arr_push", "array-in-linear-memory family (wasm layout)"),
    ("arr_set", "array-in-linear-memory family (wasm layout)"),
    ("arr_sort", "array-in-linear-memory family (wasm layout)"),
    ("assert", "wasm test-harness assertion"),
    ("assert-equal", "wasm test-harness assertion"),
    ("assert-raises", "wasm test-harness assertion"),
    ("assert-true", "wasm test-harness assertion"),
    ("base58-decode", "codec over raw byte buffers (wasm memory)"),
    ("base64-encode", "codec over raw byte buffers (wasm memory)"),
    ("base64url-decode", "codec over raw byte buffers (wasm memory)"),
    ("bigint-add", "address-based legacy limb family (string-based u128/* is the spec)"),
    ("bigint-div", "address-based legacy limb family (string-based u128/* is the spec)"),
    ("bigint-from-str", "address-based legacy limb family (string-based u128/* is the spec)"),
    ("bigint-mul", "address-based legacy limb family (string-based u128/* is the spec)"),
    ("bigint-to-str", "address-based legacy limb family (string-based u128/* is the spec)"),
    ("bit_clr", "raw linear-memory / C-ABI family (wasm layout)"),
    ("bit_get", "raw linear-memory / C-ABI family (wasm layout)"),
    ("bit_set", "raw linear-memory / C-ABI family (wasm layout)"),
    ("borsh-deserialize", "codec over raw byte buffers (wasm memory)"),
    ("borsh-serialize", "codec over raw byte buffers (wasm memory)"),
    ("buf-alloc", "raw linear-memory / C-ABI family (wasm layout)"),
    ("buf-get", "raw linear-memory / C-ABI family (wasm layout)"),
    ("buf-set!", "raw linear-memory / C-ABI family (wasm layout)"),
    ("byte-at", "raw linear-memory / C-ABI family (wasm layout)"),
    ("bytes-to-u32", "raw linear-memory / C-ABI family (wasm layout)"),
    ("case-lambda", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("clz", "raw linear-memory / C-ABI family (wasm layout)"),
    ("contract-check-param", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("contract-check-return", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("contract-wrap", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("ctz", "raw linear-memory / C-ABI family (wasm layout)"),
    ("define-values", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("delete-file", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("env/predecessor", "wasm-run host import (harness only)"),
    ("env/signer", "wasm-run host import (harness only)"),
    ("exact->inexact", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("filter-count", "array-in-linear-memory family (wasm layout)"),
    ("final-var", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("fork", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("fp/div", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/from_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/mul", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/one", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/sqrt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp/to_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/add", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/div", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/get_frac", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/get_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/is_zero", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/lt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/mul", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/set", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/set_int", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/sqrt", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("fp64/sub", "Q64.64 raw-bit fixed-point (wasm-only domain)"),
    ("hex-decode", "codec over raw byte buffers (wasm memory)"),
    ("hof/filter", "array-in-linear-memory family (wasm layout)"),
    ("hof/map", "array-in-linear-memory family (wasm layout)"),
    ("hof/reduce", "array-in-linear-memory family (wasm layout)"),
    ("http-post-dynamic", "wasm-run host import (harness only)"),
    ("inexact->exact", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("int_to_str", "raw linear-memory / C-ABI family (wasm layout)"),
    ("itoa", "raw linear-memory / C-ABI family (wasm layout)"),
    ("json-bytes-to-str", "wasm JSON register ABI"),
    ("json-extract", "wasm JSON register ABI"),
    ("json-extract-input", "wasm JSON register ABI"),
    ("json-get-float", "wasm JSON register ABI"),
    ("json-get-str", "wasm JSON register ABI"),
    ("json-get-str?", "wasm JSON register ABI"),
    ("json-return", "wasm JSON register ABI"),
    ("let*-values", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("let-values", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("limb-add", "secp256k1 limb-array ops (wasm linear memory)"),
    ("limb-cmp", "secp256k1 limb-array ops (wasm linear memory)"),
    ("limb-get", "secp256k1 limb-array ops (wasm linear memory)"),
    ("limb-mul", "secp256k1 limb-array ops (wasm linear memory)"),
    ("limb-set!", "secp256k1 limb-array ops (wasm linear memory)"),
    ("limb-sub", "secp256k1 limb-array ops (wasm linear memory)"),
    ("liq_amount0", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount0_64", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount1", "defi raw-u64 math (wasm-only domain)"),
    ("liq_amount1_64", "defi raw-u64 math (wasm-only domain)"),
    ("llm-batch", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("load_i64", "raw linear-memory / C-ABI family (wasm layout)"),
    ("malloc", "raw linear-memory / C-ABI family (wasm layout)"),
    ("map-into", "array-in-linear-memory family (wasm layout)"),
    ("mark-pure", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("mem-get", "raw linear-memory / C-ABI family (wasm layout)"),
    ("mem-get8", "raw linear-memory / C-ABI family (wasm layout)"),
    ("mem-set!", "raw linear-memory / C-ABI family (wasm layout)"),
    ("mem-set8!", "raw linear-memory / C-ABI family (wasm layout)"),
    ("near/assert", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/batch-add-key", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch-call", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch-create-account", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch-deploy", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch-transfer", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_add_key", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_create", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_delete_account", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_delete_key", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_delete_key_full_access", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_delete_key_function_call", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_delete_key_with_access_key", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_deploy_contract", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_function_call", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_stake", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_action_transfer", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_commit", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/batch_create", "wasm promise batch ABI (interp aliases near/promise_batch_*)"),
    ("near/bls12381_g1_multiexp", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_g2_multiexp", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_map_fp2_to_g2", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_map_fp_to_g1", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_p1_decompress", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_p2_decompress", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_p2_sum", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/bls12381_pairing_check", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/call-signed", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/config", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/ecdsa_verify", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/has", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/hmac_sha256", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/json_get_arr", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/json_get_u128", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/kload", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/kstore", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/load-amount", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/log_str", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/predecessor_id", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/require", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/return_json", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/signer_id", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/store-deposit", "near host ABI over registers/raw buffers (wasm-only)"),
    ("near/transfer-signed", "near host ABI over registers/raw buffers (wasm-only)"),
    ("outlayer/call", "outlayer host service (wasm harness only)"),
    ("outlayer/context", "outlayer host service (wasm harness only)"),
    ("outlayer/http-post", "outlayer host service (wasm harness only)"),
    ("outlayer/json-get", "outlayer host service (wasm harness only)"),
    ("outlayer/raw", "outlayer host service (wasm harness only)"),
    ("outlayer/rpc-call", "outlayer host service (wasm harness only)"),
    ("outlayer/send-telegram", "outlayer host service (wasm harness only)"),
    ("outlayer/sleep-ms", "outlayer host service (wasm harness only)"),
    ("outlayer/status", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-delete", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-get", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-has", "outlayer host service (wasm harness only)"),
    ("outlayer/storage-set", "outlayer host service (wasm harness only)"),
    ("outlayer/str-cat", "outlayer host service (wasm harness only)"),
    ("outlayer/str-concat", "outlayer host service (wasm harness only)"),
    ("outlayer/transfer", "outlayer host service (wasm harness only)"),
    ("outlayer/view", "outlayer host service (wasm harness only)"),
    ("outlayer/web-search", "outlayer host service (wasm harness only)"),
    ("popcnt", "raw linear-memory / C-ABI family (wasm layout)"),
    ("price64_to_tick", "defi raw-u64 math (wasm-only domain)"),
    ("price_to_tick", "defi raw-u64 math (wasm-only domain)"),
    ("ptr-add", "raw linear-memory / C-ABI family (wasm layout)"),
    ("range-reduce", "array-in-linear-memory family (wasm layout)"),
    ("rlm", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm-calls", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm-tokens", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm/config", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm/format-prompt", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm/signature", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rlm/trace", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("rpc-call", "wasm-run host import (harness only)"),
    ("send-telegram", "wasm-run host import (harness only)"),
    ("sha256-hash", "raw linear-memory / C-ABI family (wasm layout)"),
    ("show-context", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("show-vars", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("sleep-ms", "wasm-run host import (harness only)"),
    ("storage-clear-all", "mock-storage harness state (wasm test harness)"),
    ("storage-decrement", "mock-storage harness state (wasm test harness)"),
    ("storage-delete", "mock-storage harness state (wasm test harness)"),
    ("storage-get", "mock-storage harness state (wasm test harness)"),
    ("storage-get-worker", "mock-storage harness state (wasm test harness)"),
    ("storage-get-worker-from-project", "mock-storage harness state (wasm test harness)"),
    ("storage-has", "mock-storage harness state (wasm test harness)"),
    ("storage-increment", "mock-storage harness state (wasm test harness)"),
    ("storage-list-keys", "mock-storage harness state (wasm test harness)"),
    ("storage-set", "mock-storage harness state (wasm test harness)"),
    ("storage-set-if-absent", "mock-storage harness state (wasm test harness)"),
    ("storage-set-if-equals", "mock-storage harness state (wasm test harness)"),
    ("storage-set-worker", "mock-storage harness state (wasm test harness)"),
    ("storage-set-worker-public", "mock-storage harness state (wasm test harness)"),
    ("store_i64", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str-ptr", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str-slice", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str-to-num", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str_cat", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str_eq", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str_len", "raw linear-memory / C-ABI family (wasm layout)"),
    ("str_to_int", "raw linear-memory / C-ABI family (wasm layout)"),
    ("string-replace", "wasm-only emitter op (see surface_parity WASM_ONLY_DOCUMENTED for the same class)"),
    ("strlcat", "raw linear-memory / C-ABI family (wasm layout)"),
    ("strlcpy", "raw linear-memory / C-ABI family (wasm layout)"),
    ("sub-rlm", "emitter-side compile form or harness hook misread as op (over-capture)"),
    ("tick_to_price", "defi raw-u64 math (wasm-only domain)"),
    ("tick_to_price64", "defi raw-u64 math (wasm-only domain)"),
    ("tick_to_sqrtPrice64", "defi raw-u64 math (wasm-only domain)"),
    ("u128/checked_add", "raw-limb u128 variant (wasm memory)"),
    ("u128/checked_mul", "raw-limb u128 variant (wasm memory)"),
    ("u128/checked_sub", "raw-limb u128 variant (wasm memory)"),
    ("u128/is_zero", "raw-limb u128 variant (wasm memory)"),
    ("u128/load_storage", "raw-limb u128 variant (wasm memory)"),
    ("u128/store_storage", "raw-limb u128 variant (wasm memory)"),
    ("u32-to-bytes", "raw linear-memory / C-ABI family (wasm layout)"),
    ("vec-push", "array-in-linear-memory family (wasm layout)"),
    ("vec-set!", "array-in-linear-memory family (wasm layout)"),
    ("vrf-generate", "VRF prove — wasm-emitter crypto, verify-side interp parity only"),
    ("web-search", "wasm-run host import (harness only)"),
];

// ── extraction ──
// All scanners share the same conventions as surface_parity.rs (arm regex
// over call*.rs) extended with: comment stripping, multi-arm continuation
// lines ("a"\n | "b" =>), op/name == "x" comparisons, and function-body
// spans (top-level `}` at column 0) so only DISPATCH arms are counted —
// compiler special-form code inside the same files is not.

fn strip_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            i += 1;
            continue;
        }
        if !in_str && c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            break;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Top-level span of a `pub fn NAME(` … closing `}` at column 0.
fn fn_span<'a>(src: &'a str, header: &str) -> &'a str {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("header {header} not found"));
    let end = src[start..]
        .find("\n}\n")
        .map(|o| start + o + 3)
        .unwrap_or(src.len());
    &src[start..end]
}

/// Match-arm names: quoted symbol-charset tokens appearing before the first
/// `=>` on arm lines, plus continuation lines ending in `|`.
fn arm_names(lines: impl Iterator<Item = String>) -> BTreeSet<String> {
    let tok = Regex::new(r#""([A-Za-z0-9!$%&*+\-./:<=>?^_]+)""#).unwrap();
    let mut names = BTreeSet::new();
    let mut pending = false;
    for raw in lines {
        let line = strip_comment(&raw);
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let arrow = line.find("=>");
        let seg = match arrow {
            Some(a) => &line[..a],
            None => line.as_str(),
        };
        let is_chain = trimmed.ends_with('|');
        if arrow.is_some() || pending || is_chain {
            for m in tok.captures_iter(seg) {
                names.insert(m[1].to_string());
            }
        }
        pending = is_chain;
    }
    names.retain(|n| !n.starts_with("__"));
    names
}

fn file_arm_names(path: &Path) -> BTreeSet<String> {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    arm_names(src.lines().map(|l| l.to_string()))
}

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// (a) compile allowlist: quoted tokens in the BUILTIN_NAMES const.
fn extract_registry() -> BTreeSet<String> {
    let src = fs::read_to_string(src_dir().join("helpers.rs")).unwrap();
    let start = src
        .find("pub const BUILTIN_NAMES")
        .expect("BUILTIN_NAMES const");
    let body = &src[start..src[start..].find("\n];").map(|o| start + o).unwrap_or(src.len())];
    let tok = Regex::new(r#""([^"]+)""#).unwrap();
    let mut names = BTreeSet::new();
    for line in body.lines() {
        for m in tok.captures_iter(&strip_comment(line)) {
            names.insert(m[1].to_string());
        }
    }
    names
}

/// (b) typing registrations: insert_mono(\n  "name", …
fn extract_typing() -> BTreeSet<String> {
    let src = fs::read_to_string(src_dir().join("typing/types.rs")).unwrap();
    let re = Regex::new(r#"insert_mono\(\s*\n?\s*"([^"]+)""#).unwrap();
    re.captures_iter(&src)
        .map(|m| m[1].to_string())
        .collect()
}

/// (c1) bytecode VM dispatch: eval_builtin arms + the near-name matcher.
fn extract_bytecode() -> BTreeSet<String> {
    let src = fs::read_to_string(src_dir().join("bytecode/mod.rs")).unwrap();
    let mut names = arm_names(fn_span(&src, "pub fn eval_builtin(").lines().map(|l| l.to_string()));
    let near = fn_span(&src, "pub fn eval_near_builtin_match");
    let tok = Regex::new(r#""([A-Za-z0-9!$%&*+\-./:<=>?^_]+)""#).unwrap();
    for m in tok.captures_iter(near) {
        names.insert(m[1].to_string());
    }
    names
}

/// (c2) dispatch modules: each `pub fn handle(` span.
fn extract_dispatch() -> BTreeSet<String> {
    let dir = src_dir().join("dispatch");
    let mut names = BTreeSet::new();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("dispatch dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("dispatch_"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    for f in files {
        let src = fs::read_to_string(&f).unwrap();
        names.extend(arm_names(fn_span(&src, "pub fn handle(").lines().map(|l| l.to_string())));
    }
    names
}

/// (d) wasm emitter: call*.rs arm names (same file set as surface_parity).
fn extract_wasm() -> BTreeSet<String> {
    let dir = src_dir().join("wasm_emit");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("wasm_emit dir")
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
    let mut names = BTreeSet::new();
    for f in files {
        names.extend(file_arm_names(&f));
    }
    names
}

fn layer_report(name: &str, layers: &[(&str, &BTreeSet<String>)]) -> String {
    let hits: Vec<&str> = layers
        .iter()
        .filter(|(_, set)| set.contains(name))
        .map(|(n, _)| *n)
        .collect();
    format!("{name}: present in [{}]", hits.join(", "))
}

#[test]
fn builtin_layer_parity() {
    let registry = extract_registry();
    let typing = extract_typing();
    let bytecode = extract_bytecode();
    let dispatch = extract_dispatch();
    let wasm = extract_wasm();
    let interp: BTreeSet<String> = bytecode.union(&dispatch).cloned().collect();

    let special: BTreeSet<&str> = SPECIAL_FORMS.iter().copied().collect();
    let noise: BTreeSet<&str> = SCAN_NOISE.iter().copied().collect();
    let excluded = |n: &str| special.contains(n) || noise.contains(n);

    // union of every layer (minus special forms / scan noise)
    let mut union: BTreeSet<String> = BTreeSet::new();
    for set in [&registry, &typing, &bytecode, &dispatch, &wasm] {
        union.extend(set.iter().cloned());
    }
    union.retain(|n| !excluded(n));

    let layers: Vec<(&str, &BTreeSet<String>)> = vec![
        ("registry", &registry),
        ("typing", &typing),
        ("bytecode", &bytecode),
        ("dispatch", &dispatch),
        ("wasm", &wasm),
    ];
    let _ = &layers; // provenance helper only

    let interp_only: BTreeSet<&str> = INTERP_ONLY.iter().map(|(n, _)| *n).collect();
    let wasm_only: BTreeSet<&str> = WASM_ONLY.iter().map(|(n, _)| *n).collect();

    // ── phantom check: allowlist entries must still be real asymmetries ──
    for (name, _) in INTERP_ONLY {
        assert!(
            union.contains(&name.to_string()),
            "INTERP_ONLY entry '{name}' is not present in ANY layer anymore — prune it"
        );
        assert!(
            !wasm.contains(&name.to_string()),
            "INTERP_ONLY entry '{name}' now HAS a wasm emitter arm — prune the entry"
        );
    }
    for (name, _) in WASM_ONLY {
        assert!(
            union.contains(&name.to_string()),
            "WASM_ONLY entry '{name}' is not present in ANY layer anymore — prune it"
        );
        assert!(
            !interp.contains(&name.to_string()),
            "WASM_ONLY entry '{name}' now HAS interp dispatch — prune the entry"
        );
    }

    // ── direction 1: names with no wasm emitter arm ──
    let missing_wasm: Vec<&String> = union
        .iter()
        .filter(|n| !wasm.contains(*n) && !interp_only.contains(n.as_str()))
        .collect();
    assert!(
        missing_wasm.is_empty(),
        "builtin surface drift: these names exist in an interp layer (registry/typing/\
         bytecode/dispatch) but have NO wasm emitter arm — port them to wasm_emit/call*.rs \
         or add them to INTERP_ONLY with a real reason:\n  {}",
        missing_wasm
            .iter()
            .map(|n| layer_report(n, &layers))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // ── direction 2: names with no interp dispatch arm ──
    let missing_interp: Vec<&String> = union
        .iter()
        .filter(|n| !interp.contains(*n) && !wasm_only.contains(n.as_str()))
        .collect();
    assert!(
        missing_interp.is_empty(),
        "builtin surface drift: these names exist in a layer but the interpreter dispatches\
         NONE of them (unknown-builtin at runtime if the registry accepts them) — port to \
         eval_builtin / dispatch::*::handle / eval_near_builtin_match, or add to WASM_ONLY \
         with a real reason:\n  {}",
        missing_interp
            .iter()
            .map(|n| layer_report(n, &layers))
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    eprintln!(
        "layer_parity: registry {} typing {} bytecode {} dispatch {} wasm {} union {} \
         (interp-only {} wasm-only {})",
        registry.len(),
        typing.len(),
        bytecode.len(),
        dispatch.len(),
        wasm.len(),
        union.len(),
        interp_only.len(),
        wasm_only.len()
    );
}
