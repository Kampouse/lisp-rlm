// Regression tests for TASK-json-bug.md: json_get_str corrupted/dropped
// values when a single tx's args JSON was ~600+ bytes with multiple keys
// (e.g. one 384-char value followed by another key).
//
// Root cause: json_get_str's unescape destination was a compile-time
// heap_bump(256) slot; alloc_data lands key patterns / literals directly
// above it (next_data_offset synced to the heap top), so any value >256
// bytes overflowed the slot at runtime and clobbered the LATER call
// sites' patterns before their scans ran — phantom "key not found"
// (value read back as empty) plus corrupted abort literals. The dst is
// now a monotonic runtime-heap block sized to the value's escaped
// length (emit_rtheap_alloc), so values up to INPUT_BUF (16KB) work.

use lisp_rlm_wasm::compile_near_from_exprs;
use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::parse_all;
use std::sync::MutexGuard;

const ECHO_TS: &str = r#"
export function m(a: string, b: string, c: string, d: string): string {
  return `${a.length}:${b.length}:${c.length}:${d.length}`;
}
"#;

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn compile(src: &str) -> Vec<u8> {
    let ir = ts_to_lisp_source(src).unwrap();
    let exprs = parse_all(&ir).unwrap();
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).unwrap();
    compile_near_from_exprs(&exprs).unwrap()
}

fn call(wasm: &[u8], method: &str, args: &str) -> String {
    let _l = lock();
    let p = std::env::temp_dir().join(format!("json_sizes_{}.wasm", std::process::id()));
    std::fs::write(&p, wasm).unwrap();
    let state = std::env::temp_dir().join(format!("json_sizes_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&state);
    let manifest = format!("c.t.near={}", p.display());
    std::process::Command::new("./target/release/near-mock")
        .arg("cross")
        .arg(&state)
        .arg(&manifest)
        .arg("c.t.near")
        .arg(method)
        .arg(args)
        .output()
        .map(|o| {
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_default()
}

fn ret_line(out: &str) -> String {
    out.lines()
        .find(|l| l.contains("📄"))
        .map(|l| l.trim().trim_start_matches('📄').trim().to_string())
        .unwrap_or_else(|| "<no return>".into())
}

#[test]
fn json_sizes_echo_matrix() {
    // The exact TASK-json-bug.md symptom matrix (all six shapes).
    let wasm = compile(ECHO_TS);
    let cases: Vec<(serde_json::Value, &str)> = vec![
        // ({"a":10, "b":192, "c":384, "d":66}) — was: d = 0
        (
            serde_json::json!({"a": "a".repeat(10), "b": "b".repeat(192), "c": "c".repeat(384), "d": format!("01{}", "1".repeat(64))}),
            "10:192:384:66",
        ),
        // ({"a":384, "b":10}) — was: b = 0
        (
            serde_json::json!({"a": "a".repeat(384), "b": "b".repeat(10)}),
            "384:10:0:0",
        ),
        // ({"a":96, "b":384, "c":192, "d":4}) — was: c = 0
        (
            serde_json::json!({"a": "a".repeat(96), "b": "b".repeat(384), "c": "c".repeat(192), "d": "0111"}),
            "96:384:192:4",
        ),
        // ({"a":576, ...}) — was: b,c = 0
        (
            serde_json::json!({"a": "a".repeat(576), "b": "b".repeat(10), "c": "c".repeat(10), "d": "0111"}),
            "576:10:10:4",
        ),
        // ({"a":256×3, "d":4}) — 830B total; was OK (control)
        (
            serde_json::json!({"a": "a".repeat(256), "b": "b".repeat(256), "c": "c".repeat(256), "d": "0111"}),
            "256:256:256:4",
        ),
        // ({"a":700}) single key — was OK (control)
        (
            serde_json::json!({"a": "a".repeat(700)}),
            "700:0:0:0",
        ),
    ];
    for (args, want) in cases {
        let out = call(&wasm, "m", &args.to_string());
        let got = ret_line(&out);
        assert_eq!(got, want, "args_len={} args={:?}", args.to_string().len(), args.to_string().chars().take(80).collect::<String>());
    }
}

#[test]
fn json_sizes_4kb_single_value() {
    // A 4KB single value plus a trailing key — far past the old 256-byte
    // slot; must still parse and the FOLLOWING key must survive.
    let wasm = compile(ECHO_TS);
    let args = serde_json::json!({"a": "a".repeat(4096), "b": "b".repeat(10)});
    let out = call(&wasm, "m", &args.to_string());
    assert_eq!(ret_line(&out), "4096:10:0:0", "4KB single value");
}

#[test]
fn json_sizes_setpoints_shape() {
    // The exact setPoints-shaped args from tests/test_ts_bls_msig.rs:
    // {"id":"m1","msgPoint":<192-hex>,"g2gen":<384-hex>} ≈ 612 bytes,
    // three keys, the 384-hex value NOT last (followed by nothing here —
    // but content of ALL keys must round-trip byte-exact).
    let src = r#"
export function pts(id: string, msgPoint: string, g2gen: string): string {
  return `${id}:${msgPoint.length}:${g2gen.length}`;
}
export function content(id: string, msgPoint: string, g2gen: string): string {
  return `${id}|${msgPoint}|${g2gen}`;
}
"#;
    let wasm = compile(src);
    let msg = "cd".repeat(96);
    let g2gen = "07".repeat(192);
    let args = serde_json::json!({"id": "m1", "msgPoint": msg, "g2gen": g2gen});
    let s = args.to_string();
    assert!(s.len() > 600, "setPoints-shaped args must be 600+B, got {}", s.len());
    let out = call(&wasm, "pts", &s);
    assert_eq!(ret_line(&out), "m1:192:384", "setPoints-shaped lengths");

    // Byte-exact content round-trip (catches corruption, not just length).
    let out = call(&wasm, "content", &s);
    let want = format!("m1|{}|{}", msg, g2gen);
    assert_eq!(ret_line(&out), want, "setPoints-shaped content");

    // Key-order variant: the long value FIRST ({"g2gen":...,"id":"m1",
    // "msgPoint":...}) — the overflow used to eat the later keys.
    let args2 = serde_json::json!({"g2gen": g2gen, "id": "m1", "msgPoint": msg});
    let out = call(&wasm, "pts", &args2.to_string());
    assert_eq!(ret_line(&out), "m1:192:384", "setPoints-shaped reordered");
}

#[test]
fn json_sizes_16kb_input() {
    // Full INPUT_BUF-scale args (16KB): two ~8KB values — the fix
    // requirement is json_get_str handles ≥16KB args.
    let wasm = compile(ECHO_TS);
    let args = serde_json::json!({"a": "a".repeat(8000), "b": "b".repeat(7900)});
    let out = call(&wasm, "m", &args.to_string());
    assert_eq!(ret_line(&out), "8000:7900:0:0", "16KB input");
}
