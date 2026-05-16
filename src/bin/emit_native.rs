use lisp_rlm_wasm::wasi_p2_native::build_native;

fn main() {
    let core = build_native();
    println!("Core WASM: {} bytes", core.len());
    
    let core_path = "/tmp/native_http_core.wasm";
    std::fs::write(core_path, &core).unwrap();
    println!("Core saved to: {}", core_path);
    
    // Find wasi-http WIT
    let home = std::env::var("HOME").unwrap_or("/Users/asil".into());
    let reg = std::path::PathBuf::from(home).join(".cargo/registry/src");
    let wit_dir = find_wasi_http_wit(&reg);
    
    if let Some(wit) = wit_dir {
        println!("WIT: {}", wit.display());
        println!("\nCommands:");
        println!("  wasm-tools component embed '{}' {} --world bindings -o /tmp/native_http_embed.wasm", wit.display(), core_path);
        println!("  wasm-tools component new /tmp/native_http_embed.wasm -o /tmp/native_http.wasm");
    }
}

fn find_wasi_http_wit(reg: &std::path::Path) -> Option<std::path::PathBuf> {
    for entry in std::fs::read_dir(reg).ok()? {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name()?.to_str()?;
            if name.starts_with("index.crates.io") {
                for sub in std::fs::read_dir(&path).ok()? {
                    let sub = sub.ok()?;
                    let sp = sub.path();
                    if sp.file_name()?.to_str()?.starts_with("wasmtime-wasi-http") {
                        let wit = sp.join("wit");
                        if wit.exists() {
                            return Some(wit);
                        }
                    }
                }
            }
        }
    }
    None
}
