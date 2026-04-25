use std::collections::BTreeMap;

const MATH_STDLIB: &str = r#"
(define abs (lambda (x) (if (< x 0) (- 0 x) x)))
(define min (lambda (a b) (if (< a b) a b)))
(define max (lambda (a b) (if (> a b) a b)))
(define even? (lambda (n) (= (mod n 2) 0)))
(define odd? (lambda (n) (= (mod n 2) 1)))
(define gcd (lambda (a b) (if (= b 0) (abs a) (gcd b (mod a b)))))
(define square (lambda (x) (* x x)))
(define identity (lambda (x) x))
(define pow (lambda (base exp) (if (<= exp 0) 1 (* base (pow base (- exp 1))))))
(define sqrt (lambda (n) (if (< n 0) nil (if (< n 2) n (loop ((x (/ n 2))) (let ((x1 (/ (+ x (/ n x)) 2))) (if (>= x1 x) x (recur x1))))))))
(define lcm (lambda (a b) (if (or (= a 0) (= b 0)) 0 (/ (* (abs a) (abs b)) (gcd a b)))))
"#;

const STDLIB_LIST: &str = r#"
(define (identity x) x)
(define (constantly x) (lambda (_) x))
(define (compose f g) (lambda (x) (f (g x))))
(define (flip f) (lambda (a b) (f b a)))
(define (flatten-once lst) (reduce append (list) lst))
"#;

const STDLIB_STRING: &str = r#"
(define str-join (lambda (sep lst) (if (or (nil? lst) (= (len lst) 0)) "" (if (nil? (cdr lst)) (car lst) (str-concat (car lst) (str-concat sep (str-join sep (cdr lst))))))))
(define str-repeat (lambda (s n) (if (<= n 0) "" (if (= n 1) s (str-concat s (str-repeat s (- n 1)))))))
(define str-pad-left (lambda (s len pad) (if (>= (str-length s) len) s (str-pad-left (str-concat pad s) len pad))))
(define str-pad-right (lambda (s len pad) (if (>= (str-length s) len) s (str-pad-right (str-concat s pad) len pad))))
"#;

const STDLIB_CRYPTO: &str = r#"
(define hash/sha256-bytes (lambda (s) (sha256 s)))
(define hash/keccak256-bytes (lambda (s) (keccak256 s)))
"#;

/// Look up a standard-library module by name and return its Lisp source code.
///
/// # Known modules
///
/// | Name      | Contents                                       |
/// |-----------|------------------------------------------------|
/// | `"math"`  | `abs`, `min`, `max`, `even?`, `odd?`, `gcd`, `square`, `identity`, `pow`, `sqrt`, `lcm` |
/// | `"list"`  | `empty?`, `map`, `filter`, `reduce`, `find`, `some`, `every`, `reverse`, `sort`, `range`, `zip` |
/// | `"string"`| `str-join`, `str-replace`, `str-repeat`, `str-pad-left`, `str-pad-right` |
/// | `"crypto"`| `hash/sha256-bytes`, `hash/keccak256-bytes` |
///
/// Returns `None` for unknown module names.
pub fn get_stdlib_code(name: &str) -> Option<&'static str> {
    match name {
        "math" => Some(MATH_STDLIB),
        "list" => Some(STDLIB_LIST),
        "string" => Some(STDLIB_STRING),
        "crypto" => Some(STDLIB_CRYPTO),
        _ => None,
    }
}

/// Default maximum number of eval iterations before budget exceeded.
/// Prevents runaway infinite loops (e.g. tail-recursive functions with no base case).
pub const DEFAULT_EVAL_BUDGET: u64 = 10_000_000;

/// A scoped environment that maps variable names to [`LispVal`] bindings.
///
/// Uses `im::HashMap` for O(1) clone via structural sharing, making
/// save/restore patterns cheap. Bindings are mutated in-place via
/// `insert_mut`.
#[derive(Clone)]
pub struct Env {
    bindings: im::HashMap<String, LispVal>,
}

impl Env {
    /// Create a new empty environment with the standard aliases (`t`, `true`, `false`).
    pub fn new() -> Self {
        let mut env = Env {
            bindings: im::HashMap::new(),
        };
        // Common aliases
        env.insert_mut("t".to_string(), LispVal::Bool(true));
        env.insert_mut("true".to_string(), LispVal::Bool(true));
        env.insert_mut("false".to_string(), LispVal::Bool(false));
        env
    }

    /// Create an environment pre-populated with the given bindings.
    pub fn from_vec(bindings: Vec<(String, LispVal)>) -> Self {
        let mut env = Env {
            bindings: im::HashMap::new(),
        };
        for (name, val) in bindings {
            env.insert_mut(name, val);
        }
        env
    }

    /// Insert or overwrite a binding in-place.
    pub fn insert_mut(&mut self, name: String, val: LispVal) {
        self.bindings.insert(name, val);
    }

    /// Alias for `insert_mut` — kept for backward compatibility.
    pub fn push(&mut self, name: String, val: LispVal) {
        self.insert_mut(name, val);
    }

    /// Look up a binding by name, returning `None` if not found.
    pub fn get(&self, name: &str) -> Option<&LispVal> {
        self.bindings.get(name)
    }

    /// Returns `true` if a binding with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.bindings.contains_key(name)
    }

    /// Number of bindings currently in the environment.
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns `true` if the environment has no bindings.
    #[allow(clippy::len_without_is_empty)]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Look up a binding by name and return a mutable reference, or `None`.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut LispVal> {
        self.bindings.get_mut(name)
    }

    /// Iterate over all bindings.
    pub fn iter(&self) -> im::hashmap::Iter<'_, String, LispVal> {
        self.bindings.iter()
    }

    /// Remove all bindings, leaving the environment empty.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Take an O(1) snapshot of the current bindings (structural sharing).
    pub fn snapshot(&self) -> im::HashMap<String, LispVal> {
        self.bindings.clone()
    }

    /// Restore bindings from a previous snapshot.
    pub fn restore(&mut self, snap: im::HashMap<String, LispVal>) {
        self.bindings = snap;
    }

    /// Consume the environment and return the bindings as a Vec.
    pub fn into_bindings(self) -> Vec<(String, LispVal)> {
        self.bindings.into_iter().collect()
    }
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("bindings_count", &self.bindings.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// EvalState — mutable counters and runtime state separated from bindings
// ---------------------------------------------------------------------------

/// Mutable evaluation state: counters, budgets, LLM provider, etc.
///
/// Separated from [`Env`] so that bindings can be cheaply cloned via
/// structural sharing while mutable state remains independent.
pub struct EvalState {
    /// Eval iteration counter for execution budget
    pub eval_count: u64,
    /// Maximum allowed eval iterations (0 = unlimited)
    pub eval_budget: u64,
    /// Stack of env snapshots for snapshot/rollback
    pub snapshots: Vec<im::HashMap<String, LispVal>>,
    /// Persistent agent state (survives snapshots)
    pub rlm_state: im::OrdMap<String, LispVal>,
    /// Cumulative tokens across all LLM calls
    pub tokens_used: usize,
    /// Number of LLM API calls made
    pub llm_calls: usize,
    /// Current sub-rlm call depth (max 5)
    pub rlm_depth: usize,
    /// RLM iteration counter (incremented by the rlm builtin each iteration)
    pub rlm_iteration: usize,
    /// Pluggable LLM provider — when `None`, LLM builtins return an error.
    pub llm_provider: Option<Box<dyn crate::eval::llm_provider::LlmProvider>>,
}

impl EvalState {
    /// Create a new `EvalState` with [`DEFAULT_EVAL_BUDGET`].
    pub fn new() -> Self {
        EvalState {
            eval_count: 0,
            eval_budget: DEFAULT_EVAL_BUDGET,
            snapshots: Vec::new(),
            rlm_state: im::OrdMap::new(),
            tokens_used: 0,
            llm_calls: 0,
            rlm_depth: 0,
            rlm_iteration: 0,
            llm_provider: None,
        }
    }

    /// Set the LLM provider for this state.
    pub fn set_llm_provider(&mut self, provider: Box<dyn crate::eval::llm_provider::LlmProvider>) {
        self.llm_provider = Some(provider);
    }
}

impl Clone for EvalState {
    fn clone(&self) -> Self {
        EvalState {
            eval_count: self.eval_count,
            eval_budget: self.eval_budget,
            snapshots: self.snapshots.clone(),
            rlm_state: self.rlm_state.clone(),
            tokens_used: self.tokens_used,
            llm_calls: self.llm_calls,
            rlm_depth: self.rlm_depth,
            rlm_iteration: self.rlm_iteration,
            llm_provider: None, // providers are not cloned
        }
    }
}

impl std::fmt::Debug for EvalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvalState")
            .field("eval_count", &self.eval_count)
            .field("eval_budget", &self.eval_budget)
            .field("tokens_used", &self.tokens_used)
            .field("llm_calls", &self.llm_calls)
            .field("rlm_depth", &self.rlm_depth)
            .field("rlm_iteration", &self.rlm_iteration)
            .field("llm_provider", &self.llm_provider.is_some())
            .finish()
    }
}

/// A value in the Lisp interpreter.
///
/// Every datum that can appear during evaluation — including intermediate
/// control-flow markers — is represented as a `LispVal`.
///
/// # Variants
///
/// | Variant   | Meaning |
/// |-----------|---------|
/// | [`Nil`]       | The unit / null value. |
/// | [`Bool`]      | Boolean `true` / `false`. |
/// | [`Num`]       | 64-bit signed integer. |
/// | [`Float`]     | 64-bit floating-point number. |
/// | [`Str`]       | Heap-allocated string. |
/// | [`Sym`]       | Symbol (variable / operator name). |
/// | [`List`]      | Heterogeneous list of values. |
/// | [`Lambda`]    | First-class closure with captured environment. |
/// | [`Macro`]     | Macro: receives unevaluated args, returns code to evaluate. |
/// | [`Recur`]     | Control-flow marker used by `loop`/`recur` (not user-visible). |
/// | [`Map`]       | String-keyed hash map (`BTreeMap<String, LispVal>`). |
///
/// [`Nil`]: LispVal::Nil
/// [`Bool`]: LispVal::Bool
/// [`Num`]: LispVal::Num
/// [`Float`]: LispVal::Float
/// [`Str`]: LispVal::Str
/// [`Sym`]: LispVal::Sym
/// [`List`]: LispVal::List
/// [`Lambda`]: LispVal::Lambda
/// [`Macro`]: LispVal::Macro
/// [`Recur`]: LispVal::Recur
/// [`Map`]: LispVal::Map
#[derive(Clone, Debug, PartialEq)]
pub enum LispVal {
    /// The unit / null value (`nil` in Lisp).
    Nil,
    /// Boolean value.
    Bool(bool),
    /// 64-bit signed integer.
    Num(i64),
    /// 64-bit floating-point number.
    Float(f64),
    /// Heap-allocated string.
    Str(String),
    /// Symbol — a variable or operator name, resolved at eval time.
    Sym(String),
    /// Heterogeneous list of Lisp values.
    List(Vec<LispVal>),
    /// First-class closure.
    ///
    /// - `params` — parameter names.
    /// - `rest_param` — optional rest-parameter that collects extra args.
    /// - `body` — the expression to evaluate.
    /// - `closed_env` — captured lexical environment at closure-creation time.
    Lambda {
        params: Vec<String>,
        rest_param: Option<String>,
        body: Box<LispVal>,
        /// Captured lexical environment — `Rc` so cloning a Lambda is O(1)
        /// instead of exponentially expensive when closures capture other closures.
        closed_env: std::sync::Arc<Vec<(String, LispVal)>>,
    },
    /// Macro (like `Lambda` but receives *unevaluated* arguments).
    Macro {
        params: Vec<String>,
        rest_param: Option<String>,
        body: Box<LispVal>,
        closed_env: std::sync::Arc<Vec<(String, LispVal)>>,
    },
    /// Control-flow marker emitted by `recur` inside a `loop` form.
    /// Carries the new binding values for the next iteration.
    Recur(Vec<LispVal>),
    /// String-keyed dictionary backed by a `BTreeMap`.
    Map(BTreeMap<String, LispVal>),
}

impl std::fmt::Display for LispVal {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LispVal::Nil => write!(f, "nil"),
            LispVal::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            LispVal::Num(n) => write!(f, "{}", n),
            LispVal::Float(fl) => {
                let s = format!("{:.10}", fl);
                let s = s.trim_end_matches('0');
                let s = s.trim_end_matches('.');
                write!(f, "{}", s)
            }
            LispVal::Str(s) => write!(f, "\"{}\"", s),
            LispVal::Sym(s) => write!(f, "{}", s),
            LispVal::List(vals) => {
                let parts: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
                write!(f, "({})", parts.join(" "))
            }
            LispVal::Lambda { params, .. } => {
                write!(f, "#<lambda ({})>", params.join(" "))
            }
            LispVal::Macro { params, .. } => {
                write!(f, "#<macro ({})>", params.join(" "))
            }
            LispVal::Recur(vals) => {
                let parts: Vec<String> = vals.iter().map(|v| v.to_string()).collect();
                write!(f, "#<recur ({})>", parts.join(" "))
            }
            LispVal::Map(m) => {
                let entries: Vec<String> =
                    m.iter().map(|(k, v)| format!("\"{}\": {}", k, v)).collect();
                write!(f, "{{{}}}", entries.join(", "))
            }
        }
    }
}
