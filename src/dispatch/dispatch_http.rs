//! HTTP builtins: http-get, http-post, http-get-json.

use crate::dispatch::json_to_lisp;
use crate::dispatch::llm_provider::{SHARED_CLIENT, SHARED_RUNTIME};
use crate::helpers::*;
use crate::types::LispVal;

pub fn handle(name: &str, args: &[LispVal]) -> Result<Option<LispVal>, String> {
    match name {
        "http-get" => {
            let url = as_str(&args[0])?;
            let rt = &SHARED_RUNTIME;
            let body = rt.block_on(async {
                SHARED_CLIENT
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| format!("http-get: {}", e))?
                    .text()
                    .await
                    .map_err(|e| format!("http-get: {}", e))
            })?;
            Ok(Some(LispVal::Str(body)))
        }
        "http-post" => {
            let url = as_str(&args[0])?;
            let body_str = as_str(args.get(1).ok_or("http-post: need body")?)?;
            let rt = &SHARED_RUNTIME;
            let body = rt.block_on(async {
                SHARED_CLIENT
                    .post(url)
                    .header("Content-Type", "application/json")
                    .body(body_str)
                    .send()
                    .await
                    .map_err(|e| format!("http-post: {}", e))?
                    .text()
                    .await
                    .map_err(|e| format!("http-post: {}", e))
            })?;
            Ok(Some(LispVal::Str(body)))
        }
        "http-get-json" => {
            let url = as_str(&args[0])?;
            let rt = &SHARED_RUNTIME;
            let body = rt.block_on(async {
                SHARED_CLIENT
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| format!("http-get-json: {}", e))?
                    .text()
                    .await
                    .map_err(|e| format!("http-get-json: {}", e))
            })?;
            let v: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| format!("http-get-json: parse error: {}", e))?;
            Ok(Some(json_to_lisp(v)))
        }
        _ => Ok(None),
    }
}
