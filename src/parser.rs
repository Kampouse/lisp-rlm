use crate::types::LispVal;

// ---------------------------------------------------------------------------
// Token with source location
// ---------------------------------------------------------------------------

/// A single token produced by the tokenizer, carrying its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (character offset within the line).
    pub col: usize,
}

/// A value annotated with its source-code location.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub val: T,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub col: usize,
}

impl<T> Spanned<T> {
    pub fn new(val: T, line: usize, col: usize) -> Self {
        Self { val, line, col }
    }

    /// Map the inner value while preserving the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            val: f(self.val),
            line: self.line,
            col: self.col,
        }
    }
}

impl<T: std::fmt::Display> std::fmt::Display for Spanned<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.val)
    }
}

// ---------------------------------------------------------------------------
// Tokenizer + Parser
// ---------------------------------------------------------------------------

fn tokenize(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut i = 0;
    let mut line = 1usize;
    let mut col = 1usize;
    let mut token_start_col = 1usize;

    while i < len {
        let ch = chars[i];

        if in_str {
            if ch == '\n' {
                // Newline inside a string literal
                cur.push(ch);
                line += 1;
                col = 1;
                i += 1;
                continue;
            }
            cur.push(ch);
            col += 1;
            if ch == '"' {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
                in_str = false;
            }
            i += 1;
        } else if ch == '"' {
            // Start of string literal (only reached when !in_str)
            in_str = true;
            cur.push(ch);
            token_start_col = col;
            col += 1;
            i += 1;
        } else if ch == ';' && i + 1 < len && chars[i + 1] == ';' {
            // ;; line comment — skip to end of line
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            i += 2;
            while i < len && chars[i] != '\n' {
                i += 1;
                col += 1;
            }
            if i < len {
                // consume the \n
                line += 1;
                col = 1;
                i += 1;
            }
        } else if ch == '(' && i + 1 < len && chars[i + 1] == ';' {
            // (; block comment ;) — skip until matching ;)
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            i += 2;
            col += 2;
            while i + 1 < len {
                if chars[i] == '\n' {
                    line += 1;
                    col = 1;
                } else {
                    col += 1;
                }
                if chars[i] == ';' && chars[i + 1] == ')' {
                    i += 2;
                    col += 2;
                    break;
                }
                i += 1;
            }
        } else if ch == '`' {
            // Quasiquote — tokenize as special token
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            tokens.push(Token {
                text: "#quasiquote".to_string(),
                line,
                col,
            });
            col += 1;
            i += 1;
        } else if ch == ',' && i + 1 < len && chars[i + 1] == '@' {
            // Splicing unquote ,@
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            tokens.push(Token {
                text: "#unquote-splicing".to_string(),
                line,
                col,
            });
            col += 2;
            i += 2;
        } else if ch == ',' {
            // Unquote ,
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            tokens.push(Token {
                text: "#unquote".to_string(),
                line,
                col,
            });
            col += 1;
            i += 1;
        } else if ch == '(' || ch == ')' {
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            tokens.push(Token {
                text: ch.to_string(),
                line,
                col,
            });
            col += 1;
            i += 1;
        } else if ch == '\n' {
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            line += 1;
            col = 1;
            i += 1;
        } else if ch == '\r' {
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            col += 1;
            i += 1;
        } else if ch.is_whitespace() {
            if !cur.is_empty() {
                tokens.push(Token {
                    text: cur.clone(),
                    line: line,
                    col: token_start_col,
                });
                cur.clear();
            }
            col += 1;
            i += 1;
        } else {
            if cur.is_empty() {
                token_start_col = col;
            }
            cur.push(ch);
            col += 1;
            i += 1;
        }
    }

    if !cur.is_empty() {
        tokens.push(Token {
            text: cur,
            line: line,
            col: token_start_col,
        });
    }
    tokens
}

fn parse(tokens: &[Token], pos: &mut usize) -> Result<Spanned<LispVal>, String> {
    if *pos >= tokens.len() {
        return Err("ERROR: unexpected EOF".into());
    }
    let tok = &tokens[*pos];
    let line = tok.line;
    let col = tok.col;
    let span_err = |msg: &str| -> String { format!("ERROR[{}:{}]: {}", line, col, msg) };
    *pos += 1;
    match tok.text.as_str() {
        "(" => {
            let mut list = Vec::new();
            let mut last_line = line;
            let mut last_col = col;
            while *pos < tokens.len() && tokens[*pos].text != ")" {
                last_line = tokens[*pos].line;
                last_col = tokens[*pos].col;
                list.push(parse(tokens, pos)?);
            }
            if *pos >= tokens.len() {
                return Err(format!("ERROR[{}:{}]: missing )", last_line, last_col));
            }
            *pos += 1;
            Ok(Spanned::new(
                LispVal::List(list.into_iter().map(|s| s.val).collect()),
                line,
                col,
            ))
        }
        ")" => Err(span_err("unexpected )")),
        "#quasiquote" => {
            let inner = parse(tokens, pos)?;
            Ok(Spanned::new(
                LispVal::List(vec![LispVal::Sym("quasiquote".into()), inner.val]),
                line,
                col,
            ))
        }
        "#unquote" => {
            let inner = parse(tokens, pos)?;
            Ok(Spanned::new(
                LispVal::List(vec![LispVal::Sym("unquote".into()), inner.val]),
                line,
                col,
            ))
        }
        "#unquote-splicing" => {
            let inner = parse(tokens, pos)?;
            Ok(Spanned::new(
                LispVal::List(vec![LispVal::Sym("unquote-splicing".into()), inner.val]),
                line,
                col,
            ))
        }
        "nil" => Ok(Spanned::new(LispVal::Nil, line, col)),
        "true" => Ok(Spanned::new(LispVal::Bool(true), line, col)),
        "false" => Ok(Spanned::new(LispVal::Bool(false), line, col)),
        s if s.starts_with('"') => Ok(Spanned::new(
            LispVal::Str(s[1..s.len() - 1].to_string()),
            line,
            col,
        )),
        s => {
            if let Ok(n) = s.parse::<i64>() {
                Ok(Spanned::new(LispVal::Num(n), line, col))
            } else if s.contains('.') {
                s.parse::<f64>()
                    .map(|f| Spanned::new(LispVal::Float(f), line, col))
                    .or_else(|_| Ok(Spanned::new(LispVal::Sym(s.to_string()), line, col)))
            } else {
                Ok(Spanned::new(LispVal::Sym(s.to_string()), line, col))
            }
        }
    }
}

/// Parse all expressions from a string, returning spanned values with source locations.
///
/// Each [`Spanned<LispVal>`] carries the 1-based `line` and `col` where the
/// *outermost* expression started in the source.  Child elements inside lists
/// are plain `LispVal` (their spans are stripped during construction).
///
/// This is sufficient for error reporting at the top-level expression granularity.
/// To preserve child spans, the `LispVal` type would need to carry `Spanned` children
/// recursively — a deeper refactor reserved for when spans are threaded through the
/// evaluator.
///
/// Parse errors include `[line:col]` in the message.
pub fn parse_all_spanned(input: &str) -> Result<Vec<Spanned<LispVal>>, String> {
    let tokens = tokenize(input);
    let mut pos = 0;
    let mut exprs = Vec::new();
    while pos < tokens.len() {
        exprs.push(parse(&tokens, &mut pos)?);
    }
    Ok(exprs)
}

/// Parse all expressions from a string (backward-compatible API without span info).
///
/// Internally uses [`parse_all_spanned`] and discards the span metadata.
pub fn parse_all(input: &str) -> Result<Vec<LispVal>, String> {
    Ok(parse_all_spanned(input)?
        .into_iter()
        .map(|s| s.val)
        .collect())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spanned_atom() {
        let result = parse_all_spanned("42").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 1);
        assert_eq!(result[0].col, 1);
        assert_eq!(result[0].val, LispVal::Num(42));
    }

    #[test]
    fn test_spanned_multiline() {
        let input = "(+ 1\n   2)";
        let result = parse_all_spanned(input).unwrap();
        assert_eq!(result.len(), 1);
        // The list starts at line 1, col 1
        assert_eq!(result[0].line, 1);
        assert_eq!(result[0].col, 1);
    }

    #[test]
    fn test_parse_error_has_span() {
        let result = parse_all("(+ 1");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("[1:"),
            "error should have line:col, got: {}",
            err
        );
    }

    #[test]
    fn test_unexpected_close_paren() {
        let result = parse_all(")");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("[1:1]"),
            "unexpected ) should report location, got: {}",
            err
        );
    }

    #[test]
    fn test_parse_all_backward_compat() {
        let result = parse_all("(+ 1 2) (* 3 4)").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            LispVal::List(vec![
                LispVal::Sym("+".into()),
                LispVal::Num(1),
                LispVal::Num(2),
            ])
        );
    }

    #[test]
    fn test_spanned_second_expr() {
        let input = "42\n  hello";
        let result = parse_all_spanned(input).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].line, 1);
        assert_eq!(result[1].line, 2);
        assert_eq!(result[1].col, 3);
    }

    #[test]
    fn test_comment_skipped() {
        let input = ";; comment\n42";
        let result = parse_all_spanned(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 2);
        assert_eq!(result[0].col, 1);
    }
}
