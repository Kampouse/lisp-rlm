use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: near-compile <input.lisp> [output.wasm]");
        std::process::exit(1);
    }
    let src = fs::read_to_string(&args[1]).expect("read input");

    // Compile and get function names for error reporting
    let (wasm_bytes, func_names) = lisp_rlm_wasm::wasm_emit::compile_near_named(&src).unwrap_or_else(|e| {
        eprintln!("❌ Compile error: {}", e);
        std::process::exit(1);
    });

    if let Err(e) = validate_wasm(&wasm_bytes, &func_names) {
        let out = if args.len() > 2 { args[2].clone() } else { args[1].replace(".lisp", ".wasm") };
        let _ = fs::write(&out, &wasm_bytes);
        std::process::exit(1);
    }

    let out = if args.len() > 2 { args[2].clone() } else { args[1].replace(".lisp", ".wasm") };
    fs::write(&out, &wasm_bytes).expect("write WASM");
    println!("✅ {} ({} bytes) — validated", out, wasm_bytes.len());
}

fn validate_wasm(wasm: &[u8], func_names: &[String]) -> Result<(), String> {
    let mut validator = wasmparser::Validator::new();
    match validator.validate_all(wasm) {
        Ok(_) => Ok(()),
        Err(e) => {
            let err_str = e.to_string();
            // Try to extract byte offset from error message
            let offset = extract_offset(&err_str);
            let func_name = offset.and_then(|off| find_function_at_offset(wasm, off, func_names));
            match func_name {
                Some(name) => eprintln!("❌ WASM error in function `{}`: {}", name, err_str),
                None => eprintln!("❌ WASM validation error: {}", err_str),
            }
            Err(err_str)
        }
    }
}

fn extract_offset(err: &str) -> Option<usize> {
    // wasmparser errors look like "type mismatch at offset 0xd6" or "at offset 123"
    for part in err.rsplit("offset ") {
        let s = part.trim();
        // Try hex: 0x...
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            if let Ok(n) = usize::from_str_radix(hex.trim_end_matches(')'), 16) {
                return Some(n);
            }
        }
        // Try decimal
        if let Ok(n) = s.trim_end_matches(')').parse::<usize>() {
            return Some(n);
        }
    }
    None
}

fn find_function_at_offset(wasm: &[u8], target_offset: usize, func_names: &[String]) -> Option<String> {
    // Parse WASM to find code section and function body ranges
    // Code section is section ID 10
    let mut pos = 8; // skip magic + version
    while pos < wasm.len() {
        let section_id = *wasm.get(pos)? as usize;
        pos += 1;
        let (section_size, leb_bytes) = read_leb128(&wasm[pos..])?;
        pos += leb_bytes;
        let section_start = pos;
        let section_end = pos + section_size;

        if section_id == 10 {
            // Code section — contains function bodies
            let mut body_pos = section_start;
            // Skip count
            let (_count, leb) = read_leb128(&wasm[body_pos..])?;
            body_pos += leb;

            let mut func_idx = 0;
            while body_pos < section_end && func_idx < func_names.len() {
                let (body_size, leb) = read_leb128(&wasm[body_pos..])?;
                let body_start = body_pos;
                let body_end = body_pos + leb + body_size;

                if target_offset >= body_start && target_offset < body_end {
                    return Some(func_names[func_idx].clone());
                }
                body_pos = body_end;
                func_idx += 1;
            }
            // Also check wrapper functions (after user functions)
            let mut wrapper_idx = 0;
            while body_pos < section_end {
                let (body_size, leb) = read_leb128(&wasm[body_pos..])?;
                let body_start = body_pos;
                let body_end = body_pos + leb + body_size;

                if target_offset >= body_start && target_offset < body_end {
                    // Map wrapper to exported function name
                    let user_func_count = func_names.len();
                    // Wrappers correspond to exports, which map back to user functions
                    return Some(format!("<wrapper#{}>", wrapper_idx));
                }
                body_pos = body_end;
                wrapper_idx += 1;
            }
            return None;
        }
        pos = section_end;
    }
    None
}

fn read_leb128(data: &[u8]) -> Option<(usize, usize)> {
    let mut result = 0usize;
    let mut shift = 0;
    for (i, &byte) in data.iter().enumerate() {
        result |= ((byte & 0x7F) as usize) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        if shift > 63 {
            return None;
        }
    }
    None
}
