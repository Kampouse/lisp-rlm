fn main() {
    let body = wasi_http_client::Client::new()
        .get("https://httpbin.org/get")
        .send()
        .unwrap()
        .body()
        .unwrap();

    // write to stdout via println (wasi:cli provides stdout)
    let s = std::str::from_utf8(&body).unwrap_or("not utf8");
    print!("{}", s);
}
