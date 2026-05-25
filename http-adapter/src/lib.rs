wit_bindgen::generate!({
    world: "adapter",
    generate_all,
});

use exports::lisp::http_adapter::api::Guest;

struct Component;

fn split_url(url: &str) -> Result<(String, String, String), String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("bad url: {}", url))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    Ok((
        scheme.to_string(),
        authority.to_string(),
        format!("/{}", path),
    ))
}

fn do_request(
    method: wasi::http::types::Method,
    url: String,
    req_body: Option<&[u8]>,
    content_type: Option<&str>,
) -> Result<Vec<u8>, String> {
    let headers = wasi::http::types::Fields::new();
    if let Some(ct) = content_type {
        headers
            .append(&"content-type".to_string(), ct.as_bytes())
            .map_err(|e| format!("header: {:?}", e))?;
    }

    let req = wasi::http::types::OutgoingRequest::new(headers);
    req.set_method(&method)
        .map_err(|e| format!("set_method: {:?}", e))?;

    let (scheme, authority, path) = split_url(&url)?;
    let scheme = match scheme.as_str() {
        "http" => wasi::http::types::Scheme::Http,
        _ => wasi::http::types::Scheme::Https,
    };
    req.set_scheme(Some(&scheme))
        .map_err(|e| format!("set_scheme: {:?}", e))?;
    req.set_authority(Some(&authority))
        .map_err(|e| format!("set_authority: {:?}", e))?;
    req.set_path_with_query(Some(&path))
        .map_err(|e| format!("set_path: {:?}", e))?;

    if let Some(data) = req_body {
        let out_body = req.body().map_err(|_| "body() failed".to_string())?;
        {
            let stream = out_body.write().map_err(|_| "write() failed".to_string())?;
            stream
                .blocking_write_and_flush(data)
                .map_err(|e| format!("stream write: {:?}", e))?;
        }
        wasi::http::types::OutgoingBody::finish(out_body, None)
            .map_err(|_| "finish failed".to_string())?;
    }

    let response =
        wasi::http::outgoing_handler::handle(req, None).map_err(|e| format!("handle: {:?}", e))?;

    response.subscribe().block();
    let resp = response
        .get()
        .ok_or_else(|| "no response yet".to_string())?
        .map_err(|_| "content-length error".to_string())?
        .map_err(|e| format!("request error: {:?}", e))?;

    let status: u16 = resp.status();
    let body = resp.consume().map_err(|_| "consume failed".to_string())?;
    let stream = body.stream().map_err(|_| "stream failed".to_string())?;

    let mut result = Vec::new();
    loop {
        match stream.blocking_read(u64::MAX) {
            Ok(chunk) => {
                if chunk.is_empty() {
                    break;
                }
                result.extend_from_slice(&chunk);
            }
            Err(_) => break,
        }
    }

    if status >= 400 {
        return Err(format!(
            "HTTP {}: {}",
            status,
            String::from_utf8_lossy(&result)
        ));
    }
    Ok(result)
}

impl Guest for Component {
    fn http_get(url: String) -> Result<Vec<u8>, String> {
        do_request(wasi::http::types::Method::Get, url, None, None)
    }

    fn http_post(url: String, body: Vec<u8>, content_type: String) -> Result<Vec<u8>, String> {
        do_request(
            wasi::http::types::Method::Post,
            url,
            Some(&body),
            Some(&content_type),
        )
    }
}

export!(Component);
