//! JSON and Dict builtins.

use std::collections::BTreeMap;

use super::{json_to_lisp, lisp_to_json};
use crate::helpers::*;
use crate::types::LispVal;

pub fn handle(name: &str, args: &[LispVal]) -> Result<Option<LispVal>, String> {
    match name {
        // --- Dict / Map builtins ---
        "dict" => {
            let mut m = BTreeMap::new();
            let mut i = 0;
            while i + 1 < args.len() {
                let key = as_str(&args[i]).map_err(|_| "dict: keys must be strings")?;
                m.insert(key, args[i + 1].clone());
                i += 2;
            }
            Ok(Some(LispVal::Map(m)))
        }
        "dict/get" => {
            let m = match &args[0] {
                LispVal::Map(m) => m,
                _ => return Err("dict/get: expected map".into()),
            };
            let key = as_str(&args[1]).map_err(|_| "dict/get: key must be string")?;
            Ok(Some(m.get(&key).cloned().unwrap_or(LispVal::Nil)))
        }
        "dict/set" => {
            let mut m = match &args[0] {
                LispVal::Map(m) => m.clone(),
                _ => return Err("dict/set: expected map".into()),
            };
            let key = as_str(&args[1]).map_err(|_| "dict/set: key must be string")?;
            m.insert(key, args.get(2).cloned().unwrap_or(LispVal::Nil));
            Ok(Some(LispVal::Map(m)))
        }
        "dict/has?" => {
            let m = match &args[0] {
                LispVal::Map(m) => m,
                _ => return Err("dict/has?: expected map".into()),
            };
            let key = as_str(&args[1]).map_err(|_| "dict/has?: key must be string")?;
            Ok(Some(LispVal::Bool(m.contains_key(&key))))
        }
        "dict/keys" => {
            let m = match &args[0] {
                LispVal::Map(m) => m,
                _ => return Err("dict/keys: expected map".into()),
            };
            Ok(Some(LispVal::List(
                m.keys().map(|k| LispVal::Str(k.clone())).collect(),
            )))
        }
        "dict/vals" => {
            let m = match &args[0] {
                LispVal::Map(m) => m,
                _ => return Err("dict/vals: expected map".into()),
            };
            Ok(Some(LispVal::List(m.values().cloned().collect())))
        }
        "dict/remove" => {
            let mut m = match &args[0] {
                LispVal::Map(m) => m.clone(),
                _ => return Err("dict/remove: expected map".into()),
            };
            let key = as_str(&args[1]).map_err(|_| "dict/remove: key must be string")?;
            m.remove(&key);
            Ok(Some(LispVal::Map(m)))
        }
        "dict/merge" => {
            let mut m = match &args[0] {
                LispVal::Map(m) => m.clone(),
                _ => return Err("dict/merge: first arg must be map".into()),
            };
            match &args[1] {
                LispVal::Map(m2) => {
                    for (k, v) in m2 {
                        m.insert(k.clone(), v.clone());
                    }
                }
                _ => return Err("dict/merge: second arg must be map".into()),
            }
            Ok(Some(LispVal::Map(m)))
        }

        // --- JSON ---
        "json-parse" | "from-json" => {
            let s = as_str(&args[0])?;
            match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(v) => Ok(Some(json_to_lisp(v))),
                Err(e) => Err(format!("json-parse: {}", e)),
            }
        }
        "json-get" => {
            let s = as_str(&args[0])?;
            let key = as_str(&args[1])?;
            let v: serde_json::Value =
                serde_json::from_str(&s).map_err(|e| format!("json-get: parse error: {}", e))?;
            match v.get(&key) {
                Some(val) => Ok(Some(json_to_lisp(val.clone()))),
                None => Ok(Some(LispVal::Nil)),
            }
        }
        "json-get-in" => {
            let s = as_str(&args[0])?;
            let v: serde_json::Value =
                serde_json::from_str(&s).map_err(|e| format!("json-get-in: parse error: {}", e))?;
            let mut cur = &v;
            for arg in &args[1..] {
                let key = as_str(arg)?;
                cur = cur.get(&key).unwrap_or(&serde_json::Value::Null);
            }
            Ok(Some(json_to_lisp(cur.clone())))
        }
        "json-build" => {
            let j = lisp_to_json(&args[0]);
            Ok(Some(LispVal::Str(j.to_string())))
        }
        "to-json" => {
            let json_val = lisp_to_json(&args[0]);
            serde_json::to_string(&json_val)
                .map(LispVal::Str)
                .map(Some)
                .map_err(|e| format!("to-json: {}", e))
        }

        _ => Ok(None),
    }
}
