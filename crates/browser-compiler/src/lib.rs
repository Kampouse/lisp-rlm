//! Browser Lisp compiler — compiles Lisp source to WASM binaries.
//!
//! Exposes two functions via wasm-bindgen:
//! - `compile_p1(source)` → NEAR smart contract WASM
//! - `compile_p2(source)` → WASI/OutLayer WASM (with wasi:http support)

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn init() {
    console_error_panic_hook::set_once();
}

/// Compile Lisp source to a NEAR smart contract WASM binary (P1 target).
/// Returns the raw WASM bytes as a Uint8Array.
#[wasm_bindgen]
pub fn compile_p1(source: &str) -> Result<Vec<u8>, JsValue> {
    let exprs = lisp_rlm_wasm::parse_all(source)
        .map_err(|e| JsValue::from_str(&format!("Parse error: {}", e)))?;
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

/// Get the WASM binary size in bytes (useful for display).
#[wasm_bindgen]
pub fn wasm_size(wasm_bytes: &[u8]) -> usize {
    wasm_bytes.len()
}
