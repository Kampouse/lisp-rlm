//! SHARED-FIXTURE DIFFERENTIAL RUNNER (2026-10-07, hardening item 2).
//!
//! test_source_differential.rs differentiates the equiv corpus + a fuzzer
//! (program VALUES: same source through both engines). What it does NOT
//! cover is a curated, deliberate matrix of the CONTRACT-RELEVANT surface
//! — arith (checked + wrapping policy), strings, control flow, closures,
//! HOF/lists, u128 string arithmetic, and near/input dispatch — in one
//! readable place. That is this file: ~25 small programs, every one
//! executed through
//!   (a) the bytecode VM path (`run_program` compiles to bytecode), and
//!   (b) compile → wasm → wasmtime execute (compile_fuzz),
//! with results compared under the standard differential policy:
//!   * canon-equal values            → match
//!   * BOTH engines error            → match (agreement on rejection)
//!   * one-sided error / value gap   → DIVERGENCE (fails the suite)
//!
//! Fixtures that fail the wasm compile gate are SKIPPED and counted — the
//! layer_parity.rs test owns the surface-drift problem; here a skip is a
//! known allowlisted gap, not a crash of the harness. But skips are capped:
//! if more than SKIP_CAP fixtures stop compiling, this file fails loudly so
//! the shared fixture set cannot silently rot to nothing.
//!
//! KNOWN DIVERGENCES DOCUMENTED (do not "fix" by deleting):
//!   * `json-get` argument order: interp is (json-get <json> <key>), the
//!     wasm emitter treats arg0 as the KEY. json-get is therefore NOT in
//!     the shared fixtures; fixing the order needs its own task (it is a
//!     user-facing semantic gap, not a harness bug). Found 2026-10-07
//!     while building this matrix.
//!   * str->num parse-failure payload differs by design (Nil vs 0-ish);
//!     fixtures only use well-formed numerics.

use lisp_rlm_wasm::parser;
use lisp_rlm_wasm::run_program;
use lisp_rlm_wasm::tagged_value::{decode, TaggedValue};
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};
use wasmtime::*;

const STACK: usize = 512 * 1024 * 1024;
const SKIP_CAP: usize = 4;

// ── fixture ──

struct Fixture {
    label: &'static str,
    src: &'static str,
    /// Payload the wasm `input` host / interp near_context serves.
    input: &'static str,
    /// NEAR-mode fixtures compile with compile_near (near/* typing env) and
    /// export `_run`; pure fixtures use compile_fuzz + `run`.
    near: bool,
}

impl Fixture {
    fn new(label: &'static str, src: &'static str) -> Self {
        Fixture { label, src, input: "", near: false }
    }
    fn near_with_input(label: &'static str, src: &'static str, input: &'static str) -> Self {
        Fixture { label, src, input, near: true }
    }
}

fn fixtures() -> Vec<Fixture> {
    vec![
        // ── arithmetic (checked + explicit wrapping policy) ──
        Fixture::new(
            "arith/nested",
            "(define (main) (+ (* 3 7) (- 20 (* 2 (+ 1 2)))))",
        ),
        Fixture::new(
            "arith/div-mod-neg",
            "(define (main) (list (/ 47 5) (mod 47 5) (- 0 8) (* -7 (+ 3 -4))))",
        ),
        Fixture::new(
            "arith/wrap-explicit",
            "(define (main) (wrap-mul 4611686018427387904 4))",
        ),
        // runtime (not const-folded) checked overflow: both engines must
        // reject — interp errors, wasm traps
        Fixture::new(
            "arith/checked-overflow",
            "(define n (str->num \"1000000000\"))\n(define (main) (* n 1152921504606846975))",
        ),
        Fixture::new(
            "arith/min-identity",
            "(define (main) (list (max 3 9 2) (min 3 9 2) (abs (- 0 5))))",
        ),
        // ── comparison / boolean ──
        Fixture::new(
            "bool/matrix",
            "(define (main) (list (< 1 2) (>= 3 3) (= 5 5) (!= 2 3) (not (< 1 2))))",
        ),
        Fixture::new(
            "bool/if-nest",
            r#"(define (cls n)
                 (if (< n 10) "small"
                   (if (< n 100) "mid"
                     (if (< n 1000) "large" "huge"))))
                 (define (main) (list (cls 5) (cls 55) (cls 555) (cls 5555)))"#,
        ),
        // ── strings ──
        Fixture::new(
            "str/cat-chain",
            r#"(define (main) (str-cat "a" "b" "-" "cd" "!"))"#,
        ),
        Fixture::new(
            "str/ops",
            r#"(define (main)
                 (list (str-length "hello world")
                       (str-substring "hello world" 6 11)
                       (str-index-of "hello world" "o")
                       (str-contains "hello world" "wor")
                       (str-upcase "abc")))"#,
        ),
        Fixture::new(
            "str/num-parse",
            r#"(define (main) (list (str->num "1234") (str->num "-77")))"#,
        ),
        Fixture::new(
            "str/build-loop",
            r#"(define (go i acc) (if (>= i 5) acc (go (+ i 1) (str-cat acc "x"))))
                 (define (main) (go 0 ""))"#,
        ),
        // ── control flow ──
        Fixture::new(
            "flow/cond-chain",
            "(define (main) (cond ((< 42 10) 1) ((< 42 100) 2) ((< 42 1000) 3) (else 4)))",
        ),
        // FOUND (2026-10-07, this differential): `set!` on a top-level
        // value-define is a SILENT NO-OP in wasm (reads emit Call to the
        // memoized fn, writes go to a dead local via local_idx) while the
        // interp mutates the env binding — (while (< i 10) … (set! i …))
        // over a module-level define infinite-loops in wasm, terminates in
        // the interp. Tracked in TASK-COMPILER-HARDENING.md § found-asymmetries.
        // Until fixed, while fixtures bind their accumulators with let.
        Fixture::new(
            "flow/while-sum",
            "(define (main)\n  (let ((i 0) (s 0))\n    (while (< i 10) (set! s (+ s i)) (set! i (+ i 1)))\n    (list i s)))",
        ),
        Fixture::new(
            "flow/recursion",
            r#"(define (fact n) (if (= n 0) 1 (* n (fact (- n 1)))))
                 (define (main) (fact 10))"#,
        ),
        Fixture::new(
            "flow/mutual",
            r#"(define (even2? n) (if (= n 0) true (odd2? (- n 1))))
                 (define (odd2? n) (if (= n 0) false (even2? (- n 1))))
                 (define (main) (list (even2? 10) (odd2? 7)))"#,
        ),
        // ── closures / state / shadowing ──
        Fixture::new(
            "closures/counter",
            "(define c 0) (define (bump) (set! c (+ c 1)) c)\n(define (main) (bump) (bump) (bump) c)",
        ),
        Fixture::new(
            "shadow/lets",
            "(define (main) (let ((x 2)) (let ((x (+ x 10))) x)))",
        ),
        // ── lists / HOF ──
        Fixture::new(
            "lists/ops",
            "(define (main) (list (len (list 10 20 30 40)) (append (list 10 20) (list 30))))",
        ),
        Fixture::new(
            "hof/squares-sum",
            r#"(define (sq x) (* x x))
                 (define (add2 a b) (+ a b))
                 (define (main)
                   (let ((sqs (map sq (list 1 2 3 4))))
                     (let ((evs (filter (fn (x) (> x 1)) (list 0 1 2 3))))
                       (list (reduce add2 0 sqs) (len evs)))))"#,
        ),
        Fixture::new(
            "lists/cons-build",
            "(define (build n) (if (= n 0) nil (cons n (build (- n 1)))))\n(define (main) (build 6))",
        ),
        // ── u128 (decimal-string surface) ──
        Fixture::new(
            "u128/add-max",
            r#"(define (main) (u128/add "340282366920938463463374607431768211455" "1"))"#,
        ),
        Fixture::new(
            "u128/mul-chain",
            r#"(define (main) (u128/mul (u128/add "4294967296" "0") "4294967296"))"#,
        ),
        // ── near/input dispatch (compile_near + input register) ──
        Fixture::near_with_input(
            "input/num-add",
            "(define (main) (+ (str->num (near/input)) 1))",
            "41",
        ),
        Fixture::near_with_input(
            "input/str-build",
            r#"(define (main) (str-cat "v=" (near/input)))"#,
            "42",
        ),
        Fixture::near_with_input(
            "input/cond-dispatch",
            "(define (main)\n  (let ((op (str->num (near/input))))\n    (cond ((= op 1) (* 6 7)) ((= op 2) (- 0 1)) (else 0))))",
            "1",
        ),
        // ── deep nesting / mixed ──
        Fixture::new(
            "deep/nest15",
            "(define (main) (+ 1 (+ 2 (+ 3 (+ 4 (+ 5 (+ 6 (+ 7 (+ 8 (+ 9 (+ 10 (+ 11 (+ 12 (+ 13 (+ 14 15)))))))))))))))",
        ),
        Fixture::new(
            "mixed/fib",
            r#"(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))
                 (define (main) (fib 14))"#,
        ),
    ]
}

// ── engine (a): bytecode VM ──

fn interp_run(src: &str, input: &str) -> Result<LispVal, String> {
    let src = src.to_string();
    let input = input.to_string();
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let mut env = Env::new();
            let mut state = EvalState::new();
            // the wasm host serves this payload through the `input`
            // register — the interp reads near_context["input"]
            state.near_context = state.near_context.update("input".to_string(), LispVal::Str(input));
            let exprs = parser::parse_all(&src)?;
            let _ = run_program(&exprs, &mut env, &mut state)?;
            run_program(
                &[LispVal::List(vec![LispVal::Sym("main".into())])],
                &mut env,
                &mut state,
            )
        })
        .expect("spawn")
        .join()
        .map_err(|_| "interp thread panic".to_string())?
}

// ── engine (b): compile → wasm → wasmtime ──

/// Minimal NEAR-flavored host: `input` serves the fixture payload through
/// the register machinery (register_len/read_register are real); all other
/// imports return zero — exactly the promise-differential harness pattern.
struct WasmEngine {
    memory: Memory,
    store: Store<()>,
    instance: Instance,
    ret_bytes: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>,
}

impl WasmEngine {
    fn new(src: &str, input: &str, near: bool) -> Result<Self, String> {
        let wasm = if near {
            lisp_rlm_wasm::wasm_emit::compile_near(src).map_err(|e| format!("compile: {e}"))?
        } else {
            lisp_rlm_wasm::wasm_emit::compile_fuzz(src).map_err(|e| format!("compile: {e}"))?
        };
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {e}"))?;
        let mut store = Store::new(&engine, ());
        let mut linker = Linker::new(&engine);

        let fallback_memory =
            Memory::new(&mut store, MemoryType::new(4, None)).map_err(|e| format!("memory: {e}"))?;
        if module.imports().any(|i| i.module() == "env" && i.name() == "memory") {
            linker
                .define(&store, "env", "memory", fallback_memory)
                .map_err(|e| format!("link memory: {e}"))?;
        }

        let payload: std::sync::Arc<Vec<u8>> = std::sync::Arc::new(input.as_bytes().to_vec());
        let ret_bytes: std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        for import in module.imports() {
            if import.module() != "env" || import.name() == "memory" {
                continue;
            }
            let ExternType::Func(func_ty) = import.ty() else { continue };
            let params: Vec<ValType> = func_ty.params().collect();
            let results: Vec<ValType> = func_ty.results().collect();
            let ft = FuncType::new(&engine, params.clone(), results.clone());
            let name = import.name().to_string();
            let payload = payload.clone();
            let ret_bytes = ret_bytes.clone();
            let stub = Func::new(&mut store, ft, move |mut caller, args, ret| {
                let a = |i: usize| args.get(i).copied().unwrap_or(Val::I64(0)).i64().unwrap_or(0);
                let len = payload.len() as i64;
                match name.as_str() {
                    "input" => { /* payload written below on read */ }
                    "value_return" => {
                        // near wrapper convention: (len, ptr); capture bytes
                        if let Some(m) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                            let len = a(0) as usize;
                            let ptr = a(1) as usize;
                            let md = m.data(&caller);
                            let end = (ptr + len).min(md.len());
                            *ret_bytes.lock().unwrap() =
                                Some(md.get(ptr..end).map(|s| s.to_vec()).unwrap_or_default());
                        }
                        return Ok(());
                    }
                    "register_len" => {
                        ret[0] = Val::I64(len);
                        return Ok(());
                    }
                    "read_register" => {
                        // (reg_id, ptr): write payload at ptr
                        if let Some(m) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                            let ptr = a(1) as usize;
                            let mut md = m.data_mut(&mut caller);
                            if ptr < md.len() {
                                let end = (ptr + payload.len()).min(md.len());
                                md[ptr..end].copy_from_slice(&payload[..end - ptr]);
                            }
                        }
                        return Ok(());
                    }
                    _ => {}
                }
                for i in 0..ret.len() {
                    *(&mut ret[i]) = match results.get(i) {
                        Some(ValType::I32) => Val::I32(0),
                        _ => Val::I64(0),
                    };
                }
                Ok(())
            });
            linker
                .define(&store, "env", import.name(), stub)
                .map_err(|e| format!("link {}: {e}", import.name()))?;
        }

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("instantiate: {e}"))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .unwrap_or(fallback_memory);
        Ok(Self { memory, store, instance, ret_bytes })
    }

    fn run(&mut self, near: bool) -> Result<(), String> {
        // compile_near exports `_run` (promise differential convention),
        // compile_fuzz exports `run`
        let name = if near { "_run" } else { "run" };
        let f = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, name)
            .or_else(|_| {
                self.instance
                    .get_typed_func::<(), ()>(&mut self.store, if near { "run" } else { "_run" })
            })
            .map_err(|e| format!("no run export: {e}"))?;
        f.call(&mut self.store, ())
            .map_err(|e| format!("trap: {e}"))
    }

    fn result(&mut self, near: bool) -> LispVal {
        // Fuzz mode leaves the TAGGED result at TEMP_MEM (64).
        // NEAR mode serializes through value_return with per-tag shapes:
        //   nil   -> (8, TEMP_MEM) nil-sentinel bytes
        //   str   -> (len, ptr) raw string bytes
        //   array -> (8, TEMP_MEM) full TAGGED value
        //   num   -> (8, TEMP_MEM) UNTAGGED value  (compile.rs NEAR arm)
        // So near results are decoded from the captured value_return bytes.
        // Caveat: an untagged num whose low 3 bits == TAG_ARRAY(6) is
        // indistinguishable from an array pointer at this boundary — the
        // fixture set keeps near num returns off that residue for now.
        const TEMP_MEM: usize = 64; // borsh_harness::TEMP_MEM_USIZE
        let mem = self.memory.data(&self.store).to_vec();
        if near {
            let cap = self.ret_bytes.lock().unwrap().clone();
            let bytes = match cap {
                Some(b) => b,
                None => return LispVal::Nil, // no value_return observed
            };
            const NIL_SENTINEL: i64 = 0x7FFE_FEFF_FEFF_FEFE;
            if bytes.len() == 8 {
                let v = i64::from_le_bytes(bytes[..].try_into().unwrap());
                if v == NIL_SENTINEL {
                    return LispVal::Nil;
                }
                if v & 7 == 6 {
                    // TAG_ARRAY: full tagged value — decode from memory
                    return tv_to_lisp(&mem, decode(&mem, v));
                }
                return LispVal::Num(v);
            }
            return LispVal::Str(String::from_utf8_lossy(&bytes).into_owned());
        }
        let raw = if TEMP_MEM + 8 <= mem.len() {
            i64::from_le_bytes(mem[TEMP_MEM..TEMP_MEM + 8].try_into().unwrap())
        } else {
            0
        };
        tv_to_lisp(&mem, decode(&mem, raw))
    }
}

// ── tagged-value decode (crate decode + LispVal mapping) ──

fn tv_to_lisp(mem: &[u8], tv: TaggedValue) -> LispVal {
    match tv {
        TaggedValue::Num(n) => LispVal::Num(n),
        TaggedValue::Bool(b) => LispVal::Bool(b),
        TaggedValue::Nil => LispVal::Nil,
        TaggedValue::Str { ptr, len } => {
            if ptr >= 0 && len >= 0 && (ptr + len) as usize <= mem.len() {
                LispVal::Str(String::from_utf8_lossy(&mem[ptr as usize..(ptr + len) as usize]).to_string())
            } else {
                LispVal::Str(format!("<badstr {ptr:#x}/{len}>"))
            }
        }
        TaggedValue::Array { ptr, count } => {
            let mut items = Vec::new();
            for i in 0..count {
                let off = (ptr + 8 + i * 8) as usize;
                if off + 8 <= mem.len() {
                    let raw = i64::from_le_bytes(mem[off..off + 8].try_into().unwrap());
                    items.push(tv_to_lisp(mem, decode(mem, raw)));
                }
            }
            LispVal::List(items)
        }
        TaggedValue::FnRef(_) | TaggedValue::Closure(_) => LispVal::Str("<fn>".into()),
    }
}

fn wasm_run(src: &str, input: &str, near: bool) -> Result<LispVal, String> {
    let mut eng = WasmEngine::new(src, input, near)?;
    eng.run(near)?;
    Ok(eng.result(near))
}

// ── policy ──

fn canon(v: LispVal) -> LispVal {
    match v {
        LispVal::Float(f) => LispVal::Num(f.to_bits() as i64),
        LispVal::U64(u) => LispVal::Num(u as i64),
        LispVal::List(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        LispVal::Vec(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        other => other,
    }
}

fn canon_str(v: &LispVal) -> String {
    match &canon(v.clone()) {
        LispVal::Num(n) => format!("num {n}"),
        LispVal::Str(s) => format!("str {s:?}"),
        LispVal::Bool(b) => format!("bool {b}"),
        LispVal::Nil => "nil".into(),
        LispVal::List(xs) => {
            let inner: Vec<String> = xs.iter().map(canon_str).collect();
            format!("list({})", inner.join(" "))
        }
        other => format!("{other:?}"),
    }
}

/// Compile gate: the wasm compile IS the gate (typing/emit reject unknown
/// builtins, bad shapes, out-of-range literals).
fn wasm_compile_gate(src: &str, near: bool) -> Result<(), String> {
    if near {
        lisp_rlm_wasm::wasm_emit::compile_near(src).map(|_| ())
    } else {
        lisp_rlm_wasm::wasm_emit::compile_fuzz(src).map(|_| ())
    }
}

/// Known, pinned engine divergences (FOUND by this differential). Each entry
/// pins the CURRENT wasm behavior for a fixture where interp and wasm disagree.
/// A pinned bug that gets FIXED will produce an "unexpected match" failure
/// here — remove the pin when you fix the engine, not before.
/// (Do not add new pins casually: a pin is a shipped, documented bug.)
fn known_divergences() -> Vec<(&'static str, LispVal)> {
    vec![
        // wasm: `set!` on a top-level value-define writes a dead local (reads
        // emit Call to the memoized fn) — counter never increments. Interp
        // mutates the env binding: 3.
        ("closures/counter", LispVal::Num(0)),
        // wasm: recursion through a memoizable define reuses the FIRST call's
        // result — (build 6) yields six 6s. Interp: (6 5 4 3 2 1).
        (
            "lists/cons-build",
            LispVal::List(vec![
                LispVal::Num(6),
                LispVal::Num(6),
                LispVal::Num(6),
                LispVal::Num(6),
                LispVal::Num(6),
                LispVal::Num(6),
            ]),
        ),
    ]
}

#[test]
fn shared_fixture_differential() {
    let known = known_divergences();
    let mut divergences = Vec::new();
    let mut pinned = Vec::new();
    let mut stale_pins = Vec::new();
    let mut skipped = Vec::new();
    let mut matched = 0usize;

    for f in fixtures() {
        eprintln!("FIXTURE {}", f.label);
        if wasm_compile_gate(f.src, f.near).is_err() {
            skipped.push(f.label);
            eprintln!("SKIP (compile gate): {}", f.label);
            continue;
        }
        let i_res = interp_run(f.src, f.input);
        let w_res = wasm_run(f.src, f.input, f.near);
        match (&i_res, &w_res) {
            (Ok(a), Ok(b)) => {
                let (ca, cb) = (canon(a.clone()), canon(b.clone()));
                if canon_str(&ca) == canon_str(&cb) {
                    if let Some((_, _)) = known.iter().find(|(l, _)| *l == f.label) {
                        // pinned bug now MATCHES — the engine was fixed;
                        // drop the pin in the same commit as the fix
                        stale_pins.push(f.label.to_string());
                    }
                    matched += 1;
                } else if let Some((_, want)) =
                    known.iter().find(|(l, _)| *l == f.label)
                {
                    let (cw, _) = (canon(want.clone()), canon(a.clone()));
                    if canon_str(&cw) == canon_str(&cb) {
                        pinned.push(f.label.to_string());
                    } else {
                        divergences.push(format!(
                            "[{}] value mismatch (pin drift — wasm no longer matches the \
                             pinned known divergence)\n  program: {}\n  interp: {}\n  wasm:   {}",
                            f.label,
                            f.src.trim(),
                            canon_str(&ca),
                            canon_str(&cb)
                        ));
                    }
                } else {
                    divergences.push(format!(
                        "[{}] value mismatch\n  program: {}\n  interp: {}\n  wasm:   {}",
                        f.label,
                        f.src.trim(),
                        canon_str(&ca),
                        canon_str(&cb)
                    ));
                }
            }
            (Err(_), Err(_)) => matched += 1, // both reject = agreement
            (a, b) => divergences.push(format!(
                "[{}] one-sided failure\n  program: {}\n  interp: {:?}\n  wasm:   {:?}",
                f.label,
                f.src.trim(),
                a,
                b
            )),
        }
    }

    assert!(
        stale_pins.is_empty(),
        "pinned known divergences now MATCH (engine fixed — remove pins): {:?}",
        stale_pins
    );
    assert!(
        divergences.is_empty(),
        "bytecode-VM vs wasm differential divergences ({}):\n{}",
        divergences.len(),
        divergences.join("\n")
    );
    assert!(
        skipped.len() <= SKIP_CAP,
        "too many fixtures failing the wasm compile gate ({}/{}): {:?} — the shared \
         fixture set is rotting; port the surface or prune the fixture deliberately",
        skipped.len(),
        fixtures().len(),
        skipped
    );
    assert!(
        matched + skipped.len() >= 20,
        "shared fixture matrix too small: {} matched + {} skipped",
        matched,
        skipped.len()
    );
    eprintln!(
        "differential: {} fixtures, {} matched, {} pinned-known-divergent, {} skipped (cap {})",
        fixtures().len(),
        matched,
        pinned.len(),
        skipped.len(),
        SKIP_CAP
    );
}
