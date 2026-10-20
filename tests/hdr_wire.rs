//! Raw-wire probe: run the P2 component under wasmtime against a local
//! TCP listener and print the EXACT request bytes the guest sends.
//!
//! Env: EXECB_WASM (component), EXECB_STDIN (optional), HDR_PORT (default 47813).
//! The component under test must hit http://127.0.0.1:<port>/ as a literal URL.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn hdr_wire() {
    use std::io::{Read, Write};
    use wasmtime::component::{Linker, ResourceTable};
    use wasmtime::{Config, Engine};
    use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
    use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
    use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};
    use wasmtime_wasi_http::WasiHttpCtx;

    struct TestState {
        ctx: WasiCtx,
        http_ctx: WasiHttpCtx,
        table: ResourceTable,
    }
    impl WasiHttpView for TestState {
        fn http(&mut self) -> WasiHttpCtxView<'_> {
            WasiHttpCtxView { ctx: &mut self.http_ctx, table: &mut self.table, hooks: Default::default() }
        }
    }
    impl WasiView for TestState {
        fn ctx(&mut self) -> WasiCtxView<'_> { WasiCtxView { ctx: &mut self.ctx, table: &mut self.table } }
    }

    let port: u16 = std::env::var("HDR_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(47813);
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let handle = std::thread::spawn(move || {
        listener.set_nonblocking(false).ok();
        let (mut stream, addr) = listener.accept().expect("accept");
        println!("HDRWIRE: accepted connection from {}", addr);
        stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok();
        let mut got = Vec::new();
        let mut buf = [0u8; 16384];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.windows(4).any(|w| w == b"\r\n\r\n") {
                        // respond so the guest's response leg completes
                        let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                        let _ = stream.write_all(resp);
                        let _ = stream.flush();
                        // keep reading any body bytes briefly
                        let _ = stream.read(&mut buf);
                        break;
                    }
                }
                Err(e) => {
                    // timeout with partial request → respond anyway so the
                    // guest completes; print what we got
                    println!("HDRWIRE: read err after {} bytes: {}", got.len(), e);
                    let resp = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
                    let _ = stream.write_all(resp);
                    let _ = stream.flush();
                    break;
                }
            }
        }
        got
    });

    let comp_path = std::env::var("EXECB_WASM").expect("EXECB_WASM");
    let comp_bytes = std::fs::read(&comp_path).expect("read component");
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    let engine = Engine::new(&config).expect("engine");
    let component = wasmtime::component::Component::from_binary(&engine, &comp_bytes)
        .expect("component deserialize");

    let mut linker = Linker::<TestState>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("add wasi");
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker).expect("add wasi:http");

    let stdin_data = std::env::var("EXECB_STDIN").unwrap_or_else(|_| "x".to_string());
    let stdout_pipe = MemoryOutputPipe::new(1 << 20);
    let table = ResourceTable::new();
    let ctx = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(stdin_data.as_bytes().to_vec()))
        .stdout(stdout_pipe.clone())
        .build();
    let state = TestState { ctx, http_ctx: WasiHttpCtx::new(), table };
    let mut store = wasmtime::Store::new(&engine, state);
    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");

    let mut run_instance_idx = None;
    for ver in ["0.2.12", "0.2.6", "0.2.2"] {
        if let Some((_, idx)) = instance.get_export(&mut store, None, &format!("wasi:cli/run@{ver}")) {
            run_instance_idx = Some(idx);
            break;
        }
    }
    let run_instance_idx = run_instance_idx.expect("wasi:cli/run instance");
    let (_, run_func_idx) = instance
        .get_export(&mut store, Some(&run_instance_idx), "run")
        .expect("get run func");
    let run_fn = instance
        .get_func(&mut store, &run_func_idx)
        .expect("get run Func");

    let mut result_val = [wasmtime::component::Val::Bool(false)];
    let run_result = run_fn.call_async(&mut store, &[], &mut result_val).await;

    let wire = handle.join().expect("server thread");
    println!("HDRWIRE WIRE ({} bytes):", wire.len());
    println!("{}", String::from_utf8_lossy(&wire));
    let output = stdout_pipe.contents();
    println!(
        "HDRWIRE STDOUT ({} bytes): {}",
        output.len(),
        String::from_utf8_lossy(&output)
    );
    if let Err(e) = run_result {
        println!("HDRWIRE RUN ERROR: {:?}", e);
    }
}
