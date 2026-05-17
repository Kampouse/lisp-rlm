use std::io::{self, Read, Write};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).ok();
    
    let city = extract_city(&input).unwrap_or("Montreal");
    let url = format!("https://wttr.in/{}?format=j1", city);
    
    let raw = wasi_http_client::Client::new()
        .get(&url)
        .send()
        .ok()
        .and_then(|r| {
            if r.status() == 200 { r.body().ok() } else { None }
        })
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    
    // Parse temp_C from JSON: "temp_C": "22"
    let temp = extract_temp(&raw).unwrap_or(0);
    let warm = temp > 20;
    
    let output = format!("{{\"city\":\"{}\",\"temp_c\":{},\"warm\":{}}}", city, temp, warm);
    let _ = io::stdout().write(output.as_bytes());
}

fn extract_city(input: &str) -> Option<&str> {
    let key = "\"city\"";
    let start = input.find(key)?;
    let rest = &input[start + key.len()..];
    let colon = rest.find(':')?;
    let after = &rest[colon + 1..].trim_start();
    if after.starts_with('"') {
        let end = after[1..].find('"')?;
        Some(&after[1..end + 1])
    } else {
        None
    }
}

fn extract_temp(json: &str) -> Option<i32> {
    let key = "\"temp_C\"";
    let start = json.find(key)?;
    let rest = &json[start + key.len()..];
    let colon = rest.find(':')?;
    let after = &rest[colon + 1..].trim_start();
    // Skip opening quote
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    after[..end].parse().ok()
}
