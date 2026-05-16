fn main() {
    let core = lisp_rlm_wasm::wasi_p2_echo::build_echo();
    println!("Echo core: {} bytes", core.len());
    std::fs::write("/tmp/echo_core.wasm", &core).unwrap();
    
    let wit = "/Users/asil/lisp-rlm/wit-native";
    println!("Embed + component build:");
    let s = format!(
        "wasm-tools component embed '{}' /tmp/echo_core.wasm --world http-client -o /tmp/echo_embed.wasm && \
         wasm-tools component new /tmp/echo_embed.wasm -o /tmp/echo.wasm",
        wit
    );
    println!("{}", s);
}
