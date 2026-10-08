//! p2run — local P2 component runner for OutLayer agents (dev/repro tool).
//!
//! Runs a `--target=outlayer-p2` component: stdin = request JSON, stdout =
//! result. near:storage is an in-mem HashMap, near:vrf deterministic, wasi
//! provided by wasmtime-wasi p2, HTTP goes REAL (agents that post should
//! target their own bridge/relay).
//!
//! Usage:
//!   echo '{"op":"version"}' | cargo run --release --example p2run -- \
//!     <component.wasm> [-e NEAR_SENDER_ID=...]

use std::collections::HashMap;
use wasmtime::component::{bindgen, Component, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder};
use wasmtime_wasi::{WasiCtxView, WasiView};
use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};
use wasmtime_wasi_http::WasiHttpCtx;

bindgen!({
    path: [
        "wit/deps/io",
        "wit/deps/clocks",
        "wit/deps/random",
        "wit/deps/filesystem",
        "wit/deps/sockets",
        "wit/deps/cli",
        "wit/deps/http",
        "examples/harness-wit/near-storage/near-storage.wit",
        "examples/harness-wit/near-vrf/near-vrf.wit",
        "examples/harness-wit/root.wit",
    ],
    world: "harness:runner/harness-root",
});

struct HostState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: wasmtime::component::ResourceTable,
    storage: HashMap<String, Vec<u8>>,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: Default::default(),
        }
    }
}

impl wasmtime::component::HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

impl near::storage::api::Host for &mut HostState {
    fn set(&mut self, key: String, value: Vec<u8>) -> String {
        eprintln!("[host] storage-set {} <- {} bytes", key, value.len());
        self.storage.insert(key, value);
        String::new()
    }
    fn get(&mut self, key: String) -> (Vec<u8>, String) {
        match self.storage.get(&key) {
            Some(v) => (v.clone(), String::new()),
            None => (Vec::new(), String::new()),
        }
    }
}

impl near::vrf::api::Host for &mut HostState {
    fn generate(&mut self, user_seed: String) -> (String, String, String, String) {
        eprintln!("[host] vrf-generate seed={}", user_seed);
        (
            "ab".repeat(32),
            "cd".repeat(64),
            format!("vrf:repro:{}", user_seed),
            String::new(),
        )
    }
    fn pubkey(&mut self) -> (String, String) {
        ("ef".repeat(32), String::new())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: p2run <component.wasm> [-e K=V]...");
        std::process::exit(2);
    }
    let wasm_path = &args[0];
    let mut envs: Vec<(String, String)> = Vec::new();
    for a in &args[1..] {
        if let Some(kv) = a.strip_prefix("-e ") {
            if let Some((k, v)) = kv.split_once('=') {
                envs.push((k.to_string(), v.to_string()));
            }
        }
    }

    let input = {
        use std::io::Read;
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("read stdin");
        s
    };

    let engine = Engine::default();
    let component =
        Component::from_file(&engine, wasm_path).expect("load component (validate first)");

    let stdout_pipe = MemoryOutputPipe::new(64 * 1024);
    let stdout_reader = stdout_pipe.clone();

    let mut builder = WasiCtxBuilder::new();
    builder.stdin(MemoryInputPipe::new(input.as_bytes().to_vec()));
    builder.stdout(stdout_pipe);
    builder.stderr(std::io::stderr());
    for (k, v) in &envs {
        builder.env(k, v);
    }

    let mut store = Store::new(
        &engine,
        HostState {
            wasi: builder.build(),
            http: WasiHttpCtx::new(),
            table: wasmtime::component::ResourceTable::new(),
            storage: HashMap::new(),
        },
    );

    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).expect("wasi p2 linker");
    wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker).expect("http p2 linker");
    HarnessRoot::add_to_linker::<_, HostState>(&mut linker, |h| h).expect("near linker");

    let bindings =
        HarnessRoot::instantiate(&mut store, &component, &mut linker).expect("instantiate");

    eprintln!("[harness] calling wasi:cli/run ...");
    match bindings.wasi_cli_run().call_run(&mut store) {
        Ok(_) => {
            eprintln!("[harness] run() returned OK");
        }
        Err(e) => {
            eprintln!("[harness] run() ERROR:\n{:?}", e);
            if let Some(bt) = e.downcast_ref::<wasmtime::WasmBacktrace>() {
                eprintln!("[harness] backtrace:\n{}", bt);
            }
        }
    }
    let out = stdout_reader.contents().to_owned();
    let out = String::from_utf8_lossy(&out).to_string();
    println!("---- stdout ----\n{}", out);
    if out.trim().is_empty() {
        eprintln!("[harness] (empty stdout — trap likely before write)");
        std::process::exit(1);
    }
}
