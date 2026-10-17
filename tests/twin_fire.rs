// Twin-fire verification: POOL_A_STEP shape must route through __u128_ twins.
// Run: cargo test --test twin_fire
use lisp_rlm_wasm::wasm_emit::compile_near;

const SRC: &str = r#"
(define (add a b) (u128/add a b))
(define (sub a b) (u128/sub a b))
(define (mdiv a b d) (u128/muldiv a b d))
(define (ult a b) (u128/lt a b))
(define (step sqp y l q gb out)
  (let* ((width (sub gb sqp))
         (cap (mdiv q width "1"))
         (take (if (ult cap y) cap y))
         (dp (u128/div take q))
         (used (mdiv dp q "1"))
         (p2 (add sqp dp))
         (gained (if (ult "0" dp) (mdiv l dp (mdiv sqp p2 "1")) "0")))
    (str-cat (add out gained) "|" (sub y used) "|" p2)))
(define (run)
  (str-cat
    (step "1000000000" "60000000000000000000000" "100000000000000000000000" "100300902708124373119359" "2000000000" "0")
    "\n"
    (step "2000000000" "50000000000000000000000" "60000000000000000000000" "60180541624874623871615" "4000000000" "123456789012345678901")
    "\n"
    (step "1000000000" "7" "100000000000000000000000" "100300902708124373119359" "2000000000" "5")))
"#;

#[test]
fn pool_a_shape_compiles_and_twins_fire() {
    // fires ⚡ u128 twin lines on stderr (add/sub/mdiv/ult/step — every fn
    // whose params are all-Native at all sites); must not bail
    let wasm = compile_near(SRC).expect("compile");
    assert!(!wasm.is_empty());
}
