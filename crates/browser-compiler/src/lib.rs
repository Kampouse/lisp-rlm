//! Browser Lisp compiler — compiles Lisp source to WASM binaries.
//!
//! Exposes:
//! - `compile_p1(source)` → NEAR smart contract WASM
//! - `compile_p2(source)` → WASI/OutLayer WASM (with wasi:http support)
//! - `compile_pure(source)` → Pure WASM (no host deps, runnable in browser)
//! - `disassemble_wasm(bytes)` → WAT text format

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn init() {
    console_error_panic_hook::set_once();
}

/// Compile Lisp source to a NEAR smart contract WASM binary (P1 target).
/// Returns the raw WASM bytes as a Uint8Array.
#[wasm_bindgen]
pub fn compile_p1(source: &str) -> Result<Vec<u8>, JsValue> {
    let mut exprs = lisp_rlm_wasm::parse_all(source)
        .map_err(|e| JsValue::from_str(&format!("Parse error: {}", e)))?;
    lisp_rlm_wasm::clojure::desugar(&mut exprs);
    // typecheck runs inside compile_near_from_exprs without source context —
    // pre-run it here WITH source so errors get exact "(near line N)" pins
    let annotated = lisp_rlm_wasm::typing::type_check_program(&exprs, true)
        .err()
        .map(|e| lisp_rlm_wasm::annotate_type_error(&e, source));
    if let Some(msg) = annotated {
        return Err(JsValue::from_str(&format!("Compile error: {}", msg)));
    }
    lisp_rlm_wasm::compile_near_from_exprs(&exprs)
        .map_err(|e| JsValue::from_str(&format!("Compile error: {}", e)))
}

/// Compile Lisp source to a WASI/OutLayer WASM binary (P2 target).
/// Returns the raw WASM bytes as a Uint8Array.
#[wasm_bindgen]
pub fn compile_p2(source: &str) -> Result<Vec<u8>, JsValue> {
    lisp_rlm_wasm::compile_outlayer_p2_browser(source)
        .map_err(|e| JsValue::from_str(&format!("Compile error: {}", e)))
}

/// Compile Lisp source to P2 CORE WASM (before component wrapping).
/// This can be instantiated directly in browser with WASI polyfills.
#[wasm_bindgen]
pub fn compile_p2_core(source: &str) -> Result<Vec<u8>, JsValue> {
    lisp_rlm_wasm::compile_outlayer_p2_core_browser(source)
        .map_err(|e| JsValue::from_str(&format!("Compile error: {}", e)))
}

/// Compile Lisp source to a pure WASM binary (no host functions).
/// This can be instantiated and run directly in the browser.
/// Returns the raw WASM bytes as a Uint8Array.
#[wasm_bindgen]
pub fn compile_pure(source: &str) -> Result<Vec<u8>, JsValue> {
    lisp_rlm_wasm::compile_pure(source)
        .map_err(|e| JsValue::from_str(&format!("Compile error: {}", e)))
}

/// Compile TypeScript source (lisp-rlm TS dialect) to a NEAR smart contract
/// WASM binary (P1 target). Lowers TS → Lisp first.
/// Returns the raw WASM bytes as a Uint8Array.
#[wasm_bindgen]
pub fn compile_ts(source: &str) -> Result<Vec<u8>, JsValue> {
    let lisp_src = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(source)
        .map_err(|e| JsValue::from_str(&format!("TS lowering error: {}", e)))?;
    let mut exprs = lisp_rlm_wasm::parse_all(&lisp_src)
        .map_err(|e| JsValue::from_str(&format!("Parse error: {}", e)))?;
    if let Err(e) = lisp_rlm_wasm::typing::type_check_program(&exprs, true) {
        // checker errors carries no spans — annotate against the lowered lisp
        // IR (annotate_type_error no-ops when nothing matches); the TS-side
        // hint (augment_with_ts_line in the CLI) is unavailable browser-side.
        let annotated = lisp_rlm_wasm::annotate_type_error_label(&e, &lisp_src, "ir");
        return Err(JsValue::from_str(&format!("Compile error: {}", annotated)));
    }
    lisp_rlm_wasm::clojure::desugar(&mut exprs);
    lisp_rlm_wasm::compile_near_from_exprs(&exprs)
        .map_err(|e| JsValue::from_str(&format!("Compile error: {}", e)))
}

/// Lower TypeScript source (lisp-rlm TS dialect) to Lisp source text.
/// Useful for showing the intermediate representation in the UI.
#[wasm_bindgen]
pub fn ts_to_lisp(source: &str) -> Result<String, JsValue> {
    lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(source)
        .map_err(|e| JsValue::from_str(&format!("TS lowering error: {}", e)))
}

/// Disassemble WASM bytes to WAT (WebAssembly Text format).
/// Returns a human-readable string representation of the WASM module.
#[wasm_bindgen]
pub fn disassemble_wasm(wasm_bytes: &[u8]) -> Result<String, JsValue> {
    wasmprinter::print_bytes(wasm_bytes)
        .map_err(|e| JsValue::from_str(&format!("Disassembly error: {}", e)))
}

/// Get the WASM binary size in bytes (useful for display).
#[wasm_bindgen]
pub fn wasm_size(wasm_bytes: &[u8]) -> usize {
    wasm_bytes.len()
}
