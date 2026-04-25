//! Debug, state management, snapshot, I/O, shell, env, print, fmt, error builtins.

use crate::helpers::*;
use crate::types::{Env, LispVal};

pub fn handle(name: &str, args: &[LispVal], env: &mut Env) -> Result<Option<LispVal>, String> {
    match name {
        // --- Debug / print ---
        "error" => {
            let msg = args
                .first()
                .map(|v| format!("{}", v))
                .unwrap_or_else(|| "error".to_string());
            Err(msg)
        }
        "debug" | "near/log-debug" => {
            let msg = args
                .first()
                .map(|v| format!("{}", v))
                .unwrap_or_else(|| "debug".to_string());
            eprintln!("[DEBUG] {}", msg);
            Ok(Some(LispVal::Nil))
        }
        "trace" => {
            let val = args.first().cloned().unwrap_or(LispVal::Nil);
            eprintln!("[TRACE] {}", val);
            Ok(Some(val))
        }
        "inspect" => {
            let val = args.first().cloned().unwrap_or(LispVal::Nil);
            let type_str = match &val {
                LispVal::Nil => "nil",
                LispVal::Bool(_) => "boolean",
                LispVal::Num(_) => "integer",
                LispVal::Float(_) => "float",
                LispVal::Str(_) => "string",
                LispVal::List(items) => {
                    return Ok(Some(LispVal::Str(format!(
                        "list[{}]: {}",
                        items.len(),
                        val
                    ))))
                }
                LispVal::Map(m) => {
                    return Ok(Some(LispVal::Str(format!(
                        "map{{{} keys}}: {}",
                        m.len(),
                        val
                    ))))
                }
                LispVal::Lambda { params, .. } => {
                    return Ok(Some(LispVal::Str(format!(
                        "lambda({}): <function>",
                        params.len()
                    ))))
                }
                LispVal::Sym(s) => return Ok(Some(LispVal::Str(format!("symbol: {}", s)))),
                _ => "unknown",
            };
            Ok(Some(LispVal::Str(format!("{}: {}", type_str, val))))
        }
        "print" | "println" => {
            let s: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let out = s.join(" ");
            if name == "println" {
                println!("{}", out);
            } else {
                print!("{}", out);
            }
            Ok(Some(LispVal::Str(out)))
        }
        "fmt" => {
            let template = match &args[0] {
                LispVal::Str(s) => s.clone(),
                _ => return Err("fmt: need template string".into()),
            };
            let data = &args[1];
            let mut result = String::new();
            let chars: Vec<char> = template.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '{' {
                    let mut key = String::new();
                    i += 1;
                    while i < chars.len() && chars[i] != '}' {
                        key.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                    let mut found = false;
                    if let LispVal::Map(map) = data {
                        if let Some(val) = map.get(&key) {
                            match val {
                                LispVal::Str(s) => result.push_str(s),
                                _ => result.push_str(&val.to_string()),
                            }
                            found = true;
                        }
                    }
                    if !found {
                        result.push('{');
                        result.push_str(&key);
                        result.push('}');
                    }
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            Ok(Some(LispVal::Str(result)))
        }
        "read" => {
            let s = as_str(&args[0])?;
            match crate::parser::parse_all(&s) {
                Ok(exprs) => exprs
                    .into_iter()
                    .next()
                    .ok_or_else(|| "read: empty input".to_string())
                    .map(Some),
                Err(e) => Err(format!("read: parse error: {}", e)),
            }
        }
        "read-all" => {
            let s = as_str(&args[0])?;
            match crate::parser::parse_all(&s) {
                Ok(exprs) => Ok(Some(LispVal::List(exprs))),
                Err(e) => Err(format!("read-all: parse error: {}", e)),
            }
        }

        // --- Snapshot / rollback ---
        "snapshot" => {
            let snap = env.take_snapshot();
            let id = env.snapshots.len();
            env.snapshots.push(snap);
            Ok(Some(LispVal::Num(id as i64)))
        }
        "rollback" => {
            let snap = env
                .snapshots
                .pop()
                .ok_or("rollback: no snapshots on stack")?;
            env.restore_snapshot(snap);
            Ok(Some(LispVal::Bool(true)))
        }
        "rollback-to" => {
            let idx = as_num(args.first().ok_or("rollback-to: need index")?)? as usize;
            if idx >= env.snapshots.len() {
                return Err(format!("rollback-to: no snapshot at index {}", idx));
            }
            // Remove the snapshot from the stack (and all above it)
            let snap = env.snapshots.remove(idx);
            env.restore_snapshot(snap);
            Ok(Some(LispVal::Bool(true)))
        }

        // --- File I/O ---
        "file/read" | "read-file" => {
            let path = as_str(&args[0])?;
            match std::fs::read_to_string(&path) {
                Ok(s) => Ok(Some(LispVal::Str(s))),
                Err(e) => Err(format!("{}: {}", name, e)),
            }
        }
        "file/write" => {
            let path = as_str(&args[0])?;
            let content = as_str(&args[1])?;
            match std::fs::write(&path, content) {
                Ok(()) => Ok(Some(LispVal::Bool(true))),
                Err(e) => Err(format!("file/write: {}", e)),
            }
        }
        "file/exists?" | "file-exists?" => {
            let path = as_str(&args[0])?;
            Ok(Some(LispVal::Bool(std::path::Path::new(&path).exists())))
        }
        "file/list" => {
            let path = as_str(&args[0])?;
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let names: Vec<LispVal> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| LispVal::Str(e.file_name().to_string_lossy().to_string()))
                        .collect();
                    Ok(Some(LispVal::List(names)))
                }
                Err(e) => Err(format!("file/list: {}", e)),
            }
        }
        "write-file" => {
            // write-file is an alias for file/write — same behavior (raw content, no escaping)
            let path = as_str(&args[0])?;
            let content = as_str(&args[1])?;
            match std::fs::write(&path, content) {
                Ok(()) => Ok(Some(LispVal::Bool(true))),
                Err(e) => Err(format!("write-file: {}", e)),
            }
        }
        "load-file" => {
            let path = as_str(&args[0])?;
            let code = std::fs::read_to_string(&path).map_err(|e| format!("load-file: {}", e))?;
            let exprs = crate::parser::parse_all(&code)?;
            let mut result = LispVal::Nil;
            for expr in &exprs {
                result = super::lisp_eval(expr, env)?;
            }
            Ok(Some(result))
        }
        "append-file" => {
            let path = as_str(&args[0])?;
            let content = as_str(&args[1])?;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("append-file: {}", e))?;
            f.write_all(content.as_bytes())
                .map_err(|e| format!("append-file: {}", e))?;
            Ok(Some(LispVal::Bool(true)))
        }

        // --- Shell ---
        "shell" => {
            let cmd = as_str(&args[0])?;
            let allow = std::env::var("RLM_ALLOW_SHELL").unwrap_or_default();
            if allow != "1" && allow != "true" {
                return Err("shell: blocked unless RLM_ALLOW_SHELL=1 is set".into());
            }
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .output()
                .map_err(|e| format!("shell: {}", e))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!(
                    "shell: exit {:?}: {}{}",
                    output.status.code(),
                    stdout,
                    stderr
                ));
            }
            Ok(Some(LispVal::Str(stdout)))
        }

        // shell-bg: spawn a background process, return immediately with PID
        "shell-bg" => {
            let cmd = as_str(&args[0])?;
            let allow = std::env::var("RLM_ALLOW_SHELL").unwrap_or_default();
            if allow != "1" && allow != "true" {
                return Err("shell-bg: blocked unless RLM_ALLOW_SHELL=1 is set".into());
            }
            let child = std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("shell-bg: {}", e))?;
            let pid = child.id();
            // Store PID so shell-kill can use it later
            env.rlm_state.insert("__bg_pids".to_string(), {
                let mut pids = match env.rlm_state.get("__bg_pids") {
                    Some(LispVal::List(v)) => v.clone(),
                    _ => vec![],
                };
                pids.push(LispVal::Num(pid as i64));
                LispVal::List(pids)
            });
            Ok(Some(LispVal::Num(pid as i64)))
        }

        // shell-kill: kill a background process by PID
        "shell-kill" => {
            let pid = as_num(&args[0])?;
            let allow = std::env::var("RLM_ALLOW_SHELL").unwrap_or_default();
            if allow != "1" && allow != "true" {
                return Err("shell-kill: blocked unless RLM_ALLOW_SHELL=1 is set".into());
            }
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            Ok(Some(LispVal::Bool(true)))
        }

        // --- Env ---
        "env/get" => {
            let key = as_str(&args[0])?;
            match std::env::var(&key) {
                Ok(v) => Ok(Some(LispVal::Str(v))),
                Err(_) => Ok(Some(LispVal::Nil)),
            }
        }

        _ => Ok(None),
    }
}
