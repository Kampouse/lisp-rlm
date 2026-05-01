//! Clojure-style parser: [] vectors, {} maps, #{} sets, :keywords, #(%) anon fn
//!
//! Produces CljVal which desugars into lisp_rlm_wasm::LispVal.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum CljVal {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Sym(String),
    Keyword(String),     // :foo
    List(Vec<CljVal>),   // (a b c) — evaluates as call
    Vec(Vec<CljVal>),    // [a b c] — literal data
    Map(Vec<(CljVal, CljVal)>), // {:a 1 :b 2}
    Set(Vec<CljVal>),    // #{1 2 3}
    AnonFn(Vec<CljVal>), // #(...) — anonymous fn
}

impl fmt::Display for CljVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CljVal::Nil => write!(f, "nil"),
            CljVal::Bool(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            CljVal::Num(n) => {
                if *n == (*n as i64) as f64 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            CljVal::Str(s) => write!(f, "\"{}\"", s),
            CljVal::Sym(s) => write!(f, "{}", s),
            CljVal::Keyword(k) => write!(f, ":{}", k),
            CljVal::List(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, " ")?; }
                    item.fmt(f)?;
                }
                write!(f, ")")
            }
            CljVal::Vec(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, " ")?; }
                    item.fmt(f)?;
                }
                write!(f, "]")
            }
            CljVal::Map(pairs) => {
                write!(f, "{{")?;
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 { write!(f, " ")?; }
                    k.fmt(f)?;
                    write!(f, " ")?;
                    v.fmt(f)?;
                }
                write!(f, "}}")
            }
            CljVal::Set(items) => {
                write!(f, "#{{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { write!(f, " ")?; }
                    item.fmt(f)?;
                }
                write!(f, "}}")
            }
            CljVal::AnonFn(body) => {
                write!(f, "#(")?;
                for (i, item) in body.iter().enumerate() {
                    if i > 0 { write!(f, " ")?; }
                    item.fmt(f)?;
                }
                write!(f, ")")
            }
        }
    }
}

pub struct CljParser {
    chars: Vec<char>,
    pos: usize,
}

impl CljParser {
    pub fn new(input: &str) -> Self {
        Self { chars: input.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        self.pos += 1;
        c
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => { self.advance(); }
                Some(';') => {
                    // Line comment
                    while let Some(c) = self.advance() {
                        if c == '\n' { break; }
                    }
                }
                _ => break,
            }
        }
    }

    fn read_val(&mut self) -> Result<CljVal, String> {
        self.skip_whitespace_and_comments();
        match self.peek() {
            None => Err("unexpected end of input".into()),
            Some(c) => match c {
                '(' => self.read_list(),
                '[' => self.read_vec(),
                '{' => self.read_map(),
                '"' => self.read_string(),
                ':' => self.read_keyword(),
                '#' => self.read_hash_dispatch(),
                '\'' => self.read_quote(),
                ')' | ']' | '}' => Err(format!("unexpected '{}'", c)),
                _ => self.read_atom(),
            },
        }
    }

    fn read_list(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume '('
        let mut items = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.peek() == Some(')') {
                self.advance();
                return Ok(CljVal::List(items));
            }
            items.push(self.read_val()?);
        }
    }

    fn read_vec(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume '['
        let mut items = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.peek() == Some(']') {
                self.advance();
                return Ok(CljVal::Vec(items));
            }
            items.push(self.read_val()?);
        }
    }

    fn read_map(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume '{'
        let mut pairs = Vec::new();
        loop {
            self.skip_whitespace_and_comments();
            if self.peek() == Some('}') {
                self.advance();
                return Ok(CljVal::Map(pairs));
            }
            let key = self.read_val()?;
            let val = self.read_val().map_err(|e| format!("map value after key: {}", e))?;
            pairs.push((key, val));
        }
    }

    fn read_string(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume opening "
        let mut s = String::new();
        loop {
            match self.advance() {
                Some('"') => return Ok(CljVal::Str(s)),
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(c) => s.push(c),
                    None => return Err("unterminated string escape".into()),
                },
                Some(c) => s.push(c),
                None => return Err("unterminated string".into()),
            }
        }
    }

    fn read_keyword(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume ':'
        let mut kw = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == ')' || c == ']' || c == '}' || c == ',' {
                break;
            }
            kw.push(c);
            self.advance();
        }
        if kw.is_empty() {
            return Err("empty keyword".into());
        }
        Ok(CljVal::Keyword(kw))
    }

    fn read_hash_dispatch(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume '#'
        match self.peek() {
            Some('(') => {
                // Anonymous fn #(...)
                let items = match self.read_list()? {
                    CljVal::List(items) => items,
                    _ => unreachable!(),
                };
                Ok(CljVal::AnonFn(items))
            }
            Some('{') => {
                // Set #{...}
                self.advance(); // consume '{'
                let mut items = Vec::new();
                loop {
                    self.skip_whitespace_and_comments();
                    if self.peek() == Some('}') {
                        self.advance();
                        return Ok(CljVal::Set(items));
                    }
                    items.push(self.read_val()?);
                }
            }
            _ => Err(format!("unsupported # dispatch: {:?}", self.peek())),
        }
    }

    fn read_quote(&mut self) -> Result<CljVal, String> {
        self.advance(); // consume '\''
        let val = self.read_val()?;
        Ok(CljVal::List(vec![CljVal::Sym("quote".into()), val]))
    }

    fn read_atom(&mut self) -> Result<CljVal, String> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == ')' || c == ']' || c == '}' || c == ',' {
                break;
            }
            s.push(c);
            self.advance();
        }
        if s.is_empty() {
            return Err("empty atom".into());
        }
        // Classify
        if s == "nil" { return Ok(CljVal::Nil); }
        if s == "true" { return Ok(CljVal::Bool(true)); }
        if s == "false" { return Ok(CljVal::Bool(false)); }
        // Try number
        if let Ok(n) = s.parse::<f64>() {
            return Ok(CljVal::Num(n));
        }
        Ok(CljVal::Sym(s))
    }

    pub fn parse_all(input: &str) -> Result<Vec<CljVal>, String> {
        let mut parser = CljParser::new(input);
        let mut results = Vec::new();
        loop {
            parser.skip_whitespace_and_comments();
            if parser.peek().is_none() { break; }
            results.push(parser.read_val()?);
        }
        Ok(results)
    }
}
