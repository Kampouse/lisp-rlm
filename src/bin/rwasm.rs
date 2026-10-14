// Bench host: run an exported (i64)->i64 wasm fn N times through wasmtime.
// Usage: rwasm <file.wasm> <func> <arg> <repeats>
use wasmtime::*;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (path, func, arg, reps): (&str, &str, i64, u32) = (
        &a[1], &a[2], a[3].parse()?, a[4].parse()?,
    );
    let wasm = std::fs::read(path)?;
    let engine = Engine::default();
    let module = Module::new(&engine, &wasm)?;
    let mut store = Store::new(&engine, ());
    let inst = Linker::new(&engine).instantiate(&mut store, &module)?;
    let f = inst.get_typed_func::<i64, i64>(&mut store, func)?;
    // warm (JIT)
    let _ = f.call(&mut store, arg)?;
    let t = Instant::now();
    let mut acc = 0i64;
    for _ in 0..reps { acc = acc.wrapping_add(f.call(&mut store, arg)?); }
    println!("{} x{} arg={} acc={} in {:?}", func, reps, arg, acc, t.elapsed());
    Ok(())
}
