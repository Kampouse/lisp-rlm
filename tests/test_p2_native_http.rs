// P2 wasi:http native-run pins — guards the println fd_write shim and the
// 143→wasi:http POST bridge in build_combined_p2_core.
//
// History: println's fd_write sentinel resolved to cabi_realloc in the
// combined path (silent output + heap corruption), and http-post always
// imported outlayer:api/host so the component could not run under plain
// wasi:http runtimes. These pins keep both regressions out.
use lisp_rlm_wasm::wasi::compile_outlayer_p2;

static P2_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn core_wat(src: &str) -> String {
    // Serialize: the combined path dumps a core we then read — two tests
    // compiling concurrently would race on the dump file (seen flaky 9:17,
    // 9:46 runs). Path is per-pid so parallel test BINARIES can't collide.
    let _g = P2_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    compile_outlayer_p2(src).expect("compile");
    let dump = format!("/tmp/p2_core_debug.{}.wasm", std::process::id());
    let core = std::fs::read(&dump).expect("core dump written by combined path");
    wasmprinter::print_bytes(&core).expect("wasmprinter")
}

#[test]
fn test_p2_combined_core_has_fd_write_shim() {
    // get-stdout (import 23) must be called by a 4×i32→i32 shim (not just
    // by _start): the fd_write translation loop for println.
    let wat = core_wat(r#"
(define (run) (begin (println "hello") 1))
(define (probe-url) (http-get "https://example.com/x"))
"#);
    assert!(wat.contains("call 23"), "get-stdout call missing from core");
    // The shim writes *nwritten (i32.store) and returns errno 0 after a loop
    // — cheap structural proxy: more than one "call 23" site (shim + _start)
    let count = wat.matches("call 23").count();
    assert!(count >= 2, "expected >=2 get-stdout call sites (shim + _start), got {count}");
}

#[test]
fn test_p2_post_bridge_replaces_outlayer_import() {
    // With all-literal POST URLs the component must NOT import the
    // outlayer host interface — it bridges to wasi:http internally.
    let wasm = compile_outlayer_p2(r#"
(define (run)
  (str-cat "r:" (http-post "https://api.hyperliquid.xyz/info" "{\"type\":\"allMids\"}")))
"#).expect("compile");
    let wat = wasmprinter::print_bytes(&wasm).expect("print component");
    assert!(
        !wat.contains("outlayer:api/host"),
        "component must not import outlayer:api/host when POST is native, wat head: {}",
        &wat[..wat.len().min(300)]
    );
    assert!(wasmparser::Validator::new().validate_all(&wasm).is_ok(), "must validate");
    let _ = wasm.len();
}

#[test]
fn test_p2_dynamic_post_keeps_outlayer_import() {
    // Non-literal URL → http-post-dynamic → outlayer host import stays.
    let wat = core_wat(r#"
(define (run u)
  (str-cat "r:" (http-post u "{\"x\":1}")))
"#);
    // core imports the outlayer sentinel → check component-level import
    // exists (dynamic path must keep the host import)
    let wasm = compile_outlayer_p2(r#"
(define (run u)
  (str-cat "r:" (http-post u "{\"x\":1}")))
"#).expect("compile");
    let cwat = wasmprinter::print_bytes(&wasm).expect("print");
    assert!(cwat.contains("outlayer:api/host"), "dynamic POST must keep outlayer host import");
    let _ = wat;
}
