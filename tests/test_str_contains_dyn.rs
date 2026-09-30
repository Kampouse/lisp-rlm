//! Dynamic-needle str-contains regression (2026-09-30 emitter fix).
//!
//! str_contains previously hard-required a string-literal needle; any
//! runtime-built needle (TS `.includes(x)` with x a variable/template)
//! died at emit. The fix mirrors str_index_of's policy: literal →
//! compile-time fast path, dynamic → str-index-of-dyn scan, found =
//! (idx >= 0), tagged BOOL for interpreter parity.
//!
//! This asserts the exact result vector on BOTH surfaces (interpreter +
//! wasm) — a positive assertion, not just interp==wasm agreement.

#![allow(dead_code)]

use lisp_rlm_wasm::tagged_value::{decode, TaggedValue};
use lisp_rlm_wasm::*;

#[path = "borsh_harness.rs"]
mod harness;
use harness::WasmRunner;

fn tv_to_lisp(memory: &[u8], tv: TaggedValue) -> LispVal {
    match tv {
        TaggedValue::Num(n) => LispVal::Num(n),
        TaggedValue::Bool(b) => LispVal::Bool(b),
        TaggedValue::Nil => LispVal::Nil,
        TaggedValue::Str { ptr, len } => {
            let s =
                String::from_utf8_lossy(&memory[ptr as usize..(ptr + len) as usize]).to_string();
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

const PROG: &str = r#"
(define (main)
  (list (str-contains "hello world" (str-cat "wor" "ld"))
        (str-contains "hello" (str-cat "x" "yz"))
        (str-contains (str-cat "hello " "world") (str-cat "wo" "rld"))
        (str-contains "hello" "")
        (str-contains "banana" (str-substring "banana" 1 2))
        (str-index-of "banana" (str-cat "a" "na"))))
(main)
"#;

fn expected() -> LispVal {
    // found / not-found / both-runtime / empty-needle (always-contains,
    // interp parity) / substring needle / dynamic index-of
    LispVal::List(vec![
        LispVal::Bool(true),
        LispVal::Bool(false),
        LispVal::Bool(true),
        LispVal::Bool(true),
        LispVal::Bool(true),
        LispVal::Num(1),
    ])
}

#[test]
fn str_contains_dynamic_needle_interpreter() {
    let src = PROG.to_string();
    let out = std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || {
            let mut env = Env::new();
            let mut state = EvalState::new();
            let exprs = parser::parse_all(&src).expect("parse");
            let _ = run_program(&exprs, &mut env, &mut state).expect("run");
            // the program's last top-level form is the call itself
            run_program(
                &[types::LispVal::List(vec![types::LispVal::Sym("main".into())])],
                &mut env,
                &mut state,
            )
            .expect("call dyn-tests")
        })
        .expect("thread")
        .join()
        .expect("thread result");
    assert_eq!(format!("{:?}", out), format!("{:?}", expected()));
}

#[test]
fn str_contains_dynamic_needle_wasm() {
    let mut runner = WasmRunner::new(PROG).expect("wasm compile");
    runner.run().expect("wasm run");
    let tagged = runner.read_raw_result();
    let mem = runner.mem_snapshot();
    let out = tv_to_lisp(&mem, decode(&mem, tagged));
    assert_eq!(
        format!("{:?}", out),
        format!("{:?}", expected()),
        "wasm dynamic-needle str-contains must match the interpreter vector"
    );
}
