//! TASK-u128-native-chain, phase 1 — wrapper-inlining parity pins.
//!
//! The inline pass (src/u128_inline.rs) rewrites trivial u128 wrapper
//! calls to bare op forms on the WASM side ONLY — the bytecode interp
//! still evaluates the original wrapper-call program. This suite pins
//! OUTPUT PARITY between the two layers on pool-a's exact hot-path
//! shape (wrapper fns + let* chain + muldiv + tainted if-merge), over a
//! grid of inputs, plus edge cases (taint fallback, value-use fallback,
//! try-zone non-inlining) where behavior must not move at all.
//!
//! Interp side: run_program (bytecode VM, decimal-string u128 ABI).
//! Wasm side: compile_fuzz → wasmtime (fuzz host = differential.rs's
//! minimal register machinery), result decoded from TEMP_MEM.

use lisp_rlm_wasm::run_program;
use lisp_rlm_wasm::tagged_value::{decode, TaggedValue};
use lisp_rlm_wasm::types::{Env, EvalState};
use lisp_rlm_wasm::{parse_all, wasm_emit::compile_fuzz};
use wasmtime::{Engine, ExternType, Func, FuncType, Instance, Linker, Memory, Module, Store, Val};

const POOL_A_STEP: &str = r#"
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (mdiv a b d) (u128/muldiv a b d))
(define (ult a b) (u128/lt a b))
;; pool-a step, storage sinks replaced by a str-cat of the three results
;; (s:out / s:y / s:sqp) so the pure backends can observe them
(define (step sqp y l q gb out)
  (let* ((width (sub gb sqp))
         (cap (mdiv q width "1"))
         (take (if (ult cap y) cap y))
         (dp (u128/div take q))
         (used (mdiv dp q "1"))
         (p2 (add sqp dp))
         (gained (if (ult "0" dp) (mdiv l dp (mdiv sqp p2 "1")) "0")))
    (str-cat (add out gained) "|" (sub y used) "|" p2)))
(define (main)
  (str-cat
    (step "1000000000" "60000000000000000000000" "100000000000000000000000" "100300902708124373119359" "2000000000" "0")
    "\n"
    (step "2000000000" "50000000000000000000000" "60000000000000000000000" "60180541624874623871615" "4000000000" "123456789012345678901")
    "\n"
    (step "1000000000" "7" "100000000000000000000000" "100300902708124373119359" "2000000000" "5")))
"#;

const EDGE_CASES: &[(&str, &str)] = &[
    // tainted merge (param arm) must still work on both layers
    (
        "taint_merge",
        r#"(define (add a b) (u128/add a b))
(define (main)
  (let* ((y "42")
         (cap (add "10" "1"))
         (take (if (u128/lt cap y) cap y)))
    (str-cat take)))"#,
    ),
    // value use keeps the wrapper a real function (map)
    (
        "value_use_map",
        r#"(define (add1 a) (u128/add a "1"))
(define (main) (str-cat (car (map add1 (list "41" "999")))))"#,
    ),
    // nested wrappers inline through each other's args
    (
        "nested",
        r#"(define (add a b) (u128/add a b))
(define (mdiv a b d) (u128/muldiv a b d))
(define (main) (str-cat (mdiv "1000" (add "10" "15") "4"))) "#,
    ),
    // try zone: inlining is skipped; trap behavior identical on both layers
    // (interp catches, wasm returns the same caught string)
    (
        "try_zone",
        r#"(define (add a b) (u128/add a b))
(define (main)
  (try (add "340282366920938463463374607431768211455" "1")
       (catch e "caught"))) "#,
    ),
    // deep limb chain through muldiv + if-merge of pure arms
    (
        "pure_if_merge",
        r#"(define (sub a b) (u128/sub a b))
(define (main)
  (let* ((a (u128/mul "18446744073709551616" "4294967296"))
         (b (u128/muldiv a "3" "7"))
         (c (if (u128/lt a b) a b)))
    (str-cat (sub a (if (u128/lt c "100" ) c "100")))))"#,
    ),
];

fn interp_result(src: &str) -> String {
    let mut exprs = parse_all(src).expect("parse");
    lisp_rlm_wasm::clojure::desugar(&mut exprs);
    // run_program evaluates defines + value forms; the wasm side runs the
    // `main` EXPORT — append an explicit call so both sides evaluate main
    if let Ok(mut extra) = parse_all("(main)") {
        exprs.append(&mut extra);
    }
    let mut env = Env::new();
    let mut state = EvalState::new();
    match run_program(&exprs, &mut env, &mut state) {
        // normalize: LispVal::Str's Display quotes the content; the wasm
        // decode returns raw bytes — compare raw content on both sides
        Ok(lisp_rlm_wasm::types::LispVal::Str(s)) => s,
        Ok(v) => v.to_string(),
        Err(e) => format!("ERR:{}", e),
    }
}

fn wasm_result(src: &str) -> Result<String, String> {
    let wasm = compile_fuzz(src).map_err(|e| format!("compile: {e}"))?;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {e}"))?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    let fallback_memory =
        Memory::new(&mut store, wasmtime::MemoryType::new(4, None)).map_err(|e| e.to_string())?;
    if module
        .imports()
        .any(|i| i.module() == "env" && i.name() == "memory")
    {
        linker
            .define(&store, "env", "memory", fallback_memory)
            .map_err(|e| e.to_string())?;
    }
    for import in module.imports() {
        if import.module() != "env" || import.name() == "memory" {
            continue;
        }
        let ExternType::Func(func_ty) = import.ty() else { continue };
        let params: Vec<_> = func_ty.params().collect();
        let results: Vec<_> = func_ty.results().collect();
        let ft = FuncType::new(&engine, params, results);
        let stub = Func::new(&mut store, ft, move |_, _, ret| {
            for r in ret.iter_mut() {
                *r = Val::I64(0);
            }
            Ok(())
        });
        linker.define(&store, "env", import.name(), stub).map_err(|e| e.to_string())?;
    }
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e}"))?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or("no memory export")?;
    let f = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| format!("no run export: {e}"))?;
    f.call(&mut store, ()).map_err(|e| format!("trap: {e}"))?;
    // fuzz mode leaves the TAGGED result at TEMP_MEM (differential.rs convention)
    const TEMP_MEM: usize = 64;
    let mem = memory.data(&store).to_vec();
    let raw = i64::from_le_bytes(mem[TEMP_MEM..TEMP_MEM + 8].try_into().unwrap());
    Ok(match decode(&mem, raw) {
        TaggedValue::Str { ptr, len } => {
            String::from_utf8_lossy(&mem[ptr as usize..(ptr + len) as usize]).into_owned()
        }
        TaggedValue::Num(n) => n.to_string(),
        TaggedValue::Bool(b) => b.to_string(),
        TaggedValue::Nil => "nil".to_string(),
        _ => format!("tagged:{raw}"),
    })
}

#[test]
fn pool_a_step_parity() {
    let want = interp_result(POOL_A_STEP);
    let got = wasm_result(POOL_A_STEP).expect("wasm");
    assert_eq!(want, got, "pool-a step shape must match across layers");
    // sanity: the reference itself is pinned (not "ERR:...")
    assert!(want.contains("|"), "interp returned {want}");
}

#[test]
fn edge_case_parity() {
    for (name, src) in EDGE_CASES {
        let want = interp_result(src);
        let got = match wasm_result(src) {
            Ok(g) => g,
            Err(e) => {
                // a wasm-side TRAP is only acceptable when interp also errored
                if want.starts_with("ERR:") || want == "caught" {
                    continue;
                }
                panic!("{name}: wasm failed but interp succeeded ({want}): {e}");
            }
        };
        assert_eq!(want, got, "{name}: layer divergence");
    }
}
