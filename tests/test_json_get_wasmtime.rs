//! Test json-get inline WASM function with wasmtime

use std::fs;
use wasmtime::*;

#[test]
fn test_json_get_inline_wasm() -> Result<()> {
    let lisp = r#"(define (main)
  (let ((resp (outlayer/http-get "https://httpbin.org/uuid")))
    (let ((val (outlayer/json-get resp "uuid")))
      (wasi/write_stdout val))))
"#;

    let lisp_path = "/tmp/test_jg_wt.lisp";
    fs::write(lisp_path, lisp)?;

    let compile = std::process::Command::new("cargo")
        .args([
            "run",
            "--release",
            "--bin",
            "emit_p2",
            "--",
            lisp_path,
            "/tmp/test_jg_wt.wasm",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("RUST_LOG", "")
        .output()?;

    if !compile.status.success() {
        eprintln!(
            "stderr: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
    }

    let wasm_bytes = fs::read("/tmp/_p2_core_debug.wasm")?;

    let engine = Engine::default();
    let module = Module::from_binary(&engine, &wasm_bytes)?;
    let mut store = Store::new(&engine, ());

    let mut linker = Linker::new(&engine);

    linker.func_wrap(
        "outlayer:api/host",
        "http-get",
        |_url_ptr: i32, _url_len: i32, _ret_ptr: i32| {},
    )?;

    linker.func_wrap(
        "wasi_snapshot_preview1",
        "fd_write",
        |mut caller: Caller<'_, ()>,
         fd: i32,
         iov_ptr: i32,
         _iov_count: i32,
         _nwritten_ptr: i32|
         -> i32 {
            let mem = caller
                .get_export("memory")
                .unwrap()
                .into_memory()
                .unwrap();
            let ptr = u32::from_le_bytes(
                mem.data(&caller)[iov_ptr as usize..iov_ptr as usize + 4]
                    .try_into()
                    .unwrap(),
            );
            let len = u32::from_le_bytes(
                mem.data(&caller)[iov_ptr as usize + 4..iov_ptr as usize + 8]
                    .try_into()
                    .unwrap(),
            );
            eprintln!("fd_write(fd={}, ptr={}, len={})", fd, ptr, len);
            if len > 0 && len < 10000 {
                let data =
                    &mem.data(&caller)[ptr as usize..(ptr + len) as usize];
                eprintln!("  data: {}", String::from_utf8_lossy(data));
            }
            len as i32
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance.get_memory(&mut store, "memory").unwrap();

    // Write test JSON at offset 4096
    let json = r#"{"uuid":"test-123"}"#;
    let json_bytes = json.as_bytes();
    memory.data_mut(&mut store)[4096..4096 + json_bytes.len()]
        .copy_from_slice(json_bytes);

    // Pre-populate ret_ptr area (offset 280)
    memory.data_mut(&mut store)[280..284]
        .copy_from_slice(&0u32.to_le_bytes());
    memory.data_mut(&mut store)[284..288]
        .copy_from_slice(&4096u32.to_le_bytes());
    memory.data_mut(&mut store)[288..292]
        .copy_from_slice(&(json_bytes.len() as u32).to_le_bytes());

    // Check memory before running
    let json_mem: Vec<u8> =
        memory.data(&store)[4096..4096 + json_bytes.len()].to_vec();
    eprintln!("json at 4096: {}", String::from_utf8_lossy(&json_mem));
    let key_mem: Vec<u8> = memory.data(&store)[312..316].to_vec();
    eprintln!("key at 312: {}", String::from_utf8_lossy(&key_mem));

    // Run _start
    let start =
        instance.get_typed_func::<(), ()>(&mut store, "_start")?;
    start.call(&mut store, ())?;

    Ok(())
}
