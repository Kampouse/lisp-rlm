// lisp-diff — form-level interp-vs-wasm divergence finder (TASK-DX-AGREE item 3)
//
// Runs every top-level VALUE form of a .lisp file through BOTH backends:
//   * interp : crate tree-walker (program::run_program, shared env)
//   * wasm   : compile_fuzz + wasmtime, with the defines seen so far as
//              prefix and the form wrapped as (define (main) <form>)
// and prints the FIRST divergent form with both backends' results, or
// IDENTICAL when every compared form agrees. Exit 0 = identical (or only
// skipped forms), 1 = divergence found, 2 = usage/parse error.
//
// This is the CLI twin of tests/differential.rs (fixture-based) and of
// fuzz_common::lockstep_first_divergence (op-level spec-vs-rust VM).
// Known cross-backend semantic gaps (see the differential header) show up
// here as divergences on purpose — the tool reports, it does not judge.

use lisp_rlm_wasm::parser;
use lisp_rlm_wasm::run_program;
use lisp_rlm_wasm::tagged_value::{decode, TaggedValue};
use lisp_rlm_wasm::types::{Env, EvalState, LispVal};
use wasmtime::{Engine, ExternType, Func, FuncType, Instance, Linker, Memory, Module, Store, Val, ValType};

const STACK: usize = 512 * 1024 * 1024;

fn usage() -> ! {
    eprintln!("usage: lisp-diff <file.lisp>");
    eprintln!();
    eprintln!("  Compares each top-level value form through the interp and the");
    eprintln!("  wasm (compile_fuzz + wasmtime) backends. Top-level (define ...)");
    eprintln!("  forms are interp-evaluated for env effects and prepended to the");
    eprintln!("  wasm prefix. Prints the first divergent form with both results,");
    eprintln!("  or IDENTICAL. Exit: 0 identical, 1 divergence, 2 usage error.");
    std::process::exit(2);
}

fn tv_to_lisp(mem: &[u8], tv: TaggedValue) -> LispVal {
    match tv {
        TaggedValue::Num(n) => LispVal::Num(n),
        TaggedValue::Bool(b) => LispVal::Bool(b),
        TaggedValue::Nil => LispVal::Nil,
        TaggedValue::Str { ptr, len } => {
            if ptr >= 0 && len >= 0 && (ptr + len) as usize <= mem.len() {
                LispVal::Str(
                    String::from_utf8_lossy(&mem[ptr as usize..(ptr + len) as usize]).to_string(),
                )
            } else {
                LispVal::Str(format!("<badstr {ptr:#x}/{len}>"))
            }
        }
        TaggedValue::Array { ptr, count } => {
            let mut items = Vec::new();
            for i in 0..count {
                let off = (ptr + 8 + i * 8) as usize;
                if off + 8 <= mem.len() {
                    let raw = i64::from_le_bytes(mem[off..off + 8].try_into().unwrap());
                    items.push(tv_to_lisp(mem, decode(mem, raw)));
                }
            }
            LispVal::List(items)
        }
        TaggedValue::FnRef(_) | TaggedValue::Closure(_) => LispVal::Str("<fn>".into()),
    }
}

fn canon(v: LispVal) -> LispVal {
    match v {
        LispVal::Float(f) => LispVal::Num(f.to_bits() as i64),
        LispVal::U64(u) => LispVal::Num(u as i64),
        LispVal::List(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        LispVal::Vec(xs) => LispVal::List(xs.into_iter().map(canon).collect()),
        other => other,
    }
}

/// Fuzz-mode wasm run: stub hosts, execute `run`, decode TEMP_MEM.
fn wasm_form_result(src: &str) -> Result<LispVal, String> {
    let wasm = lisp_rlm_wasm::wasm_emit::compile_fuzz(src).map_err(|e| format!("compile: {e}"))?;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).map_err(|e| format!("module: {e}"))?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);
    let fallback_memory = Memory::new(&mut store, wasmtime::MemoryType::new(4, None))
        .map_err(|e| format!("memory: {e}"))?;
    if module
        .imports()
        .any(|i| i.module() == "env" && i.name() == "memory")
    {
        linker
            .define(&store, "env", "memory", fallback_memory)
            .map_err(|e| format!("link memory: {e}"))?;
    }
    for import in module.imports() {
        if import.module() != "env" || import.name() == "memory" {
            continue;
        }
        let ExternType::Func(func_ty) = import.ty() else { continue };
        let params: Vec<ValType> = func_ty.params().collect();
        let results: Vec<ValType> = func_ty.results().collect();
        let ft = FuncType::new(&engine, params.clone(), results.clone());
        let stub = Func::new(&mut store, ft, move |_caller, _args, ret| {
            for i in 0..ret.len() {
                *(&mut ret[i]) = match results.get(i) {
                    Some(ValType::I32) => Val::I32(0),
                    _ => Val::I64(0),
                };
            }
            Ok(())
        });
        linker
            .define(&store, "env", import.name(), stub)
            .map_err(|e| format!("link {}: {e}", import.name()))?;
    }
    let instance: Instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("instantiate: {e}"))?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .unwrap_or(fallback_memory);
    let f = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .map_err(|e| format!("no run export: {e}"))?;
    f.call(&mut store, ()).map_err(|e| format!("trap: {e}"))?;
    const TEMP_MEM: usize = 64;
    let mem = memory.data(&store).to_vec();
    let raw = if TEMP_MEM + 8 <= mem.len() {
        i64::from_le_bytes(mem[TEMP_MEM..TEMP_MEM + 8].try_into().unwrap())
    } else {
        0
    };
    Ok(tv_to_lisp(&mem, decode(&mem, raw)))
}

fn form_to_string(v: &LispVal) -> String {
    // Debug is the dev-tool format: unambiguous about types
    // (Num(42) vs Str("42") is exactly the class of bug this tool hunts).
    format!("{:?}", v)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        usage();
    }
    let path = args[1].clone();
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lisp-diff: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    let exprs = match parser::parse_all(&source) {
        Ok(xs) => xs,
        Err(e) => {
            eprintln!("lisp-diff: parse error: {e}");
            std::process::exit(2);
        }
    };

    let code = std::thread::Builder::new()
        .name("lisp-diff".into())
        .stack_size(STACK)
        .spawn(move || run_diff(&exprs, &source))
        .expect("spawn diff thread");
    match code.join() {
        Ok(0) => {
            println!("IDENTICAL");
            std::process::exit(0);
        }
        Ok(1) => std::process::exit(1),
        Ok(_) => std::process::exit(2),
        Err(_) => {
            eprintln!("lisp-diff: internal panic");
            std::process::exit(2);
        }
    }
}

fn is_top_level_define(form: &LispVal) -> bool {
    if let LispVal::List(list) = form {
        if let Some(LispVal::Sym(head)) = list.first() {
            let h = head.as_str();
            return h == "define"
                || h == "defmacro"
                || h == "require"
                || h == "export"
                || h == "pure"
                || h == "memory"
                || h == "table"
                || h == "global";
        }
    }
    false
}

fn run_diff(exprs: &[LispVal], _source: &str) -> i32 {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let mut prefix_src = String::new();
    let mut compared = 0usize;
    let mut skipped = 0usize;

    for (i, form) in exprs.iter().enumerate() {
        state.eval_count = 0;
        let interp_result = run_program(&[form.clone()], &mut env, &mut state);

        let form_src = source_of(form);
        if is_top_level_define(form) {
            if let Err(e) = &interp_result {
                println!(
                    "FORM {i}: {form_src}\n  interp ERROR: {e}\n  (top-level define failed; stopping)",
                );
                return 1;
            }
            prefix_src.push_str(&form_src);
            prefix_src.push('\n');
            continue;
        }

        let wasm_src = format!("{prefix_src}(define (main) {form_src})\n");
        let wasm_result = wasm_form_result(&wasm_src);
        let interp_ok = interp_result.is_ok();
        let wasm_ok = wasm_result.is_ok();
        if !(interp_ok && wasm_ok) {
            // One side errored: if BOTH errored, treat as agreeing errors.
            if !interp_ok && !wasm_ok {
                skipped += 1;
                eprintln!("  [{}] both backends errored — counted as agreeing", i);
                prefix_src.push_str(&form_src);
                prefix_src.push('\n');
                continue;
            }
            let (side, msg) = if !interp_ok {
                ("interp", interp_result.err().unwrap())
            } else {
                ("wasm", wasm_result.err().unwrap())
            };
            println!(
                "FORM {i}: {form_src}\n  {side} ERROR: {msg}\n  (one-sided error = divergence)",
            );
            return 1;
        }
        let iv = interp_result.unwrap();
        let wv = wasm_result.unwrap();

        compared += 1;
        if canon(iv.clone()) != canon(wv.clone()) {
            println!("DIVERGENT at form {i} / {compared} compared:");
            println!("  form   : {}", form_src);
            println!("  interp : {}", form_to_string(&iv));
            println!("  wasm   : {}", form_to_string(&wv));
            return 1;
        }
        prefix_src.push_str(&form_src);
        prefix_src.push('\n');
    }

    eprintln!(
        "  compared {compared} form(s), {skipped} agreed-as-errors, {} top-level defines",
        exprs.len() - compared - skipped
    );
    0
}

/// Minimal sexp printer for forms (we do not keep source spans).
fn source_of(v: &LispVal) -> String {
    match v {
        LispVal::Sym(s) => s.clone(),
        LispVal::Str(s) => format!("{:?}", s),
        LispVal::Num(n) => n.to_string(),
        LispVal::Float(f) => format!("{}", f),
        LispVal::U64(u) => format!("{}u", u),
        LispVal::Bool(b) => format!("{}", b),
        LispVal::Nil => "nil".to_string(),
        LispVal::List(xs) => {
            let inner: Vec<String> = xs.iter().map(source_of).collect();
            format!("({})", inner.join(" "))
        }
        LispVal::Vec(xs) => {
            let inner: Vec<String> = xs.iter().map(source_of).collect();
            format!("[{}]", inner.join(" "))
        }
        LispVal::Map(m) => {
            let inner: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{} {}", k, source_of(v)))
                .collect();
            format!("{{{}}}", inner.join(" "))
        }
        other => format!("{:?}", other),
    }
}
