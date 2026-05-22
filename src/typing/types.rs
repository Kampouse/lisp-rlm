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
        )
}

/// Known NEAR host function names (the part after "near/").
/// Derived from the HOST_FUNCS table in wasm_emit.rs.
const KNOWN_NEAR_FUNCS: &[&str] = &[
    "store", "load", "remove", "has_key",
    "storage_read", "storage_write", "storage_has_key", "storage_remove",
    "return", "return_str", "return_value", "value_return",
    "log",
    "input",
    "panic",
    "current_account_id", "account_id",
    "signer_account_id", "signer_public_key",
    "signer_account_pk",
    "predecessor_account_id", "predecessor",
    "attached_deposit",
    "block_index", "block_height", "block_timestamp",
    "ed25519_verify", "p256_verify",
    "sha256", "keccak256", "keccak512",
    "random_seed",
    "prepaid_gas", "used_gas",
    "promise_create", "promise_then", "promise_and", "promise_result",
    "promise_batch_create", "promise_batch_then",
    "promise_batch_action_create_account",
    "promise_batch_action_deploy_contract",
    "promise_batch_action_function_call",
    "promise_batch_action_transfer",
    "promise_batch_action_stake",
    "promise_batch_action_add_key_with_full_access",
    "promise_batch_action_add_key_with_function_call",
    "promise_batch_action_delete_key",
    "promise_batch_action_delete_account",
    "log_utf8", "log_utf16",
    "abort",
    "global_contract_set", "global_contract_status",
    // JSON convenience builtins
    "json_get_int", "json_get_str", "json_return_int", "json_return_str",
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
    Any, // escape hatch
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
        for name in &["+", "-", "*", "/", "mod", "min", "max", "wrap-add", "wrap-sub", "wrap-mul"] {
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
        env.insert_mono(
            "str-length".to_string(),
            TcType::Arrow(
                vec![TcType::Con(TcCon::Str)],
                Box::new(TcType::Con(TcCon::Int)),
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

        env
    }

    /// NEAR host function builtins. These are effectful, so we type them as
    /// returning `any` where the result could be anything. The point is to
    /// avoid "undefined variable" errors, not to enforce effect discipline.
    pub fn with_near_builtins() -> Self {
        let mut env = Self::with_pure_builtins();
        let str_ty = TcType::Con(TcCon::Str);
        let int_ty = TcType::Con(TcCon::Int);
        let bool_ty = TcType::Con(TcCon::Bool);
        let any_ty = TcType::Con(TcCon::Any);

        // near/input : () → str
        env.insert_mono("near/input".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/return_str : str → any (terminates)
        env.insert_mono("near/return_str".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/return_value : str → any
        env.insert_mono("near/return_value".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/storage_read : str → str
        env.insert_mono("near/storage_read".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())));
        // near/storage_write : str → str → any
        env.insert_mono("near/storage_write".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone()], Box::new(any_ty.clone())));
        // near/storage_has_key : str → bool
        env.insert_mono("near/storage_has_key".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(bool_ty.clone())));
        // near/storage_remove : str → any
        env.insert_mono("near/storage_remove".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/log : str → any
        env.insert_mono("near/log".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/account_id : () → str
        env.insert_mono("near/account_id".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/predecessor : () → str
        env.insert_mono("near/predecessor".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/signer_account_id : () → str
        env.insert_mono("near/signer_account_id".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/signer_public_key : () → str
        env.insert_mono("near/signer_public_key".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/attached_deposit : () → int
        env.insert_mono("near/attached_deposit".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        // near/block_timestamp : () → int
        env.insert_mono("near/block_timestamp".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        // near/block_height : () → int
        env.insert_mono("near/block_height".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        // near/ed25519_verify : str → str → str → int
        env.insert_mono("near/ed25519_verify".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone(), str_ty.clone()], Box::new(int_ty.clone())));
        // near/p256_verify : str → str → str → int
        env.insert_mono("near/p256_verify".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone(), str_ty.clone()], Box::new(int_ty.clone())));
        // near/sha256 : str → str
        env.insert_mono("near/sha256".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())));
        // near/keccak256 : str → str
        env.insert_mono("near/keccak256".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(str_ty.clone())));
        // near/random_seed : () → str
        env.insert_mono("near/random_seed".into(), TcType::Arrow(vec![], Box::new(str_ty.clone())));
        // near/prepaid_gas : () → int
        env.insert_mono("near/prepaid_gas".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        // near/used_gas : () → int
        env.insert_mono("near/used_gas".into(), TcType::Arrow(vec![], Box::new(int_ty.clone())));
        // near/value_return : str → any
        env.insert_mono("near/value_return".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/panic : str → any
        env.insert_mono("near/panic".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(any_ty.clone())));
        // near/promise_create : str → str → str → int → int → int
        env.insert_mono("near/promise_create".into(), TcType::Arrow(vec![str_ty.clone(), str_ty.clone(), str_ty.clone(), int_ty.clone(), int_ty.clone()], Box::new(int_ty.clone())));
        // near/promise_then : int → str → str → str → int → int → int
        env.insert_mono("near/promise_then".into(), TcType::Arrow(vec![int_ty.clone(), str_ty.clone(), str_ty.clone(), str_ty.clone(), int_ty.clone(), int_ty.clone()], Box::new(int_ty.clone())));
        // near/promise_and : int → int → int
        env.insert_mono("near/promise_and".into(), TcType::Arrow(vec![int_ty.clone(), int_ty.clone()], Box::new(int_ty.clone())));
        // near/promise_result : int → str
        env.insert_mono("near/promise_result".into(), TcType::Arrow(vec![int_ty.clone()], Box::new(str_ty.clone())));

        // String builtins used in NEAR contracts
        env.insert_mono("str-len".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(int_ty.clone())));
        env.insert_mono("str-slice".into(), TcType::Arrow(vec![str_ty.clone(), int_ty.clone(), int_ty.clone()], Box::new(str_ty.clone())));
        env.insert_mono("str-from-bytes".into(), TcType::Arrow(vec![TcType::Con(TcCon::List(Box::new(int_ty.clone())))], Box::new(str_ty.clone())));
        env.insert_mono("str-to-bytes".into(), TcType::Arrow(vec![str_ty.clone()], Box::new(TcType::Con(TcCon::List(Box::new(int_ty.clone()))))));

        // Dict builtins (string-keyed flat array)
        let dict_ty = TcType::Con(TcCon::List(Box::new(any_ty.clone()))); // dicts are tagged arrays
        env.insert_mono("dict".into(), TcType::Arrow(vec![], Box::new(dict_ty.clone()))); // variadic — type checker just accepts any arity
        env.insert_mono("dict/get".into(), TcType::Arrow(vec![dict_ty.clone(), str_ty.clone()], Box::new(any_ty.clone())));
        env.insert_mono("dict/set".into(), TcType::Arrow(vec![dict_ty.clone(), str_ty.clone(), any_ty.clone()], Box::new(dict_ty.clone())));
        env.insert_mono("dict/has?".into(), TcType::Arrow(vec![dict_ty.clone(), str_ty.clone()], Box::new(TcType::Con(TcCon::Bool))));
        env.insert_mono("dict/keys".into(), TcType::Arrow(vec![dict_ty.clone()], Box::new(TcType::Con(TcCon::List(Box::new(str_ty.clone()))))));
        env.insert_mono("dict/vals".into(), TcType::Arrow(vec![dict_ty.clone()], Box::new(TcType::Con(TcCon::List(Box::new(any_ty.clone()))))));

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
            TcCon::Any => write!(f, "any"),
        }
    }
}
