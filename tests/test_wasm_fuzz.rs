//! Differential fuzz harness: ClosureVM vs tagged WASM emitter.
//!
//! Compiles a Lisp program with both:
//!   1. The ClosureVM interpreter (lisp_eval)
//!   2. The WASM emitter (compile_fuzz → wasmtime)
//!
//! Then compares results. The WASM emitter uses 3-bit tagged values:
//!   bottom 3 bits = type tag, upper 61 bits = payload
//!   TAG_NUM=0, TAG_BOOL=1, TAG_FNREF=2, TAG_CLOSURE=3, TAG_NIL=4, TAG_STR=5

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};
use lisp_rlm_wasm::wasm_emit::compile_fuzz;

// Tagged-value decoding: single source of truth is the runtime contract
// module (src/tagged_value.rs). The old local constants missed TAG_ARRAY=6,
// so valid array returns were misclassified as "INVALID TAG".
use lisp_rlm_wasm::tagged_value::{self, TaggedValue};
use std::sync::{Arc, Mutex};

const TEMP_MEM_OFF: usize = tagged_value::TEMP_MEM as usize;
const MAX_ARRAY_ELEMS: i64 = 4096;
const MAX_DECODE_DEPTH: u32 = 16;

enum DecodeErr {
    /// Value kind carries no comparable content (fnref/closure) — skip, as before.
    NotComparable,
    /// Pointer/length/count is bogus — heap-corruption-class bug signal.
    Corrupt(String),
}

/// Deep-decode a tagged i64 against WASM memory into a comparable LispVal.
/// Reads actual string bytes and array elements, so content is compared,
/// not just tags. Out-of-bounds pointers/lengths are corruption reports.
fn decode_deep(mem: &[u8], tagged: i64, depth: u32) -> Result<LispVal, DecodeErr> {
    if depth > MAX_DECODE_DEPTH {
        return Err(DecodeErr::NotComparable);
    }
    match tagged_value::decode(mem, tagged) {
        TaggedValue::Num(n) => Ok(LispVal::Num(n)),
        TaggedValue::Bool(b) => Ok(LispVal::Bool(b)),
        TaggedValue::Nil => Ok(LispVal::Nil),
        TaggedValue::Str { ptr, len } => {
            let ok = ptr >= 0 && len >= 0 && (ptr as usize) + (len as usize) <= mem.len();
            if !ok {
                return Err(DecodeErr::Corrupt(format!(
                    "Str ptr/len out of bounds: ptr={} len={} mem_len={}",
                    ptr,
                    len,
                    mem.len()
                )));
            }
            let bytes = &mem[ptr as usize..(ptr + len) as usize];
            match String::from_utf8(bytes.to_vec()) {
                Ok(s) => Ok(LispVal::Str(s)),
                Err(_) => Err(DecodeErr::Corrupt(format!(
                    "Str bytes not UTF-8: ptr={} len={}",
                    ptr, len
                ))),
            }
        }
        TaggedValue::Array { ptr, count } => {
            if count < 0 || count > MAX_ARRAY_ELEMS {
                return Err(DecodeErr::Corrupt(format!(
                    "Array implausible count: {} at ptr={}",
                    count, ptr
                )));
            }
            let base = ptr as usize;
            let end = base.saturating_add(8).saturating_add(count as usize * 8);
            if ptr < 0 || end > mem.len() {
                return Err(DecodeErr::Corrupt(format!(
                    "Array out of bounds: ptr={} count={} mem_len={}",
                    ptr,
                    count,
                    mem.len()
                )));
            }
            let mut elems = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let off = base + 8 + i * 8;
                let t = i64::from_le_bytes(mem[off..off + 8].try_into().unwrap());
                elems.push(decode_deep(mem, t, depth + 1)?);
            }
            Ok(LispVal::List(elems))
        }
        // Function references carry no comparable content
        TaggedValue::FnRef(_) | TaggedValue::Closure(_) => Err(DecodeErr::NotComparable),
    }
}

/// Structural equality over the comparable shapes (Num/Bool/Nil/Str/List
/// thereof). LispVal doesn't derive PartialEq (Memoized etc. make that
/// ill-defined), so the harness defines exactly what it compares.
fn comparable_eq(a: &LispVal, b: &LispVal) -> bool {
    match (a, b) {
        (LispVal::Num(x), LispVal::Num(y)) => x == y,
        (LispVal::Bool(x), LispVal::Bool(y)) => x == y,
        (LispVal::Nil, LispVal::Nil) => true,
        (LispVal::Str(x), LispVal::Str(y)) => x == y,
        (LispVal::List(xs), LispVal::List(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys.iter()).all(|(x, y)| comparable_eq(x, y))
        }
        _ => false,
    }
}

/// Normalize a ClosureVM result for deep comparison. Returns Some for value
/// shapes the wasm side can represent (Num/Bool/Nil/Str, Lists thereof),
/// None when the wasm emitter genuinely can't express the value.
fn comparable_lispval(v: &LispVal) -> Option<LispVal> {
    match v {
        LispVal::Num(_) | LispVal::Bool(_) | LispVal::Str(_) => Some(v.clone()),
        LispVal::Nil => Some(LispVal::Nil),
        LispVal::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(comparable_lispval(it)?);
            }
            Some(LispVal::List(out))
        }
        _ => None,
    }
}

/// Set up wasmtime with NEAR host function stubs, run the WASM module,
/// and capture observable behavior: the tagged return word at TEMP_MEM,
/// a post-run memory snapshot, host log_utf8 payloads, and fd_write stdout.
///
/// `log_utf8` and `fd_write` are implemented for real (capture); every other
/// import is a returning-0 stub as before.
#[cfg(not(target_arch = "wasm32"))]
/// Shared wasmtime Engine — creation + JIT setup is expensive; reuse it
/// across every fuzz case instead of paying per-case.
static SHARED_ENGINE: std::sync::OnceLock<wasmtime::Engine> = std::sync::OnceLock::new();

fn shared_engine() -> &'static wasmtime::Engine {
    SHARED_ENGINE.get_or_init(wasmtime::Engine::default)
}

/// What the wasm run produced (post-run memory included for deep decoding).
struct WasmRunOutcome {
    raw: i64,
    mem: Vec<u8>,
    /// Captured near/log_utf8 payloads — one entry per call, bytes as written.
    host_logs: Vec<Vec<u8>>,
    /// Captured fd_write bytes for fd 1/2 (WASI-style modules).
    stdout: Vec<u8>,
}

enum WasmRunError {
    /// Link/instantiate/module failure (harness or emitter bug class)
    Setup(String),
    /// Module executed and trapped
    Trap(String),
}

fn run_wasm_fuzz_deep(wasm: &[u8]) -> Result<WasmRunOutcome, WasmRunError> {
    use wasmtime::*;

    let engine = shared_engine();
    let module =
        Module::new(&engine, wasm).map_err(|e| WasmRunError::Setup(format!("module: {}", e)))?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    let host_logs: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let stdout_cap: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));

    // Check if module imports memory (NEAR mode) or declares it internally
    let needs_imported_memory = module
        .imports()
        .any(|i| i.module() == "env" && i.name() == "memory");

    if needs_imported_memory {
        let memory = Memory::new(&mut store, MemoryType::new(4, None))
            .map_err(|e| WasmRunError::Setup(format!("memory: {}", e)))?;
        linker
            .define(&store, "env", "memory", memory)
            .map_err(|e| WasmRunError::Setup(format!("link memory: {}", e)))?;
    }

    // Define stubs for every non-memory import, matching exact signatures.
    // log_utf8 and fd_write are real: they capture output for comparison.
    for import in module.imports() {
        if import.module() == "env" && import.name() == "memory" {
            continue;
        }
        let ty = import.ty();
        if let wasmtime::ExternType::Func(func_ty) = ty {
            let name = import.name().to_string();
            let params: Vec<ValType> = func_ty.params().collect();
            let results: Vec<ValType> = func_ty.results().collect();
            let ft = FuncType::new(&engine, params.clone(), results.clone());

            let stub = if name == "log_utf8" {
                // near log_utf8(log_len: u64, log_ptr: u64) — note len comes first
                let hl = Arc::clone(&host_logs);
                Func::new(&mut store, ft, move |mut caller, params, _ret| {
                    let len = params.first().and_then(|v| v.i64()).unwrap_or(0);
                    let ptr = params.get(1).and_then(|v| v.i64()).unwrap_or(0);
                    let mut captured = false;
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let data = mem.data(&caller);
                        if len >= 0 && ptr >= 0 && (ptr as usize) + (len as usize) <= data.len() {
                            hl.lock()
                                .unwrap()
                                .push(data[ptr as usize..(ptr + len) as usize].to_vec());
                            captured = true;
                        }
                    }
                    if !captured {
                        hl.lock()
                            .unwrap()
                            .push(format!("<log_utf8 oob len={} ptr={}", len, ptr).into_bytes());
                    }
                    Ok(())
                })
            } else if name == "fd_write" {
                // WASI fd_write(fd, iovs, iovs_len, nwritten_ptr) -> errno
                let so = Arc::clone(&stdout_cap);
                Func::new(&mut store, ft, move |mut caller, params, ret| {
                    let fd = params.first().and_then(|v| v.i32()).unwrap_or(-1);
                    let iovs = params.get(1).and_then(|v| v.i32()).unwrap_or(0) as u32 as usize;
                    let iovs_len = params.get(2).and_then(|v| v.i32()).unwrap_or(0) as u32 as usize;
                    let nw_ptr = params.get(3).and_then(|v| v.i32()).unwrap_or(0) as u32 as usize;
                    let mut total: u32 = 0;
                    let mut buf: Vec<u8> = Vec::new();
                    let mut err = 0i32;
                    if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                        let data = mem.data(&caller).to_vec();
                        for i in 0..iovs_len {
                            let base = iovs.saturating_add(i.saturating_mul(8));
                            if base.saturating_add(8) > data.len() {
                                err = 8; // EINVAL-ish
                                break;
                            }
                            let b = u32::from_le_bytes(data[base..base + 4].try_into().unwrap())
                                as usize;
                            let l = u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap())
                                as usize;
                            if b.saturating_add(l) <= data.len() {
                                buf.extend_from_slice(&data[b..b + l]);
                                total = total.saturating_add(l as u32);
                            }
                        }
                        if err == 0 {
                            let _ = mem.write(&mut caller, nw_ptr, &total.to_le_bytes());
                            if fd == 1 || fd == 2 {
                                so.lock().unwrap().extend_from_slice(&buf);
                            }
                        }
                    }
                    ret[0] = Val::I32(err);
                    Ok(())
                })
            } else {
                Func::new(&mut store, ft, move |_, _, ret| {
                    for (i, r) in ret.iter_mut().enumerate() {
                        *r = match results.get(i) {
                            Some(ValType::I32) => Val::I32(0),
                            _ => Val::I64(0),
                        };
                    }
                    Ok(())
                })
            };

            linker
                .define(&store, import.module(), import.name(), stub)
                .map_err(|e| WasmRunError::Setup(format!("link {}: {}", name, e)))?;
        }
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| WasmRunError::Setup(format!("instantiate: {}", e)))?;

    // Get memory from instance export (works for both internal and imported memory)
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| WasmRunError::Setup("no memory export".to_string()))?;

    // Call "run" export
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| WasmRunError::Setup(format!("no 'run' export: {}", e)))?;
    run.call(&mut store, ())
        .map_err(|e| WasmRunError::Trap(format!("{}", e)))?;

    // Snapshot memory + the tagged return word at TEMP_MEM
    let mem = memory.data(&store).to_vec();
    let raw = i64::from_le_bytes(mem[TEMP_MEM_OFF..TEMP_MEM_OFF + 8].try_into().unwrap());
    let logs_snapshot = host_logs.lock().unwrap().clone();
    let stdout_snapshot = stdout_cap.lock().unwrap().clone();
    Ok(WasmRunOutcome {
        raw,
        mem,
        host_logs: logs_snapshot,
        stdout: stdout_snapshot,
    })
}

/// Run a single fuzz test case: compare ClosureVM vs WASM.
#[cfg(not(target_arch = "wasm32"))]
/// Outcome of one fuzz case — makes skips VISIBLE. Without this, a probe can
/// pass vacuously (e.g. emitter doesn't support an op → compile skip → Ok()).
#[derive(Debug, PartialEq, Clone)]
enum FuzzVerdict {
    /// Deep value compare ran and matched; logs compared when wasm logged.
    Matched { logs_checked: bool },
    /// Pinned divergence accepted (documented, see is_pinned_print_return).
    Pinned,
    /// ClosureVM errored — comparison skipped (intentional divergence pin).
    SkippedVmError(String),
    /// wasm emitter can't compile this construct — comparison skipped.
    SkippedCompile,
    /// VM result not representable/comparable (float/vec/lambda/...) — skipped.
    Uncomparable,
}

fn fuzz_one(source: &str) -> Result<(), String> {
    let result = fuzz_one_verdict(source).map(|_| ());
    if let Err(ref e) = result {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/fuzz_errors.log")
        {
            let _ = writeln!(f, "FAIL {:?}: {}", source, e);
        }
    }
    result
}

/// Probe-friendly: returns the verdict so tests can assert NON-VACUOUS
/// comparison (Matched) instead of a silent skip.
fn fuzz_one_checked(source: &str) -> Result<FuzzVerdict, String> {
    fuzz_one_verdict(source)
}

fn fuzz_one_verdict(source: &str) -> Result<FuzzVerdict, String> {
    // 1. Parse
    let exprs = parse_all(source).map_err(|e| format!("parse error: {}", e))?;
    if exprs.is_empty() {
        return Ok(FuzzVerdict::Uncomparable);
    }

    // 2. ClosureVM evaluation — load stdlib
    let stdlib_exprs = match lisp_rlm_wasm::types::get_stdlib_code("core") {
        Some(code) => parse_all(code).map_err(|e| format!("stdlib parse: {}", e))?,
        None => Vec::new(),
    };
    let mut env = Env::new();
    let mut state = EvalState::new();

    // Evaluate stdlib
    for expr in &stdlib_exprs {
        lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state)
            .map_err(|e| format!("stdlib VM error: {}", e))?;
    }

    // Evaluate user expressions
    let mut cl_result = LispVal::Nil;
    for expr in &exprs {
        match lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state) {
            Ok(v) => cl_result = v,
            Err(e) => {
                // ClosureVM errored (e.g., div-by-zero, type error).
                // WASM handles these gracefully (returns 0, coerces types).
                // This is an intentional divergence — not a bug.
                return Ok(FuzzVerdict::SkippedVmError(e));
            }
        }
    }

    // If the source defines (run), call it to get the actual result
    if let Some(LispVal::Lambda { .. }) | Some(LispVal::BuiltinFn(_)) = env.get("run") {
        let run_call = parse_all("(run)").map_err(|e| format!("run parse: {}", e))?;
        if let Some(expr) = run_call.first() {
            match lisp_rlm_wasm::lisp_eval(expr, &mut env, &mut state) {
                Ok(v) => cl_result = v,
                Err(e) => {
                    // Same as above: VM error is acceptable divergence.
                    return Ok(FuzzVerdict::SkippedVmError(e));
                }
            }
        }
    }

    // 3. Compile to WASM
    // If the last expression is already (define (run) ...), use source as-is.
    // Otherwise wrap it in (define (run) ...) for the export wrapper.
    let last = &exprs[exprs.len() - 1];
    let is_define_run = matches!(last, LispVal::List(v) if v.len() >= 3
        && matches!(&v[0], LispVal::Sym(s) if s == "define")
        && matches!(&v[1], LispVal::List(n) if n.len() >= 1 && matches!(&n[0], LispVal::Sym(nm) if nm == "run")));

    let full_source = if is_define_run {
        source.to_string()
    } else if exprs.len() > 1 {
        let mut s = String::new();
        for expr in &exprs[..exprs.len() - 1] {
            s.push_str(&expr.to_string());
            s.push('\n');
        }
        s.push_str(&format!("(define (run) {})", last));
        s
    } else {
        format!("(define (run) {})", last)
    };

    let wasm = match compile_fuzz(&full_source) {
        Ok(w) => w,
        Err(_) => {
            // WASM emitter supports a subset of the language (no first-class closures,
            // no higher-order functions, etc.). Compilation failure is intentional
            // divergence, not a bug — skip comparison.
            return Ok(FuzzVerdict::SkippedCompile);
        }
    };

    // 4. Execute WASM (capturing logs + post-run memory)
    let outcome = match run_wasm_fuzz_deep(&wasm) {
        Ok(o) => o,
        Err(WasmRunError::Setup(e)) => return Err(format!("wasm setup: {}", e)),
        Err(WasmRunError::Trap(e)) => {
            return Err(format!(
                "WASM TRAP (ClosureVM succeeded): source={:?}\n  ClosureVM: {:?}\n  trap: {}",
                source, cl_result, e
            ));
        }
    };

    // 5. Deep comparison — content, not just tags.
    //
    // String and list returns are compared BY CONTENT now (the old harness
    // only checked tag validity for those, hiding wrong-content bugs).
    // Out-of-bounds ptr/len are reported as corruption — heap-corruption class.
    let expected = comparable_lispval(&cl_result);
    let decoded = decode_deep(&outcome.mem, outcome.raw, 0);

    match (expected, decoded) {
        (Some(exp), Ok(got)) => {
            if comparable_eq(&got, &exp) {
                check_logs(&state, &outcome)?;
                Ok(FuzzVerdict::Matched {
                    logs_checked: !outcome.host_logs.is_empty(),
                })
            } else if is_pinned_print_return(&cl_result, &got, &outcome.host_logs) {
                // Documented divergence — see is_pinned_print_return.
                Ok(FuzzVerdict::Pinned)
            } else {
                Err(format!(
                    "MISMATCH: source={:?}\n  ClosureVM: {:?}\n  WASM:      {:?} (raw tagged 0x{:016x})\n  logs: {:?}",
                    source,
                    cl_result,
                    got,
                    outcome.raw as u64,
                    log_strings(&outcome)
                ))
            }
        }
        (Some(_), Err(DecodeErr::Corrupt(c))) => Err(format!(
            "CORRUPT WASM VALUE: source={:?}\n  ClosureVM: {:?}\n  wasm decode: {} (raw tagged 0x{:016x})",
            source, cl_result, c, outcome.raw as u64
        )),
        (Some(_), Err(DecodeErr::NotComparable)) => Err(format!(
            "TYPE DIVERGENCE: source={:?}\n  ClosureVM: {:?}\n  WASM:      function reference (raw tagged 0x{:016x})",
            source, cl_result, outcome.raw as u64
        )),
        (None, Err(DecodeErr::Corrupt(c))) => Err(format!(
            "CORRUPT WASM VALUE (VM value not comparable): source={:?}\n  wasm decode: {} (raw tagged 0x{:016x})",
            source, c, outcome.raw as u64
        )),
        // VM value not comparable (float/vec/lambda/...) — nothing strict to
        // check on the wasm side; deep-decode success subsumes the old tag check.
        (None, _) => Ok(FuzzVerdict::Uncomparable),
    }
}

/// Pinned divergence (found 2026-09-10 during the deep-compare upgrade):
/// `(print x)` returns `Str(rendered)` in the ClosureVM
/// (src/dispatch/dispatch_state.rs, "print" arm) but `nil` in wasm
/// (src/wasm_emit/call_near_io.rs print arm returns TAG_NIL).
/// WASM is the semantic reference (GAPS.md 2026-08-26 anchor decision), so
/// this is pinned until the interpreter is aligned. Recognized ONLY when the
/// VM result is exactly the rendered text that was just logged and wasm
/// returned nil — anything else still fails.
fn is_pinned_print_return(cl: &LispVal, wasm: &LispVal, host_logs: &[Vec<u8>]) -> bool {
    match (cl, wasm) {
        (LispVal::Str(s), LispVal::Nil) => host_logs
            .iter()
            .any(|l| String::from_utf8_lossy(l).trim_end() == *s),
        _ => false,
    }
}

fn log_strings(outcome: &WasmRunOutcome) -> Vec<String> {
    outcome
        .host_logs
        .iter()
        .map(|l| String::from_utf8_lossy(l).to_string())
        .collect()
}

/// Compare the print channel: every wasm log_utf8 payload must match the
/// trailing entries of the VM's log (wasm executes only `run`, so its logs
/// are a suffix of the VM's top-level + run logs). Newline-insensitive per
/// entry (println's trailing newline differs between sides).
fn check_logs(state: &EvalState, outcome: &WasmRunOutcome) -> Result<(), String> {
    if outcome.host_logs.is_empty() {
        return Ok(());
    }
    let wasm_logs: Vec<String> = outcome
        .host_logs
        .iter()
        .map(|l| String::from_utf8_lossy(l).trim_end().to_string())
        .collect();
    let vm_logs: Vec<String> = state
        .logs
        .iter()
        .map(|s| s.trim_end().to_string())
        .collect();
    if wasm_logs.len() > vm_logs.len() {
        return Err(format!(
            "LOG MISMATCH: wasm logged {} entries, VM only {}\n  WASM: {:?}\n  VM:   {:?}",
            wasm_logs.len(),
            vm_logs.len(),
            wasm_logs,
            vm_logs
        ));
    }
    let tail = &vm_logs[vm_logs.len() - wasm_logs.len()..];
    if tail != wasm_logs.as_slice() {
        return Err(format!(
            "LOG MISMATCH:\n  VM:   {:?}\n  WASM: {:?}",
            tail, wasm_logs
        ));
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;

    // === Deep-compare probes (2026-09-10 upgrade) ===
    // These exercise channels the old tag-only comparison was blind to:
    // string CONTENT, list CONTENT, and the print/log channel.
    // All assert fuzz_one_checked == Matched — a NON-VACUOUS comparison
    // (a compile-skip or VM-error-skip would fail the assert; this catches
    // probe rot when an op silently leaves the emitter surface).

    #[test]
    fn deep_string_return_content() {
        // Old harness: Str on both sides → tag check only. Now: byte-exact.
        for src in [
            r#"(define (run) (str-cat "foo" "bar"))"#,
            r#"(define (run) "literal")"#,
            r#"(define (run) (str-cat "" ""))"#,
        ] {
            let v = fuzz_one_checked(src).unwrap_or_else(|e| panic!("{}: {}", src, e));
            assert_eq!(
                v,
                FuzzVerdict::Matched {
                    logs_checked: false
                },
                "vacuous or divergent: {}",
                src
            );
        }
    }

    #[test]
    fn deep_list_return_content() {
        for src in [
            "(define (run) (list 1 2 3))",
            "(define (run) (list))",
            "(define (run) (list 1 (list 2 3) 4))",
        ] {
            let v = fuzz_one_checked(src).unwrap_or_else(|e| panic!("{}: {}", src, e));
            assert_eq!(
                v,
                FuzzVerdict::Matched {
                    logs_checked: false
                },
                "vacuous or divergent: {}",
                src
            );
        }
    }

    #[test]
    fn deep_list_of_strings() {
        let v = fuzz_one_checked(r#"(define (run) (list "a" "b"))"#).unwrap();
        assert_eq!(
            v,
            FuzzVerdict::Matched {
                logs_checked: false
            }
        );
    }

    #[test]
    fn deep_print_channel() {
        // print renders per interpreter to_string() in NEAR mode — logs must match
        let v = fuzz_one_checked(r#"(define (run) (begin (print "hello") 42))"#)
            .unwrap_or_else(|e| panic!("{}", e));
        assert_eq!(v, FuzzVerdict::Matched { logs_checked: true });
        let v = fuzz_one_checked("(define (run) (begin (print 7) 8))").unwrap();
        assert_eq!(v, FuzzVerdict::Matched { logs_checked: true });
    }

    #[test]
    fn deep_string_arith_mixed() {
        // number->string is INTERPRETER-ONLY (dispatch_arithmetic.rs) — the
        // emitter can't compile it, so this is SkippedCompile, not Matched.
        // The lesson: unchecked probes rot silently. Pinned here as documentation.
        let v = fuzz_one_checked(r#"(define (run) (str-cat "x" (number->string 42)))"#).unwrap();
        assert_eq!(v, FuzzVerdict::SkippedCompile);
    }

    // ── Edge probes: the corners where divergences hide ──

    #[test]
    fn edge_unicode_strings() {
        // UTF-8: byte vs char semantics (str-length, str-cat content)
        for src in [
            r#"(define (run) (str-cat "café" "日本語"))"#,
            r#"(define (run) (str-length "café"))"#,
            r#"(define (run) (str-length "日本語"))"#,
            r#"(define (run) (str-contains "héllo" "é"))"#,
        ] {
            let v = fuzz_one_checked(src).unwrap_or_else(|e| panic!("{}: {}", src, e));
            assert_eq!(
                v,
                FuzzVerdict::Matched {
                    logs_checked: false
                },
                "diverges: {}",
                src
            );
        }
    }

    #[test]
    fn edge_empty_needle() {
        // (str-index-of "abc" "") and (str-contains "abc" "") — edge semantics
        let v = fuzz_one_checked(r#"(define (run) (str-index-of "abc" ""))"#);
        let v = v.unwrap_or_else(|e| panic!("index-of empty: {}", e));
        assert_eq!(
            v,
            FuzzVerdict::Matched {
                logs_checked: false
            }
        );
        let v = fuzz_one_checked(r#"(define (run) (str-contains "abc" ""))"#).unwrap();
        assert_eq!(
            v,
            FuzzVerdict::Matched {
                logs_checked: false
            }
        );
    }

    #[test]
    fn edge_str_cat_coercion() {
        // PINNED (2026-09-10): str-cat does NOT coerce numbers — the VM errors
        // ("expected string argument... convert explicitly with (to-string x)")
        // → SkippedVmError. If this ever becomes Matched or a TRAP-fire, the
        // coercion contract changed and callers must be re-audited.
        let v = fuzz_one_checked(r#"(define (run) (str-cat "x" 42))"#).unwrap();
        assert!(
            matches!(v, FuzzVerdict::SkippedVmError(_)),
            "str-cat coercion contract changed: {:?}",
            v
        );
    }

    #[test]
    fn edge_to_string_types() {
        // to-string is the sanctioned num→str op (both surfaces support it).
        // Each type renders — content compared byte-exact.
        for src in [
            "(define (run) (to-string 42))",
            "(define (run) (to-string -7))",
            "(define (run) (to-string 0))",
            r#"(define (run) (str-cat "n=" (to-string 123)))"#,
            "(define (run) (to-string true))",
            "(define (run) (to-string nil))",
            r#"(define (run) (to-string "abc"))"#, // Display quotes strings — does wasm?
        ] {
            let v = fuzz_one_checked(src).unwrap_or_else(|e| panic!("{}: {}", src, e));
            assert_eq!(
                v,
                FuzzVerdict::Matched {
                    logs_checked: false
                },
                "diverges: {}",
                src
            );
        }
    }

    #[test]
    fn fuzz_basic_arithmetic() {
        assert!(fuzz_one("(+ 1 2)").is_ok(), "1+2");
        assert!(fuzz_one("(+ -10 25)").is_ok(), "-10+25");
        assert!(fuzz_one("(+ 0 0)").is_ok(), "0+0");
        assert!(fuzz_one("(+ 9223372036854775807 0)").is_ok(), "max+0");
        assert!(fuzz_one("(+ -9223372036854775808 0)").is_ok(), "min+0");
    }

    #[test]
    fn fuzz_subtraction() {
        let r = fuzz_one("(- 10 3)");
        if r.is_err() {
            eprintln!("sub: {:?}", r);
        }
        assert!(r.is_ok());
        let r = fuzz_one("(- 0 0)");
        if r.is_err() {
            eprintln!("0-0: {:?}", r);
        }
        assert!(r.is_ok());
        let r = fuzz_one("(- 5)");
        if r.is_err() {
            eprintln!("neg5: {:?}", r);
        }
        assert!(r.is_ok());
    }

    #[test]
    fn fuzz_multiplication() {
        assert!(fuzz_one("(* 3 7)").is_ok());
        assert!(fuzz_one("(* 0 999)").is_ok());
        assert!(fuzz_one("(* -1 1)").is_ok());
    }

    #[test]
    fn fuzz_division() {
        assert!(fuzz_one("(/ 10 3)").is_ok());
        assert!(fuzz_one("(/ 100 10)").is_ok());
        assert!(fuzz_one("(/ -7 2)").is_ok());
    }

    #[test]
    fn fuzz_modulo() {
        assert!(fuzz_one("(mod 10 3)").is_ok());
        assert!(fuzz_one("(mod 7 7)").is_ok());
    }

    #[test]
    fn fuzz_comparisons() {
        assert!(fuzz_one("(> 5 3)").is_ok());
        assert!(fuzz_one("(< 1 2)").is_ok());
        assert!(fuzz_one("(>= 5 5)").is_ok());
        assert!(fuzz_one("(<= 3 3)").is_ok());
        assert!(fuzz_one("(= 7 7)").is_ok());
        assert!(fuzz_one("(!= 3 4)").is_ok());
        assert!(fuzz_one("(= 0 0)").is_ok());
        assert!(fuzz_one("(= -1 -1)").is_ok());
    }

    #[test]
    fn fuzz_logic() {
        let cases = [
            ("(and 1 2)", "and-1-2"),
            ("(and 0 1)", "and-0-1"),
            ("(or 0 1)", "or-0-1"),
            ("(or 0 0)", "or-0-0"),
            ("(not 1)", "not-1"),
            ("(not 0)", "not-0"),
        ];
        for (src, label) in &cases {
            if let Err(e) = fuzz_one(src) {
                panic!("{} failed: {}", label, e);
            }
        }
    }

    #[test]
    fn fuzz_if() {
        assert!(fuzz_one("(if 1 42 99)").is_ok());
        assert!(fuzz_one("(if 0 42 99)").is_ok());
        assert!(fuzz_one("(if nil 42 99)").is_ok());
        assert!(fuzz_one("(if false 42 99)").is_ok());
        assert!(fuzz_one("(if true 42)").is_ok());
    }

    #[test]
    fn fuzz_let() {
        assert!(fuzz_one("(let ((x 10) (y 20)) (+ x y))").is_ok());
        assert!(fuzz_one("(let ((a 5)) (* a a))").is_ok());
    }

    #[test]
    fn fuzz_begin() {
        assert!(fuzz_one("(begin 1 2 3)").is_ok());
        assert!(fuzz_one("(begin (+ 1 2) (* 3 4))").is_ok());
    }

    #[test]
    fn fuzz_set() {
        assert!(fuzz_one("(define (run) (let ((x 10)) (set! x 20) x))").is_ok());
    }

    #[test]
    fn fuzz_function_call() {
        assert!(fuzz_one("(define (add a b) (+ a b))\n(add 3 4)").is_ok());
        assert!(fuzz_one("(define (square x) (* x x))\n(square 7)").is_ok());
        assert!(fuzz_one("(define (id x) x)\n(id 42)").is_ok());
    }

    #[test]
    fn fuzz_nested_calls() {
        assert!(fuzz_one("(define (f x) (+ x 1))\n(f (f (f 0)))").is_ok());
    }

    #[test]
    fn fuzz_fibonacci() {
        assert!(fuzz_one(
            "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))\n(fib 10)"
        )
        .is_ok());
    }

    // ClosureVM doesn't support `while` as a built-in
    #[test]
    #[ignore]
    fn fuzz_while_loop() {
        assert!(fuzz_one(
            "(define (run) (let ((sum 0) (i 0)) (while (< i 10) (set! sum (+ sum i)) (set! i (+ i 1))) sum))"
        ).is_ok());
    }

    // ClosureVM doesn't support `for` loop compilation
    #[test]
    #[ignore]
    fn fuzz_for_loop() {
        assert!(
            fuzz_one("(define (run) (let ((sum 0)) (for i 1 11 (set! sum (+ sum i))) sum))")
                .is_ok()
        );
    }

    #[test]
    fn fuzz_abs() {
        assert!(fuzz_one("(abs -5)").is_ok());
        assert!(fuzz_one("(abs 3)").is_ok());
        assert!(fuzz_one("(abs 0)").is_ok());
    }

    #[test]
    fn fuzz_zero_distinguishing() {
        // The whole point of tagging: Num(0), Bool(false), Nil are all different
        let r = fuzz_one("0");
        if r.is_err() {
            eprintln!("0: {:?}", r);
        }
        assert!(r.is_ok(), "0");
        let r = fuzz_one("false");
        if r.is_err() {
            eprintln!("false: {:?}", r);
        }
        assert!(r.is_ok(), "false");
        let r = fuzz_one("nil");
        if r.is_err() {
            eprintln!("nil: {:?}", r);
        }
        assert!(r.is_ok(), "nil");
        let r = fuzz_one("true");
        if r.is_err() {
            eprintln!("true: {:?}", r);
        }
        assert!(r.is_ok(), "true");
        let r = fuzz_one("1");
        if r.is_err() {
            eprintln!("1: {:?}", r);
        }
        assert!(r.is_ok(), "1");
    }

    #[test]
    fn fuzz_chained_arithmetic() {
        assert!(fuzz_one("(+ 1 (* 2 3) (- 10 5))").is_ok());
        assert!(fuzz_one("(* (+ 1 2) (- 10 3) (/ 100 2))").is_ok());
    }

    #[test]
    fn fuzz_closure() {
        assert!(fuzz_one(
            "(define (make-adder n) (lambda (x) (+ x n)))\n(define (run) ((make-adder 10) 5))"
        )
        .is_ok());
    }

    #[test]
    fn fuzz_conditional_arithmetic() {
        assert!(fuzz_one("(if (> 3 2) (+ 1 2) (* 3 4))").is_ok());
        assert!(fuzz_one("(if (< 1 0) 999 42)").is_ok());
    }

    #[test]
    fn fuzz_deep_nesting() {
        assert!(fuzz_one("(+ (+ (+ 1 2) (+ 3 4)) (+ (+ 5 6) (+ 7 8)))").is_ok());
    }

    #[test]
    fn fuzz_identity_functions() {
        assert!(fuzz_one("(define (run) (let ((x 42)) x))").is_ok());
        assert!(fuzz_one("(define (id x) x) (define (run) (id 42))").is_ok());
    }

    #[test]
    fn fuzz_bool_ops_edge_cases() {
        assert!(fuzz_one("(and true true)").is_ok());
        assert!(fuzz_one("(and true false)").is_ok());
        assert!(fuzz_one("(and false false)").is_ok());
        assert!(fuzz_one("(or true false)").is_ok());
        assert!(fuzz_one("(or false false)").is_ok());
        assert!(fuzz_one("(or true true)").is_ok());
    }

    #[test]
    fn fuzz_closure_set() {
        // Closure captures a mutable cell and increments it on each call
        assert!(fuzz_one(
            "(define (make-counter init) (let ((n init)) (lambda () (set! n (+ n 1)) n)))\n(define (run) ((make-counter 5)))"
        ).is_ok());
    }

    #[test]
    fn fuzz_recursive_sum() {
        // sum(n) = 0 + 1 + ... + n via recursion
        assert!(fuzz_one(
            "(define (sum n) (if (= n 0) 0 (+ n (sum (- n 1)))))\n(define (run) (sum 10))"
        )
        .is_ok());
    }

    #[test]
    fn fuzz_higher_order() {
        // Function that takes another function as an argument
        assert!(fuzz_one(
            "(define (apply-twice f x) (f (f x)))\n(define (inc x) (+ x 1))\n(define (run) (apply-twice inc 5))"
        ).is_ok());
    }

    #[test]
    fn fuzz_nested_let_arithmetic() {
        // Three levels of nested let, each building on the previous
        assert!(
            fuzz_one("(let ((a 3)) (let ((b (+ a 4))) (let ((c (* b 2))) (+ a b c))))").is_ok()
        );
    }

    #[test]
    fn fuzz_multi_let() {
        // let with 3 bindings
        assert!(fuzz_one("(let ((a 10) (b 20) (c 30)) (+ a (+ b c)))").is_ok());
    }

    #[test]
    fn fuzz_let_with_if() {
        // let binding whose value depends on if
        assert!(fuzz_one("(let ((x (if (> 3 2) 10 20))) x)").is_ok());
    }

    #[test]
    fn fuzz_recursive_fact() {
        // Factorial via recursion
        assert!(fuzz_one(
            "(define (fact n) (if (< n 2) 1 (* n (fact (- n 1)))))\n(define (run) (fact 6))"
        )
        .is_ok());
    }

    #[test]
    fn fuzz_closure_in_let() {
        // Closure defined in let, immediately invoked
        assert!(fuzz_one("(let ((add3 (lambda (x) (+ x 3)))) (add3 7))").is_ok());
    }

    #[test]
    fn fuzz_set_in_nested_let() {
        // set! mutates a binding from an outer let scope
        assert!(
            fuzz_one("(define (run) (let ((x 10)) (let ((y 20)) (set! x (+ x y)) x)))").is_ok()
        );
    }
}

// ── Property-based differential fuzz ──
//
// Generates random Lisp programs, runs on ClosureVM and WASM, compares results.
// Catches bugs in emitter instruction lowering that hardcoded tests miss.

#[cfg(not(target_arch = "wasm32"))]
mod prop {
    use super::*;
    use proptest::prelude::*;
    use proptest::prop_compose;

    /// Generate a random safe integer.
    fn safe_int() -> impl Strategy<Value = i64> {
        -1000i64..1000i64
    }

    /// Generate a random arithmetic operator.
    fn arith_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("+"), Just("-"), Just("*"), Just("/"), Just("mod")]
    }

    /// Generate a random comparison operator.
    fn cmp_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![
            Just(">"),
            Just("<"),
            Just(">="),
            Just("<="),
            Just("="),
            Just("!=")
        ]
    }

    /// Generate a random logic operator.
    fn logic_op() -> impl Strategy<Value = &'static str> {
        prop_oneof![Just("and"), Just("or"), Just("not")]
    }

    /// Generate a simple expression (depth 0 — leaf or binary op on leaves).
    fn leaf_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            safe_int().prop_map(|n| n.to_string()),
            Just("true".into()),
            Just("false".into()),
            Just("nil".into()),
        ]
    }

    /// Generate a numeric leaf (integers only — avoids WASM/ClosureVM type coercion divergence).
    fn num_leaf() -> impl Strategy<Value = String> {
        safe_int().prop_map(|n| n.to_string())
    }

    /// Generate a binary expression at depth 1: (op leaf leaf).
    /// Arithmetic and comparison ops use numeric leaves only to avoid
    /// WASM vs ClosureVM type coercion divergence (WASM coerces bool/nil to numbers).
    fn binary_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            (arith_op(), num_leaf(), num_leaf())
                .prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
            (cmp_op(), num_leaf(), num_leaf())
                .prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
            (logic_op(), leaf_expr(), leaf_expr())
                .prop_map(|(op, l, r)| format!("({} {} {})", op, l, r)),
        ]
    }

    /// Generate an if expression: (if cond then else).
    fn if_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr(), leaf_expr())
            .prop_map(|(c, t, e)| format!("(if {} {} {})", c, t, e))
    }

    /// Generate a let expression: (let ((x val)) body).
    fn let_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr()).prop_map(|(val, body)| format!("(let ((x {})) {})", val, body))
    }

    /// Generate a begin expression: (begin a b).
    fn begin_expr() -> impl Strategy<Value = String> {
        (leaf_expr(), leaf_expr()).prop_map(|(a, b)| format!("(begin {} {})", a, b))
    }

    /// Generate a recursive function definition + call that returns a number.
    /// Produces a full multi-expression program (no program() wrapping needed).
    /// Pattern: (define (f x) (if (< x base) base (+ step (f (- x step)))))
    fn recursive_fn_expr() -> impl Strategy<Value = String> {
        (safe_int(), safe_int(), safe_int()).prop_map(|(base, step, arg)| {
            let base = base % 5; // keep base small to limit recursion depth
            let step = if step % 2 == 0 { 1 } else { -1 }; // ±1 step
            let arg = arg % 10; // keep arg small
            format!(
                "(define (f x) (if (< x {}) {} (+ 1 (f (- x {})))))\n(define (run) (f {}))",
                base, base, step, arg
            )
        })
    }

    /// Generate a closure that captures a number and returns a number.
    /// Pattern: (let ((f (lambda (x) (+ x N)))) (f M))
    fn closure_expr() -> impl Strategy<Value = String> {
        (safe_int(), safe_int())
            .prop_map(|(n, m)| format!("(let ((f (lambda (x) (+ x {})))) (f {}))", n, m))
    }

    /// Generate a set! expression that returns a number.
    /// Pattern: (let ((x N)) (set! x M) x)
    fn set_expr() -> impl Strategy<Value = String> {
        (safe_int(), safe_int()).prop_map(|(n, m)| format!("(let ((x {})) (set! x {}) x)", n, m))
    }

    /// Generate a let with two bindings that returns a number.
    /// Pattern: (let ((x N) (y M)) (+ x y))
    fn multi_let_expr() -> impl Strategy<Value = String> {
        (safe_int(), safe_int()).prop_map(|(n, m)| format!("(let ((x {}) (y {})) (+ x y))", n, m))
    }

    /// Generate a nested let that returns a number.
    /// Pattern: (let ((x N)) (let ((y (+ x 1))) (+ x y)))
    fn nested_let_expr() -> impl Strategy<Value = String> {
        safe_int().prop_map(|n| format!("(let ((x {})) (let ((y (+ x 1))) (+ x y)))", n))
    }

    /// Generate a loop/recur expression that returns a number.
    /// Pattern: (loop [i 0] (< i N) (recur (+ i 1)) i)
    fn loop_expr() -> impl Strategy<Value = String> {
        (1i64..10i64).prop_map(|n| format!("(loop [i 0] (< i {}) (recur (+ i 1)) i)", n))
    }

    /// Generate a define + call pattern that returns a number.
    /// Produces a full multi-expression program (no program() wrapping needed).
    fn fn_call_expr() -> impl Strategy<Value = String> {
        (safe_int(), safe_int())
            .prop_map(|(a, b)| format!("(define (f x) (+ x {}))\n(define (run) (f {}))", a, b))
    }

    // ══════════════════════════════════════════════════════════════
    // String & list strategies (2026-09-10) — make the deep value
    // comparison actually BITE during fuzzing. Surface restricted to the
    // all-3-surface ops (interp ∩ emit ∩ checker — corpus/COVERAGE.md §D,
    // tests/equiv/e24 + e30): str-cat, str-concat, str-contains,
    // str-index-of, str-length, str-substring; list, len, car, cdr, cons.
    // ══════════════════════════════════════════════════════════════

    /// Safe ASCII alphabet for generated string literals — no quote/backslash,
    /// so embedding in source needs no escaping.
    const STR_ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -_.!?*()<>[]{}@#$%^&+=/:;|~`'";

    fn safe_char() -> impl Strategy<Value = char> {
        (0u8..STR_ALPHA.len() as u8).prop_map(|i| STR_ALPHA[i as usize] as char)
    }

    /// Random string literal source (quoted), 0..=14 chars, incl. empty.
    fn str_lit() -> impl Strategy<Value = String> {
        proptest::collection::vec(safe_char(), 0..14)
            .prop_map(|cs| format!("\"{}\"", cs.into_iter().collect::<String>()))
    }

    /// String-valued expression (≤2 levels of nesting).
    fn string_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            4 => str_lit(),
            2 => (str_lit(), str_lit())
                .prop_map(|(a, b)| format!("(str-cat {} {})", a, b)),
            1 => (str_lit(), str_lit(), str_lit())
                .prop_map(|(a, b, c)| format!("(str-cat {} {} {})", a, b, c)),
            1 => (str_lit(), str_lit())
                .prop_map(|(a, b)| format!("(str-concat {} {})", a, b)),
            2 => safe_int().prop_map(|n| format!("(to-string {})", n)),
            2 => (safe_int(), str_lit())
                .prop_map(|(n, s)| format!("(str-cat (to-string {}) {})", n, s)),
            1 => (str_lit(), 0u16..16, 0u16..16).prop_map(|(s, i, j)| {
                // in-range slice: start ≤ end ≤ len (char-count ≈ byte-count for ASCII)
                let n = s.len() as u16 - 2; // s includes the two quotes
                let start = i % (n + 1);
                let end = start + (j % (n + 1 - start));
                format!("(str-substring {} {} {})", s, start, end)
            }),
            1 => (str_lit(), str_lit()).prop_map(|(a, b)| {
                // nested: substring of a concatenation
                format!("(str-cat (str-substring {} 0 1) {})", a, b)
            }),
        ]
    }

    /// Value expression of any comparable type (num / bool / string / list).
    fn value_expr() -> impl Strategy<Value = String> {
        prop_oneof![
            3 => num_leaf(),
            1 => Just("true".into()),
            1 => Just("false".into()),
            1 => Just("nil".into()),
            3 => string_expr(),
            2 => proptest::collection::vec(value_leaf(), 0..4)
                .prop_map(|xs| {
                    let inner: Vec<String> = xs.into_iter().collect();
                    format!("(list {})", inner.join(" "))
                }),
        ]
    }

    fn value_leaf() -> impl Strategy<Value = String> {
        prop_oneof![
            2 => num_leaf(),
            1 => Just("true".into()),
            1 => Just("false".into()),
            1 => Just("nil".into()),
            2 => str_lit(),
        ]
    }

    /// ── string property programs ──

    /// (str-cat a b ...) — content-compared Str result.
    fn str_cat_prog() -> impl Strategy<Value = String> {
        proptest::collection::vec(str_lit(), 1..4).prop_map(|xs| {
            let inner: Vec<String> = xs.into_iter().collect();
            program_inner(format!("(str-cat {})", inner.join(" ")))
        })
    }

    /// (str-substring s start end) with in-range indices.
    fn str_substring_prog() -> impl Strategy<Value = String> {
        (str_lit(), 0u16..16, 0u16..16).prop_map(|(s, i, j)| {
            let n = s.len() as u16 - 2;
            let start = i % (n + 1);
            let end = start + (j % (n + 1 - start));
            program_inner(format!("(str-substring {} {} {})", s, start, end))
        })
    }

    /// (str-length s) — Num result.
    fn str_length_prog() -> impl Strategy<Value = String> {
        string_expr().prop_map(|s| program_inner(format!("(str-length {})", s)))
    }

    /// (str-contains haystack needle) / (str-index-of haystack needle) —
    /// Bool/Num results. Empty needles included on purpose: the edges are
    /// where divergences hide.
    fn str_search_prog() -> impl Strategy<Value = String> {
        (str_lit(), str_lit(), proptest::bool::ANY).prop_map(|(h, n, idx)| {
            let op = if idx { "str-index-of" } else { "str-contains" };
            program_inner(format!("({} {} {})", op, h, n))
        })
    }

    /// ── list property programs ──

    /// (list v...) — mixed-type elements, deep-compared as arrays.
    fn list_literal_prog() -> impl Strategy<Value = String> {
        proptest::collection::vec(value_leaf(), 0..5).prop_map(|xs| {
            let inner: Vec<String> = xs.into_iter().collect();
            program_inner(format!("(list {})", inner.join(" ")))
        })
    }

    /// len / car / cdr / cons / nth over a generated literal list.
    fn list_ops_prog() -> impl Strategy<Value = String> {
        (proptest::collection::vec(value_leaf(), 1..5), 0u8..255).prop_map(|(xs, op)| {
            let inner: Vec<String> = xs.clone().into_iter().collect();
            let lst = format!("(list {})", inner.join(" "));
            let n = xs.len();
            let body = match op % 5 {
                0 => format!("(len {})", lst),
                1 => format!("(car {})", lst),
                2 => format!("(cdr {})", lst),
                3 => format!("(cons {} {})", value_leaf_fixed(op), lst),
                _ => format!("(nth {} {})", (op as usize) % n, lst),
            };
            program_inner(body)
        })
    }

    /// Deterministic extra element for cons (keeps strategy monadic-free).
    fn value_leaf_fixed(seed: u8) -> String {
        match seed % 4 {
            0 => "7".into(),
            1 => "true".into(),
            2 => "\"k\"".into(),
            _ => "nil".into(),
        }
    }

    /// Strings through control flow — feature-pair coverage (the historical
    /// bug classes lived at op × control-flow intersections).
    fn string_control_flow_prog() -> impl Strategy<Value = String> {
        (str_lit(), str_lit(), 0i64..50, 0u8..255).prop_map(|(a, b, n, k)| {
            let body = match k % 4 {
                0 => format!("(let ((s {})) (str-cat s {}))", a, b),
                1 => format!("(if (< {} {}) {} {})", n, n + k as i64 + 1, a, b),
                2 => format!("(define (g s) (str-cat s \"!\"))\n(define (run) (g {}))", a),
                _ => format!("(begin (str-cat {} {}) {})", a, b, n),
            };
            if k % 4 == 2 {
                body
            } else {
                program_inner(body)
            }
        })
    }

    /// Print channel: (begin (println X) value) — log comparison bites.
    /// print is deliberately NOT in tail position (pinned return-value
    /// divergence is a separate concern).
    fn print_channel_prog() -> impl Strategy<Value = String> {
        (value_expr(), num_leaf())
            .prop_map(|(x, n)| program_inner(format!("(begin (println {}) {})", x, n)))
    }

    /// Wrap an expression body in (define (run) ...).
    fn program_inner(body: String) -> String {
        format!("(define (run) {})", body)
    }

    /// Wrap any expression in (define (run) ...).
    fn program(inner: impl Strategy<Value = String>) -> impl Strategy<Value = String> {
        inner.prop_map(|e| format!("(define (run) {})", e))
    }

    /// Cases per property for the proptest gate. Small by default so the
    /// normal suite stays fast; the deep pass is parallel_torture below.
    /// Override at runtime with PROPTEST_CASES=NNN.
    fn fuzz_cases() -> u32 {
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(256)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(fuzz_cases()))]
        /// Differential fuzz: random leaf expressions.
        #[test]
        fn prop_leaf(expr in program(leaf_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("leaf mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random binary arithmetic/comparison/logic.
        #[test]
        fn prop_binary(expr in program(binary_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("binary mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random if expressions.
        #[test]
        fn prop_if(expr in program(if_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("if mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random let expressions.
        #[test]
        fn prop_let(expr in program(let_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("let mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: random begin expressions.
        #[test]
        fn prop_begin(expr in program(begin_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("begin mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: chained arithmetic with 2-6 args.
        #[test]
        fn prop_chained(
            op in arith_op(),
            count in 2usize..=6,
            val in safe_int()
        ) {
            let args: Vec<String> = (0..count).map(|_| format!("{}", val)).collect();
            let expr = format!("(define (run) ({} {}))", op, args.join(" "));
            if let Err(e) = fuzz_one(&expr) {
                panic!("chained mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: nested if (1-4 levels deep).
        #[test]
        fn prop_nested_if(depth in 1usize..=4) {
            let mut s = "42".to_string();
            for _ in 0..depth {
                s = format!("(if 1 {} 0)", s);
            }
            let expr = format!("(define (run) {})", s);
            if let Err(e) = fuzz_one(&expr) {
                panic!("nested-if mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: mixed let with 1-3 bindings.
        #[test]
        fn prop_let_multi(
            n in 1usize..=3,
            body in leaf_expr()
        ) {
            let bindings: Vec<String> = (0..n)
                .map(|i| format!("(x{} {})", i, (i as i64 + 1) * 10))
                .collect();
            let expr = format!("(define (run) (let ({}) {}))", bindings.join(" "), body);
            if let Err(e) = fuzz_one(&expr) {
                panic!("let-multi mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: recursive function (define + call).
        #[test]
        fn prop_recursive_fn(expr in recursive_fn_expr()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("recursive-fn mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: closure capturing and applying a value.
        #[test]
        fn prop_closure(expr in program(closure_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("closure mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: set! mutation.
        #[test]
        fn prop_set_bang(expr in program(set_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("set! mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: let with two bindings.
        #[test]
        fn prop_multi_let(expr in program(multi_let_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("multi-let mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: nested let.
        #[test]
        fn prop_nested_let(expr in program(nested_let_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("nested-let mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: loop/recur.
        #[test]
        fn prop_loop(expr in program(loop_expr())) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("loop mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: define + call pattern.
        #[test]
        fn prop_fn_call(expr in fn_call_expr()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("fn-call mismatch: {}\nsource: {}", e, expr);
            }
        }

        // ── Extended generators ──

        /// Differential fuzz: cond expression (3 clauses).
        #[test]
        fn prop_cond(
            a in safe_int(), b in safe_int(), c in safe_int(), d in safe_int()
        ) {
            let expr = format!(
                "(define (run) (cond ((= {} {}) {}) ((= {} {}) {}) (1 {})))",
                a, a, b, a, b, c, d
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("cond mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: progn expression.
        #[test]
        fn prop_progn(
            a in safe_int(), b in safe_int(), c in safe_int()
        ) {
            let expr = format!("(define (run) (progn {} {} {}))", a, b, c);
            if let Err(e) = fuzz_one(&expr) {
                panic!("progn mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: nested closure (make-adder pattern).
        #[test]
        fn prop_nested_closure(n in safe_int(), m in safe_int()) {
            let expr = format!(
                "(define (make-adder n) (lambda (x) (+ x n)))\n(define (run) ((make-adder {}) {}))",
                n, m
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("nested-closure mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: closure with set! (make-counter pattern).
        #[test]
        fn prop_closure_set(n in safe_int()) {
            let n = n % 50; // keep small to avoid overflow
            let expr = format!(
                "(define (make-counter init) (let ((n init)) (lambda () (set! n (+ n 1)) n)))\n(define (run) ((make-counter {})))",
                n
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("closure-set mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: higher-order function (apply-twice pattern).
        #[test]
        fn prop_higher_order(n in safe_int()) {
            let expr = format!(
                "(define (apply-twice f x) (f (f x)))\n(define (inc x) (+ x 1))\n(define (run) (apply-twice inc {}))",
                n
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("higher-order mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: max/min builtins.
        #[test]
        fn prop_max_min(a in safe_int(), b in safe_int()) {
            let expr = format!("(define (run) (+ (max {} {}) (min {} {})))", a, b, a, b);
            if let Err(e) = fuzz_one(&expr) {
                panic!("max-min mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: map with named function.
        #[test]
        fn prop_map(n in safe_int(), _m in safe_int()) {
            let expr = format!(
                "(define (dbl x) (* x 2))\n(define (run) (car (map dbl (list (+ {} 1)))))",
                n
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("map mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: filter with named function.
        #[test]
        fn prop_filter(a in safe_int(), b in safe_int()) {
            // Ensure at least one positive element so car doesn't fail on empty list
            let b = if a <= 0 && b <= 0 { 1 } else { b };
            let expr = format!(
                "(define (is-pos x) (> x 0))\n(define (run) (car (filter is-pos (list {} {}))))",
                a, b
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("filter mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: reduce with named function.
        #[test]
        fn prop_reduce(a in safe_int(), b in safe_int()) {
            let expr = format!(
                "(define (my-add x acc) (+ x acc))\n(define (run) (reduce my-add 0 (list {} {})))",
                a, b
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("reduce mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: mutual recursion (even/odd).
        #[test]
        fn prop_mutual_recursion(n in 0i64..10i64) {
            let expr = format!(
                "(define (my-even n) (if (= n 0) 1 (my-odd (- n 1))))\n\
                 (define (my-odd n) (if (= n 0) 0 (my-even (- n 1))))\n\
                 (define (run) (my-even {}))",
                n
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("mutual-rec mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: deeply nested arithmetic (stress test).
        #[test]
        fn prop_deep_arith(depth in 2usize..=5, n in safe_int()) {
            let mut s = format!("{}", n);
            for _ in 0..depth {
                s = format!("(+ {} 1)", s);
            }
            let expr = format!("(define (run) {})", s);
            if let Err(e) = fuzz_one(&expr) {
                panic!("deep-arith mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: nested closures (double wrapping).
        #[test]
        fn prop_double_closure(a in safe_int(), b in safe_int(), _c in safe_int()) {
            let expr = format!(
                "(define (make-adder n) (lambda (x) (+ x n)))\n\
                 (define (make-apply f) (lambda (x) (f x)))\n\
                 (define (run) ((make-apply (make-adder {})) {}))",
                a, b
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("double-closure mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: shadowing in nested let.
        #[test]
        fn prop_shadow_let(a in safe_int(), b in safe_int()) {
            let expr = format!(
                "(define (run) (let ((x {})) (let ((x {})) x)))",
                a, b
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("shadow-let mismatch: {}\nsource: {}", e, expr);
            }
        }

        /// Differential fuzz: value define (fn value in variable).
        #[test]
        fn prop_value_define(n in safe_int()) {
            let expr = format!(
                "(define f (lambda (x) (+ x {})))\n(define (run) (f {}))",
                n, n
            );
            if let Err(e) = fuzz_one(&expr) {
                panic!("value-define mismatch: {}\nsource: {}", e, expr);
            }
        }

        // ═══ String & list differential fuzz (2026-09-10) ═══
        // These drive the DEEP value comparison: Str content, list content,
        // and the log channel — the channels the old tag-only harness never
        // checked. Surface: all-3-surface ops only (see strategy docs).

        #[test]
        fn prop_str_cat(expr in str_cat_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("str-cat content mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_str_substring(expr in str_substring_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("str-substring content mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_str_length(expr in str_length_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("str-length mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_str_search(expr in str_search_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("str-contains/index-of mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_list_literal(expr in list_literal_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("list content mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_list_ops(expr in list_ops_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("list-ops mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_string_control_flow(expr in string_control_flow_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("string×control-flow mismatch: {}\nsource: {}", e, expr);
            }
        }

        #[test]
        fn prop_print_channel(expr in print_channel_prog()) {
            if let Err(e) = fuzz_one(&expr) {
                panic!("print-channel mismatch: {}\nsource: {}", e, expr);
            }
        }
    }

    /// Parallel differential torture — same generator space as the proptest gate
    /// above, but cases are spread across all cores (rayon chunks) and the
    /// wasmtime Engine is shared. This is the "run more tests" path:
    ///
    ///   WASM_TORTURE_CASES=50000 cargo test --test test_wasm_fuzz parallel_torture
    ///   WASM_TORTURE_SKIP=1 cargo test            # skip it in quick runs
    ///
    /// Defaults to 10_000 total cases split across the property mix.
    /// On mismatch: prints the property name, full source, and error; re-run the
    /// source through fuzz_one / the ddmin shrinker for minimization.
    mod torture {
        use super::super::*;
        use super::{
            arith_op, begin_expr, binary_expr, closure_expr, fn_call_expr, if_expr, leaf_expr,
            let_expr, loop_expr, multi_let_expr, nested_let_expr, num_leaf, program,
            recursive_fn_expr, safe_int, set_expr,
        };
        use proptest::prelude::*;
        use proptest::test_runner::{Config as RunnerConfig, TestRunner};

        #[test]
        fn parallel_torture() {
            use rayon::prelude::*;

            if std::env::var("WASM_TORTURE_SKIP").is_ok() {
                eprintln!("parallel_torture: skipped (WASM_TORTURE_SKIP set)");
                return;
            }
            let total: usize = std::env::var("WASM_TORTURE_CASES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10_000);

            type Ctor = fn() -> BoxedStrategy<String>;
            let props: Vec<(&'static str, Ctor)> = vec![
                ("leaf", || program(leaf_expr()).boxed()),
                ("binary", || program(binary_expr()).boxed()),
                ("if", || program(if_expr()).boxed()),
                ("let", || program(let_expr()).boxed()),
                ("begin", || program(begin_expr()).boxed()),
                ("chained", || {
                    (arith_op(), num_leaf())
                        .prop_map(|(op, val)| {
                            let args: Vec<String> = (0..4).map(|_| val.to_string()).collect();
                            format!("(define (run) ({} {}))", op, args.join(" "))
                        })
                        .boxed()
                }),
                ("recursive_fn", || recursive_fn_expr().boxed()),
                ("closure", || program(closure_expr()).boxed()),
                ("set_bang", || program(set_expr()).boxed()),
                ("multi_let", || program(multi_let_expr()).boxed()),
                ("nested_let", || program(nested_let_expr()).boxed()),
                ("loop", || program(loop_expr()).boxed()),
                ("fn_call", || fn_call_expr().boxed()),
                ("cond", || {
                    (safe_int(), safe_int(), safe_int(), safe_int())
                        .prop_map(|(a, b, c, d)| {
                            format!("(define (run) (if (> {a} {b}) (+ {c} {d}) (- {c} {d})))")
                        })
                        .boxed()
                }),
            ];

            let per_prop = (total / props.len()).max(1);
            let chunk = 64usize;
            let t0 = std::time::Instant::now();

            let mut failures: Vec<(String, String)> = Vec::new();
            for (name, ctor) in &props {
                let name = *name;
                let n_chunks = (per_prop + chunk - 1) / chunk;
                let mut prop_fails: Vec<(String, String)> = (0..n_chunks)
                    .into_par_iter()
                    .filter_map(|ci| {
                        // Each rayon task builds its own strategy (not Sync-shareable)
                        let strat = ctor();
                        let mut runner = TestRunner::new(RunnerConfig::default());
                        let remaining = per_prop.saturating_sub(ci * chunk).min(chunk);
                        for _ in 0..remaining {
                            let tree = match strat.new_tree(&mut runner) {
                                Ok(t) => t,
                                Err(e) => {
                                    return Some((
                                        name.to_string(),
                                        format!("generator reject: {e}"),
                                    ))
                                }
                            };
                            let source = tree.current().clone();
                            if let Err(e) = fuzz_one(&source) {
                                return Some((name.to_string(), format!("{e}\nsource: {source}")));
                            }
                        }
                        None
                    })
                    .collect();
                failures.append(&mut prop_fails);
            }

            let dt = t0.elapsed().as_secs_f64();
            let n = per_prop * props.len();
            eprintln!(
                "parallel_torture: {n} cases across {} properties in {dt:.1}s ({:.0} cases/s)",
                props.len(),
                n as f64 / dt
            );
            assert!(
                failures.is_empty(),
                "differential mismatches:\n{:#?}",
                failures
            );
        }
    }
}
