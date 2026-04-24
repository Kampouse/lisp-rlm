use std::collections::{BTreeMap, HashMap};

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
(define empty? (lambda (lst) (if (nil? lst) true (= (len lst) 0))))
(define map (lambda (f lst) (if (empty? lst) (list) (cons (f (car lst)) (map f (cdr lst))))))
(define filter (lambda (pred lst) (if (empty? lst) (list) (if (pred (car lst)) (cons (car lst) (filter pred (cdr lst))) (filter pred (cdr lst))))))
(define reduce (lambda (f init lst) (if (empty? lst) init (reduce f (f init (car lst)) (cdr lst)))))
(define find (lambda (pred lst) (if (empty? lst) nil (if (pred (car lst)) (car lst) (find pred (cdr lst))))))
(define some (lambda (pred lst) (if (empty? lst) false (if (pred (car lst)) true (some pred (cdr lst))))))
(define every (lambda (pred lst) (if (empty? lst) true (if (pred (car lst)) (every pred (cdr lst)) false))))
(define reverse (lambda (lst) (if (empty? lst) (list) (loop ((acc (list)) (cur lst)) (if (empty? cur) acc (recur (cons (car cur) acc) (cdr cur)))))))
(define sort (lambda (lst) (if (empty? lst) (list) (if (empty? (cdr lst)) lst (let ((pivot (car lst)) (rest (cdr lst))) (append (sort (filter (lambda (x) (< x pivot)) rest)) (cons pivot (sort (filter (lambda (x) (>= x pivot)) rest)))))))))
(define range (lambda (start end) (if (>= start end) (list) (cons start (range (+ start 1) end)))))
(define zip (lambda (a b) (if (or (empty? a) (empty? b)) (list) (cons (list (car a) (car b)) (zip (cdr a) (cdr b))))))
"#;

const STDLIB_STRING: &str = r#"
(define str-join (lambda (sep lst) (if (or (nil? lst) (= (len lst) 0)) "" (if (nil? (cdr lst)) (car lst) (str-concat (car lst) (str-concat sep (str-join sep (cdr lst))))))))
(define str-replace (lambda (s old new) (str-join new (str-split s old))))
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
/// Internally the bindings are stored in a `Vec<(String, LispVal)>` with an
/// accompanying `HashMap<String, usize>` index for O(1) lookups by name.
/// Lexical scoping is achieved by recording the binding-vector length before
/// entering a scope and calling [`Env::truncate`] on exit.
///
/// # Execution budget
///
/// The two public fields [`eval_count`](Env::eval_count) and
/// [`eval_budget`](Env::eval_budget) together implement an execution-budget
/// mechanism: every call to [`lisp_eval`](crate::lisp_eval) increments
/// `eval_count`; if it exceeds `eval_budget` the evaluator returns an error.
/// Set `eval_budget` to `0` to disable the limit.
#[derive(Clone, Debug)]
pub struct Env {
    bindings: Vec<(String, LispVal)>,
    index: HashMap<String, usize>,
    /// Eval iteration counter for execution budget
    pub eval_count: u64,
    /// Maximum allowed eval iterations (0 = unlimited)
    pub eval_budget: u64,
}

impl Env {
    /// Create a new empty environment with [`DEFAULT_EVAL_BUDGET`].
    pub fn new() -> Self {
        Env {
            bindings: Vec::new(),
            index: HashMap::new(),
            eval_count: 0,
            eval_budget: DEFAULT_EVAL_BUDGET,
        }
    }

    /// Create an environment pre-populated with the given bindings.
    pub fn from_vec(bindings: Vec<(String, LispVal)>) -> Self {
        let mut index = HashMap::new();
        for (i, (name, _)) in bindings.iter().enumerate() {
            index.insert(name.clone(), i);
        }
        Env { bindings, index, eval_count: 0, eval_budget: DEFAULT_EVAL_BUDGET }
    }

    /// Insert or overwrite a binding, shadowing any previous binding with the
    /// same name.
    pub fn push(&mut self, name: String, val: LispVal) {
        let idx = self.bindings.len();
        self.bindings.push((name.clone(), val));
        self.index.insert(name, idx);
    }

    /// Look up a binding by name, returning `None` if not found.
    pub fn get(&self, name: &str) -> Option<&LispVal> {
        let idx = *self.index.get(name)?;
        Some(&self.bindings[idx].1)
    }

    /// Returns `true` if a binding with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
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

    /// Truncate the binding vector to `new_len`, removing any bindings added
    /// after that point.  The name-index is updated so that shadowed bindings
    /// are correctly restored.
    pub fn truncate(&mut self, new_len: usize) {
        for i in (new_len..self.bindings.len()).rev() {
            let name = &self.bindings[i].0;
            if let Some(idx) = self.index.get(name) {
                if *idx >= new_len {
                    let mut found = false;
                    for j in (0..new_len).rev() {
                        if self.bindings[j].0 == *name {
                            self.index.insert(name.clone(), j);
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        self.index.remove(name);
                    }
                }
            }
        }
        self.bindings.truncate(new_len);
    }

    /// Look up a binding by name and return a mutable reference, or `None`.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut LispVal> {
        let idx = *self.index.get(name)?;
        Some(&mut self.bindings[idx].1)
    }

    /// Iterate over all bindings in insertion order.
    pub fn iter(&self) -> std::slice::Iter<'_, (String, LispVal)> {
        self.bindings.iter()
    }

    /// Iterate over all bindings mutably in insertion order.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, (String, LispVal)> {
        self.bindings.iter_mut()
    }

    /// Consume the environment and return the inner binding vector.
    pub fn into_bindings(self) -> Vec<(String, LispVal)> {
        self.bindings
    }

    /// Remove all bindings, leaving the environment empty.
    pub fn clear(&mut self) {
        self.bindings.clear();
        self.index.clear();
    }
}

impl std::ops::Index<usize> for Env {
    type Output = (String, LispVal);
    fn index(&self, index: usize) -> &Self::Output {
        &self.bindings[index]
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
        closed_env: Box<Vec<(String, LispVal)>>,
    },
    /// Macro (like `Lambda` but receives *unevaluated* arguments).
    Macro {
        params: Vec<String>,
        rest_param: Option<String>,
        body: Box<LispVal>,
        closed_env: Box<Vec<(String, LispVal)>>,
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
