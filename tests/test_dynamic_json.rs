//! Dynamic JSON keys — `near.jsonGetStr(runtimeExpr)` (2026-09-13).
//! Before: only string-literal keys compiled; dynamic keys were a hard
//! compile error ("key must be a string literal").
//! After: the key is evaluated, copied to a runtime-heap scratch as a
//! `"key":` pattern, and looked up via the shared __json_get scanner.
//! Results are heap-copied out of the shared stdout_buf so consecutive
//! dynamic reads can't clobber each other. Miss → nil → `??` fallback
//! (same miss-gate semantics as the literal path, 2026-08-31).

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function dynHit(): string {
  const key = "na" + "me";
  return near.jsonGetStr(key) ?? "MISS";
}
export function dynMiss(): string {
  const key = "no" + "thing";
  return near.jsonGetStr(key) ?? "fb";
}
export function dynNested(): string {
  const k = near.jsonGetStr("k") ?? "";
  return near.jsonGetStr(k) ?? "MISS";
}
export function dynTwo(): string {
  const a = near.jsonGetStr("k" + "1") ?? "-A";
  const b = near.jsonGetStr("k" + "2") ?? "-B";
  return `${a}-${b}`;
}
export function dynThreeWithMiss(): string {
  const a = near.jsonGetStr("k" + "1") ?? "";
  const m = near.jsonGetStr("z" + "z") ?? "-M";
  const b = near.jsonGetStr("k" + "2") ?? "";
  return `${a}${m}${b}`;
}
export function dynWs(): string {
  return near.jsonGetStr("pre" + "tty") ?? "MISS";
}
export function dynLong(): string {
  return near.jsonGetStr("lo" + "ng") ?? "MISS";
}
// Dynamic-key jsonGetInt (2026-09-13, second pass): __json_get lookup +
// shared __str_to_num parse; miss → nil → ?? fallback.
export function dynIntHit(): string {
  const key = "co" + "unt";
  return toStr((near.jsonGetInt(key) ?? 0) + 1); // 43
}
export function dynIntMiss(): string {
  return toStr(near.jsonGetInt("zz" + "z") ?? 7); // 7
}
export function dynIntNeg(): string {
  // ALSO regression for the export-wrapper ShrU bug: negative Num returns
  // were logical-shifted into ~2^61 garbage at the host boundary.
  return toStr(near.jsonGetInt("d" + "elta") ?? 0); // -5
}
export function dynIntTwo(): string {
  const a = near.jsonGetInt("a" + "1") ?? 0;
  const b = near.jsonGetInt("b" + "2") ?? 0;
  return toStr(a * 10 + b); // 12
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

struct Call<'a> {
    method: &'a str,
    args: &'a str,
}

fn run(c: Call) -> String {
    let _l = lock();
    let ir = ts_to_lisp_source(SRC).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("dynk_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("dynk_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("dynk.t.near={}", p.display());
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("dynk.t.near")
        .arg(c.method)
        .arg(c.args)
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all.lines()
        .rev()
        .find(|l| l.contains('📄'))
        .unwrap_or(&all)
        .to_string()
}

const ARGS: &str = r#"{"name":"JP","k":"name","k1":"AAA","k2":"BBB","pretty":"spaced","count":42,"a1":1,"b2":2,"delta":-5}"#;

#[test]
fn dynamic_key_hit() {
    let r = run(Call {
        method: "dynHit",
        args: r#"{"name":"JP"}"#,
    });
    assert!(r.contains("JP"), "dynHit: {r}");
}

#[test]
fn dynamic_key_miss_fires_fallback() {
    let r = run(Call {
        method: "dynMiss",
        args: r#"{"name":"x"}"#,
    });
    assert!(r.contains("fb"), "dynMiss: {r}");
}

#[test]
fn dynamic_key_from_json_itself() {
    let r = run(Call {
        method: "dynNested",
        args: r#"{"k":"name","name":"deep"}"#,
    });
    assert!(r.contains("deep"), "dynNested: {r}");
}

#[test]
fn two_dynamic_reads_do_not_clobber() {
    let r = run(Call {
        method: "dynTwo",
        args: r#"{"k1":"AAA","k2":"BBB"}"#,
    });
    assert!(r.contains("AAA-BBB"), "dynTwo: {r}");
}

#[test]
fn miss_between_hits_keeps_results() {
    let r = run(Call {
        method: "dynThreeWithMiss",
        args: r#"{"k1":"AAA","k2":"BBB"}"#,
    });
    assert!(r.contains("AAA-MBBB"), "dynThreeWithMiss: {r}");
}

#[test]
fn whitespace_after_colon_matches() {
    let r = run(Call {
        method: "dynWs",
        args: r#"{"pretty":   "spaced"}"#,
    });
    assert!(r.contains("spaced"), "dynWs: {r}");
}

#[test]
fn long_value_round_trips() {
    let long_v = "a".repeat(103);
    let args = format!(r#"{{"long":"{long_v}"}}"#);
    let r = run(Call {
        method: "dynLong",
        args: &args,
    });
    assert!(r.contains(&long_v), "dynLong: {r}");
}

#[test]
fn dynamic_int_key_hit() {
    let r = run(Call {
        method: "dynIntHit",
        args: ARGS,
    });
    assert!(r.contains("43"), "dynIntHit: {r}");
}

#[test]
fn dynamic_int_key_miss_fires_fallback() {
    let r = run(Call {
        method: "dynIntMiss",
        args: ARGS,
    });
    assert!(r.contains("7"), "dynIntMiss: {r}");
}

#[test]
fn dynamic_int_negative_value() {
    // also regression for the export-wrapper ShrU bug: negative Num
    // returns were logical-shifted into ~2^61 garbage at the host boundary
    let r = run(Call {
        method: "dynIntNeg",
        args: ARGS,
    });
    assert!(r.contains("-5"), "dynIntNeg: {r}");
}

#[test]
fn dynamic_int_two_reads_no_clobber() {
    let r = run(Call {
        method: "dynIntTwo",
        args: ARGS,
    });
    assert!(r.contains("12"), "dynIntTwo: {r}");
}
