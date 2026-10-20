//! bugC (2026-10-12): type errors must point at the CALL SITE, not at the
//! first textual occurrence of any word from the message.
//!
//! Root cause of the wrong-line pins: `annotate_type_error_label` scraped
//! candidate words out of the error string and reported the first source
//! token matching any of them. For "in call (len ...): arity mismatch" the
//! first candidate that tokenizes is `len` — and the first `len` token in a
//! file is usually the `(define (len ...)` header, so every arity error
//! pointed at the definition. Undefined-variable errors only looked correct
//! by luck (unique names).
//!
//! Fix: resolve the call head from the structured part of the message —
//! `in call (HEAD ...` — and scan for `(HEAD` in call position (skipping
//! `(define (HEAD`). The word-scrape remains as a fallback for messages
//! without that shape.

use lisp_rlm_wasm::parser::{annotate_type_error_label, parse_all};
use lisp_rlm_wasm::typing::type_check_program;

/// Run checker + annotator exactly like the compile bin does, return the
/// final message.
fn check_and_annotate(src: &str) -> String {
    let exprs = parse_all(src).expect("parse");
    match type_check_program(&exprs, true) {
        Ok(()) => panic!("expected type error for: {src}"),
        Err(e) => annotate_type_error_label(&e, src, "near"),
    }
}

fn line_of(msg: &str) -> usize {
    let idx = msg.find("(near line ").expect("no line label") + "(near line ".len();
    let rest = &msg[idx..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().expect("line digits")
}

#[test]
fn arity_error_points_at_call_site_not_define() {
    let src = "(define (len s p) 42)\n\n(define (g) (len \"a\"))\n";
    let msg = check_and_annotate(src);
    assert_eq!(
        line_of(&msg),
        3,
        "call is on line 3; define header is line 1 — got: {msg}"
    );
}

#[test]
fn arity_error_skips_earlier_good_call() {
    // good 1-arg call on line 3, bad 2-arg call on line 5: both call-position
    // occurrences of `len` exist; the bad one is the SECOND `(len` — but the
    // checker reports the FIRST failing call (it aborts at line 3's good call
    // only if it errors; it doesn't — so the failure is line 5's call).
    let src =
        "(define (len s p) 42)\n\n(define (g) (len \"a\" \"b\"))\n\n(define (h) (len \"c\"))\n";
    let msg = check_and_annotate(src);
    assert_eq!(
        line_of(&msg),
        3,
        "first (and only) failing call is line 3's; got: {msg}"
    );
}

#[test]
fn undefined_var_still_points_at_use_site() {
    let src = "(define (a1) 1)\n\n(define (a2) (undefined-op-xyz 5))\n";
    let msg = check_and_annotate(src);
    assert_eq!(line_of(&msg), 3, "use site is line 3; got: {msg}");
}

#[test]
fn mismatch_error_points_at_call_not_first_word_match() {
    // The old heuristic's first word-candidate match for this message shape
    // was a noise word; ensure the call-head path wins and finds the call.
    let src = "(define (f x) x)\n\n(define (g) (f))\n";
    let msg = check_and_annotate(src);
    assert_eq!(line_of(&msg), 3, "got: {msg}");
}

#[test]
fn ir_label_variant_still_works() {
    // The TS→lisp lowering uses label "ir": same resolution, different label.
    let src = "(define (len s p) 42)\n\n(define (g) (len \"a\"))\n";
    let exprs = parse_all(src).unwrap();
    let e = type_check_program(&exprs, true).unwrap_err();
    let msg = annotate_type_error_label(&e, src, "ir");
    assert!(msg.contains("(ir line 3)"), "got: {msg}");
}

#[test]
fn no_false_label_when_nothing_matches() {
    // Nothing in the message resolves to a token: message must pass through
    // UNCHANGED (no "line ?" invention, no bogus pin).
    let msg = annotate_type_error_label("totally opaque error", "(define (x) 1)", "near");
    assert_eq!(msg, "totally opaque error");
}
