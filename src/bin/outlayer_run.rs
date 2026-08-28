//! outlayer-run: run a compiled OutLayer WASI program in wasmtime with REAL
//! host support (HTTP via curl, canonical split-interface imports).
//!
//! Usage: outlayer-run <file.lisp> [stdin-bytes]
//!
//! Prints the RESULT_BUF payload; string results are decoded from memory.

use wasmtime::*;
use lisp_rlm_wasm::wasi::compile_outlayer;

fn main() {
    let path = std::env::args().nth(1).expect("usage: outlayer-run <file.lisp> [stdin]");
    let stdin_data = std::env::args().nth(2).unwrap_or_default();
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));

    let wasm = compile_outlayer(&src).unwrap_or_else(|e| panic!("compile: {e}"));
    println!("📦 {} → {} bytes of wasm", path, wasm.len());

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).expect("validate");
    let mut store = Store::new(&engine, ());

    // ── WASI stubs ──
    let sd = stdin_data.clone().into_bytes();
    let fd_read_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        move |mut caller, args, results| {
            let iov_ptr = args[1].unwrap_i32() as usize;
            let nread_ptr = args[3].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let buf_len = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    let n = sd.len().min(buf_len);
                    if buf_ptr + n <= data.len() { data[buf_ptr..buf_ptr+n].copy_from_slice(&sd[..n]); }
                    if nread_ptr + 4 <= data.len() { data[nread_ptr..nread_ptr+4].copy_from_slice(&(n as u32).to_le_bytes()); }
                }
            }
            results[0] = Val::I32(0); Ok(())
        });
    let fd_write_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |mut caller, args, results| {
            // Echo stdout to the console
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                let iov_ptr = args[1].unwrap_i32() as usize;
                if iov_ptr + 8 <= data.len() {
                    let p = u32::from_le_bytes(data[iov_ptr..iov_ptr+4].try_into().unwrap()) as usize;
                    let l = u32::from_le_bytes(data[iov_ptr+4..iov_ptr+8].try_into().unwrap()) as usize;
                    if p + l <= data.len() {
                        print!("{}", String::from_utf8_lossy(&data[p..p+l]));
                    }
                }
            }
            results[0] = Val::I32(args[2].unwrap_i32()); Ok(())
        });
    let proc_exit_fn = Func::new(&mut store, FuncType::new(&engine, vec![ValType::I32], vec![]),
        |_, args, _| Err(wasmtime::Error::msg(format!("proc_exit({})", args[0].unwrap_i32()))));
    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    // ── REAL http-get (canonical ABI: url_ptr, url_len, ret_area) ──
    let http_get_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32; 3], vec![]),
        |mut caller, args, _| {
            let url_ptr = args[0].unwrap_i32() as usize;
            let url_len = args[1].unwrap_i32() as usize;
            let ret_area = args[2].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if url_ptr + url_len <= data.len() && ret_area + 16 <= data.len() {
                    let url = String::from_utf8_lossy(&data[url_ptr..url_ptr+url_len]).to_string();
                    eprintln!("🌐 http-get {url}");
                    let resp = std::process::Command::new("curl")
                        .args(["-s", "--max-time", "10", &url]).output();
                    let body = match resp {
                        Ok(o) if o.status.success() => o.stdout,
                        _ => Vec::new(),
                    };
                    let dst = ret_area + 16;
                    let n = body.len().min(data.len().saturating_sub(dst));
                    if n > 0 { data[dst..dst+n].copy_from_slice(&body[..n]); }
                    data[ret_area+4..ret_area+8].copy_from_slice(&(dst as u32).to_le_bytes());
                    data[ret_area+8..ret_area+12].copy_from_slice(&(n as u32).to_le_bytes());
                }
            }
            Ok(())
        });

    // ── Stub hosts (canonical split interfaces) ──
    let stub = move |store: &mut Store<()>, n: usize, r: bool| -> Func {
        let eng = store.engine().clone();
        if r {
            Func::new(store, FuncType::new(&eng, vec![ValType::I32; n], vec![ValType::I32]),
                |_, _, results| { results[0] = Val::I32(0); Ok(()) })
        } else {
            Func::new(store, FuncType::new(&eng, vec![ValType::I32; n], vec![]),
                |_, _, _| Ok(()))
        }
    };
    let http_post_fn = stub(&mut store, 7, false);
    let view_fn = stub(&mut store, 9, false);
    let call_fn = stub(&mut store, 17, false);
    let transfer_fn = stub(&mut store, 11, false);
    let raw_fn = stub(&mut store, 5, false);
    let storage_set_fn = stub(&mut store, 5, false);
    let storage_get_fn = stub(&mut store, 3, false);
    let storage_has_fn = stub(&mut store, 2, true);
    let storage_delete_fn = stub(&mut store, 2, true);
    let storage_incr_fn = Func::new(&mut store,
        FuncType::new(&engine, vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32], vec![]),
        |_, _, _| Ok(()));
    let storage_sia_fn = stub(&mut store, 5, false);
    let storage_sie_fn = stub(&mut store, 7, false);
    let storage_lk_fn = stub(&mut store, 3, false);
    let storage_ca_fn = stub(&mut store, 1, false);

    let mut linker = Linker::new(&engine);
    linker.define(&store, "wasi_snapshot_preview1", "fd_read", fd_read_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_write", fd_write_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "random_get", random_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_sizes_get", environ_sizes_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "environ_get", environ_get_fn).unwrap();
    linker.define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "view", view_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "call", call_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "transfer", transfer_fn).unwrap();
    linker.define(&store, "near:rpc/api@0.1.0", "raw", raw_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "http-get", http_get_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "http-post", http_post_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-set", storage_set_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-get", storage_get_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-has", storage_has_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-delete", storage_delete_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-increment", storage_incr_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-set-if-absent", storage_sia_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-set-if-equals", storage_sie_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-list-keys", storage_lk_fn).unwrap();
    linker.define(&store, "outlayer:api/host@0.1.0", "storage-clear-all", storage_ca_fn).unwrap();

    let instance = linker.instantiate(&mut store, &module).expect("instantiate");
    let start = instance.get_typed_func::<(), ()>(&mut store, "_start").expect("_start");
    match start.call(&mut store, ()) {
        Ok(()) => {}
        Err(trap) => {
            let msg = trap.to_string();
            let is_exit = msg.contains("proc_exit")
                || trap.source().map(|s| s.to_string().contains("proc_exit")).unwrap_or(false);
            if !is_exit { panic!("run failed: {msg}"); }
        }
    }

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let data = memory.data(&store);
    let payload = i64::from_le_bytes(data[65536..65536+8].try_into().unwrap());
    let ptr = (payload & 0xFFFFFFFF) as u32 as usize;
    let len = ((payload >> 32) as u32) as usize;
    if len > 0 && len < 1_000_000 && ptr + len <= data.len() && ptr > 0 {
        let s = String::from_utf8_lossy(&data[ptr..ptr+len]);
        println!("📄 result (str, {len} bytes): {s}");
    } else {
        println!("📄 result (num): {payload}");
    }
}
