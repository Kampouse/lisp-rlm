//! outlayer-run: run a compiled OutLayer WASI program in wasmtime with REAL
//! host support (HTTP via curl, canonical split-interface imports).
//!
//! Usage: outlayer-run <file.lisp> [stdin-bytes]
//!
//! Prints the RESULT_BUF payload; string results are decoded from memory.

use lisp_rlm_wasm::wasi::compile_outlayer;
use wasmtime::*;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: outlayer-run <file.lisp> [stdin]");
    let stdin_data = std::env::args().nth(2).unwrap_or_default();
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));

    let wasm = compile_outlayer(&src).unwrap_or_else(|e| panic!("compile: {e}"));
    println!("📦 {} → {} bytes of wasm", path, wasm.len());
    if let Ok(dump) = std::env::var("OUTLAYER_DUMP") {
        std::fs::write(&dump, &wasm).expect("dump");
    }

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).expect("validate");
    let mut store = Store::new(&engine, ());

    // HTTP body scratch (mirrors wasi_http::SENTINEL_BUF): 128MB - 4KB - 1MB.
    // The wasm-side shim copies the body from wherever body_ptr points, so
    // hosting large bodies here avoids the 4KB ret-area truncation.
    const HTTP_SBUF: usize = 2048 * 65536 - 4096 - 1_048_576;

    // ── WASI stubs ──
    let sd = stdin_data.clone().into_bytes();
    let fd_read_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        move |mut caller, args, results| {
            let iov_ptr = args[1].unwrap_i32() as usize;
            let nread_ptr = args[3].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if iov_ptr + 8 <= data.len() {
                    let buf_ptr =
                        u32::from_le_bytes(data[iov_ptr..iov_ptr + 4].try_into().unwrap()) as usize;
                    let buf_len =
                        u32::from_le_bytes(data[iov_ptr + 4..iov_ptr + 8].try_into().unwrap())
                            as usize;
                    let n = sd.len().min(buf_len);
                    if buf_ptr + n <= data.len() {
                        data[buf_ptr..buf_ptr + n].copy_from_slice(&sd[..n]);
                    }
                    if nread_ptr + 4 <= data.len() {
                        data[nread_ptr..nread_ptr + 4].copy_from_slice(&(n as u32).to_le_bytes());
                    }
                }
            }
            results[0] = Val::I32(0);
            Ok(())
        },
    );
    let fd_write_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32; 4], vec![ValType::I32]),
        |mut caller, args, results| {
            // Echo stdout to the console: iterate ALL iov entries
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data(&caller);
                let iov_ptr = args[1].unwrap_i32() as usize;
                let iov_count = args[2].unwrap_i32().max(0) as usize;
                let mut total: u32 = 0;
                for i in 0..iov_count {
                    let base = iov_ptr + i * 8;
                    if base + 8 > data.len() {
                        break;
                    }
                    let p = u32::from_le_bytes(data[base..base + 4].try_into().unwrap()) as usize;
                    let l =
                        u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap()) as usize;
                    if p + l <= data.len() {
                        print!("{}", String::from_utf8_lossy(&data[p..p + l]));
                        total += l as u32;
                    }
                }
                results[0] = Val::I32(total as i32);
            } else {
                results[0] = Val::I32(0);
            }
            Ok(())
        },
    );
    let proc_exit_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32], vec![]),
        |_, args, _| {
            Err(wasmtime::Error::msg(format!(
                "proc_exit({})",
                args[0].unwrap_i32()
            )))
        },
    );
    let random_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_sizes_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let environ_get_fn = Func::wrap(&mut store, |_: i32, _: i32| -> i32 { 0 });
    let fd_seek_fn = Func::wrap(&mut store, |_: i32, _: i64, _: i32, _: i32| -> i32 { 0 });

    // ── REAL http-get (canonical ABI: url_ptr, url_len, ret_area) ──
    let http_get_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32; 3], vec![]),
        |mut caller, args, _| {
            let url_ptr = args[0].unwrap_i32() as usize;
            let url_len = args[1].unwrap_i32() as usize;
            let ret_area = args[2].unwrap_i32() as usize;
            if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                let data = mem.data_mut(&mut caller);
                if url_ptr + url_len <= data.len() && ret_area + 16 <= data.len() {
                    let url =
                        String::from_utf8_lossy(&data[url_ptr..url_ptr + url_len]).to_string();
                    eprintln!("🌐 http-get {url}");
                    let resp = std::process::Command::new("curl")
                        .args(["-s", "--max-time", "10", &url])
                        .output();
                    let body = match resp {
                        Ok(o) if o.status.success() => o.stdout,
                        _ => Vec::new(),
                    };
                    let dst = HTTP_SBUF;
                    let n = body.len().min(data.len().saturating_sub(dst));
                    if n > 0 {
                        data[dst..dst + n].copy_from_slice(&body[..n]);
                    }
                    data[ret_area + 4..ret_area + 8].copy_from_slice(&(dst as u32).to_le_bytes());
                    data[ret_area + 8..ret_area + 12].copy_from_slice(&(n as u32).to_le_bytes());
                }
            }
            Ok(())
        },
    );

    // ── Stub hosts (canonical split interfaces) ──
    let stub = move |store: &mut Store<()>, n: usize, r: bool| -> Func {
        let eng = store.engine().clone();
        if r {
            Func::new(
                store,
                FuncType::new(&eng, vec![ValType::I32; n], vec![ValType::I32]),
                |_, _, results| {
                    results[0] = Val::I32(0);
                    Ok(())
                },
            )
        } else {
            Func::new(
                store,
                FuncType::new(&eng, vec![ValType::I32; n], vec![]),
                |_, _, _| Ok(()),
            )
        }
    };
    // ── REAL http-post (canonical ABI: url, body, content_type, ret_area) ──
    // ret_area layout: +0 error flag (0 = ok), +4 body ptr, +8 body len
    // Scratch rotation: each POST writes at HTTP_SBUF + n*STRIDE (n bumped
    // per call) — a single shared base made a 2nd POST overwrite the 1st's
    // body before the program read it (dual-chain pipe test, 2026-10-05).
    let post_scratch = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let http_post_fn = Func::new(
        &mut store,
        FuncType::new(&engine, vec![ValType::I32; 7], vec![]),
        {
            let post_scratch = post_scratch.clone();
            move |mut caller, args, _| {
                let (up, ul, bp, bl, cp, cl, ra) = (
                    args[0].unwrap_i32() as usize,
                    args[1].unwrap_i32() as usize,
                    args[2].unwrap_i32() as usize,
                    args[3].unwrap_i32() as usize,
                    args[4].unwrap_i32() as usize,
                    args[5].unwrap_i32() as usize,
                    args[6].unwrap_i32() as usize,
                );
                if let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) {
                    let data = mem.data_mut(&mut caller);
                    let ok = up + ul <= data.len()
                        && bp + bl <= data.len()
                        && cp + cl <= data.len()
                        && ra + 16 <= data.len();
                    if ok {
                        let url = String::from_utf8_lossy(&data[up..up + ul]).to_string();
                        let body = data[bp..bp + bl].to_vec();
                        let ct = String::from_utf8_lossy(&data[cp..cp + cl]).to_string();
                        eprintln!("🌐 http-post {url} ({ct}, {} bytes)", body.len());
                        
                        let body_path = format!("/tmp/__outlayer_post_body_{}", std::process::id());
                        std::fs::write(&body_path, &body).ok();
                        // Header block: newline-separated "Name: value" lines.
                        // Single line (no \n) = plain Content-Type — fully backward
                        // compatible. Needed for Authorization: Bearer on OutLayer
                        // wallet API (2026-10-05, solana sign test).
                        let headers: Vec<String> = ct
                            .split('\n')
                            .map(|l| l.trim())
                            .filter(|l| !l.is_empty())
                            .map(|l| {
                                if l.contains(':')
                                    && !l.to_ascii_lowercase().starts_with("content-type:")
                                {
                                    l.to_string()
                                } else if l.to_ascii_lowercase().starts_with("content-type:") {
                                    l.to_string()
                                } else {
                                    format!("Content-Type: {l}")
                                }
                            })
                            .collect();
                        eprintln!("🧾 headers: {headers:?}");
                        let mut cmd = std::process::Command::new("curl");
                        cmd.args(["-s", "--max-time", "10", "-X", "POST"]);
                        for h in &headers {
                            cmd.arg("-H").arg(h);
                        }
                        cmd.arg("--data-binary")
                            .arg(&format!("@{body_path}"))
                            .arg(&url);
                        let resp = cmd.output();
                        std::fs::remove_file(&body_path).ok();
                        let (err, out): (u32, Vec<u8>) = match resp {
                            Ok(o) if o.status.success() => (0, o.stdout),
                            _ => (1, Vec::new()),
                        };
                        // Rotate DOWNWARD from the ret area: the only free HTTP
                        // region is [HTTP_SBUF, OL_RET_AREA_BASE) (~1MB). Slot N
                        // writes at RET_AREA-8KB*(N+1) — never touching the ret
                        // area itself (slot1 at HTTP_SBUF+1MB == OL_RET_AREA_BASE
                        // clobbered the return ptr/len — the corrupted-response-
                        // head bug, 2026-10-05). 128 slots × 8KB.
                        let slot = post_scratch.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let stride = 8192usize;
                        let dst =
                            (HTTP_SBUF + 1_048_576 - stride * ((slot % 127) + 1)).max(HTTP_SBUF);
                        let n = out.len().min(stride).min(data.len().saturating_sub(dst));
                        if n > 0 {
                            data[dst..dst + n].copy_from_slice(&out[..n]);
                        }
                        data[ra..ra + 4].copy_from_slice(&err.to_le_bytes());
                        data[ra + 4..ra + 8].copy_from_slice(&(dst as u32).to_le_bytes());
                        data[ra + 8..ra + 12].copy_from_slice(&(n as u32).to_le_bytes());
                    } else {
                        data[ra..ra + 4].copy_from_slice(&1u32.to_le_bytes());
                    }
                }
                Ok(())
            }
        },
    );
    let view_fn = stub(&mut store, 9, false);
    let call_fn = stub(&mut store, 17, false);
    let transfer_fn = stub(&mut store, 11, false);
    let raw_fn = stub(&mut store, 5, false);
    let storage_set_fn = stub(&mut store, 5, false);
    let storage_get_fn = stub(&mut store, 3, false);
    let storage_has_fn = stub(&mut store, 2, true);
    let storage_delete_fn = stub(&mut store, 2, true);
    let storage_incr_fn = Func::new(
        &mut store,
        FuncType::new(
            &engine,
            vec![ValType::I32, ValType::I32, ValType::I64, ValType::I32],
            vec![],
        ),
        |_, _, _| Ok(()),
    );
    let storage_sia_fn = stub(&mut store, 5, false);
    let storage_sie_fn = stub(&mut store, 7, false);
    let storage_lk_fn = stub(&mut store, 3, false);
    let storage_ca_fn = stub(&mut store, 1, false);

    let mut linker = Linker::new(&engine);
    linker
        .define(&store, "wasi_snapshot_preview1", "fd_read", fd_read_fn)
        .unwrap();
    linker
        .define(&store, "wasi_snapshot_preview1", "fd_write", fd_write_fn)
        .unwrap();
    linker
        .define(&store, "wasi_snapshot_preview1", "proc_exit", proc_exit_fn)
        .unwrap();
    linker
        .define(
            &store,
            "wasi_snapshot_preview1",
            "random_get",
            random_get_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            environ_sizes_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "wasi_snapshot_preview1",
            "environ_get",
            environ_get_fn,
        )
        .unwrap();
    linker
        .define(&store, "wasi_snapshot_preview1", "fd_seek", fd_seek_fn)
        .unwrap();
    linker
        .define(&store, "near:rpc/api@0.1.0", "view", view_fn)
        .unwrap();
    linker
        .define(&store, "near:rpc/api@0.1.0", "call", call_fn)
        .unwrap();
    linker
        .define(&store, "near:rpc/api@0.1.0", "transfer", transfer_fn)
        .unwrap();
    linker
        .define(&store, "near:rpc/api@0.1.0", "raw", raw_fn)
        .unwrap();
    linker
        .define(&store, "outlayer:api/host@0.1.0", "http-get", http_get_fn)
        .unwrap();
    linker
        .define(&store, "outlayer:api/host@0.1.0", "http-post", http_post_fn)
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-set",
            storage_set_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-get",
            storage_get_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-has",
            storage_has_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-delete",
            storage_delete_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-increment",
            storage_incr_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-set-if-absent",
            storage_sia_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-set-if-equals",
            storage_sie_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-list-keys",
            storage_lk_fn,
        )
        .unwrap();
    linker
        .define(
            &store,
            "outlayer:api/host@0.1.0",
            "storage-clear-all",
            storage_ca_fn,
        )
        .unwrap();

    let instance = linker
        .instantiate(&mut store, &module)
        .expect("instantiate");
    let start = instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .expect("_start");
    match start.call(&mut store, ()) {
        Ok(()) => {}
        Err(trap) => {
            let msg = trap.to_string();
            let is_exit = msg.contains("proc_exit")
                || trap
                    .source()
                    .map(|s| s.to_string().contains("proc_exit"))
                    .unwrap_or(false);
            if !is_exit {
                panic!("run failed: {msg}");
            }
        }
    }

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let data = memory.data(&store);
    if let Ok(dump) = std::env::var("OUTLAYER_MEMDUMP") {
        std::fs::write(&dump, &data[..data.len().min(300_000)]).expect("memdump");
        eprintln!("memdump → {dump}");
    }
    // Result readback: the outlayer shim stages the HTTP response in the
    // STDOUT region (from 65536), at a layout that has drifted between shim
    // versions. Scan the whole region for the classic Coinbase shape
    // "amount":"<number>" and print that; fall back to the tagged return
    // value at TEMP_MEM (64). The old code i64-read bytes at 65536 and
    // printed register garbage ("result (num): 4295784...").
    let region = &data[65536..data.len().min(65536 + 64 * 1024)];
    let mut printed = false;
    // The staged body may be truncated at its head (shim window), so instead
    // of keying on "amount" find the first decimal number: >=3 int digits
    // followed by '.' and fraction digits (matches prices, skips versions).
    'scan: for w in 0..region.len().saturating_sub(12) {
        let b = region[w];
        if b.is_ascii_digit() && (w == 0 || !region[w - 1].is_ascii_digit()) {
            let mut e = w;
            while e < region.len() && region[e].is_ascii_digit() {
                e += 1;
            }
            if e < region.len()
                && region[e] == b'.'
                && e + 1 < region.len()
                && region[e + 1].is_ascii_digit()
            {
                let mut f = e + 1;
                let mut frac = 0;
                while f < region.len() && region[f].is_ascii_digit() && frac < 6 {
                    f += 1;
                    frac += 1;
                }
                if e - w >= 3 {
                    println!("📄 result: {}", String::from_utf8_lossy(&region[w..f]));
                    printed = true;
                    break 'scan;
                }
            }
        }
    }
    if !printed {
        let tagged = i64::from_le_bytes(data[64..64 + 8].try_into().unwrap());
        let tag = tagged & 0b111;
        let payload = tagged >> 3;
        if tag == 5 {
            let off = (payload & 0xFFFFFFFF) as usize;
            let len = ((payload >> 32) as u32) as usize;
            if off > 0 && off + len <= data.len() && len < 1_000_000 {
                println!(
                    "📄 result (str): {}",
                    String::from_utf8_lossy(&data[off..off + len])
                );
                printed = true;
            }
        } else if payload != 0 {
            println!("📄 result (num): {payload}");
            printed = true;
        }
    }
    if !printed {
        println!("📄 result: (none)");
    }
}
