use crate::types::LispVal;

// ---------------------------------------------------------------------------
// Tokenizer + Parser
// ---------------------------------------------------------------------------

fn tokenize(input: &str) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        if in_str {
            cur.push(ch);
            if ch == '"' {
                tokens.push(cur.clone());
                cur.clear();
                in_str = false;
            }
            i += 1;
        } else if ch == '"' && !in_str {
            in_str = true;
            cur.push(ch);
            i += 1;
        } else if ch == ';' && i + 1 < len && chars[i + 1] == ';' {
            // ;; line comment — skip to end of line
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
        } else if ch == '(' && i + 1 < len && chars[i + 1] == ';' {
            // (; block comment ;) — skip until matching ;)
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            i += 2;
            while i + 1 < len {
                if chars[i] == ';' && chars[i + 1] == ')' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if ch == '\'' {
            // Quote shorthand: 'x => (quote x)
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            tokens.push("#quote".to_string());
            i += 1;
        } else if ch == '#' && i + 1 < len && chars[i + 1] == '\\' {
            // Character literal: #\a, #\space, #\newline
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            i += 2; // skip #\
            if i < len {
                // Collect the character name
                let mut char_name = String::new();
                while i < len && !chars[i].is_whitespace() && chars[i] != '(' && chars[i] != ')' {
                    char_name.push(chars[i]);
                    i += 1;
                    // If first char after \ is a letter and next is not a letter, stop
                    if char_name.len() == 1 && (i >= len || !chars[i].is_ascii_alphabetic()) {
                        break;
                    }
                }
                tokens.push(format!("#char:{}", char_name));
            }
        } else if ch == '`' {
            // Quasiquote — tokenize as special token
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            tokens.push("#quasiquote".to_string());
            i += 1;
        } else if ch == ',' && i + 1 < len && chars[i + 1] == '@' {
            // Splicing unquote ,@
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            tokens.push("#unquote-splicing".to_string());
            i += 2;
        } else if ch == ',' {
            // Unquote ,
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            tokens.push("#unquote".to_string());
            i += 1;
        } else if ch == '(' || ch == ')' {
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            tokens.push(ch.to_string());
            i += 1;
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(cur.clone());
                cur.clear();
            }
            i += 1;
        } else {
            cur.push(ch);
            i += 1;
        }
    }

    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn parse(tokens: &[String], pos: &mut usize) -> Result<LispVal, String> {
    if *pos >= tokens.len() {
        return Err("unexpected EOF".into());
    }
    let tok = &tokens[*pos];
    *pos += 1;
    match tok.as_str() {
        "(" => {
            let mut list = Vec::new();
            while *pos < tokens.len() && tokens[*pos] != ")" {
                list.push(parse(tokens, pos)?);
            }
            if *pos >= tokens.len() {
                return Err("missing )".into());
            }
            *pos += 1;
            Ok(LispVal::List(list))
        }
        ")" => Err("unexpected )".into()),
        "#quote" => {
            // 'form => (quote form)
            let inner = parse(tokens, pos)?;
            Ok(LispVal::List(vec![LispVal::Sym("quote".into()), inner]))
        }
        t if t.starts_with("#char:") => {
            // Character literal: #char:a => (char a)
            let name = &t[6..];
            let ch = match name.to_lowercase().as_str() {
                "space" => ' ',
                "newline" => '\n',
                "tab" => '\t',
                "return" => '\r',
                "nul" | "null" => '\0',
                "delete" | "del" => '\u{7f}',
                "escape" => '\u{1b}',
                s => s.chars().next().unwrap_or('\0'),
            };
            Ok(LispVal::Str(ch.to_string()))
        }
        "#quasiquote" => {
            // (` form) => (quasiquote form)
            let inner = parse(tokens, pos)?;
            Ok(LispVal::List(vec![
                LispVal::Sym("quasiquote".into()),
                inner,
            ]))
        }
        "#unquote" => {
            // (, form) => (unquote form)
            let inner = parse(tokens, pos)?;
            Ok(LispVal::List(vec![LispVal::Sym("unquote".into()), inner]))
        }
        "#unquote-splicing" => {
            // (,@ form) => (unquote-splicing form)
            let inner = parse(tokens, pos)?;
            Ok(LispVal::List(vec![
                LispVal::Sym("unquote-splicing".into()),
                inner,
            ]))
        }
        "nil" => Ok(LispVal::Nil),
        "true" => Ok(LispVal::Bool(true)),
        "false" => Ok(LispVal::Bool(false)),
        s if s.starts_with('"') => {
            let inner = if s.len() >= 2 { &s[1..s.len() - 1] } else { "" };
            let processed = inner
                .replace("\\n", "\n")
                .replace("\\t", "\t")
                .replace("\\\\", "\\")
                .replace("\\\"", "\"");
            Ok(LispVal::Str(processed))
        }
        s => {
            // Special float literals
            if s == "+nan.0" || s == "+nan.0f" || s == "nan" {
                return Ok(LispVal::Float(f64::NAN));
            }
            if s == "+inf.0" || s == "+inf.0f" || s == "+inf" {
                return Ok(LispVal::Float(f64::INFINITY));
            }
            if s == "-inf.0" || s == "-inf.0f" || s == "-inf" {
                return Ok(LispVal::Float(f64::NEG_INFINITY));
            }
            // Fraction literal: 3/4 → 0.75
            if s.contains('/') && !s.starts_with('/') {
                let parts: Vec<&str> = s.split('/').collect();
                if parts.len() == 2 {
                    if let (Ok(num), Ok(den)) = (parts[0].parse::<f64>(), parts[1].parse::<f64>()) {
                        if den != 0.0 {
                            return Ok(LispVal::Float(num / den));
                        }
                    }
                }
            }
            if let Ok(n) = s.parse::<i64>() {
                Ok(LispVal::Num(n))
            } else if s.contains('.') {
                s.parse::<f64>()
                    .map(LispVal::Float)
                    .or_else(|_| Ok(LispVal::Sym(s.to_string())))
            } else {
                Ok(LispVal::Sym(s.to_string()))
            }
        }
    }
}

pub fn parse_all(input: &str) -> Result<Vec<LispVal>, String> {
    let tokens = tokenize(input);
    let mut pos = 0;
    let mut exprs = Vec::new();
    while pos < tokens.len() {
        exprs.push(parse(&tokens, &mut pos)?);
    }
    Ok(exprs)
}

// Stubs for span-aware API (used by error reporting)
pub struct Spanned<T> {
    pub val: T,
    pub line: usize,
    pub col: usize,
}

pub fn parse_all_spanned(_input: &str) -> Result<Vec<Spanned<crate::types::LispVal>>, String> {
    Err("parse_all_spanned not available (old parser)".into())
}
