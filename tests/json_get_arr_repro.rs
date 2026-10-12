//! json_get_arr repro — does the emitted scanner actually work E2E?
//! Cases: flat hit, empty array, miss -> nil (?? fires), dot-path key.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const SRC: &str = r#"
export function arrFlat(): string {
  const a = near.jsonGetArr("ids") ?? null;
  return a === null ? "NIL" : toStr(a.length) + ":" + a[0] + "/" + a[1];
}
export function arrEmpty(): string {
  const a = near.jsonGetArr("empty") ?? null;
  return a === null ? "NIL" : toStr(a.length);
}
export function arrMiss(): string {
  const a = near.jsonGetArr("nope") ?? null;
  return a === null ? "NIL" : toStr(a.length);
}
export function arrDot(): string {
  const a = near.jsonGetArr("tx.items") ?? null;
  return a === null ? "NIL" : toStr(a.length);
}
"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn run(method: &str, args: &str) -> String {
    let args = if args.is_empty() {
        r#"{"ids":[10,20],"empty":[],"tx":{"items":[1,2,3]}}"#
    } else {
        args
    };
    let _l = lock();
    let ir = match ts_to_lisp_source(SRC) {
        Ok(s) => s,
        Err(e) => panic!("transpile failed: {}", e),
    };
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    let wasm = compile_near_from_exprs(&exprs).unwrap();
    let p = std::env::temp_dir().join(format!("jga_{}.wasm", std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    let st = std::env::temp_dir().join(format!("jga_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&st);
    let manifest = format!("jga.t.near={}", p.display());
    let input = r#"{"ids":[10,20],"empty":[],"tx":{"items":[1,2,3]}}"#;
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(st.to_str().unwrap())
        .arg(&manifest)
        .arg("jga.t.near")
        .arg(method)
        .arg(args)
        .output()
        .expect("near-mock spawn");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    all
}

#[test]
fn json_get_arr_e2e() {
    // flat hit
    let o = run("arrFlat", "");
    println!("arrFlat => {}", o);
    assert!(o.contains("2:10/20"), "arrFlat failed: {}", o);
    // empty array
    let o = run("arrEmpty", "");
    println!("arrEmpty => {}", o);
    assert!(o.contains("0"), "arrEmpty failed: {}", o);
    // miss -> nil, ?? fallback / explicit null check
    let o = run("arrMiss", "");
    println!("arrMiss => {}", o);
    assert!(o.contains("NIL"), "arrMiss failed: {}", o);
    // dot-path
    let o = run("arrDot", "");
    println!("arrDot => {}", o);
    assert!(o.contains("3"), "arrDot failed: {}", o);
}
