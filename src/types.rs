use std::collections::{BTreeMap, HashMap};

pub const DEFAULT_EVAL_GAS_LIMIT: u64 = 1_000_000;

#[inline]
pub fn check_gas(gas: &mut u64) -> Result<(), String> {
    if *gas == 0 {
        return Err("out of gas".into());
    }
    *gas -= 1;
    Ok(())
}

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

pub fn get_stdlib_code(name: &str) -> Option<&'static str> {
    match name {
        "math" => Some(MATH_STDLIB),
        "list" => Some(STDLIB_LIST),
        "string" => Some(STDLIB_STRING),
        "crypto" => Some(STDLIB_CRYPTO),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct Env {
    bindings: Vec<(String, LispVal)>,
    index: HashMap<String, usize>,
}

impl Env {
    pub fn new() -> Self {
        Env {
            bindings: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn from_vec(bindings: Vec<(String, LispVal)>) -> Self {
        let mut index = HashMap::new();
        for (i, (name, _)) in bindings.iter().enumerate() {
            index.insert(name.clone(), i);
        }
        Env { bindings, index }
    }

    pub fn push(&mut self, name: String, val: LispVal) {
        let idx = self.bindings.len();
        self.bindings.push((name.clone(), val));
        self.index.insert(name, idx);
    }

    pub fn get(&self, name: &str) -> Option<&LispVal> {
        let idx = *self.index.get(name)?;
        Some(&self.bindings[idx].1)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }
    #[allow(clippy::len_without_is_empty)]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

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

    pub fn get_mut(&mut self, name: &str) -> Option<&mut LispVal> {
        let idx = *self.index.get(name)?;
        Some(&mut self.bindings[idx].1)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (String, LispVal)> {
        self.bindings.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, (String, LispVal)> {
        self.bindings.iter_mut()
    }

    pub fn into_bindings(self) -> Vec<(String, LispVal)> {
        self.bindings
    }

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

#[derive(Clone, Debug, PartialEq)]
pub enum LispVal {
    Nil,
    Bool(bool),
    Num(i64),
    Float(f64),
    Str(String),
    Sym(String),
    List(Vec<LispVal>),
    Lambda {
        params: Vec<String>,
        rest_param: Option<String>,
        body: Box<LispVal>,
        closed_env: Box<Vec<(String, LispVal)>>,
    },
    Recur(Vec<LispVal>),
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
