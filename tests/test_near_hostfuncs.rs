//! Tests for new NEAR host function Lisp forms:
//!   near/ed25519_verify, near/promise_yield_create, near/promise_yield_resume,
//!   near/ecrecover, near/promise_batch_action_function_call_weight,
//!   near/json_return_bool, near/json_return_null, near/json_return_raw,
//!   near/json_return_u128, near/json_get_bool,
//!   near/storage_usage, near/account_locked_balance, near/signer_account_pk
//!
//! Each test compiles a Lisp program using compile_fuzz, validates WASM structure,
//! and checks that the correct imports are present.

use lisp_rlm_wasm::near_validate::validate_near_wasm;
use lisp_rlm_wasm::wasm_emit::compile_fuzz;

/// Helper: compile a Lisp source to WASM, validate, and return the WASM bytes.
fn compile_and_validate(source: &str) -> Result<Vec<u8>, String> {
    let wasm = compile_fuzz(source)?;
    // Validate with wasmtime — will catch malformed WASM
    let _ = wasmtime::Module::from_binary(&wasmtime::Engine::default(), &wasm)
        .map_err(|e| format!("wasmtime validation: {}", e))?;
    Ok(wasm)
}

/// Helper: check that a WASM binary imports a specific function from "env".
fn has_import(wasm: &[u8], name: &str) -> bool {
    let wat = wasmprinter::print_bytes(wasm).unwrap_or_default();
    for line in wat.lines() {
        let trimmed = line.trim();
        if trimmed.contains("(import \"env\"") && trimmed.contains(&format!("\"{}\"", name)) {
            return true;
        }
    }
    false
}

// ── ed25519_verify ──

#[test]
fn test_ed25519_verify_compiles() {
    let source = r#"
        (define (verify-sig sp sl mp ml pp pl)
            (near/ed25519_verify sp sl mp ml pp pl))
    "#;
    let wasm = compile_and_validate(source).expect("ed25519_verify should compile");
    assert!(
        has_import(&wasm, "ed25519_verify"),
        "WASM should import ed25519_verify"
    );
}

// ── promise_yield_create ──

#[test]
fn test_promise_yield_create_compiles() {
    let source = r#"
        (define (do-yield ap al mp ml xp xl g gw rid)
            (near/promise_yield_create ap al mp ml xp xl g gw rid))
    "#;
    let wasm = compile_and_validate(source).expect("promise_yield_create should compile");
    assert!(
        has_import(&wasm, "promise_yield_create"),
        "WASM should import promise_yield_create"
    );
}

// ── promise_yield_resume ──

#[test]
fn test_promise_yield_resume_compiles() {
    let source = r#"
        (define (do-resume dp dl dp2 dl2)
            (near/promise_yield_resume dp dl dp2 dl2))
    "#;
    let wasm = compile_and_validate(source).expect("promise_yield_resume should compile");
    assert!(
        has_import(&wasm, "promise_yield_resume"),
        "WASM should import promise_yield_resume"
    );
}

// ── ecrecover ──

#[test]
fn test_ecrecover_compiles() {
    let source = r#"
        (define (do-ecrecover hp hl sp sl v ic rid)
            (near/ecrecover hp hl sp sl v ic rid))
    "#;
    let wasm = compile_and_validate(source).expect("ecrecover should compile");
    assert!(
        has_import(&wasm, "ecrecover"),
        "WASM should import ecrecover"
    );
}

// ── promise_batch_action_function_call_weight ──

#[test]
fn test_promise_batch_action_function_call_weight_compiles() {
    let source = r#"
        (define (do-weighted pi ml mp al al2 lo hi gw)
            (near/promise_batch_action_function_call_weight pi ml mp al al2 lo hi gw))
    "#;
    let wasm = compile_and_validate(source)
        .expect("promise_batch_action_function_call_weight should compile");
    assert!(
        has_import(&wasm, "promise_batch_action_function_call_weight"),
        "WASM should import promise_batch_action_function_call_weight"
    );
}

// ── Wrong arg count errors ──

#[test]
fn test_ed25519_verify_wrong_arg_count() {
    let source = "(define (bad) (near/ed25519_verify 1 2 3))";
    let result = compile_fuzz(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("ed25519_verify requires 6 args"));
}

#[test]
fn test_promise_yield_create_wrong_arg_count() {
    let source = "(define (bad) (near/promise_yield_create 1 2 3))";
    let result = compile_fuzz(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("promise_yield_create requires 9 args"));
}

#[test]
fn test_promise_yield_resume_wrong_arg_count() {
    let source = "(define (bad) (near/promise_yield_resume 1))";
    let result = compile_fuzz(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("promise_yield_resume requires 4 args"));
}

#[test]
fn test_ecrecover_wrong_arg_count() {
    let source = "(define (bad) (near/ecrecover 1 2 3))";
    let result = compile_fuzz(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("ecrecover requires 7 args"));
}

#[test]
fn test_promise_batch_action_function_call_weight_wrong_arg_count() {
    let source = "(define (bad) (near/promise_batch_action_function_call_weight 1 2 3))";
    let result = compile_fuzz(source);
    assert!(result.is_err(), "should fail with wrong arg count");
    assert!(result
        .unwrap_err()
        .contains("promise_batch_action_function_call_weight requires 8 args"));
}

// ── Integration: run with noop stubs via wasmtime ──

fn run_with_noop_stubs(source: &str, export_name: &str) {
    let wasm = compile_fuzz(source).expect("compile");
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::from_binary(&engine, &wasm).expect("parse");
    let mut store = wasmtime::Store::new(&engine, ());
    let mut linker = wasmtime::Linker::new(&engine);

    // Provide memory if imported
    let needs_imported_memory = module
        .imports()
        .any(|i| i.module() == "env" && i.name() == "memory");
    if needs_imported_memory {
        let memory =
            wasmtime::Memory::new(&mut store, wasmtime::MemoryType::new(4, None)).unwrap();
        linker.define(&store, "env", "memory", memory).unwrap();
    }

    // Stub all env imports with noops
    use wasmtime::{ExternType, Func, FuncType, Val, ValType};
    for import in module.imports() {
        if import.module() == "env" && import.name() != "memory" {
            if let ExternType::Func(func_ty) = import.ty() {
                let params: Vec<ValType> = func_ty.params().collect();
                let results: Vec<ValType> = func_ty.results().collect();
                let rc = results.len();
                let ft = FuncType::new(&engine, params, results.clone());
                let stub = Func::new(&mut store, ft, move |_, _, ret| {
                    for i in 0..rc {
                        ret[i] = match results[i] {
                            ValType::I32 => Val::I32(0),
                            ValType::F32 => Val::F32(0),
                            ValType::F64 => Val::F64(0u64),
                            _ => Val::I64(0),
                        };
                    }
                    Ok(())
                });
                linker.define(&store, "env", import.name(), stub).unwrap();
            }
        }
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .expect("instantiate");

    // Find and call the export
    let func = instance
        .get_func(&mut store, export_name)
        .unwrap_or_else(|| panic!("export '{}' not found", export_name));
    func.call(&mut store, &[], &mut [])
        .expect("call should succeed");
}

#[test]
fn test_ed25519_verify_runs_with_noop_stubs() {
    let source = r#"
        (define (verify-sig sp sl mp ml pp pl)
            (near/ed25519_verify sp sl mp ml pp pl))
    "#;
    // compile_fuzz exports as "run" for NEAR convention
    run_with_noop_stubs(source, "run");
}

#[test]
fn test_promise_yield_create_runs_with_noop_stubs() {
    let source = r#"
        (define (do-yield ap al mp ml xp xl g gw rid)
            (near/promise_yield_create ap al mp ml xp xl g gw rid))
    "#;
    run_with_noop_stubs(source, "run");
}

#[test]
fn test_ecrecover_runs_with_noop_stubs() {
    let source = r#"
        (define (do-ecrecover hp hl sp sl v ic rid)
            (near/ecrecover hp hl sp sl v ic rid))
    "#;
    run_with_noop_stubs(source, "run");
}

#[test]
fn test_promise_batch_action_function_call_weight_runs_with_noop_stubs() {
    let source = r#"
        (define (do-weighted pi ml mp al al2 lo hi gw)
            (near/promise_batch_action_function_call_weight pi ml mp al al2 lo hi gw))
    "#;
    run_with_noop_stubs(source, "run");
}

#[test]
fn test_promise_yield_resume_runs_with_noop_stubs() {
    let source = r#"
        (define (do-resume dp dl dp2 dl2)
            (near/promise_yield_resume dp dl dp2 dl2))
    "#;
    run_with_noop_stubs(source, "run");
}

// ── JSON Extensions ──

#[test]
fn test_json_return_bool_compiles() {
    let source = r#"
        (define (return-bool x)
            (near/json_return_bool x))
    "#;
    let wasm = compile_and_validate(source).expect("json_return_bool should compile");
    assert!(
        has_import(&wasm, "value_return"),
        "WASM should import value_return"
    );
}

#[test]
fn test_json_return_null_compiles() {
    let source = r#"
        (define (return-null)
            (near/json_return_null))
    "#;
    let wasm = compile_and_validate(source).expect("json_return_null should compile");
    assert!(
        has_import(&wasm, "value_return"),
        "WASM should import value_return"
    );
}

#[test]
fn test_json_return_raw_compiles() {
    let source = r#"
        (define (return-raw s)
            (near/json_return_raw s))
    "#;
    let wasm = compile_and_validate(source).expect("json_return_raw should compile");
    assert!(
        has_import(&wasm, "value_return"),
        "WASM should import value_return"
    );
}

#[test]
fn test_json_return_u128_compiles() {
    let source = r#"
        (define (return-u128 off)
            (near/json_return_u128 off))
    "#;
    let wasm = compile_and_validate(source).expect("json_return_u128 should compile");
    assert!(
        has_import(&wasm, "value_return"),
        "WASM should import value_return"
    );
}

#[test]
fn test_json_get_bool_compiles() {
    let source = r#"
        (define (get-bool)
            (near/json_get_bool "is_active"))
    "#;
    let wasm = compile_and_validate(source).expect("json_get_bool should compile");
    assert!(
        has_import(&wasm, "input"),
        "WASM should import input"
    );
}

// ── Minor host funcs ──

#[test]
fn test_storage_usage_compiles() {
    let source = r#"
        (define (get-usage)
            (near/storage_usage))
    "#;
    let wasm = compile_and_validate(source).expect("storage_usage should compile");
    assert!(
        has_import(&wasm, "storage_usage"),
        "WASM should import storage_usage"
    );
}

#[test]
fn test_account_locked_balance_compiles() {
    let source = r#"
        (define (get-locked)
            (near/account_locked_balance))
    "#;
    let wasm = compile_and_validate(source).expect("account_locked_balance should compile");
    assert!(
        has_import(&wasm, "account_locked_balance"),
        "WASM should import account_locked_balance"
    );
}

#[test]
fn test_signer_account_pk_compiles() {
    let source = r#"
        (define (get-pk)
            (near/signer_account_pk))
    "#;
    let wasm = compile_and_validate(source).expect("signer_account_pk should compile");
    assert!(
        has_import(&wasm, "signer_account_pk"),
        "WASM should import signer_account_pk"
    );
}
