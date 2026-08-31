//! Core types for the Hindley-Milner–inspired type checker.
//!
//! Types are inferred bidirectionally for pure expressions.
//! No effects, no mutation — just pure data transformations.

use std::collections::HashMap;

/// Returns true for compiler builtin names that the type checker should
/// accept without an explicit type signature. These are host functions
/// and special ops that the WASM emitter knows how to compile.
fn is_builtin_wildcard(name: &str) -> bool {
    if name.starts_with("near/") {
        // Validate against known NEAR host functions — typos should be caught
        return is_known_near_func(&name[5..]);
    }
    // OutLayer/P1 host function prefix — accept all outlayer/* functions
    // The emitter provides specific error messages for non-OutLayer targets
    if name.starts_with("outlayer/") {
        return true;
    }
    name.starts_with("json")
        || name.starts_with("u128/")
        || name.starts_with("borsh-")
        || name.starts_with("wasm/")
        || matches!(
            name,
            "print"
                | "println"
                | "array"
                | "defconst"
                | "export"
                | "memory"
                | "module"
                | "borsh-schema"
                | "extend-runtime"
                | "vec-nth"
                | "list"
                // P1/OutLayer HTTP functions (emitter guards with wasi_mode)
                | "http-get"
                | "http-post"
                // P1/OutLayer storage aliases (kebab-case)
                | "storage-set"
                | "storage-get"
                | "storage-has"
                | "storage-delete"
                | "storage-increment"
                | "storage-decrement"
                | "storage-set-if-absent"
                | "storage-set-if-equals"
                | "storage-list-keys"
                | "storage-clear-all"
                // P1 context functions (OutLayer env)
                | "env/signer"
                | "env/predecessor"
                | "schnorr-verify"
        )
}

/// Known NEAR host function names (the part after "near/").
/// Derived from the HOST_FUNCS table in wasm_emit.rs.
const KNOWN_NEAR_FUNCS: &[&str] = &[
    "store", "load", "store_num", "load_num", "remove", "has_key",
    "kv", "kv-get",
    "storage_read", "storage_write", "storage_has_key", "storage_remove",
    "return", "return_str", "return_value", "value_return",
    "log",
    "input",
    "panic",
    "current_account_id",
    "account_id",
    "signer_account_id",
    "signer_account_pk",
    "predecessor_account_id",
    "predecessor",
    "attached_deposit",
    "deposit-gte",
    "block_index", "block_height", "block_timestamp",
    "ed25519_verify", "p256_verify",
    "sha256", "keccak256", "keccak512",
    "random_seed",
    "ripemd160", "ecrecover",
    "alt_bn128_g1_multiexp", "alt_bn128_g1_sum", "alt_bn128_pairing_check",
    "bls12381_p1_sum", "bls12381_p2_sum",
    "bls12381_g1_multiexp", "bls12381_g2_multiexp",
    "prepaid_gas",
    "used_gas",
    "promise_create",
    "promise_then",
    "promise_and",
    "promise_result",
    "promise_batch_create",
    "promise_batch_then",
    "promise_batch_action_create_account",
    "promise_batch_action_deploy_contract",
    "promise_batch_action_function_call",
    "promise_batch_action_transfer",
    "promise_batch_action_stake",
    "promise_batch_action_add_key_with_full_access",
    "promise_batch_action_add_key_with_function_call",
    "promise_batch_action_delete_key",
    "promise_batch_action_delete_account",
    "call",
    "log_utf8", "log_utf16",
    "signer_to_buf",
    "write_amount",
    "abort",
    // JSON convenience builtins
    "json_get_int",
    "json_get_str",
    "json_return_int",
    "json_return_str",
];

fn is_known_near_func(name: &str) -> bool {
    KNOWN_NEAR_FUNCS.contains(&name)
}

/// A type variable ID. Allocated fresh by the type checker.
pub type TVarId = u32;

/// Core types for the pure subset.
#[derive(Clone, Debug, PartialEq)]
pub enum TcType {
    /// Type variable — to be resolved by unification.
    Var(TVarId),
    /// Concrete type constructor with optional type arguments.
    Con(TcCon),
    /// Function type: argument types → return type.
    Arrow(Vec<TcType>, Box<TcType>),
    /// forall-quantified type (polymorphic).
    #[allow(dead_code)]
    Forall(Vec<TVarId>, Box<TcType>),
}

/// Type constructors.
#[derive(Clone, Debug, PartialEq)]
pub enum TcCon {
    Nil,
    Bool,
    Int,
    Float,
    Num, // int | float (for polymorphic arithmetic)
    Str,
    Sym,
    List(Box<TcType>),             // homogeneous list
    Map(Box<TcType>, Box<TcType>), // key → val
    Tuple(Vec<TcType>),
    Opt(Box<TcType>), // T | nil — storage reads and other maybe-missing values
    Ptr,              // raw WASM pointer — distinct from tagged Num
    Any,              // escape hatch
}

/// A type scheme: forall α1..αn. τ
/// Used for polymorphic let-bindings.
#[derive(Clone, Debug)]
pub struct Scheme {
    pub vars: Vec<TVarId>,
    pub ty: TcType,
}

/// The type-checking environment: maps variable names to type schemes.
#[derive(Clone, Debug)]
pub struct TcEnv {
    bindings: HashMap<String, Scheme>,
    /// When true, any `near/*` symbol is accepted as `'any → 'any → ... → 'any`.
    /// This avoids having to enumerate every host function while still catching
    /// undefined user variables.
    near_wildcard: bool,
    /// When true, the checker is in a `pure` block — effectful operations
    /// (near/storage_write, near/log, etc.) are forbidden.
    pub pure_mode: bool,
    /// Storage schema: maps literal storage keys to the type of value stored there.
    /// Populated by near/storage_write, checked by near/storage_read.
    #[allow(dead_code)]
    pub storage_schema: HashMap<String, TcType>,
}
impl TcEnv {
    pub fn new() -> Self {
        TcEnv {
            bindings: HashMap::new(),
            near_wildcard: false,
            pure_mode: false,
            storage_schema: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, scheme: Scheme) {
        self.bindings.insert(name, scheme);
    }

    pub fn get(&self, name: &str) -> Option<&Scheme> {
        if let Some(scheme) = self.bindings.get(name) {
            return Some(scheme);
        }
        // Wildcard: accept any compiler builtin that isn't explicitly
        // registered. The type checker's job is to catch user bugs —
        // undefined user vars, arity mismatches — not validate every
        // host function signature.
        if self.near_wildcard && is_builtin_wildcard(name) {
            thread_local! {
                static WILDCARD: std::cell::RefCell<Scheme> = std::cell::RefCell::new(Scheme {
                    vars: vec![0],
                    ty: TcType::Var(0),
                });
            }
            return WILDCARD.with(|s| {
                let ptr: *const Scheme = s.as_ptr();
                unsafe { Some(&*ptr) }
            });
        }
        None
    }

    /// Insert a monomorphic (no quantified vars) binding.
    pub fn insert_mono(&mut self, name: String, ty: TcType) {
        self.insert(name, Scheme { vars: vec![], ty });
    }

    /// Standard pure builtins with their type schemes.
    pub fn with_pure_builtins() -> Self {
        let mut env = TcEnv::new();

        // Arithmetic: num → num → num
        for name in &[
            "+", "-", "*", "/", "mod", "min", "max", "wrap-add", "wrap-sub", "wrap-mul",
        ] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Num), TcType::Con(TcCon::Num)],
                    Box::new(TcType::Con(TcCon::Num)),
                ),
            );
        }

        // Comparison: num → num → bool
        for name in &["=", "!=", "<", ">", "<=", ">="] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Num), TcType::Con(TcCon::Num)],
                    Box::new(TcType::Con(TcCon::Bool)),
                ),
            );
        }

        // abs : num → num
        env.insert_mono(
            "abs".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // muldiv : num → num → num → num  (a*b/c with 128-bit intermediate)
        env.insert_mono(
            "muldiv".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Num),
                    TcType::Con(TcCon::Num),
                    TcType::Con(TcCon::Num),
                ],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // isqrt : num → num
        env.insert_mono(
            "isqrt".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // ctz : num → num  (count trailing zeros)
        env.insert_mono(
            "ctz".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // ── Bitwise intrinsics: shl, shr, band, bor, bnot ──
        for name in &["shl", "shr", "band", "bor"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Num), TcType::Con(TcCon::Num)],
                    Box::new(TcType::Con(TcCon::Num)),
                ),
            );
        }
        env.insert_mono(
            "bnot".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // ── Linear memory struct intrinsics ──
        // malloc : num → num  (allocates n bytes, returns tagged handle)
        env.insert_mono(
            "malloc".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );
        // store_i64 : num → num → num → nil  (handle, byte_offset, value)
        env.insert_mono(
            "store_i64".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Num),
                    TcType::Con(TcCon::Num),
                    TcType::Con(TcCon::Num),
                ],
                Box::new(TcType::Con(TcCon::Nil)),
            ),
        );
        // load_i64 : num → num → num  (handle, byte_offset → value)
        env.insert_mono(
            "load_i64".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num), TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );

        // mem-set! : ptr → num → nil  (raw memory write, untagged addr)
        env.insert_mono(
            "mem-set!".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Ptr), TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Nil)),
            ),
        );
        // mem-get : ptr → num  (raw memory read, untagged addr)
        env.insert_mono(
            "mem-get".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Ptr)],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );
        // ptr-add : ptr → anyint → ptr  (pointer arithmetic for raw-memory arena
        // addressing; the offset may be an untyped int literal). (2026-08-29)
        // Also accepts num → ptr → ptr. HashMap insert_mono is last-wins, so the
        // most generic signature must be inserted LAST.
        env.insert_mono(
            "ptr-add".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Ptr), TcType::Con(TcCon::Any)],
                Box::new(TcType::Con(TcCon::Ptr)),
            ),
        );


        // itoa : num → str
        env.insert_mono(
            "itoa".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );

        // not : 'a → bool
        {
            let a = TcType::Var(0);
            env.insert(
                "not".to_string(),
                Scheme {
                    vars: vec![0],
                    ty: TcType::Arrow(vec![a], Box::new(TcType::Con(TcCon::Bool))),
                },
            );
        }

        // Borsh serialization (also works in fuzz mode)
        for name in &["borsh-serialize", "borsh-deserialize", "array"] {
            // These take variable args, so use Any
            env.insert_mono(name.to_string(), TcType::Con(TcCon::Any));
        }

        // String ops
        for name in &["str-concat", "string-append"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                    Box::new(TcType::Con(TcCon::Str)),
                ),
            );
        }
        // str-cat: strings-only variadic concat (interpreter + wasm_emit agree;
        // Num args are NOT stringified — wasm untag assumes TAG_STR, so the
        // interpreter hard-errors instead of wasm's silent mis-read)
        env.insert_mono(
            "str-cat".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // Mutable byte buffers (wasm-only surface; repr = tagged string)
        env.insert_mono(
            "limb-add".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "limb-sub".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "limb-mul".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "limb-cmp".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "limb-get".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Int)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "limb-set!".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        env.insert_mono(
            "buf-alloc".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Int)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        env.insert_mono(
            "buf-get".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Int)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "buf-set!".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        env.insert_mono(
            "str-ptr".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "str-length".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        // String aliases
        env.insert_mono(
            "string-length".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        env.insert_mono(
            "str-substring".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // byte-at: (byte-at s i) -> byte code at index i (0 if out of range).
        // Numeric char access for symbol interning in lisp-written interpreters —
        // without it, tokens ≥2 chars cannot get distinct hash codes. (2026-08-29)
        // Result is Num (tagged arithmetic value), index accepts Int (literal) or Num.
        env.insert_mono(
            "byte-at".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Num),
                ],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );
        env.insert_mono(
            "byte-at".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Int),
                ],
                Box::new(TcType::Con(TcCon::Num)),
            ),
        );
        // str= / str!= : string equality (TS frontend M2)
        for name in &["str=", "str!="] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                    Box::new(TcType::Con(TcCon::Bool)),
                ),
            );
        }
        env.insert_mono(
            "str-contains".to_string(),
            // Bool, not Int (wasm-fuzz find #4, 2026-08-27): the emitted
            // code pushes a tagged bool at runtime and println shows
            // true/false, but the checker signature said Int — so
            // (if (str-contains ...) 1 2) failed branch unification
            // with int ≠ bool while the interp accepted it.
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Bool)),
            ),
        );
        env.insert_mono(
            "str-index-of".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        // Scalar string transforms (round 2, 2026-08-27): previously
        // interp-only, now wasm-emitted (call_string.rs str_case/str_trim/
        // str_starts_with/str_ends_with/str_replace). wasm is ASCII-bounded;
        // interp is Rust Unicode — divergence documented in COVERAGE.md §D.
        for name in &["str-upcase", "string-upcase", "str-downcase", "string-downcase", "str-trim"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Str)],
                    Box::new(TcType::Con(TcCon::Str)),
                ),
            );
        }
        for name in &["str-starts-with", "string-prefix?", "str-ends-with", "string-suffix?"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                    Box::new(TcType::Con(TcCon::Bool)),
                ),
            );
        }
        env.insert_mono(
            "str-replace".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                ],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );

        // List-shaped string builtins (round 3, 2026-08-27): split/chunk/
        // string->list return List(Str) as zero-copy views; str-join/
        // list->string stringify elements via __to_string (interp parity).
        let tstr = TcType::Con(TcCon::Str);
        let list_str = TcType::Con(TcCon::List(Box::new(tstr.clone())));
        for name in &["str-split", "str-split-exact"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![tstr.clone(), tstr.clone()],
                    Box::new(list_str.clone()),
                ),
            );
        }
        env.insert_mono(
            "str-chunk".to_string(),
            TcType::Arrow(
                vec![tstr.clone(), TcType::Con(TcCon::Int)],
                Box::new(list_str.clone()),
            ),
        );
        env.insert_mono(
            "string->list".to_string(),
            TcType::Arrow(vec![tstr.clone()], Box::new(list_str.clone())),
        );
        env.insert_mono(
            "str-join".to_string(),
            TcType::Arrow(
                vec![tstr.clone(), list_str.clone()],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        env.insert_mono(
            "list->string".to_string(),
            TcType::Arrow(
                vec![list_str.clone()],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // String builtins registered in the emitter/dispatch but previously
        // missing here (corpus e24 finding, 2026-08-27) — without these the
        // checker rejected valid wasm compilable programs.
        env.insert_mono(
            "str-repeat".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Int)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        env.insert_mono(
            "to-string".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // string->number : str → num  (aliases: str->num, str-to-num)
        for name in &["string->number", "str->num", "str-to-num"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(
                    vec![TcType::Con(TcCon::Str)],
                    Box::new(TcType::Con(TcCon::Num)),
                ),
            );
        }

        // JSON path accessors
        // json-get: (str, str) → num  — extracts numeric value at dot-path
        env.insert_mono(
            "json-get".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        // json-get-str: (str, str) → str  — extracts string value at dot-path
        env.insert_mono(
            "json-get-str".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // json-decode-bytes: (str) → str  — decodes "[123,34,...]" byte array to string
        env.insert_mono(
            "json-decode-bytes".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );
        // json-get-float: (str, str) → num  — extracts float as integer (price * 100)
        env.insert_mono(
            "json-get-float".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str), TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        // json-extract: (str, str, str, ...) → array  — extracts multiple values at once
        // json/get: (str) → num  — single-key shorthand from input (NEAR)
        env.insert_mono(
            "json/get".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );
        // json-return: (any) → nil  — return JSON result (host)
        env.insert_mono(
            "json-return".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Any)],
                Box::new(TcType::Con(TcCon::Nil)),
            ),
        );
        // json-extract: variadic — (str, str, str, ...) → array
        // Type checker just needs to know it exists; emitter validates arity
        let arr_ty = TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any))));
        env.insert_mono(
            "json-extract".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                ],
                Box::new(arr_ty),
            ),
        );

        // List ops with polymorphism: ('a list → 'a)
        let a = TcType::Var(0);
        let list_a = TcType::Con(TcCon::List(Box::new(a.clone())));
        env.insert(
            "car".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![list_a.clone()], Box::new(a.clone())),
            },
        );
        env.insert(
            "cdr".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![list_a.clone()], Box::new(list_a.clone())),
            },
        );

        // cons : 'a → ('a list) → ('a list)
        let a2 = TcType::Var(0);
        let list_a2 = TcType::Con(TcCon::List(Box::new(a2.clone())));
        env.insert(
            "cons".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![a2.clone(), list_a2.clone()], Box::new(list_a2.clone())),
            },
        );

        // list : ('a ...) → ('a list) — variadic, same type
        // Approximate as 'a → ('a list) for now (1-arg version)
        let a3 = TcType::Var(0);
        let list_a3 = TcType::Con(TcCon::List(Box::new(a3.clone())));
        env.insert(
            "list".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![a3], Box::new(list_a3)),
            },
        );

        // len : 'a list → int
        let a4 = TcType::Var(0);
        env.insert(
            "len".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(
                    vec![TcType::Con(TcCon::List(Box::new(a4)))],
                    Box::new(TcType::Con(TcCon::Int)),
                ),
            },
        );

        // append : ('a list) → ('a list) → ('a list)
        let a5 = TcType::Var(0);
        let list_a5 = TcType::Con(TcCon::List(Box::new(a5.clone())));
        env.insert(
            "append".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![list_a5.clone(), list_a5.clone()], Box::new(list_a5)),
            },
        );

        // Higher-order: map : ('a → 'b) → ('a list) → ('b list)
        let a6 = TcType::Var(0);
        let b6 = TcType::Var(1);
        env.insert(
            "map".to_string(),
            Scheme {
                vars: vec![0, 1],
                ty: TcType::Arrow(
                    vec![
                        TcType::Arrow(vec![a6.clone()], Box::new(b6.clone())),
                        TcType::Con(TcCon::List(Box::new(a6))),
                    ],
                    Box::new(TcType::Con(TcCon::List(Box::new(b6)))),
                ),
            },
        );

        // filter : ('a → bool) → ('a list) → ('a list)
        let a7 = TcType::Var(0);
        env.insert(
            "filter".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(
                    vec![
                        TcType::Arrow(vec![a7.clone()], Box::new(TcType::Con(TcCon::Bool))),
                        TcType::Con(TcCon::List(Box::new(a7.clone()))),
                    ],
                    Box::new(TcType::Con(TcCon::List(Box::new(a7)))),
                ),
            },
        );

        // reduce : ('a → 'b → 'a) → 'a → ('b list) → 'a
        let a8 = TcType::Var(0);
        let b8 = TcType::Var(1);
        env.insert(
            "reduce".to_string(),
            Scheme {
                vars: vec![0, 1],
                ty: TcType::Arrow(
                    vec![
                        TcType::Arrow(vec![a8.clone(), b8.clone()], Box::new(a8.clone())),
                        a8.clone(),
                        TcType::Con(TcCon::List(Box::new(b8))),
                    ],
                    Box::new(a8),
                ),
            },
        );

        // Predicates
        for name in &[
            "nil?", "null?", "list?", "pair?", "number?", "string?", "bool?", "boolean?", "empty?",
            "zero?",
        ] {
            let a = TcType::Var(0);
            env.insert(
                name.to_string(),
                Scheme {
                    vars: vec![0],
                    ty: TcType::Arrow(vec![a], Box::new(TcType::Con(TcCon::Bool))),
                },
            );
        }

        // json-quote : 'a → str (tag-aware JSON scalar encoder)
        let aq = TcType::Var(0);
        env.insert(
            "json-quote".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![aq], Box::new(TcType::Con(TcCon::Str))),
            },
        );

        // json-set : str → str → str → str
        // (top-level key set/replace; value arg is pre-encoded JSON text —
        // the mirror of json-quote's output)
        env.insert_mono(
            "json-set".to_string(),
            TcType::Arrow(
                vec![
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                    TcType::Con(TcCon::Str),
                ],
                Box::new(TcType::Con(TcCon::Str)),
            ),
        );

        // to-string : 'a → str
        let a9 = TcType::Var(0);
        env.insert(
            "to-string".to_string(),
            Scheme {
                vars: vec![0],
                ty: TcType::Arrow(vec![a9], Box::new(TcType::Con(TcCon::Str))),
            },
        );

        // Conversions
        // assert : bool → nil
        env.insert_mono(
            "assert".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Bool)],
                Box::new(TcType::Con(TcCon::Nil)),
            ),
        );
        env.insert_mono(
            "to-float".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Float)),
            ),
        );
        env.insert_mono(
            "to-int".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Num)],
                Box::new(TcType::Con(TcCon::Int)),
            ),
        );

        // ── u128 builtins (string-based decimal values, pure functions) ──
        // Typed here too so wasm entry points get the emitter's clear
        // "u128 builtins not yet implemented for wasm target" error instead
        // of a misleading "undefined variable" from the checker.
        {
            let str_ty = TcType::Con(TcCon::Str);
            let int_ty = TcType::Con(TcCon::Int);
            let bool_ty = TcType::Con(TcCon::Bool);
            for name in &["u128/add", "u128/sub", "u128/mul", "u128/div", "u128/mod"] {
                env.insert_mono(
                    name.to_string(),
                    TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(str_ty.clone())),
                );
            }
            for name in &["u128/lt", "u128/gt", "u128/eq"] {
                env.insert_mono(
                    name.to_string(),
                    TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(bool_ty.clone())),
                );
            }
            env.insert_mono(
                "u128/from-i64".into(),
                TcType::Arrow(vec![int_ty], Box::new(str_ty.clone())),
            );
            env.insert_mono(
                "u128/to-i64".into(),
                TcType::Arrow(vec![str_ty.clone()], Box::new(TcType::Con(TcCon::Int))),
            );
            env.insert_mono(
                "u128/is-zero".into(),
                TcType::Arrow(vec![str_ty], Box::new(bool_ty)),
            );
        }

        env
    }

    /// NEAR host function builtins. These are effectful, so we type them as
    /// returning `any` where the result could be anything. The point is to
    /// avoid "undefined variable" errors, not to enforce effect discipline.
    pub fn with_near_builtins() -> Self {
        let mut env = Self::with_pure_builtins();
        let str_ty = TcType::Con(TcCon::Str);
        let int_ty = TcType::Con(TcCon::Int);
        let num_ty = TcType::Con(TcCon::Num);
        let bool_ty = TcType::Con(TcCon::Bool);
        let nil_ty = TcType::Con(TcCon::Nil);
        let any_ty = TcType::Con(TcCon::Any);

        // near/input : () → str
        env.insert_mono(
            "near/input".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/return_str : str → any (terminates)
        env.insert_mono(
            "near/return_str".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())),
        );
        // near/return_value : str → any
        env.insert_mono(
            "near/return_value".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())),
        );
        // near/storage_read : str → str
        env.insert_mono(
            "near/storage_read".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // storage_read is in emitter host table at index 18
        env.insert_mono(
            "storage_read".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // near/storage_write : str → str → nil
        env.insert_mono(
            "near/storage_write".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/storage_has_key : str → bool
        env.insert_mono(
            "near/storage_has_key".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(bool_ty.clone())),
        );
        // near/storage_remove : str → nil
        env.insert_mono(
            "near/storage_remove".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(nil_ty.clone())),
        );
        // near/log : str → nil
        env.insert_mono(
            "near/log".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(nil_ty.clone())),
        );
        // near/account_id : () → str
        env.insert_mono(
            "near/account_id".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/predecessor : () → str
        env.insert_mono(
            "near/predecessor".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/signer_account_id : () → str
        env.insert_mono(
            "near/signer_account_id".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/load : str → num  (reads tagged i64 from storage)
        env.insert_mono(
            "near/load".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(num_ty.clone())),
        );
        // near/store : str → num → nil  (writes tagged i64 to storage)
        env.insert_mono(
            "near/store".into(),
            TcType::Arrow(vec![str_ty.clone(), num_ty.clone()], Box::new(TcType::Con(TcCon::Nil))),
        );
        // near/signer_public_key : () → str
        env.insert_mono(
            "near/signer_public_key".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/attached_deposit : () → int
        env.insert_mono(
            "near/attached_deposit".into(),
            TcType::Arrow(vec![], Box::new(int_ty.clone())),
        );
        // near/attached_deposit_u128 : () → str (decimal u128 string,
        // rendered via the shared __h_u128_to_str helper)
        env.insert_mono(
            "near/attached_deposit_u128".into(),
            TcType::Arrow(vec![], Box::new(TcType::Con(TcCon::Str))),
        );
        // near/store_u128 : str → int → nil  (key, tagged pointer)
        env.insert_mono(
            "near/store_u128".into(),
            TcType::Arrow(
                vec![str_ty.clone(), int_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/load_u128 : str → int (key) → int (returns tagged pointer)
        env.insert_mono(
            "near/load_u128".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(int_ty.clone())),
        );
        // near/call : str → str → str → int → int → nil
        // (target, method, args_json, gas, deposit)
        env.insert_mono(
            "near/call".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), str_ty.clone(), int_ty.clone(), int_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/transfer : str → int → nil  (account_id, amount_yocto)
        env.insert_mono(
            "near/transfer".into(),
            TcType::Arrow(
                vec![str_ty.clone(), int_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/call-await : str → str → str → int → str → int → str → nil
        // (target, method, args_json, gas, callback_name, cb_gas, cb_args_json)
        env.insert_mono(
            "near/call-await".into(),
            TcType::Arrow(
                vec![
                    str_ty.clone(),
                    str_ty.clone(),
                    str_ty.clone(),
                    int_ty.clone(),
                    str_ty.clone(),
                    int_ty.clone(),
                    str_ty.clone(),
                ],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/transfer_u128 : str → str → nil  (account_id, amount decimal)
        env.insert_mono(
            "near/transfer_u128".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/promise_yield_create : str → str → int → int → int
        // (method, args_json, gas, weight) → data_id (u64 as int)
        env.insert_mono(
            "near/promise_yield_create".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), int_ty.clone(), int_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );
        // near/promise_yield_resume : int → str → int
        // (data_id, payload) → data_id
        env.insert_mono(
            "near/promise_yield_resume".into(),
            TcType::Arrow(
                vec![int_ty.clone(), str_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );
        // near/block_timestamp : () → int
        // near/deposit-gte : int → int → bool (lo, hi literal only; emitter takes 1-2)
        env.insert_mono("near/deposit-gte".into(), TcType::Arrow(vec![int_ty.clone(), int_ty.clone()], Box::new(bool_ty.clone())));
        env.insert_mono("near/block_timestamp".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/block_height : () → int
        env.insert_mono(
            "near/block_height".into(),
            TcType::Arrow(vec![], Box::new(int_ty.clone())),
        );
        // near/ed25519_verify : str → str → str → int
        env.insert_mono(
            "near/ed25519_verify".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), str_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );
        // near/p256_verify : str → str → str → int
        env.insert_mono(
            "near/p256_verify".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), str_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );

        // schnorr-verify : str -> str -> str -> int (BIP-340, runtime bytes)
        env.insert_mono(
            "schnorr-verify".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), str_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );
        // near/schnorr_verify : str -> str -> str -> int (BIP-340 secp256k1, stitched WASM)
        env.insert_mono(
            "near/schnorr_verify".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone(), str_ty.clone()],
                Box::new(int_ty.clone()),
            ),
        );
        // near/sha256 : str → str
        env.insert_mono(
            "near/sha256".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // sha256-hash : str → str (raw 32-byte digest string; wasm call_near_crypto.rs)
        env.insert_mono(
            "sha256-hash".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // near/keccak256 : str → str
        env.insert_mono(
            "near/keccak256".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // near/random_seed : () → str
        env.insert_mono(
            "near/random_seed".into(),
            TcType::Arrow(vec![], Box::new(str_ty.clone())),
        );
        // near/prepaid_gas : () → int
        env.insert_mono(
            "near/prepaid_gas".into(),
            TcType::Arrow(vec![], Box::new(int_ty.clone())),
        );
        // near/used_gas : () → int
        env.insert_mono(
            "near/used_gas".into(),
            TcType::Arrow(vec![], Box::new(int_ty.clone())),
        );
        // near/value_return : str → any
        env.insert_mono(
            "near/value_return".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())),
        );
        // near/panic : str → any
        env.insert_mono(
            "near/panic".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())),
        );
        // near/promise_create : str → str → str → int → int → int
        env.insert_mono(
            "near/promise_create".into(),
            TcType::Arrow(
                vec![
                    str_ty.clone(),
                    str_ty.clone(),
                    str_ty.clone(),
                    int_ty.clone(),
                    int_ty.clone(),
                ],
                Box::new(int_ty.clone()),
            ),
        );
        // near/promise_then : int → str → str → str → int → int → int
        env.insert_mono(
            "near/promise_then".into(),
            TcType::Arrow(
                vec![
                    int_ty.clone(),
                    str_ty.clone(),
                    str_ty.clone(),
                    str_ty.clone(),
                    int_ty.clone(),
                    int_ty.clone(),
                ],
                Box::new(int_ty.clone()),
            ),
        );
        // near/promise_and: variadic — accepts any number of promise indices (emitter has two impls)
        // near/promise_result: exactly 1 arg (promise idx) → status int.
        // (The old "0-arg wildcard" comment was wrong: the emitter only ever
        // handled 1-arg — 0-arg panicked at a[0]; hard-error guard added
        // 2026-08-31 in call_near_promise.rs.)

        // String builtins used in NEAR contracts
        env.insert_mono(
            "str-len".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(int_ty.clone())),
        );
        // ── wallet-factory byte/string builtins (wasm call_string.rs, fb825ba) ──
        // hex-encode : str → str
        env.insert_mono(
            "hex-encode".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // hex-decode : str → str (raw bytes as string; wasm call_string.rs)
        env.insert_mono(
            "hex-decode".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // base64-decode : str → str (raw bytes as string)
        env.insert_mono(
            "base64-decode".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        // str-contains-byte : str → int → bool
        env.insert_mono(
            "str-contains-byte".into(),
            TcType::Arrow(
                vec![str_ty.clone(), int_ty.clone()],
                Box::new(bool_ty.clone()),
            ),
        );
        // str-repeat : str → int → str
        env.insert_mono(
            "str-repeat".into(),
            TcType::Arrow(
                vec![str_ty.clone(), int_ty.clone()],
                Box::new(str_ty.clone()),
            ),
        );
        // near/store-bytes : str → str → nil
        env.insert_mono(
            "near/store-bytes".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone()],
                Box::new(TcType::Con(TcCon::Nil)),
            ),
        );
        // near/load-bytes : str → str
        env.insert_mono(
            "near/load-bytes".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())),
        );
        env.insert_mono(
            "str-slice".into(),
            TcType::Arrow(
                vec![str_ty.clone(), int_ty.clone(), int_ty.clone()],
                Box::new(str_ty.clone()),
            ),
        );
        env.insert_mono(
            "str-cat".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone()],
                Box::new(str_ty.clone()),
            ),
        );
        // str-concat / string-append are variadic — type check skips them
        // (handled by dispatching to binary str-cat in the emitter)
        env.insert_mono(
            "u32-to-bytes".into(),
            TcType::Arrow(vec![int_ty.clone()], Box::new(str_ty.clone())),
        );
        env.insert_mono(
            "bytes-to-u32".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(num_ty.clone())),
        );

        // ── String aliases ──
        env.insert_mono(
            "str-to-num".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "str_cat".into(),
            TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(str_ty.clone())),
        );
        env.insert_mono(
            "str_eq".into(),
            TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(bool_ty.clone())),
        );
        env.insert_mono(
            "str_len".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "int_to_str".into(),
            TcType::Arrow(vec![num_ty.clone()], Box::new(str_ty.clone())),
        );

        // ── Arithmetic ──
        env.insert_mono(
            "max".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "min".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "mod".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );

        // ── Wrapping arithmetic ──
        env.insert_mono(
            "wrap-add".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "wrap-sub".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "wrap-mul".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );

        // ── Bitwise ──
        env.insert_mono(
            "band".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "bor".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "shl".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );
        env.insert_mono(
            "shr".into(),
            TcType::Arrow(vec![num_ty.clone(), num_ty.clone()], Box::new(num_ty.clone())),
        );

        // ── Memory ──
        env.insert_mono(
            "mem-get8".into(),
            TcType::Arrow(vec![num_ty.clone()], Box::new(num_ty.clone())),
        );

        // ── NEAR host fns (missing) ──
        env.insert_mono(
            "near/signer_account_pk".into(),
            TcType::Arrow(vec![], Box::new(int_ty.clone())),
        );
        env.insert_mono(
            "near/log_num".into(),
            TcType::Arrow(vec![num_ty.clone()], Box::new(nil_ty.clone())),
        );
        env.insert_mono(
            "near/log_utf16".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(nil_ty.clone())),
        );

        // ── Test helpers ──
        env.insert_mono(
            "assert-equal".into(),
            TcType::Arrow(vec![any_ty.clone(), any_ty.clone()], Box::new(nil_ty.clone())),
        );
        env.insert_mono(
            "assert-true".into(),
            TcType::Arrow(vec![any_ty.clone()], Box::new(nil_ty.clone())),
        );
        env.insert_mono(
            "assert-raises".into(),
            TcType::Arrow(vec![any_ty.clone()], Box::new(nil_ty.clone())),
        );

        // ── Coercion ──
        env.insert_mono(
            "str".into(),
            TcType::Arrow(vec![num_ty.clone()], Box::new(str_ty.clone())),
        );
        // ── Logical ──
        env.insert_mono(
            "not".into(),
            TcType::Arrow(vec![any_ty.clone()], Box::new(bool_ty.clone())),
        );

        // Array/vec builtins (emitter: call_list.rs — runtime heap arrays)
        let any_arr_ty = TcType::Con(TcCon::List(Box::new(TcType::Con(TcCon::Any))));
        env.insert_mono("vec-nth".into(), TcType::Arrow(vec![any_arr_ty.clone(), int_ty.clone()], Box::new(TcType::Con(TcCon::Any))));
        env.insert_mono("vec-length".into(), TcType::Arrow(vec![any_arr_ty.clone()], Box::new(int_ty.clone())));
        env.insert_mono("vec-push".into(), TcType::Arrow(vec![any_arr_ty.clone(), TcType::Con(TcCon::Any)], Box::new(any_arr_ty.clone())));
        env.insert_mono("vec-set!".into(), TcType::Arrow(vec![any_arr_ty.clone(), int_ty.clone(), TcType::Con(TcCon::Any)], Box::new(TcType::Con(TcCon::Nil))));
        env.insert_mono("near/json_get_arr".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_arr_ty.clone())));
        // lisp-rlm list builtins (emitter: call_list.rs)
        env.insert_mono("list".into(), TcType::Arrow(vec![any_ty.clone(), any_ty.clone(), any_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("nth".into(), TcType::Arrow(vec![any_arr_ty.clone(), int_ty.clone()], Box::new(TcType::Con(TcCon::Any))));
        env.insert_mono("len".into(), TcType::Arrow(vec![any_arr_ty.clone()], Box::new(int_ty.clone())));
        env.insert_mono("car".into(), TcType::Arrow(vec![any_arr_ty.clone()], Box::new(TcType::Con(TcCon::Any))));
        env.insert_mono("cdr".into(), TcType::Arrow(vec![any_arr_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("cons".into(), TcType::Arrow(vec![TcType::Con(TcCon::Any), any_arr_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("append".into(), TcType::Arrow(vec![any_arr_ty.clone(), any_arr_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("array".into(), TcType::Arrow(vec![any_ty.clone(), any_ty.clone(), any_ty.clone()], Box::new(any_arr_ty.clone())));
        // HOFs on heap arrays (emitter: call_list.rs)
        env.insert_mono("map".into(), TcType::Arrow(vec![any_ty.clone(), any_arr_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("filter".into(), TcType::Arrow(vec![any_ty.clone(), any_arr_ty.clone()], Box::new(any_arr_ty.clone())));
        env.insert_mono("reduce".into(), TcType::Arrow(vec![any_ty.clone(), any_ty.clone(), any_arr_ty.clone()], Box::new(TcType::Con(TcCon::Any))));

        // NEAR storage (emitter names)
        env.insert_mono(
            "near/storage_set".into(),
            TcType::Arrow(
                vec![str_ty.clone(), str_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        // near/storage_get : str → (opt str) — nil on miss, str on hit.
        // Forces callers through (default x fallback) / TS `??` to handle the miss.
        env.insert_mono(
            "near/storage_get".into(),
            TcType::Arrow(
                vec![str_ty.clone()],
                Box::new(TcType::Con(TcCon::Opt(Box::new(str_ty.clone())))),
            ),
        );
        env.insert_mono(
            "near/storage_has".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(bool_ty.clone())),
        );
        env.insert_mono(
            "near/storage_remove".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(nil_ty.clone())),
        );
        // NEAR numeric-keyed storage (8-byte LE i64 keys — gas-efficient)
        env.insert_mono(
            "near/store_num".into(),
            TcType::Arrow(
                vec![int_ty.clone(), int_ty.clone()],
                Box::new(nil_ty.clone()),
            ),
        );
        env.insert_mono(
            "near/load_num".into(),
            TcType::Arrow(vec![int_ty.clone()], Box::new(int_ty.clone())),
        );
        // near/return: str → nil (returns nil after setting return value)
        env.insert_mono(
            "near/return".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(nil_ty.clone())),
        );
        // near/return_str: str → any (terminates execution, return type is 'any' as escape hatch)
        env.insert_mono("near/return_str".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        env.insert_mono("near/store-bytes".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(nil_ty.clone())));
        env.insert_mono("near/load-bytes".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())));
        env.insert_mono("near/predecessor_account_id".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        env.insert_mono("near/current_account_id".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        env.insert_mono("near/signer_to_buf".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        env.insert_mono("near/write_amount".into(), TcType::Arrow(vec![int_ty.clone()], Box::new(nil_ty.clone())));
        env.insert_mono("near/block_index".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        env.insert_mono("near/block_timestamp".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        env.insert_mono("near/ed25519_verify".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone(), str_ty.clone()], Box::new(int_ty.clone())));
        env.insert_mono("hex-encode".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())));

        // Dict builtins (string-keyed flat array)
        let dict_ty = TcType::Con(TcCon::List(Box::new(any_ty.clone()))); // dicts are tagged arrays
        env.insert_mono(
            "dict".into(),
            TcType::Arrow(vec![], Box::new(dict_ty.clone())),
        ); // variadic — type checker just accepts any arity
        env.insert_mono(
            "dict/get".into(),
            TcType::Arrow(
                vec![dict_ty.clone(), str_ty.clone()],
                Box::new(any_ty.clone()),
            ),
        );
        env.insert_mono(
            "dict/set".into(),
            TcType::Arrow(
                vec![dict_ty.clone(), str_ty.clone(), any_ty.clone()],
                Box::new(dict_ty.clone()),
            ),
        );
        env.insert_mono(
            "dict/has?".into(),
            TcType::Arrow(
                vec![dict_ty.clone(), str_ty.clone()],
                Box::new(TcType::Con(TcCon::Bool)),
            ),
        );
        env.insert_mono(
            "dict/keys".into(),
            TcType::Arrow(
                vec![dict_ty.clone()],
                Box::new(TcType::Con(TcCon::List(Box::new(str_ty.clone())))),
            ),
        );
        env.insert_mono(
            "dict/vals".into(),
            TcType::Arrow(
                vec![dict_ty.clone()],
                Box::new(TcType::Con(TcCon::List(Box::new(any_ty.clone())))),
            ),
        );

        // Borsh builtins — variadic (schema name + field values)
        // borsh-serialize: str → any* → nil (serializes fields per schema, returns nil after value_return)
        env.insert_mono("borsh-serialize".into(), TcType::Arrow(vec![str_ty.clone(), any_ty.clone()], Box::new(nil_ty.clone())));
        // borsh-deserialize: str → str → any (takes schema name + bytes, returns tagged value/array)
        env.insert_mono("borsh-deserialize".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(any_ty.clone())));

        // str : variadic string builder (accepts 1+ args)
        let any_b = TcType::Var(1);
        env.insert(
            "str".to_string(),
            Scheme {
                vars: vec![1],
                ty: TcType::Arrow(
                    vec![any_b.clone(), any_b],
                    Box::new(TcType::Con(TcCon::Str)),
                ),
            },
        );

        // ── u128 builtins (string-based decimal values) ──
        // Arithmetic: str → str → str
        for name in &["u128/add", "u128/sub", "u128/mul", "u128/div", "u128/mod"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(str_ty.clone())),
            );
        }
        // Comparisons: str → str → bool
        for name in &["u128/lt", "u128/gt", "u128/eq"] {
            env.insert_mono(
                name.to_string(),
                TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(bool_ty.clone())),
            );
        }
        // u128/from-i64 : int → str
        env.insert_mono(
            "u128/from-i64".into(),
            TcType::Arrow(vec![int_ty], Box::new(str_ty.clone())),
        );
        // u128/to-i64 : str → int
        env.insert_mono(
            "u128/to-i64".into(),
            TcType::Arrow(vec![str_ty.clone()], Box::new(TcType::Con(TcCon::Int))),
        );
        // u128/is-zero : str → bool
        env.insert_mono(
            "u128/is-zero".into(),
            TcType::Arrow(vec![str_ty], Box::new(bool_ty)),
        );

        env.near_wildcard = true;
        env
    }
}

impl std::fmt::Display for TcType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TcType::Var(id) => write!(f, "'t{}", id),
            TcType::Con(con) => write!(f, "{}", con),
            TcType::Arrow(args, ret) => {
                if args.len() == 1 {
                    write!(f, "({} → {})", args[0], ret)
                } else {
                    let arg_strs: Vec<String> = args.iter().map(|a| a.to_string()).collect();
                    write!(f, "({} → {})", arg_strs.join(" → "), ret)
                }
            }
            TcType::Forall(vars, ty) => {
                let var_strs: Vec<String> = vars.iter().map(|v| format!("'t{}", v)).collect();
                write!(f, "(∀ {} {})", var_strs.join(" "), ty)
            }
        }
    }
}

impl std::fmt::Display for TcCon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TcCon::Nil => write!(f, "nil"),
            TcCon::Bool => write!(f, "bool"),
            TcCon::Int => write!(f, "int"),
            TcCon::Float => write!(f, "float"),
            TcCon::Num => write!(f, "num"),
            TcCon::Str => write!(f, "str"),
            TcCon::Sym => write!(f, "sym"),
            TcCon::List(t) => write!(f, "(list {})", t),
            TcCon::Map(k, v) => write!(f, "(map {} {})", k, v),
            TcCon::Tuple(ts) => {
                let s: Vec<String> = ts.iter().map(|t| t.to_string()).collect();
                write!(f, "(tuple {})", s.join(" "))
            }
            TcCon::Opt(t) => write!(f, "(opt {})", t),
            TcCon::Ptr => write!(f, "ptr"),
            TcCon::Any => write!(f, "any"),
        }
    }
}
