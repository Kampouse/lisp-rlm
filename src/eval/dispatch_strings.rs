//! String builtins: str-concat, str-contains, str-length, str-substring, str-split,
//! str-split-exact, str-trim, str-index-of, str-upcase, str-downcase, str-starts-with,
//! str-ends-with, str=, str!=, str-chunk, str-join, to-string
//!
//! All indices and lengths are in **characters** (Unicode code points), not bytes.

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
            let parts: Vec<LispVal> = if delim.is_empty() {
                s.chars().map(|c| LispVal::Str(c.to_string())).collect()
            } else {
                s.split(&delim)
                    .filter(|p| !p.is_empty())
                    .map(|p| LispVal::Str(p.to_string()))
                    .collect()
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
            // Return character offset (not byte offset) for consistency with str-substring
            let idx = haystack
                .find(&needle)
                .map(|byte_pos| haystack[..byte_pos].chars().count() as i64)
                .unwrap_or(-1);
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
        "str-replace" => {
            let s = as_str(&args[0])?;
            let from = as_str(args.get(1).ok_or("str-replace: need pattern")?)?;
            let to = as_str(args.get(2).ok_or("str-replace: need replacement")?)?;
            Ok(Some(LispVal::Str(s.replace(&from.as_str(), &to.as_str()))))
        }
        "str=" => {
            let a = as_str(args.first().ok_or("str=: need 2 args")?)?;
            let b = as_str(args.get(1).ok_or("str=: need 2 args")?)?;
            Ok(Some(LispVal::Bool(a == b)))
        }
        "str!=" => {
            let a = as_str(args.first().ok_or("str!=: need 2 args")?)?;
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
            let sep = as_str(args.first().ok_or("str-join: need (separator list)")?)?;
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
        // -- Tier 1: String operations --
        "string->list" => match args.first() {
            Some(LispVal::Str(s)) => Ok(Some(LispVal::List(
                s.chars().map(|c| LispVal::Str(c.to_string())).collect(),
            ))),
            _ => Err("string->list: need string".into()),
        },
        "list->string" => match args.first() {
            Some(LispVal::List(l)) => {
                let s: String = l
                    .iter()
                    .map(|v| match v {
                        LispVal::Str(s) => s.clone(),
                        _ => v.to_string(),
                    })
                    .collect();
                Ok(Some(LispVal::Str(s)))
            }
            _ => Err("list->string: need list".into()),
        },
        "string<?" => {
            let a = match args.first() {
                Some(LispVal::Str(s)) => s,
                _ => return Err("string<?: need strings".into()),
            };
            let b = match args.get(1) {
                Some(LispVal::Str(s)) => s,
                _ => return Err("string<?: need strings".into()),
            };
            Ok(Some(LispVal::Bool(a < b)))
        }
        "string->number" => match args.first() {
            Some(LispVal::Str(s)) => {
                if let Ok(n) = s.parse::<i64>() {
                    Ok(Some(LispVal::Num(n)))
                } else if let Ok(f) = s.parse::<f64>() {
                    Ok(Some(LispVal::Float(f)))
                } else {
                    Ok(Some(LispVal::Bool(false)))
                }
            }
            _ => Err("string->number: need string".into()),
        },
        // R7RS string aliases
        "string-length" => handle("str-length", args),
        "string-append" => handle("str-concat", args),
        "substring" => handle("str-substring", args),
        "string-contains" => handle("str-contains", args),
        "string-upcase" => handle("str-upcase", args),
        "string-downcase" => handle("str-downcase", args),
        "string-copy" => handle("str-substring", args),
        "string-index" => handle("str-index-of", args),
        "string=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a == b } else { false }
        })).unwrap_or(false)))),
        "string<?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a < b } else { false }
        })).unwrap_or(false)))),
        "string>?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a > b } else { false }
        })).unwrap_or(false)))),
        "string<=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a <= b } else { false }
        })).unwrap_or(false)))),
        "string>=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a >= b } else { false }
        })).unwrap_or(false)))),
        "string-ci=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a.to_lowercase() == b.to_lowercase() } else { false }
        })).unwrap_or(false)))),
        "string-ci<?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a.to_lowercase() < b.to_lowercase() } else { false }
        })).unwrap_or(false)))),
        "string-ci>?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a.to_lowercase() > b.to_lowercase() } else { false }
        })).unwrap_or(false)))),
        "string-ci<=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a.to_lowercase() <= b.to_lowercase() } else { false }
        })).unwrap_or(false)))),
        "string-ci>=?" => Ok(Some(LispVal::Bool(args.get(0).and_then(|a| args.get(1).map(|b| {
            if let (LispVal::Str(a), LispVal::Str(b)) = (a, b) { a.to_lowercase() >= b.to_lowercase() } else { false }
        })).unwrap_or(false)))),
        "string-foldcase" => match args.first() {
            Some(LispVal::Str(s)) => Ok(Some(LispVal::Str(s.to_lowercase()))),
            _ => Err("string-foldcase: need string".into()),
        },
        // ── Character predicates (chars are strings in lisp-rlm) ──
        "char?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(true))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char=?" | "char<?" | "char>?" | "char<=?" | "char>=?" => {
            let a = match args.first() {
                Some(LispVal::Str(s)) if s.chars().count() == 1 => s.chars().next().unwrap(),
                _ => return Ok(Some(LispVal::Bool(false))),
            };
            let b = match args.get(1) {
                Some(LispVal::Str(s)) if s.chars().count() == 1 => s.chars().next().unwrap(),
                _ => return Ok(Some(LispVal::Bool(false))),
            };
            let eq = match name {
                "char=?" => a == b,
                "char<?" => a < b,
                "char>?" => a > b,
                "char<=?" => a <= b,
                "char>=?" => a >= b,
                _ => false,
            };
            Ok(Some(LispVal::Bool(eq)))
        }
        "char-ci=?" | "char-ci<?" | "char-ci>?" | "char-ci<=?" | "char-ci>=?" => {
            let a = match args.first() {
                Some(LispVal::Str(s)) if s.chars().count() == 1 => s.to_lowercase().chars().next().unwrap(),
                _ => return Ok(Some(LispVal::Bool(false))),
            };
            let b = match args.get(1) {
                Some(LispVal::Str(s)) if s.chars().count() == 1 => s.to_lowercase().chars().next().unwrap(),
                _ => return Ok(Some(LispVal::Bool(false))),
            };
            let eq = match name {
                "char-ci=?" => a == b,
                "char-ci<?" => a < b,
                "char-ci>?" => a > b,
                "char-ci<=?" => a <= b,
                "char-ci>=?" => a >= b,
                _ => false,
            };
            Ok(Some(LispVal::Bool(eq)))
        }
        "char-alphabetic?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(s.chars().next().unwrap().is_alphabetic()))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char-numeric?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(s.chars().next().unwrap().is_numeric()))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char-whitespace?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(s.chars().next().unwrap().is_whitespace()))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char-upper-case?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(s.chars().next().unwrap().is_uppercase()))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char-lower-case?" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => Ok(Some(LispVal::Bool(s.chars().next().unwrap().is_lowercase()))),
            _ => Ok(Some(LispVal::Bool(false))),
        },
        "char-upcase" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => {
                let c = s.chars().next().unwrap().to_uppercase().collect::<String>();
                Ok(Some(LispVal::Str(c)))
            }
            _ => Err("char-upcase: need char".into()),
        },
        "char-downcase" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => {
                let c = s.chars().next().unwrap().to_lowercase().collect::<String>();
                Ok(Some(LispVal::Str(c)))
            }
            _ => Err("char-downcase: need char".into()),
        },
        "char-foldcase" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => {
                let c = s.chars().next().unwrap().to_lowercase().collect::<String>();
                Ok(Some(LispVal::Str(c)))
            }
            _ => Err("char-foldcase: need char".into()),
        },
        "digit-value" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => {
                let c = s.chars().next().unwrap();
                Ok(Some(LispVal::Num(c.to_digit(10).map(|d| d as i64).unwrap_or(-1))))
            }
            _ => Err("digit-value: need char".into()),
        },
        "char->integer" => match args.first() {
            Some(LispVal::Str(s)) if s.chars().count() == 1 => {
                Ok(Some(LispVal::Num(s.chars().next().unwrap() as i64)))
            }
            _ => Err("char->integer: need char".into()),
        },
        "integer->char" => match args.first() {
            Some(LispVal::Num(n)) if *n >= 0 && *n <= 0x10ffff => {
                Ok(Some(LispVal::Str(char::from_u32(*n as u32).unwrap_or('\0').to_string())))
            }
            _ => Err("integer->char: need non-negative integer".into()),
        },
        "display" => handle("print", args),
        "newline" => { println!(); Ok(Some(LispVal::Nil)) }
        "write" => handle("inspect", args),
        _ => Ok(None),
    }
}
