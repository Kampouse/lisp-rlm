//! Shared helper functions for the eval module.

/// Truncate a string to `max_len` characters, appending "..." if truncated.
///
/// Correctly handles multi-byte UTF-8: truncation is at character boundaries,
/// never mid-codepoint.
pub fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}...[truncated]", truncated)
    }
}

/// Strip markdown code fences from LLM output.
/// Strip ALL markdown code fences from LLM output, handling:
/// - Single fenced blocks: ```lisp ... ```
/// - Multiple fenced blocks: ```lisp ... ``` ... ```lisp ... ```
/// - Nested/broken fences
/// - Returns all code content concatenated
pub fn strip_markdown_fences(s: &str) -> String {
    let mut result = Vec::new();
    let mut in_fence = false;
    let mut buf = String::new();

    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_fence {
                // Closing fence — flush buffer
                in_fence = false;
                if !buf.trim().is_empty() {
                    result.push(buf.trim().to_string());
                    buf.clear();
                }
            } else {
                // Opening fence — start collecting
                in_fence = true;
                // If there's content on the same line after ```, capture it
                let after = trimmed
                    .trim_start_matches("```lisp")
                    .trim_start_matches("```scheme")
                    .trim_start_matches("```clisp")
                    .trim_start_matches("```common-lisp")
                    .trim_start_matches("```")
                    .trim();
                if !after.is_empty() {
                    buf.push_str(after);
                    buf.push('\n');
                }
            }
            continue;
        }
        if in_fence {
            buf.push_str(line);
            buf.push('\n');
        } else {
            // Outside fence — check if line looks like Lisp (starts with ( or ;)
            let maybe_code =
                trimmed.starts_with('(') || trimmed.starts_with(';') || trimmed.starts_with('"');
            if maybe_code {
                result.push(trimmed.to_string());
            }
        }
    }

    // Flush any remaining buffer (unclosed fence)
    if !buf.trim().is_empty() {
        result.push(buf.trim().to_string());
    }

    result.join("\n")
}

/// Try to extract the first valid Lisp expression from a string.
/// Used as fallback when parse_all fails on multi-expression code.
#[allow(dead_code)]
pub fn extract_first_valid_expr(code: &str) -> Option<lisp_core::types::LispVal> {
    use lisp_core::parser::parse_all;

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
