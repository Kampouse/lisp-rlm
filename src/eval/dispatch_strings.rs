//! String builtins: str-concat, str-contains, str-length, str-substring, str-split,
//! str-split-exact, str-trim, str-index-of, str-upcase, str-downcase, str-starts-with,
//! str-ends-with, str=, str!=, str-chunk, str-join, to-string

use crate::helpers::*;
use crate::types::LispVal;

pub fn handle(name: &str, args: &[LispVal]) -> Result<Option<LispVal>, String> {
    match name {
        "str-concat" => {
            let parts: Vec<String> = args
                .iter()
                .map(|a| match a {
                    LispVal::Str(s) => s.clone(),
                    _ => a.to_string(),
                })
                .collect();
            Ok(Some(LispVal::Str(parts.join(""))))
        }
        "str-contains" => Ok(Some(LispVal::Bool(
            as_str(&args[0])?.contains(&as_str(&args[1])?),
        ))),
        "to-string" => Ok(Some(LispVal::Str(args[0].to_string()))),
        "str-length" => {
            let s = as_str(&args[0])?;
            Ok(Some(LispVal::Num(s.chars().count() as i64)))
        }
        "str-substring" => {
            let s = as_str(&args[0])?;
            let start = as_num(args.get(1).ok_or("str-substring: need start")?)? as usize;
            let end = as_num(args.get(2).ok_or("str-substring: need end")?)? as usize;
            let chars: Vec<char> = s.chars().collect();
            if start > end || end > chars.len() {
                return Err(format!(
                    "str-substring: indices out of range ({}..{} for len {})",
                    start,
                    end,
                    chars.len()
                ));
            }
            Ok(Some(LispVal::Str(chars[start..end].iter().collect())))
        }
        "str-split" => {
            let s = as_str(&args[0])?;
            let delim = as_str(args.get(1).ok_or("str-split: need delimiter")?)?;
            let parts: Vec<LispVal> = if delim.len() == 1 {
                s.split(delim.chars().next().unwrap())
                    .filter(|p| !p.is_empty())
                    .map(|p| LispVal::Str(p.to_string()))
                    .collect()
            } else {
                let char_set: Vec<char> = delim.chars().collect();
                let mut parts = Vec::new();
                let mut current = String::new();
                for ch in s.chars() {
                    if char_set.contains(&ch) {
                        if !current.is_empty() {
                            parts.push(LispVal::Str(std::mem::take(&mut current)));
                        }
                    } else {
                        current.push(ch);
                    }
                }
                if !current.is_empty() {
                    parts.push(LispVal::Str(current));
                }
                parts
            };
            Ok(Some(LispVal::List(parts)))
        }
        "str-split-exact" => {
            let s = as_str(&args[0])?;
            let delim = as_str(args.get(1).ok_or("str-split-exact: need delimiter")?)?;
            let parts: Vec<LispVal> = s
                .split(&delim)
                .map(|p| LispVal::Str(p.to_string()))
                .collect();
            Ok(Some(LispVal::List(parts)))
        }
        "str-trim" => {
            let s = as_str(&args[0])?;
            Ok(Some(LispVal::Str(s.trim().to_string())))
        }
        "str-index-of" => {
            let haystack = as_str(&args[0])?;
            let needle = as_str(args.get(1).ok_or("str-index-of: need needle")?)?;
            let idx = haystack.find(&needle).map(|i| i as i64).unwrap_or(-1);
            Ok(Some(LispVal::Num(idx)))
        }
        "str-upcase" => Ok(Some(LispVal::Str(as_str(&args[0])?.to_uppercase()))),
        "str-downcase" => Ok(Some(LispVal::Str(as_str(&args[0])?.to_lowercase()))),
        "str-starts-with" => {
            let s = as_str(&args[0])?;
            let prefix = as_str(args.get(1).ok_or("str-starts-with: need prefix")?)?;
            Ok(Some(LispVal::Bool(s.starts_with(&prefix))))
        }
        "str-ends-with" => {
            let s = as_str(&args[0])?;
            let suffix = as_str(args.get(1).ok_or("str-ends-with: need suffix")?)?;
            Ok(Some(LispVal::Bool(s.ends_with(&suffix))))
        }
        "str=" => {
            let a = as_str(args.get(0).ok_or("str=: need 2 args")?)?;
            let b = as_str(args.get(1).ok_or("str=: need 2 args")?)?;
            Ok(Some(LispVal::Bool(a == b)))
        }
        "str!=" => {
            let a = as_str(args.get(0).ok_or("str!=: need 2 args")?)?;
            let b = as_str(args.get(1).ok_or("str!=: need 2 args")?)?;
            Ok(Some(LispVal::Bool(a != b)))
        }
        "str-chunk" => {
            let s = as_str(&args[0])?;
            let n = as_num(args.get(1).ok_or("str-chunk: need n")?)? as usize;
            if n == 0 {
                return Err("str-chunk: n must be > 0".into());
            }
            let chars: Vec<char> = s.chars().collect();
            let total = chars.len();
            let chunk_size = (total + n - 1) / n; // ceil division
            if chunk_size == 0 {
                return Ok(Some(LispVal::List(vec![
                    LispVal::Str(String::new());
                    n.min(total + 1)
                ])));
            }
            let mut chunks: Vec<LispVal> = Vec::new();
            let mut i = 0;
            while i < total {
                let end = (i + chunk_size).min(total);
                let chunk: String = chars[i..end].iter().collect();
                chunks.push(LispVal::Str(chunk));
                i += chunk_size;
            }
            Ok(Some(LispVal::List(chunks)))
        }
        "str-join" => {
            let sep = as_str(args.get(0).ok_or("str-join: need (separator list)")?)?;
            let lst = match args.get(1) {
                Some(LispVal::List(l)) => l,
                Some(LispVal::Nil) => return Ok(Some(LispVal::Str(String::new()))),
                Some(other) => return Err(format!("str-join: expected list, got {}", other)),
                None => return Err("str-join: need (separator list)".into()),
            };
            let parts: Vec<String> = lst
                .iter()
                .map(|v| match v {
                    LispVal::Str(s) => s.clone(),
                    _ => v.to_string(),
                })
                .collect();
            Ok(Some(LispVal::Str(parts.join(&sep))))
        }
        _ => Ok(None),
    }
}
