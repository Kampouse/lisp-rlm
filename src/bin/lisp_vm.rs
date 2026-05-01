//! On-chain Lisp VM — deploy as a NEAR contract
//! View call: eval Lisp expressions on-chain

use lisp_rlm_wasm::parser::parse_all;
use lisp_rlm_wasm::types::{Env, EvalState, LispVal, DEFAULT_EVAL_BUDGET};
use lisp_rlm_wasm::eval::lisp_eval;
use lisp_rlm_wasm::helpers::is_truthy;

// Avoid wasm_emit dependency — only use bytecode VM

#[cfg(target_arch = "wasm32")]
mod near {
    // Minimal NEAR SDK-like host functions
    // In reality you'd use near-sdk, but let's see the size without it
    
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn log_utf8(len: u64, ptr: u64);
        fn value_return(len: u64, ptr: u64);
        fn input(register_id: u64);
        fn register_len(register_id: u64) -> u64;
        fn read_register(register_id: u64, ptr: u64);
        fn storage_read(key_len: u64, key_ptr: u64, register_id: u64) -> u64;
        fn storage_write(key_len: u64, key_ptr: u64, value_len: u64, value_ptr: u64, register_id: u64) -> u64;
    }
}

const RESULT_BUF: usize = 16384;
static mut RESULT_BUF_DATA: [u8; RESULT_BUF] = [0u8; RESULT_BUF];
const INPUT_BUF: usize = 32768;
static mut INPUT_BUF_DATA: [u8; INPUT_BUF] = [0u8; INPUT_BUF];

unsafe fn read_input() -> String {
    near::input(0);
    let len = near::register_len(0) as usize;
    if len == 0 { return "{}".to_string(); }
    near::read_register(0, INPUT_BUF as u64);
    String::from_utf8_lossy(&INPUT_BUF_DATA[..len]).to_string()
}

fn eval_lisp(source: &str) -> String {
    let exprs = match parse_all(source) {
        Ok(e) => e,
        Err(e) => return format!("{{\"error\": \"parse: {}\"}}", e),
    };
    
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut result = LispVal::Nil;
    
    for expr in exprs {
        match lisp_eval(&expr, &mut env, &mut state, DEFAULT_EVAL_BUDGET) {
            Ok(v) => result = v,
            Err(e) => return format!("{{\"error\": \"eval: {}\"}}", e),
        }
    }
    
    format!("{{\"result\": {}}}", lisp_to_json(&result))
}

fn lisp_to_json(val: &LispVal) -> String {
    match val {
        LispVal::Num(n) => n.to_string(),
        LispVal::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        LispVal::Str(s) => format!("\"{}\"", s),
        LispVal::Nil => "null".to_string(),
        LispVal::List(items) => {
            let parts: Vec<String> = items.iter().map(lisp_to_json).collect();
            format!("[{}]", parts.join(","))
        }
        _ => format!("\"{}\"", val),
    }
}

#[no_mangle]
pub unsafe extern "C" fn eval() {
    let input = read_input();
    // Input is JSON: {"expr": "(+ 1 2)"}
    let expr = extract_json_string(&input, "expr");
    let output = eval_lisp(&expr);
    
    let bytes = output.as_bytes();
    let len = bytes.len().min(RESULT_BUF);
    RESULT_BUF_DATA[..len].copy_from_slice(&bytes[..len]);
    near::value_return(len as u64, RESULT_BUF as u64);
}

fn extract_json_string(input: &str, key: &str) -> String {
    let pattern = format!("\"{}\":", key);
    if let Some(idx) = input.find(&pattern) {
        let rest = &input[idx + pattern.len()..];
        let rest = rest.trim();
        if rest.starts_with('"') {
            if let Some(end) = rest[1..].find('"') {
                return rest[1..end+1].to_string();
            }
        }
    }
    input.to_string()
}
