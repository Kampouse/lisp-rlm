//! Differential test: wallet factory in Rust vs lisp-rlm
//!
//! Compiles the same wallet factory logic in both Rust (raw WASM) and lisp-rlm,
//! then runs both in wasmtime with mocked NEAR host functions. Compares final
//! storage state to verify identical behavior.

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use wasmtime::*;

type Storage = Arc<Mutex<HashMap<Vec<u8>, Vec<u8>>>>;
type Registers = Arc<Mutex<HashMap<i64, Vec<u8>>>>;

/// Run a WASM module's `run` export with mocked NEAR host functions.
/// Returns the final storage state and return value.
fn run_module(
    wasm: &[u8],
    predecessor: &[u8],
) -> Result<(HashMap<Vec<u8>, Vec<u8>>, Vec<u8>), String> {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm).map_err(|e| format!("module: {}", e))?;

    let storage: Storage = Arc::new(Mutex::new(HashMap::new()));
    let registers: Registers = Arc::new(Mutex::new(HashMap::new()));
    let return_val: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let pred_bytes = predecessor.to_vec();

    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    for import in module.imports() {
        if import.module() != "env" { continue; }
        let (params, results) = match import.ty() {
            ExternType::Func(func_ty) => {
                let p: Vec<ValType> = func_ty.params().collect();
                let r: Vec<ValType> = func_ty.results().collect();
                (p, r)
            }
            _ => continue,
        };
        let name_str = import.name().to_string();
        let name_for_linker = name_str.clone();
        let storage_c = storage.clone();
        let registers_c = registers.clone();
        let return_val_c = return_val.clone();
        let pred_c = pred_bytes.clone();

        let func = Func::new(
            &mut store,
            FuncType::new(&engine, params, results),
            move |mut caller, args, ret| {
                // Get memory from the instance (exported, not imported)
                let mem = caller.get_export("memory").and_then(|e| e.into_memory());
                let data = mem.map(|m| m.data(&caller)).unwrap_or_default();

                match name_str.as_str() {
                    "storage_write" => {
                        // storage_write(key_len, key_ptr, val_len, val_ptr, register_id)
                        let key_len = args[0].unwrap_i64() as usize;
                        let key_ptr = args[1].unwrap_i64() as usize;
                        let val_len = args[2].unwrap_i64() as usize;
                        let val_ptr = args[3].unwrap_i64() as usize;

                        let key = if key_ptr + key_len <= data.len() {
                            data[key_ptr..key_ptr + key_len].to_vec()
                        } else {
                            eprintln!("WARN: storage_write key OOB ptr={} len={}", key_ptr, key_len);
                            vec![]
                        };
                        let val = if val_ptr + val_len <= data.len() {
                            data[val_ptr..val_ptr + val_len].to_vec()
                        } else {
                            eprintln!("WARN: storage_write val OOB ptr={} len={}", val_ptr, val_len);
                            vec![]
                        };
                        eprintln!("storage_write key={:?} val={:?}", String::from_utf8_lossy(&key), &val);
                        storage_c.lock().unwrap().insert(key, val);
                        if !ret.is_empty() { ret[0] = Val::I64(0); }
                        Ok(())
                    }
                    "storage_read" => {
                        // storage_read(key_len, key_ptr, register_id) -> i64
                        let key_len = args[0].unwrap_i64() as usize;
                        let key_ptr = args[1].unwrap_i64() as usize;
                        let register_id = args[2].unwrap_i64();

                        let key = if key_ptr + key_len <= data.len() {
                            data[key_ptr..key_ptr + key_len].to_vec()
                        } else {
                            vec![]
                        };
                        eprintln!("storage_read key={:?}", String::from_utf8_lossy(&key));

                        let found = if let Some(val) = storage_c.lock().unwrap().get(&key) {
                            registers_c.lock().unwrap().insert(register_id, val.clone());
                            1i64
                        } else {
                            0i64
                        };
                        if !ret.is_empty() { ret[0] = Val::I64(found); }
                        Ok(())
                    }
                    "register_len" => {
                        let register_id = args[0].unwrap_i64();
                        let len = registers_c
                            .lock().unwrap()
                            .get(&register_id)
                            .map(|v| v.len() as i64)
                            .unwrap_or(-1i64);
                        if !ret.is_empty() { ret[0] = Val::I64(len); }
                        Ok(())
                    }
                    "read_register" => {
                        let register_id = args[0].unwrap_i64();
                        let ptr = args[1].unwrap_i64() as usize;
                        if let Some(bytes) = registers_c.lock().unwrap().get(&register_id).cloned() {
                            if let Some(mem) = mem {
                                let mut md = mem.data_mut(caller);
                                let end = (ptr + bytes.len()).min(md.len());
                                if ptr < md.len() {
                                    md[ptr..end].copy_from_slice(&bytes[..end - ptr]);
                                }
                            }
                        }
                        Ok(())
                    }
                    "predecessor_account_id" => {
                        let register_id = args[0].unwrap_i64();
                        eprintln!("predecessor_account_id -> reg {}", register_id);
                        registers_c.lock().unwrap().insert(register_id, pred_c.clone());
                        Ok(())
                    }
                    "current_account_id" => {
                        let register_id = args[0].unwrap_i64();
                        registers_c.lock().unwrap().insert(register_id, b"factory.kampy.testnet".to_vec());
                        Ok(())
                    }
                    "signer_account_id" => {
                        let register_id = args[0].unwrap_i64();
                        registers_c.lock().unwrap().insert(register_id, pred_c.clone());
                        Ok(())
                    }
                    "signer_account_pk" => {
                        let register_id = args[0].unwrap_i64();
                        registers_c.lock().unwrap().insert(register_id, vec![0u8; 32]);
                        Ok(())
                    }
                    "input" => {
                        let register_id = args[0].unwrap_i64();
                        registers_c.lock().unwrap().insert(register_id, vec![]);
                        Ok(())
                    }
                    "value_return" => {
                        let len = args[0].unwrap_i64() as usize;
                        let ptr = args[1].unwrap_i64() as usize;
                        let val = if ptr + len <= data.len() {
                            data[ptr..ptr + len].to_vec()
                        } else {
                            vec![]
                        };
                        eprintln!("value_return len={} val={:?}", len, &val);
                        *return_val_c.lock().unwrap() = val;
                        Ok(())
                    }
                    _ => {
                        for r in ret.iter_mut() { *r = Val::I64(0); }
                        Ok(())
                    }
                }
            },
        );
        linker.define(&store, "env", &name_for_linker, func).unwrap();
    }

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {}", e))?;

    // Try _run (lisp-rlm) then run (Rust)
    let run_fn = instance
        .get_export(&mut store, "_run")
        .and_then(|e| e.into_func())
        .or_else(|| instance.get_export(&mut store, "run").and_then(|e| e.into_func()))
        .ok_or("no 'run' or '_run' export")?;

    run_fn.call(&mut store, &[], &mut []).map_err(|e| format!("call run: {}", e))?;

    let s = storage.lock().unwrap().clone();
    let r = return_val.lock().unwrap().clone();
    Ok((s, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_lisp() -> Vec<u8> {
        let status = Command::new("cargo")
            .args(["run", "--release", "--bin", "near-compile", "--",
                   "/tmp/wallet_factory.lisp", "/tmp/wallet_factory_test.wasm"])
            .current_dir("/Users/asil/.openclaw/workspace/lisp-rlm")
            .status()
            .expect("failed to run near-compile");
        assert!(status.success(), "near-compile failed");
        std::fs::read("/tmp/wallet_factory_test.wasm").expect("read wasm")
    }

    fn compile_rust() -> Vec<u8> {
        let status = Command::new("cargo")
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .current_dir("/tmp/wallet_factory_rust")
            .status()
            .expect("failed to build rust ref");
        assert!(status.success(), "rust build failed");
        std::fs::read("/tmp/wallet_factory_rust/target/wasm32-unknown-unknown/release/wallet_factory_ref.wasm")
            .expect("read rust wasm")
    }

    #[test]
    fn wallet_factory_code_size() {
        let lisp_wasm = compile_lisp();
        let rust_wasm = compile_rust();
        let predecessor = b"kampy.testnet";

        let (lisp_storage, lisp_return) = run_module(&lisp_wasm, predecessor).expect("lisp module failed");
        let (rust_storage, rust_return) = run_module(&rust_wasm, predecessor).expect("rust module failed");

        eprintln!("Lisp keys: {:?}", lisp_storage.keys().map(|k| String::from_utf8_lossy(k)).collect::<Vec<_>>());
        eprintln!("Rust keys: {:?}", rust_storage.keys().map(|k| String::from_utf8_lossy(k)).collect::<Vec<_>>());
        eprintln!("Lisp code_size: {:?}", lisp_storage.get(b"code_size".as_slice()));
        eprintln!("Rust code_size: {:?}", rust_storage.get(b"code_size".as_slice()));
        eprintln!("Lisp return: {:?}", lisp_return);
        eprintln!("Rust return: {:?}", rust_return);

        let lisp_size = lisp_storage.get(b"code_size".as_slice());
        let rust_size = rust_storage.get(b"code_size".as_slice());
        assert!(lisp_size.is_some(), "lisp should have code_size key");
        assert!(rust_size.is_some(), "rust should have code_size key");
        assert_eq!(lisp_size, rust_size, "code_size storage must match");
    }

    #[test]
    fn wallet_factory_code_hash() {
        let lisp_wasm = compile_lisp();
        let rust_wasm = compile_rust();
        let predecessor = b"kampy.testnet";

        let (lisp_storage, _) = run_module(&lisp_wasm, predecessor).unwrap();
        let (rust_storage, _) = run_module(&rust_wasm, predecessor).unwrap();

        let lisp_hash = lisp_storage.get(b"code_hash".as_slice());
        let rust_hash = rust_storage.get(b"code_hash".as_slice());
        assert!(lisp_hash.is_some(), "lisp should have code_hash");
        assert!(rust_hash.is_some(), "rust should have code_hash");
        assert_eq!(lisp_hash, rust_hash, "code_hash storage must match");
    }

    #[test]
    fn wallet_factory_valid_wasm() {
        let lisp_wasm = compile_lisp();
        let rust_wasm = compile_rust();
        let engine = Engine::default();
        Module::new(&engine, &lisp_wasm).expect("lisp WASM should be valid");
        Module::new(&engine, &rust_wasm).expect("rust WASM should be valid");
    }
}
