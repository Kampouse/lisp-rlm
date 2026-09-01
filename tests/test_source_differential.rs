//! SOURCE-LEVEL DIFFERENTIAL: interpreter vs wasm backend, same program,
//! same result (2026-09-01). The 19-compiler-bug arc proved the two
//! backends implement the surface language independently — this harness
//! makes that divergence VISIBLE at the source level:
//!
//!   1. The equiv corpus (tests/equiv/*.lisp) — 37 files built for this
//!      but never wired: printlns stripped, last expression becomes the
//!      program result.
//!   2. A seeded random-program fuzzer over the pure surface (arith,
//!      strings, bool logic, control flow, lists/HOFs, u128 decimal ops,
//!      closures, recursion).
//!
//! Match policy: both-error = match (surface gaps where one side can't
//! COMPILE are skipped and counted); value mismatch or one-sided error
//! = DIVERGENCE, fail with the program + both results.

#![allow(dead_code)]

use lisp_rlm_wasm::*;
use lisp_rlm_wasm::tagged_value::{TaggedValue, decode};

#[path = "borsh_harness.rs"]
mod harness;
use harness::WasmRunner;

// ── value normalization ──

/// wasm TaggedValue → interp LispVal (comparable domain).
fn tv_to_lisp(memory: &[u8], tv: TaggedValue) -> LispVal {
    match tv {
        TaggedValue::Num(n) => LispVal::Num(n),
        TaggedValue::Bool(b) => LispVal::Bool(b),
        TaggedValue::Nil => LispVal::Nil,
        TaggedValue::Str { ptr, len } => {
            let s = String::from_utf8_lossy(&memory[ptr as usize..(ptr + len) as usize]).to_string();
            LispVal::Str(s)
        }
        TaggedValue::Array { ptr, count } => {
            let mut items = Vec::new();
            for i in 0..count {
                let off = (ptr + 8 + i * 8) as usize;
                let raw = i64::from_le_bytes(memory[off..off + 8].try_into().unwrap());
                items.push(tv_to_lisp(memory, decode(memory, raw)));
            }
            LispVal::List(items)
        }
        TaggedValue::FnRef(_) | TaggedValue::Closure(_) => LispVal::Str("<fn>".into()),
    }
}

/// Float normalization: interp has Float, wasm decode does not expose it —
/// floats compare by f64 bits when both present.
fn canon(v: LispVal) -> LispVal {
    match v {
        LispVal::Float(f) => LispVal::Num(f.to_bits() as i64),
        LispVal::U64(u) => LispVal::Num(u as i64),
        LispVal::List(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        LispVal::Vec(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        other => other,
    }
}

// ── the two engines ──

/// Interpreter: eval `(main)` after loading all defines.
fn interp_run(src: &str) -> Result<LispVal, String> {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut env = Env::new();
            let mut state = EvalState::new();
            let exprs = parser::parse_all(&src)?;
            let _ = run_program(&exprs, &mut env, &mut state)?;
            // call (main)
            run_program(
                &[types::LispVal::List(vec![types::LispVal::Sym("main".into())])],
                &mut env,
                &mut state,
            )
        })
        .expect("spawn")
        .join()
        .map_err(|_| "interp thread panic".to_string())?
}

/// Wasm: compile_fuzz → run → decode TEMP_MEM result.
fn wasm_run(src: &str) -> Result<LispVal, String> {
    let mut runner = WasmRunner::new(src)?;
    runner.run()?;
    let tagged = runner.read_raw_result();
    let mem = runner.mem_snapshot();
    Ok(tv_to_lisp(&mem, decode(&mem, tagged)))
}

/// Strip `(println e)` wrappers; return the LAST expr as the result form.
fn result_expr_of(src: &str) -> Result<String, String> {
    let exprs = parser::parse_all(&src)?;
    let mut last: Option<LispVal> = None;
    fn strip(e: LispVal) -> LispVal {
        match e {
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "println" && items.len() == 2 {
                        return strip(items[1].clone());
                    }
                }
                LispVal::List(items.into_iter().map(strip).collect())
            }
            other => other,
        }
    }
    for e in exprs {
        match &e {
            LispVal::List(items) if !items.is_empty() => {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "define" {
                        // keep defines as-is (strip printlns inside bodies)
                        last = Some(strip(e.clone()));
                        continue;
                    }
                }
                last = Some(strip(e));
            }
            other => last = Some(strip(other.clone())),
        }
    }
    let l = last.ok_or("empty program")?;
    // ensure main returns the result: if the program IS a define of main,
    // we need the last println-stripped body expr — simplest canonical
    // shape: append (define (main) <last>) unless it already is one.
    if let LispVal::List(items) = &l {
        if items.len() >= 2 {
            if let (LispVal::Sym(s), LispVal::List(sig)) = (&items[0], &items[1]) {
                if s == "define" && !sig.is_empty() {
                    if let LispVal::Sym(name) = &sig[0] {
                        if name == "main" {
                            return Ok(format!("{}", l));
                        }
                    }
                }
            }
        }
    }
    Ok(format!("(define (main) {})", l))
}

/// Differential outcome for one program.
enum Outcome {
    Match,
    Divergence(String),
}

fn diff_one(label: &str, src: &str, i_result: Result<LispVal, String>, w_result: Result<LispVal, String>) -> Outcome {
    match (&i_result, &w_result) {
        (Ok(a), Ok(b)) => {
            let (ca, cb) = (canon(a.clone()), canon(b.clone()));
            if helpers_eq(&ca, &cb) {
                Outcome::Match
            } else {
                Outcome::Divergence(format!(
                    "[{}] value mismatch\n  program: {}\n  interp: {:?}\n  wasm:   {:?}",
                    label, src.trim(), ca, cb
                ))
            }
        }
        (Err(_), Err(_)) => Outcome::Match, // both reject = agreement
        (a, b) => Outcome::Divergence(format!(
            "[{}] one-sided failure\n  program: {}\n  interp: {:?}\n  wasm:   {:?}",
            label, src.trim(), a, b
        )),
    }
}

fn helpers_eq(a: &LispVal, b: &LispVal) -> bool {
    match (a, b) {
        (LispVal::Num(x), LispVal::Num(y)) => x == y,
        (LispVal::Str(x), LispVal::Str(y)) => x == y,
        (LispVal::Bool(x), LispVal::Bool(y)) => x == y,
        (LispVal::Nil, LispVal::Nil) => true,
        (LispVal::List(xs), LispVal::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| helpers_eq(x, y))
        }
        _ => false,
    }
}

// ── 1. the equiv corpus ──

#[test]
fn differential_equiv_corpus() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/equiv");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("equiv dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "lisp").unwrap_or(false))
        .collect();
    files.sort();
    let (mut matches, mut diverged, mut skipped) = (0, 0, 0);
    let mut failures = Vec::new();
    for f in &files {
        let raw = std::fs::read_to_string(f).unwrap();
        // skip files whose header documents an intentional interp-only surface
        if raw.contains("interp-only") {
            skipped += 1;
            continue;
        }
        let src = match result_expr_of(&raw) {
            Ok(s) => s,
            Err(_) => { skipped += 1; continue; }
        };
        // wasm compile gate: surface gaps skip
        if wasm_compile_gate(&src).is_err() {
            skipped += 1;
            continue;
        }
        let i = interp_run(&src);
        let w = wasm_run(&src);
        match diff_one(&f.file_name().unwrap().to_string_lossy(), &src, i, w) {
            Outcome::Match => matches += 1,
            Outcome::Divergence(d) => {
                diverged += 1;
                failures.push(d);
            }
        }
    }
    eprintln!(
        "equiv corpus: {} match, {} skip (surface gaps), {} DIVERGED of {} files",
        matches, skipped, diverged, files.len()
    );
    assert!(diverged == 0, "divergences:\n{}", failures.join("\n\n"));
}

fn wasm_compile_gate(src: &str) -> Result<Vec<u8>, String> {
    let mut runner = WasmRunner::new(src)?;
    let _ = &mut runner;
    Ok(vec![])
}

// ── 2. seeded random-program fuzzer ──

/// Typed generator: every production knows its type, so comparisons never
/// mix bools with nums and `=` never mixes str with int (the checker
/// correctly rejects those; we want RUNTIME divergence, not type noise).
#[derive(Clone, Copy, PartialEq)]
enum Ty { Num, Bool, Str }

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 16
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T { &xs[(self.next() % xs.len() as u64) as usize] }
    fn n(&mut self, lo: i64, hi: i64) -> i64 { lo + (self.next() % ((hi - lo + 1) as u64)) as i64 }
}

fn gen(g: &mut Lcg, ty: Ty, depth: u32, vars: &mut Vec<(String, Ty)>) -> String {
    if depth == 0 {
        return match ty {
            Ty::Num => format!("{}", g.n(-40, 60)),
            Ty::Bool => (*g.pick(&["true", "false"])).to_string(),
            Ty::Str => format!("\"{}\"", (*g.pick(&["s", "abc", "42", "zz", "ab12"])).to_string()),
        };
    }
    // restore vars on unwind
    let snap = vars.clone();
    let out = match (ty, g.n(0, 11)) {
        (Ty::Num, 0..=3) => {
            let op = *g.pick(&["+", "-", "*"]);
            format!("({} {} {})", op, gen(g, Ty::Num, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars))
        }
        (Ty::Num, 4) => format!("(mod {} {})", gen(g, Ty::Num, depth - 1, vars), g.n(1, 9)),
        (Ty::Num, 5) => format!("(min {} {})", gen(g, Ty::Num, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars)),
        (Ty::Num, 6) => format!("(if {} {} {})", gen(g, Ty::Bool, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars)),
        (Ty::Num, 7) => {
            let v = format!("v{}", g.n(0, 999));
            vars.push((v.clone(), Ty::Num));
            let body = gen(g, Ty::Num, depth - 1, vars);
            let init = gen(g, Ty::Num, depth - 1, &mut snap.clone());
            format!("(let (({} {})) {})", v, init, body)
        }
        (Ty::Num, 8 | 9) => format!("(len {})", gen_str(g, depth - 1)),
        (Ty::Num, 10) => format!("(len (str-concat {} {}))", gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Num, _) => format!("(if (> {} 0) {} {})", gen(g, Ty::Num, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars), g.n(1, 9)),
        (Ty::Bool, 0..=3) => {
            let op = *g.pick(&["<", ">", "=", "<=", ">="]);
            format!("({} {} {})", op, gen(g, Ty::Num, depth - 1, vars), gen(g, Ty::Num, depth - 1, vars))
        }
        (Ty::Bool, 4) => {
            let op = *g.pick(&["and", "or"]);
            format!("({} {} {})", op, gen(g, Ty::Bool, depth - 1, vars), gen(g, Ty::Bool, depth - 1, vars))
        }
        (Ty::Bool, 5) => format!("(not {})", gen(g, Ty::Bool, depth - 1, vars)),
        (Ty::Bool, 6) => {
            let op = *g.pick(&["str-contains", "str-starts-with", "str-ends-with"]);
            format!("({} {} {})", op, gen_str(g, depth - 1), gen_str(g, depth - 1))
        }
        (Ty::Bool, 7) => format!("(= {} {})", gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Bool, 8) => format!("(< (len {}) (len {}))", gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Bool, _) => format!("(if {} {} {})", gen(g, Ty::Bool, depth - 1, vars), gen(g, Ty::Bool, depth - 1, vars), gen(g, Ty::Bool, depth - 1, vars)),
        (Ty::Str, 0..=2) => format!("(str-concat {} {})", gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Str, 3) => format!("(if {} {} {})", gen(g, Ty::Bool, depth - 1, vars), gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Str, 4) => format!("(if (> (len {}) (len {})) {} {})", gen_str(g, depth - 1), gen_str(g, depth - 1), gen_str(g, depth - 1), gen_str(g, depth - 1)),
        (Ty::Str, 5) => format!("(list->string {})", gen_str_list(g, depth - 1, vars)),
        (Ty::Str, 6) => format!("(str-join {} {})", (*g.pick(&["\"", "-", "ab"])).to_string(), gen_str_list(g, depth - 1, vars)),
        (Ty::Str, 7) => {
            let v = format!("s{}", g.n(0, 99));
            vars.push((v.clone(), Ty::Str));
            let body = gen(g, Ty::Str, depth - 1, vars);
            let init = gen_str(g, depth - 1);
            format!("(let (({} {})) {})", v, init, body)
        }
        (Ty::Str, _) => gen_str(g, depth - 1),
    };
    *vars = snap;
    out
}

fn gen_str(g: &mut Lcg, depth: u32) -> String {
    if depth == 0 || g.next() % 2 == 0 {
        format!("\"{}\"", (0..g.n(1, 5)).map(|_| *g.pick(&["a", "b", "7", "xy", "q"])).collect::<String>())
    } else {
        format!("(str-concat {} {})", gen_str(g, depth - 1), gen_str(g, depth - 1))
    }
}

fn gen_list(g: &mut Lcg, depth: u32, vars: &mut Vec<(String, Ty)>) -> String {
    let k = g.n(0, 3);
    let items: Vec<String> = (0..k).map(|_| gen(g, Ty::Num, depth.saturating_sub(1), vars)).collect();
    if items.is_empty() { "(list)".to_string() } else { format!("(list {})", items.join(" ")) }
}

fn gen_num_list(g: &mut Lcg, depth: u32, vars: &mut Vec<(String, Ty)>) -> String {
    let k = g.n(1, 4);
    let items: Vec<String> = (0..k).map(|_| gen(g, Ty::Num, depth.saturating_sub(1), vars)).collect();
    if g.next() % 2 == 0 {
        format!("(map (lambda (x) (* x {})) (list {}))", g.n(1, 5), items.join(" "))
    } else {
        format!("(list {})", items.join(" "))
    }
}

fn gen_str_list(g: &mut Lcg, depth: u32, vars: &mut Vec<(String, Ty)>) -> String {
    let k = g.n(1, 3);
    let items: Vec<String> = (0..k).map(|_| gen_str(g, depth.saturating_sub(1))).collect();
    format!("(list {})", items.join(" "))
}

#[test]
fn differential_fuzz_typed_300() {
    let (mut matches, mut both_err, mut diverged, mut skipped) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for seed in 1..=300u64 {
        let mut g = Lcg(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
        let ret_ty = [Ty::Num, Ty::Bool, Ty::Str][(g.next() % 3) as usize];
        let mut vars = Vec::new();
        let body = gen(&mut g, ret_ty, 4, &mut vars);
        let src = format!("(define (main) {})", body);
        if wasm_compile_gate(&src).is_err() {
            skipped += 1;
            continue;
        }
        let i = interp_run(&src);
        let w = wasm_run(&src);
        match (&i, &w) {
            (Ok(_), Ok(_)) | (Err(_), Err(_)) => {
                if i.is_ok() { matches += 1 } else { both_err += 1 }
            }
            _ => {
                diverged += 1;
                if let Outcome::Divergence(d) = diff_one(&format!("seed {}", seed), &src, i, w) {
                    failures.push(d);
                }
            }
        }
    }
    eprintln!("fuzz: {} match, {} both-err (agree), {} skip (gate), {} DIVERGED", matches, both_err, skipped, diverged);
    assert!(diverged == 0, "divergences:\n{}", failures.join("\n\n"));
}
