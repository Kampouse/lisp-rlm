//! Structured evaluation error with optional source location (span).
//!
//! Currently the rest of the crate still uses `Result<_, String>` for error
//! reporting.  `EvalError` is introduced here as the *target* type — its
//! [`Display`] impl produces `"ERROR[line:col]: message"` when span info is
//! attached, or plain `"ERROR: message"` otherwise.
//!
//! Two `From` impls allow painless conversion between `EvalError` and `String`
//! so existing code can adopt the new type incrementally.

use std::fmt;

// ---------------------------------------------------------------------------
// EvalError
// ---------------------------------------------------------------------------

/// A structured evaluation error carrying an optional source location.
///
/// # Display format
///
/// | Span present | Output                          |
/// |--------------|---------------------------------|
/// | yes          | `ERROR[line:col]: message`      |
/// | no           | `ERROR: message`                |
///
/// Optionally a `source` file/label can be set which prefixes the location:
/// `ERROR[source:line:col]: message`.
#[derive(Debug, Clone)]
pub struct EvalError {
    /// Human-readable error description.
    pub message: String,
    /// 1-based line number (if known).
    pub line: Option<usize>,
    /// 1-based column number (if known).
    pub col: Option<usize>,
    /// Source file or label (e.g. `"repl"`, `"test.lisp"`).
    pub source: Option<String>,
}

impl EvalError {
    /// Create a new error with just a message (no location info).
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            line: None,
            col: None,
            source: None,
        }
    }

    /// Attach a source-code location to the error (builder-style).
    pub fn with_span(mut self, line: usize, col: usize) -> Self {
        self.line = Some(line);
        self.col = Some(col);
        self
    }

    /// Attach a source label (builder-style).
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ERROR")?;
        if let Some(ref src) = self.source {
            write!(f, "[{}", src)?;
            if let (Some(line), Some(col)) = (self.line, self.col) {
                write!(f, ":{}:{}", line, col)?;
            }
            write!(f, "]")?;
        } else if let (Some(line), Some(col)) = (self.line, self.col) {
            write!(f, "[{}:{}]", line, col)?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for EvalError {}

// ---------------------------------------------------------------------------
// Conversions — keep the existing String-based error plumbing working.
// ---------------------------------------------------------------------------

/// Allow `EvalError` to be returned where `String` errors are expected.
impl From<EvalError> for String {
    fn from(err: EvalError) -> String {
        err.to_string()
    }
}

/// Allow bare `String` errors to be upgraded to `EvalError`.
impl From<String> for EvalError {
    fn from(msg: String) -> Self {
        EvalError::new(msg)
    }
}

/// Convenience helper that creates a formatted error **string**.
///
/// This is a drop-in replacement for the existing `Err(format!("…"))` pattern:
///
/// ```ignore
/// // Before
/// return Err(format!("not a function: {}", v));
///
/// // After (same type, but goes through EvalError's Display)
/// return Err(err(&format!("not a function: {}", v)));
/// ```
///
/// At the moment the output is simply `"ERROR: <msg>"`, but once spans are
/// threaded through the evaluator the message will include location info.
#[allow(dead_code)]
pub fn err(msg: &str) -> String {
    EvalError::new(msg).to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_no_span() {
        let e = EvalError::new("division by zero");
        assert_eq!(e.to_string(), "ERROR: division by zero");
    }

    #[test]
    fn display_with_span() {
        let e = EvalError::new("division by zero").with_span(5, 12);
        assert_eq!(e.to_string(), "ERROR[5:12]: division by zero");
    }

    #[test]
    fn display_with_source_and_span() {
        let e = EvalError::new("division by zero")
            .with_source("test.lisp")
            .with_span(5, 12);
        assert_eq!(e.to_string(), "ERROR[test.lisp:5:12]: division by zero");
    }

    #[test]
    fn display_with_source_no_span() {
        let e = EvalError::new("division by zero").with_source("repl");
        assert_eq!(e.to_string(), "ERROR[repl]: division by zero");
    }

    #[test]
    fn err_helper() {
        assert_eq!(err("oops"), "ERROR: oops");
    }

    #[test]
    fn from_eval_error_to_string() {
        let e = EvalError::new("bad").with_span(1, 1);
        let s: String = e.into();
        assert_eq!(s, "ERROR[1:1]: bad");
    }

    #[test]
    fn from_string_to_eval_error() {
        let e: EvalError = "something broke".to_string().into();
        assert_eq!(e.message, "something broke");
        assert!(e.line.is_none());
    }
}
