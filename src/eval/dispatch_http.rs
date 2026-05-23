//! HTTP builtins: http-get, http-post, http-get-json.

use super::json_to_lisp;
use super::llm_provider::{SHARED_CLIENT, SHARED_RUNTIME};
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
            let ct: String = args.get(2)
                .and_then(|v| as_str(v).ok())
                .unwrap_or_else(|| "application/json".to_string());
            let rt = &SHARED_RUNTIME;
            let body = rt.block_on(async {
                SHARED_CLIENT
                    .post(url)
                    .header("Content-Type", ct)
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
