//! Shared helper functions for the eval module.

/// Truncate a string to `max_len` chars, appending "..." if truncated.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...[truncated]", &s[..max_len])
    }
}

/// Strip markdown code fences from LLM output.
pub fn strip_markdown_fences(s: &str) -> String {
    s.trim()
        .trim_start_matches("```lisp")
        .trim_start_matches("```scheme")
        .trim_start_matches("```clisp")
        .trim_start_matches("```common-lisp")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}

/// Try to extract the first valid Lisp expression from a string.
/// Used as fallback when parse_all fails on multi-expression code.
pub fn extract_first_valid_expr(code: &str) -> Option<crate::types::LispVal> {
    use crate::parser::parse_all;

    let chars: Vec<char> = code.chars().collect();
    let mut depth = 0i32;
    let mut start = None;

    for i in 0..chars.len() {
        match chars[i] {
            '(' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        let sub: String = chars[s..=i].iter().collect();
                        if let Ok(mut exprs) = parse_all(&sub) {
                            if !exprs.is_empty() {
                                return Some(exprs.remove(0));
                            }
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}
