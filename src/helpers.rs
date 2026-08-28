use crate::types::LispVal;

/// All builtin function names recognized by the VM and tree-walker.
pub const BUILTIN_NAMES: &[&str] = &[
    "+", "-", "*", "/", "mod",
    "=", "==", "!=", "/=", "<", ">", "<=", ">=",
    "list", "vec", "vec-nth", "vec-assoc", "vec-len", "vec-conj", "vec-contains?", "vec-slice", "vec?", "car", "cdr", "cons", "len", "append", "nth",
    "str-cat", "str-concat", "str-contains", "to-string", "str-length",
    "str-substring", "str-split", "str-split-exact", "str-trim",
    "str-index-of", "str-upcase", "str-downcase",
    "str-starts-with", "str-ends-with", "str=", "str!=",
    "json-decode-bytes",
    "nil?",
    "list?",
    "number?",
    "string?",
    "map?",
    "bool?",
    "to-float",
    "to-int",
    "to-num",
    "type?",
    "u64-and", "u64-or", "u64-xor", "u64-shr", "u64-shl", "u64-not", "u64-mul-hi",
    "dict",
    "dict/get",
    "dict/set",
    "dict/has?",
    "dict/keys",
    "dict/vals",
    "dict/remove",
    "dict/merge",
    "json-array-len",
    "json-array-get",
    "json-parse",
    "json-get",
    "json-get-in",
    "json-build",
    "from-json",
    "json/get",
    "error",
    "empty?",
    "range",
    "reverse",
    "sort",
    "zip",
    "map",
    "filter",
    "reduce",
    "find",
    "some",
    "every",
    "print",
    "println",
    "file/read",
    "file/write",
    "file/exists?",
    "file/list",
    "env/get",
    "rlm/signature",
    "rlm/format-prompt",
    "rlm/trace",
    "rlm/config",
    "write-file",
    "read-file",
    "append-file",
    "file-exists?",
    "shell",
    "shell-bg",
    "shell-kill",
    "http-get",
    "http-post",
    "http-get-json",
    "llm",
    "llm-code",
    "contract-check-param",
    "contract-check-return",
    "contract-wrap",
    "check",
    "check!",
    "matches?",
    "valid-type?",
    "type-of",
    "defschema",
    "validate",
    "schema",
    "infer-type",
    "snapshot",
    "rollback",
    "rollback-to",
    "rlm",
    "read-all",
    "load-file",
    "sub-rlm",
    "rlm-tokens",
    "rlm-calls",
    "show-vars",
    "str-chunk",
    "str-join",
    "llm-batch",
    "show-context",
    "final",
    "final-var",
    // Tier 1: Scheme stdlib
    "abs",
    "min",
    "max",
    "floor",
    "ceiling",
    "round",
    "sqrt",
    "number->string",
    "zero?",
    "positive?",
    "negative?",
    "even?",
    "odd?",
    "equal?",
    "eq?",
    "symbol=?",
    "procedure?",
    "symbol?",
    "symbol->string",
    "string->symbol",
    "member",
    "assoc",
    "partition",
    "fold-left",
    "fold-right",
    "for-each",
    "cons*",
    "string->list",
    "list->string",
    "string<?",
    "string->number",
    "apply",
    "eval",
    "delete-file",
    // R7RS aliases
    "null?",
    "boolean?",
    "pair?",
    "length",
    "eqv?",
    "boolean=?",
    "string-length",
    "string-append",
    "substring",
    "string-contains",
    "string-upcase",
    "string-downcase",
    "string-copy",
    "string=?",
    "string>?",
    "string<=?",
    "string>=?",
    "exact-integer-sqrt",
    "exp",
    "rational?",
    "string-ci=?",
    "string-ci<?",
    "string-ci>?",
    "string-ci<=?",
    "string-ci>=?",
    "string-foldcase",
    "string-ref",
    "string-replace",
    "display",
    "write",
    "newline",
    "modulo",
    "remainder",
    "quotient",
    "expt",
    "atan",
    "list-ref",
    "list-tail",
    "list-copy",
    "values",
    "call-with-values",
    "fork",
    "fork-exec",
    "mark-pure",
    "memoize",
    "par-map",
    "par-filter",
    "snapshot",
    "rollback",
    "fmt",
    "inspect",
    "to-json",
    "str-replace",
    "force",
    "make-promise",
    "promise?",
    "macro?",
    "delay",
    "try-catch-impl",
    "define-values",
    "let-values",
    "let*-values",
    "macroexpand",
    "case-lambda",
    "make-case-lambda",
    "assv",
    "assq",
    "memv",
    "memq",
    "char->integer",
    "integer->char",
    "exact",
    "inexact",
    "exact->inexact",
    "inexact->exact",
    // Runtime
    "now",
    "elapsed",
    "sleep",
    "save-state",
    "load-state",
    "doc",
    "memoize",
    // Additional builtins for bytecode VM
    "inc",
    "dec",
    "first",
    "rest",
    "int?",
    "float?",
    "take",
    "drop",
    "last",
    "butlast",
    "pow",
    "dict-ref",
    "dict-set",
    "string-suffix?",
    "string-prefix?",
    "str->num",
    "tag-test", "get-field",
    // NEAR mock builtins
    "near-reset", "near-promises", "near-register", "near-register-source", "near-contracts",
    "near/store", "near/load", "near/remove", "near/has", "near/has_key", "near/kv", "near/storage_usage",
    "near/storage_set", "near/storage_get", "near/storage_has", "near/storage_remove",
    "near/storage_write", "near/storage_read", "near/storage_has_key",
    "near/account_id", "near/signer_id", "near/predecessor_id", "near/block_height", "near/block_timestamp",
    "near/attached_deposit", "near/prepaid_gas", "near/random_seed", "near/input",
    "near/return", "near/return_str", "near/return_json", "near/json_return_str",
    "near/panic", "near/abort", "near/assert", "near/require",
    "near/log", "near/log_str",
    "near/promise_create", "near/promise_then", "near/promise_and",
    "near/promise_results_count", "near/promise_result", "near/promise_return",
    "near/call", "near/transfer", "near/transfer_u128", "near/deploy_contract",
    "near/batch_create", "near/batch_action_create", "near/batch_action_function_call",
    "near/batch_action_transfer", "near/batch_action_deploy_contract", "near/batch_action_stake",
    "near/batch_action_add_key", "near/batch_action_delete_key", "near/batch_action_delete_account",
    "near/batch_action_delete_key_full_access", "near/batch_action_delete_key_function_call",
    "near/batch_action_delete_key_with_access_key",
    "near/batch_commit",
    "near/keccak256", "near/keccak512", "near/sha256",
    "near/ed25519_verify", "near/ecdsa_verify", "near/hmac_sha256",
    "near/iter_prefix", "near/iter_range", "near/iter_next",
    "near/config", "near/current_account_id", "near/signer_account_id", "near/predecessor_account_id",
    // u128 builtins (string-based decimal values, NEAR yocto scale)
    "u128/add", "u128/sub", "u128/mul", "u128/div", "u128/mod",
    "u128/lt", "u128/gt", "u128/eq",
    "u128/from-i64", "u128/to-i64", "u128/is-zero",
    // u128 address family (raw limbs in linear memory — wasm_emit/call_u128.rs;
    // names that don't collide with the string-based spec, commit 4b1403e)
    "u128/store", "u128/load", "u128/load_high", "u128/new",
    "u128/from_yocto", "u128/to_str", "u128/from_str",
    "u128/fit_i64", "u128/checked_to_i64", "u128/to_i64", "u128/from_i64",
    "tag-test",
    "get-field",
    "sha256",
    "tagged-hash",
    "schnorr-verify",
];

pub fn is_builtin_name(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name)
}

/// Truthiness for the interpreter (bytecode VM + dispatch builtins).
///
/// Falsy: `nil`, `#f`, integer `0`, float `0.0`.
/// Truthy: everything else — including `""` and `()` (deliberate: only
/// numeric zero flips; empty string/list stay truthy. GAPS.md round 3, fix 1).
///
/// NOTE: the WASM runtime has its own `$is_truthy` WAT fragment (src/runtime.rs)
/// with nil/bool-only semantics. Reconcile when the wasm branch-on-zero
/// semantics are unified (see GAPS.md "wasm truthiness divergence").
pub fn is_truthy(v: &LispVal) -> bool {
    !matches!(
        v,
        LispVal::Nil
            | LispVal::Bool(false)
            | LispVal::Num(0)
            | LispVal::Float(0.0)
    )
}

/// Deep structural equality (Scheme's equal?).
pub fn lisp_equal(a: &LispVal, b: &LispVal) -> bool {
    match (a, b) {
        (LispVal::Num(x), LispVal::Num(y)) => x == y,
        (LispVal::Float(x), LispVal::Float(y)) => x == y,
        (LispVal::Float(x), LispVal::Num(y)) => *x == *y as f64,
        (LispVal::Num(x), LispVal::Float(y)) => *x as f64 == *y,
        (LispVal::Str(x), LispVal::Str(y)) => x == y,
        (LispVal::Bool(x), LispVal::Bool(y)) => x == y,
        (LispVal::Nil, LispVal::Nil) => true,
        (LispVal::Sym(x), LispVal::Sym(y)) => x == y,
        (LispVal::List(x), LispVal::List(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| lisp_equal(a, b))
        }
        (LispVal::Map(x), LispVal::Map(y)) => {
            if x.len() != y.len() {
                return false;
            }
            for (k, v) in x {
                match y.get(k) {
                    Some(yv) if lisp_equal(v, yv) => {}
                    _ => return false,
                }
            }
            true
        }
        _ => false,
    }
}

pub fn as_num(v: &LispVal) -> Result<i64, String> {
    match v {
        LispVal::Num(n) => Ok(*n),
        LispVal::Float(f) => Ok(*f as i64),
        _ => Err(format!("expected number, got {}", v)),
    }
}

pub fn as_float(v: &LispVal) -> Result<f64, String> {
    match v {
        LispVal::Float(f) => Ok(*f),
        LispVal::Num(n) => Ok(*n as f64),
        _ => Err(format!("expected number, got {}", v)),
    }
}

pub fn any_float(args: &[LispVal]) -> bool {
    args.iter().any(|a| matches!(a, LispVal::Float(_)))
}

pub fn as_str(v: &LispVal) -> Result<String, String> {
    match v {
        LispVal::Str(s) => Ok(s.clone()),
        LispVal::Sym(s) => Ok(s.clone()),
        LispVal::Num(n) => Ok(n.to_string()),
        LispVal::Float(f) => Ok(f.to_string()),
        _ => Err(format!("expected string, got {}", v)),
    }
}

pub fn do_arith(
    args: &[LispVal],
    op_int: fn(i64, i64) -> i64,
    op_float: fn(f64, f64) -> f64,
) -> Result<LispVal, String> {
    do_arith_impl(args, op_int, op_float)
}

fn do_arith_impl(
    args: &[LispVal],
    op_int: fn(i64, i64) -> i64,
    op_float: fn(f64, f64) -> f64,
) -> Result<LispVal, String> {
    if args.len() < 2 {
        // Allow 1-arg: (+ x) = x, (* x) = x, etc
        if args.len() == 1 {
            return Ok(args[0].clone());
        }
        return Err("arith needs 1+ args".into());
    }
    if any_float(args) {
        let init = as_float(&args[0])?;
        let res: Result<f64, String> = args[1..]
            .iter()
            .try_fold(init, |a, b| Ok(op_float(a, as_float(b)?)));
        Ok(LispVal::Float(res?))
    } else {
        let init = as_num(&args[0])?;
        let res: Result<i64, String> = args[1..]
            .iter()
            .try_fold(init, |a, b| Ok(op_int(a, as_num(b)?)));
        Ok(LispVal::Num(res?))
    }
}

/// Checked integer variant for money-bearing ops (+ - *): traps on i64
/// overflow AND on results outside the tagged payload range [-2^60, 2^60),
/// matching the wasm emitter (semantic anchor). No silent release-mode wrap,
/// no process panic.
pub fn do_arith_checked(
    args: &[LispVal],
    op_name: &str,
    op_int: fn(i64, i64) -> Option<i64>,
    op_float: fn(f64, f64) -> f64,
) -> Result<LispVal, String> {
    if args.len() < 2 {
        if args.len() == 1 {
            return Ok(args[0].clone());
        }
        return Err("arith needs 1+ args".into());
    }
    if any_float(args) {
        let init = as_float(&args[0])?;
        let res: Result<f64, String> = args[1..]
            .iter()
            .try_fold(init, |a, b| Ok(op_float(a, as_float(b)?)));
        Ok(LispVal::Float(res?))
    } else {
        let init = as_num(&args[0])?;
        let res: Result<i64, String> = args[1..].iter().try_fold(init, |a, b| {
            let r = op_int(a, as_num(b)?)
                .ok_or_else(|| format!("integer overflow in {}", op_name))?;
            crate::bytecode::check_num_range(r, op_name)
        });
        Ok(LispVal::Num(res?))
    }
}

pub fn parse_params(val: &LispVal) -> Result<(Vec<String>, Option<String>), String> {
    match val {
        LispVal::List(p) => {
            let mut params = Vec::new();
            let mut rest_param = None;
            let mut seen_rest = false;
            for v in p {
                match v {
                    LispVal::Sym(s) if s == "&rest" => {
                        seen_rest = true;
                    }
                    LispVal::Sym(s) if seen_rest => {
                        rest_param = Some(s.clone());
                        seen_rest = false;
                    }
                    LispVal::Sym(s) => {
                        params.push(s.clone());
                    }
                    _ => return Err("param must be sym".into()),
                }
            }
            Ok((params, rest_param))
        }
        _ => Err("params must be list".into()),
    }
}

pub fn match_pattern(pattern: &LispVal, value: &LispVal) -> Option<Vec<(String, LispVal)>> {
    match pattern {
        LispVal::Sym(s) if s == "_" => Some(vec![]),
        LispVal::Sym(s) if s == "else" => Some(vec![]),
        LispVal::Sym(s) if s.starts_with('?') => Some(vec![(s[1..].to_string(), value.clone())]),
        LispVal::Sym(s) => Some(vec![(s.clone(), value.clone())]),
        LispVal::Num(n) => {
            if value == &LispVal::Num(*n) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::Float(f) => {
            if let LispVal::Float(vf) = value {
                if (*f - *vf).abs() < f64::EPSILON {
                    Some(vec![])
                } else {
                    None
                }
            } else {
                None
            }
        }
        LispVal::Str(s) => {
            if value == &LispVal::Str(s.clone()) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::Bool(b) => {
            if value == &LispVal::Bool(*b) {
                Some(vec![])
            } else {
                None
            }
        }
        LispVal::List(pats) if !pats.is_empty() => {
            // Tagged sum-type pattern: (:type::variant ?f1 ?f2 ...)
            if let LispVal::Sym(tag) = &pats[0] {
                if tag.contains("::") && !tag.starts_with('?') {
                    if let LispVal::Tagged { type_name, variant_id, fields } = value {
                        if tag != &format!("{}::{}", type_name, variant_id) {
                            return None;
                        }
                        let field_pats = &pats[1..];
                        if fields.len() != field_pats.len() {
                            return None;
                        }
                        let mut all = vec![];
                        for (p, v) in field_pats.iter().zip(fields.iter()) {
                            if let Some(b) = match_pattern(p, v) {
                                all.extend(b);
                            } else {
                                return None;
                            }
                        }
                        return Some(all);
                    } else {
                        return None;
                    }
                }
            }
            if let LispVal::Sym(s) = &pats[0] {
                if s == "list" {
                    if let LispVal::List(vals) = value {
                        if vals.len() == pats.len() - 1 {
                            let mut all = vec![];
                            for (p, v) in pats[1..].iter().zip(vals.iter()) {
                                if let Some(b) = match_pattern(p, v) {
                                    all.extend(b);
                                } else {
                                    return None;
                                }
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if s == "cons" && pats.len() == 3 {
                    if let LispVal::List(vals) = value {
                        if !vals.is_empty() {
                            let mut all = vec![];
                            if let Some(b1) = match_pattern(&pats[1], &vals[0]) {
                                all.extend(b1);
                            } else {
                                return None;
                            }
                            if let Some(b2) =
                                match_pattern(&pats[2], &LispVal::List(vals[1..].to_vec()))
                            {
                                all.extend(b2);
                            } else {
                                return None;
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    if let LispVal::List(vals) = value {
                        if vals.len() == pats.len() {
                            let mut all = vec![];
                            for (p, v) in pats.iter().zip(vals.iter()) {
                                if let Some(b) = match_pattern(p, v) {
                                    all.extend(b);
                                } else {
                                    return None;
                                }
                            }
                            Some(all)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            } else {
                if let LispVal::List(vals) = value {
                    if vals.len() == pats.len() {
                        let mut all = vec![];
                        for (p, v) in pats.iter().zip(vals.iter()) {
                            if let Some(b) = match_pattern(p, v) {
                                all.extend(b);
                            } else {
                                return None;
                            }
                        }
                        Some(all)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
        _ => None,
    }
}

/// Get documentation string for a builtin.
pub fn get_doc(name: &str) -> Option<&'static str> {
    Some(match name {
        // Arithmetic
        "+" => "(+ a b ...) — Add numbers. Single arg: identity. No args: 0.",
        "-" => "(- a b ...) — Subtract. (- x) negates.",
        "*" => "(* a b ...) — Multiply. Single arg: identity. No args: 1.",
        "/" => "(/ a b ...) — Divide. Returns float if any arg is float.",
        "mod" => "(mod a b) — Modulo.",
        "abs" => "(abs n) — Absolute value.",
        "min" | "max" => "(min a b ...) / (max a b ...) — Min/max of numbers.",
        "sqrt" => "(sqrt n) — Square root (returns float).",
        "expt" => "(expt base exp) — Exponentiation.",

        // Comparison
        "=" | "==" => "(= a b) — Numeric equality (int or float).",
        "!=" | "/=" => "(!= a b) — Numeric inequality.",
        "<" | ">" | "<=" | ">=" => "(< a b) — Numeric comparison.",
        "equal?" => "(equal? a b) — Deep structural equality.",

        // Collections
        "list" => "(list a b ...) — Create a list.",
        "car" => "(car lst) — First element of list.",
        "cdr" => "(cdr lst) — Rest of list.",
        "cons" => "(cons head tail) — Prepend to list.",
        "len" | "length" => "(len lst) — Length of list or string.",
        "append" => "(append l1 l2) — Concatenate lists.",
        "nth" | "list-ref" => "(nth lst i) — Get element by index (0-based).",
        "reverse" => "(reverse lst) — Reverse a list.",
        "sort" => "(sort lst) — Sort numbers (ascending) or strings (lexicographic).",
        "range" => {
            "(range n) / (range start end) / (range start end step) — Generate list of numbers."
        }
        "zip" => "(zip l1 l2) — Zip two lists into pairs.",

        // HOFs
        "map" => "(map f lst) — Apply f to each element, return new list.",
        "filter" => "(filter pred lst) — Keep elements matching predicate.",
        "reduce" => "(reduce f init lst) — Fold left: f(accumulator, element).",
        "find" => "(find pred lst) — First element matching predicate, or nil.",
        "some" => "(some pred lst) — True if any element satisfies predicate.",
        "every" => "(every pred lst) — True if all elements satisfy predicate.",
        "for-each" => "(for-each f lst) — Apply f to each element for side effects. Returns nil.",
        "fold-left" => "(fold-left f init lst) — Left fold.",
        "fold-right" => "(fold-right f init lst) — Right fold.",

        // Strings
        "str-concat" | "string-append" => "(str-concat s1 s2 ...) — Concatenate strings.",
        "str-length" | "string-length" => "(str-length s) — String length.",
        "str-substring" | "substring" => "(str-substring s start end) — Substring.",
        "str-split" => "(str-split s delim) — Split string by delimiter.",
        "str-contains" | "string-contains" => {
            "(str-contains s sub) — Check if string contains substring."
        }
        "str-trim" => "(str-trim s) — Trim whitespace.",
        "str-upcase" | "string-upcase" => "(str-upcase s) — Uppercase.",
        "str-downcase" | "string-downcase" => "(str-downcase s) — Lowercase.",
        "str-replace" | "string-replace" => "(str-replace s old new) — Replace substring.",
        "str-join" => "(str-join lst sep) — Join list of strings with separator.",

        // Predicates
        "nil?" | "null?" => "(nil? x) — Is x nil?",
        "list?" | "pair?" => "(list? x) — Is x a list?",
        "number?" => "(number? x) — Is x a number?",
        "string?" => "(string? x) — Is x a string?",
        "map?" => "(map? x) — Is x a dict?",
        "bool?" | "boolean?" => "(bool? x) — Is x a boolean?",
        "procedure?" => "(procedure? x) — Is x callable (lambda)?",
        "symbol?" => "(symbol? x) — Is x a symbol?",
        "zero?" => "(zero? n) — Is n zero?",
        "positive?" => "(positive? n) — Is n > 0?",
        "negative?" => "(negative? n) — Is n < 0?",
        "even?" => "(even? n) — Is n even?",
        "odd?" => "(odd? n) — Is n odd?",
        "empty?" => "(empty? x) — Is list/string empty?",

        // Dict
        "dict" => "(dict k1 v1 k2 v2 ...) — Create a dict.",
        "dict/get" => "(dict/get d key) — Get value from dict. Returns nil if missing.",
        "dict/set" => "(dict/set d key val) — Set key in dict (returns new dict).",
        "dict/has?" => "(dict/has? d key) — Check if key exists.",
        "dict/keys" => "(dict/keys d) — List of keys.",
        "dict/vals" => "(dict/vals d) — List of values.",

        // IO
        "print" => "(print x) — Print value without newline.",
        "println" => "(println x) — Print value with newline.",
        "read-file" => "(read-file path) — Read file contents as string.",
        "write-file" => "(write-file path content) — Write string to file.",
        "load-file" => "(load-file path) — Load and evaluate a Lisp file.",

        // Special forms
        "define" => "(define name val) or (define (f params...) body) — Define binding.",
        "set!" => "(set! name val) — Mutate existing binding.",
        "lambda" => "(lambda (params...) body) — Create anonymous function.",
        "if" => "(if cond then else?) — Conditional.",
        "cond" => "(cond (test expr) ... (else expr)) — Multi-branch conditional.",
        "begin" => "(begin e1 e2 ...) — Sequence expressions, return last.",
        "let" => "(let ((var val) ...) body) — Local bindings.",
        "let*" => "(let* ((var val) ...) body) — Sequential local bindings.",
        "letrec" => "(letrec ((var val) ...) body) — Recursive local bindings.",
        "and" => "(and e1 e2 ...) — Short-circuit logical AND.",
        "or" => "(or e1 e2 ...) — Short-circuit logical OR.",
        "when" => "(when cond body...) — Execute body if cond is truthy.",
        "unless" => "(unless cond body...) — Execute body if cond is falsy.",

        // Eval
        "apply" => "(apply f arg1 ... arglist) — Apply function to spread args.",
        "eval" => "(eval expr) — Evaluate a Lisp expression.",
        "error" => "(error msg) — Raise an error.",
        "type-of" => "(type-of x) — Return type name as string.",
        "infer-type" => "(infer-type f) — Probe a pure lambda to infer its type signature.",
        "pure-type" => "(pure-type f) — Return the pure type annotation, or nil.",

        // Runtime
        "now" => "(now) — Current Unix timestamp (milliseconds).",
        "elapsed" => "(elapsed since) — Milliseconds since given timestamp.",
        "sleep" => "(sleep ms) — Sleep for milliseconds.",
        "save-state" => "(save-state path val) — Serialize value to JSON file.",
        "load-state" => "(load-state path) — Load value from JSON file.",

        _ => return None,
    })
}

// ─────────────────────────────────────────────────────
// Sum-type registry (deftype)
// ─────────────────────────────────────────────────────
// Uses RefCell inside thread_local! — WASM-compatible (no Mutex/LazyLock).

use std::cell::RefCell;
use std::collections::HashMap;

/// A registered variant constructor.
#[derive(Clone, Debug)]
pub struct TypeVariant {
    pub type_name: String,
    pub variant_id: u16,
    pub n_fields: u8,
}

thread_local! {
    static TYPE_REGISTRY: RefCell<HashMap<String, TypeVariant>> = RefCell::new(HashMap::new());
}

/// Register a type definition. Silently ignores duplicate constructor names.
pub fn register_type(type_name: &str, variants: &[(&str, u8)]) {
    TYPE_REGISTRY.with(|reg| {
        let mut reg = reg.borrow_mut();
        for (i, (name, n_fields)) in variants.iter().enumerate() {
            let variant = TypeVariant {
                type_name: type_name.to_string(),
                variant_id: i as u16,
                n_fields: *n_fields,
            };
            reg.entry(name.to_string()).or_insert(variant);
        }
    });
}

/// Look up a constructor. Returns None if not a registered constructor.
pub fn lookup_constructor(name: &str) -> Option<TypeVariant> {
    TYPE_REGISTRY.with(|reg| reg.borrow().get(name).cloned())
}

/// Clear all registered types. Used by tests.
pub fn clear_type_registry() {
    TYPE_REGISTRY.with(|reg| reg.borrow_mut().clear());
}
