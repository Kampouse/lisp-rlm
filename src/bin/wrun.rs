use wasmtime::*;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let wasm = std::fs::read(&path)?;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm)?;
    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    linker.func_wrap("env", "read_register", |_: Caller<'_, ()>, _: i64, _: i64| {})?;
    linker.func_wrap("env", "register_len", |_: i64| -> i64 { 0 })?;
    linker.func_wrap("env", "input", |_: Caller<'_, ()>, _: i64| {})?;
    linker.func_wrap("env", "value_return", |_: Caller<'_, ()>, _: i64, _: i64| {})?;
    // any other env imports the module needs get zero-arity stubs by name
    for imp in module.imports() {
        if imp.module() == "env"
            && !matches!(imp.name(), "read_register" | "register_len" | "input" | "value_return")
        {
            let name = imp.name().to_string();
            let f = Func::new(
                &mut store,
                FuncType::new(&engine, vec![], vec![]),
                |_, _, _| Ok(()),
            );
            let _ = linker.define(&mut store, "env", &name, f);
        }
    }

    let inst = linker.instantiate(&mut store, &module)?;
    let run = inst.get_typed_func::<(), ()>(&mut store, "run")?;
    run.call(&mut store, ())?;
    let mem = inst.get_memory(&mut store, "memory").unwrap();
    let mut rb = [0u8; 8];
    mem.read(&mut store, 64, &mut rb)?;
    let r = i64::from_le_bytes(rb);
    let tag = r & 7;
    let payload = r >> 3;
    if tag == 5 {
        // string: payload = len<<32 | ptr — print via memory read
        let mem = inst.get_memory(&mut store, "memory").unwrap();
        let ptr = (payload & 0xFFFF_FFFF) as usize;
        let len = ((payload as u64) >> 32) as usize;
        let mut buf = vec![0u8; len];
        mem.read(&mut store, ptr, &mut buf)?;
        println!("str: {}", String::from_utf8_lossy(&buf));
    } else if tag == 1 {
        println!("num: {}", payload as i64);
    } else {
        println!("raw={} tag={} payload={}", r, tag, payload);
    }
    Ok(())
}
