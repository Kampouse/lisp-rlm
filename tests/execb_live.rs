//! Live-run the compiled exec-b P2 component under wasmtime with real
//! wasi:http — mirrors test_lisp_p2_wasi_http_live.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::test]
async fn execb_p2_live() {
    use wasmtime::component::{Linker, ResourceTable};
    use wasmtime::{Config, Engine};
    use wasmtime_wasi::p2::pipe::MemoryInputPipe;
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
            WasiHttpCtxView {
                ctx: &mut self.http_ctx,
                table: &mut self.table,
                hooks: Default::default(),
            }
        }
    }
    impl WasiView for TestState {
        fn ctx(&mut self) -> WasiCtxView<'_> {
            WasiCtxView {
                ctx: &mut self.ctx,
                table: &mut self.table,
            }
        }
    }

    let comp_path = std::env::var("EXECB_WASM")
        .unwrap_or_else(|_| "/tmp/exec-b-v4-clean.wasm".to_string());
    let comp_bytes = std::fs::read(&comp_path).expect("read component");
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
    let engine = Engine::new(&config).expect("engine");
    let component = wasmtime::component::Component::from_binary(&engine, &comp_bytes)
        .expect("component deserialize");

    let mut linker = Linker::<TestState>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).expect("add wasi");
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
        .expect("add wasi:http");
    // env.* NEAR host stubs: some probe components import them; the real
    // exec-b component resolves these internally (stdin shims) instead.

    let stdin_data = std::env::var("EXECB_STDIN").unwrap_or_else(|_| {
        // 2 lines, like the outlayer worker feeds
        "Hr3vwxi6mKAcP1EGyuePG5wNNpowRiQJSC3wjt24Cksu\nhttps://svalin-api.noobshole.workers.dev/v1/risk/chercher.near\n".to_string()
    });
    let stdout_pipe = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(1 << 20);
    let table = ResourceTable::new();
    let ctx = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(stdin_data.as_bytes().to_vec()))
        .stdout(stdout_pipe.clone())
        .build();
    let state = TestState {
        ctx,
        http_ctx: WasiHttpCtx::new(),
        table,
    };
    let mut store = wasmtime::Store::new(&engine, state);
    let instance = linker
        .instantiate_async(&mut store, &component)
        .await
        .expect("instantiate");

    // version-tolerant: probe known wasi:cli/run versions
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
    match run_fn.call_async(&mut store, &[], &mut result_val).await {
        Ok(()) => {}
        Err(e) => panic!("exec-b component execution failed: {:?}", e),
    }
    let output = stdout_pipe.contents();
    println!(
        "STDOUT ({} bytes): {}",
        output.len(),
        String::from_utf8_lossy(&output)
    );
    // [STDIN-DBG] dump scratch words at STDIN_LEN..+16 (98304..98320)

}
