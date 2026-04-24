use crate::types::LispVal;

// ---------------------------------------------------------------------------
// Token with source location
// ---------------------------------------------------------------------------

/// A single token produced by the tokenizer, carrying its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    /// 1-based line number where the token starts.
    pub line: usize,
    /// 1-based column number where the token starts (character offset, not byte).
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
// Tokenizer
// ---------------------------------------------------------------------------

/// Tracks position in source and builds tokens with correct start locations.
///
/// The key invariant: `token_start` is `Some((line, col))` when `cur` is
/// non-empty, and `None` when `cur` is empty.  This is enforced by:
/// - `flush()` clears `cur` and sets `token_start = None`
/// - `begin_token()` sets `token_start = Some(current_pos)` when `cur` is empty
struct TokenBuilder {
    tokens: Vec<Token>,
    cur: String,
    line: usize,
    col: usize,
    /// `(line, col)` where the current accumulated token started.
    /// `None` iff `cur` is empty.
    token_start: Option<(usize, usize)>,
}

impl TokenBuilder {
    fn new() -> Self {
        TokenBuilder {
            tokens: Vec::new(),
            cur: String::new(),
            line: 1,
            col: 1,
            token_start: None,
        }
    }

    /// Mark the start of a new accumulated token at the current position.
    /// Only has effect if no token is currently being accumulated.
    fn begin_token(&mut self) {
        if self.token_start.is_none() {
            self.token_start = Some((self.line, self.col));
        }
    }

    /// Flush the accumulated text as a token (if any).
    fn flush(&mut self) {
        if !self.cur.is_empty() {
            let (line, col) = self
                .token_start
                .expect("token_start must be set when cur is non-empty");
            self.tokens.push(Token {
                text: std::mem::take(&mut self.cur),
                line,
                col,
            });
        }
        self.token_start = None;
    }

    /// Emit a single-character/symbol token immediately at the current position.
    fn emit(&mut self, text: &str) {
        self.flush();
        self.tokens.push(Token {
            text: text.to_string(),
            line: self.line,
            col: self.col,
        });
        // token_start stays None — next accumulation will re-set it.
    }

    fn advance(&mut self) {
        self.col += 1;
    }

    fn newline(&mut self) {
        self.line += 1;
        self.col = 1;
    }
}

fn tokenize(input: &str) -> Vec<Token> {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut b = TokenBuilder::new();
    let mut i = 0;
    let mut in_str = false;

    while i < len {
        let ch = chars[i];

        if in_str {
            if ch == '\n' {
                b.cur.push(ch);
                b.newline();
                i += 1;
                continue;
            }
            b.cur.push(ch);
            b.advance();
            if ch == '"' {
                b.flush();
                in_str = false;
            }
            i += 1;
        } else {
            match ch {
                '"' => {
                    in_str = true;
                    b.token_start = Some((b.line, b.col));
                    b.cur.push(ch);
                    b.advance();
                    i += 1;
                }
                ';' if i + 1 < len && chars[i + 1] == ';' => {
                    b.flush();
                    i += 2;
                    while i < len && chars[i] != '\n' {
                        i += 1;
                        b.advance();
                    }
                    if i < len {
                        b.newline();
                        i += 1;
                    }
                }
                '(' if i + 1 < len && chars[i + 1] == ';' => {
                    b.flush();
                    i += 2;
                    b.advance();
                    b.advance();
                    while i + 1 < len {
                        if chars[i] == '\n' {
                            b.newline();
                        } else {
                            b.advance();
                        }
                        if chars[i] == ';' && chars[i + 1] == ')' {
                            i += 2;
                            b.advance();
                            b.advance();
                            break;
                        }
                        i += 1;
                    }
                }
                '`' => {
                    b.emit("#quasiquote");
                    b.advance();
                    i += 1;
                }
                ',' if i + 1 < len && chars[i + 1] == '@' => {
                    b.emit("#unquote-splicing");
                    b.advance();
                    b.advance();
                    i += 2;
                }
                ',' => {
                    b.emit("#unquote");
                    b.advance();
                    i += 1;
                }
                '(' | ')' => {
                    b.emit(&ch.to_string());
                    b.advance();
                    i += 1;
                }
                '\n' => {
                    b.flush();
                    b.newline();
                    i += 1;
                }
                '\r' => {
                    b.flush();
                    b.advance();
                    i += 1;
                }
                _ if ch.is_whitespace() => {
                    b.flush();
                    b.advance();
                    i += 1;
                }
                _ => {
                    b.begin_token();
                    b.cur.push(ch);
                    b.advance();
                    i += 1;
                }
            }
        }
    }

    b.flush();
    b.tokens
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

fn parse(tokens: &[Token], pos: &mut usize) -> Result<Spanned<LispVal>, String> {
    if *pos >= tokens.len() {
        return Err("ERROR: unexpected EOF".into());
    }
    let tok = &tokens[*pos];
    let line = tok.line;
    let col = tok.col;
    *pos += 1;
    match tok.text.as_str() {
        "(" => {
            let mut list = Vec::new();
            while *pos < tokens.len() && tokens[*pos].text != ")" {
                list.push(parse(tokens, pos)?);
            }
            if *pos >= tokens.len() {
                return Err(format!("ERROR[{}:{}]: missing )", line, col));
            }
            *pos += 1;
            Ok(Spanned::new(
                LispVal::List(list.into_iter().map(|s| s.val).collect()),
                line,
                col,
            ))
        }
        ")" => Err(format!("ERROR[{}:{}]: unexpected )", line, col)),
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
        s if s.starts_with('"') => {
            let inner = if s.len() >= 2 {
                &s[1..s.len() - 1]
            } else {
                ""
            };
            let processed = inner
                .replace("\\n", "\n")
                .replace("\\t", "\t")
                .replace("\\\\", "\\")
                .replace("\\\"", "\"");
            Ok(Spanned::new(LispVal::Str(processed), line, col))
        }
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

    // --- Tokenizer position tracking ---

    #[test]
    fn test_token_positions_simple() {
        let tokens = tokenize("(+ 1 2)");
        assert_eq!(tokens.len(), 5);
        assert_eq!(
            tokens[0],
            Token {
                text: "(".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "+".into(),
                line: 1,
                col: 2
            }
        );
        assert_eq!(
            tokens[2],
            Token {
                text: "1".into(),
                line: 1,
                col: 4
            }
        );
        assert_eq!(
            tokens[3],
            Token {
                text: "2".into(),
                line: 1,
                col: 6
            }
        );
        assert_eq!(
            tokens[4],
            Token {
                text: ")".into(),
                line: 1,
                col: 7
            }
        );
    }

    #[test]
    fn test_token_positions_multiline() {
        let tokens = tokenize("foo\n  bar\nbaz");
        assert_eq!(tokens.len(), 3);
        assert_eq!(
            tokens[0],
            Token {
                text: "foo".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "bar".into(),
                line: 2,
                col: 3
            }
        );
        assert_eq!(
            tokens[2],
            Token {
                text: "baz".into(),
                line: 3,
                col: 1
            }
        );
    }

    #[test]
    fn test_string_then_identifier_columns() {
        let tokens = tokenize("\"hello\" world");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "\"hello\"".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "world".into(),
                line: 1,
                col: 9
            }
        );
    }

    #[test]
    fn test_string_with_newline() {
        // String spans two lines — token should record start position (line 1, col 1)
        let tokens = tokenize("\"line1\nline2\" x");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "\"line1\nline2\"".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "x".into(),
                line: 2,
                col: 8
            }
        );
    }

    #[test]
    fn test_quasiquote_column() {
        let tokens = tokenize("`a");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "#quasiquote".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "a".into(),
                line: 1,
                col: 2
            }
        );
    }

    #[test]
    fn test_unquote_splicing_column() {
        let tokens = tokenize(",@x");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "#unquote-splicing".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "x".into(),
                line: 1,
                col: 3
            }
        );
    }

    #[test]
    fn test_comment_then_token() {
        let tokens = tokenize(";; comment\n42");
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token {
                text: "42".into(),
                line: 2,
                col: 1
            }
        );
    }

    #[test]
    fn test_block_comment() {
        let tokens = tokenize("(; skip ;) 99");
        // (; skip ;) = 11 chars (col 1-11), space at col 12, "99" starts at col 13
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token {
                text: "99".into(),
                line: 1,
                col: 13
            }
        );
    }

    #[test]
    fn test_crlf_line_endings() {
        let tokens = tokenize("a\r\n  b");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "a".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "b".into(),
                line: 2,
                col: 3
            }
        );
    }

    #[test]
    fn test_trailing_token_after_string() {
        let tokens = tokenize("(\"hi\" x)");
        // ( "hi" x )  — 4 tokens
        // 1 2    7 8    (col positions)
        assert_eq!(tokens.len(), 4);
        assert_eq!(
            tokens[0],
            Token {
                text: "(".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "\"hi\"".into(),
                line: 1,
                col: 2
            }
        );
        assert_eq!(
            tokens[2],
            Token {
                text: "x".into(),
                line: 1,
                col: 7
            }
        );
        assert_eq!(
            tokens[3],
            Token {
                text: ")".into(),
                line: 1,
                col: 8
            }
        );
    }

    #[test]
    fn test_adjacent_parens() {
        let tokens = tokenize("()");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "(".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: ")".into(),
                line: 1,
                col: 2
            }
        );
    }

    #[test]
    fn test_multiple_spaces() {
        let tokens = tokenize("a   b");
        assert_eq!(tokens.len(), 2);
        assert_eq!(
            tokens[0],
            Token {
                text: "a".into(),
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1],
            Token {
                text: "b".into(),
                line: 1,
                col: 5
            }
        );
    }

    // --- Spanned parse results ---

    #[test]
    fn test_spanned_atom() {
        let result = parse_all_spanned("42").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], Spanned::new(LispVal::Num(42), 1, 1));
    }

    #[test]
    fn test_spanned_multiline() {
        let result = parse_all_spanned("(+ 1\n   2)").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line, 1);
        assert_eq!(result[0].col, 1);
    }

    #[test]
    fn test_spanned_second_expr() {
        let result = parse_all_spanned("42\n  hello").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], Spanned::new(LispVal::Num(42), 1, 1));
        assert_eq!(result[1], Spanned::new(LispVal::Sym("hello".into()), 2, 3));
    }

    // --- Backward compat ---

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

    // --- Parse errors with spans ---

    #[test]
    fn test_parse_error_has_span() {
        let err = parse_all("(+ 1").unwrap_err();
        assert!(
            err.contains("[1:"),
            "error should have line:col, got: {}",
            err
        );
    }

    #[test]
    fn test_unexpected_close_paren() {
        let err = parse_all(")").unwrap_err();
        assert!(
            err.contains("[1:1]"),
            "unexpected ) should report location, got: {}",
            err
        );
    }

    #[test]
    fn test_missing_close_paren_multiline() {
        let input = "42\n  (foo\nbar";
        let err = parse_all(input).unwrap_err();
        assert!(
            err.contains("[2:3]"),
            "missing ) should point to opening (, got: {}",
            err
        );
    }

    #[test]
    fn test_deeply_nested_error() {
        let err = parse_all("(a (b (c").unwrap_err();
        assert!(
            err.contains("[1:"),
            "deeply nested missing ) should report location, got: {}",
            err
        );
    }

    // --- Edge cases ---

    #[test]
    fn test_empty_input() {
        let result = parse_all("").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_only_whitespace() {
        let result = parse_all("   \n  \t  \n").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_float_parsing() {
        let result = parse_all("3.14").unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            LispVal::Float(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn test_negative_number() {
        let result = parse_all("-42").unwrap();
        assert_eq!(result[0], LispVal::Num(-42));
    }
}
