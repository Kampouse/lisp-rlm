//! Core types for the Hindley-Milner–inspired type checker.
//!
//! Types are inferred bidirectionally for pure expressions.
//! No effects, no mutation — just pure data transformations.

use std::collections::HashMap;
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
    Forall(Vec<TVarId>, Box<TcType>),
}

/// Type constructors.
#[derive(Clone, Debug, PartialEq)]
pub enum TcCon {
    Nil,
    Bool,
    Int,
    Float,
    Num,       // int | float (for polymorphic arithmetic)
    Str,
    Sym,
    List(Box<TcType>),     // homogeneous list
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
}

impl TcEnv {
    pub fn new() -> Self {
        TcEnv {
            bindings: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, scheme: Scheme) {
        self.bindings.insert(name, scheme);
    }

    pub fn get(&self, name: &str) -> Option<&Scheme> {
        self.bindings.get(name)
    }

    /// Insert a monomorphic (no quantified vars) binding.
    pub fn insert_mono(&mut self, name: String, ty: TcType) {
        self.insert(name, Scheme { vars: vec![], ty });
    }

    /// Standard pure builtins with their type schemes.
    pub fn with_pure_builtins() -> Self {
        let mut env = TcEnv::new();

        // Arithmetic: num → num → num
        for name in &["+", "-", "*", "/", "mod", "min", "max"] {
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
                ty: TcType::Arrow(
                    vec![a2.clone(), list_a2.clone()],
                    Box::new(list_a2.clone()),
                ),
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
                ty: TcType::Arrow(
                    vec![list_a5.clone(), list_a5.clone()],
                    Box::new(list_a5),
                ),
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
                        TcType::Arrow(
                            vec![a8.clone(), b8.clone()],
                            Box::new(a8.clone()),
                        ),
                        a8.clone(),
                        TcType::Con(TcCon::List(Box::new(b8))),
                    ],
                    Box::new(a8),
                ),
            },
        );

        // Predicates
        for name in &["nil?", "null?", "list?", "pair?", "number?", "string?", "bool?", "boolean?", "empty?"] {
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
