fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_wat <file.wasm>");
    let wasm = std::fs::read(&path).expect("read");
    let wat = wasmprinter::print_bytes(&wasm).expect("wat");
    print!("{wat}");
}
