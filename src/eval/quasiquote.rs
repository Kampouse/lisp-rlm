//! Quasiquote expansion — transforms quasiquote forms into list-building code.

use crate::types::LispVal;

/// Expand a quasiquote form into code that constructs the equivalent list.
///
/// Handles:
/// - `(quasiquote (unquote x))` → `x` (evaluate)
/// - `(quasiquote (unquote-splicing x))` → splice x's elements into parent list
/// - `(quasiquote (a b c))` → `(list (expand a) (expand b) (expand c))`
/// - `(quasiquote sym)` → `(quote sym)`
pub fn expand_quasiquote(form: &LispVal) -> Result<LispVal, String> {
    match form {
        LispVal::List(items) => {
            // Check for (unquote x)
            if items.len() == 2 {
                if let LispVal::Sym(s) = &items[0] {
                    if s == "unquote" {
                        return Ok(items[1].clone());
                    }
                }
            }

            // Check if any element uses unquote-splicing
            let has_splice = items.iter().any(|item| {
                if let LispVal::List(splice_items) = item {
                    splice_items.len() == 2
                        && matches!(&splice_items[0], LispVal::Sym(s) if s == "unquote-splicing")
                } else {
                    false
                }
            });

            if has_splice {
                // Build (append seg1 seg2 ...) where each segment is either
                // (list expanded_elem ...) for non-splice elements
                // or the spliced expr directly for (unquote-splicing x)
                let mut segments: Vec<LispVal> = Vec::new();
                let mut current_list: Vec<LispVal> = vec![LispVal::Sym("list".to_string())];

                for item in items {
                    if let LispVal::List(splice_items) = item {
                        if splice_items.len() == 2 {
                            if let LispVal::Sym(s) = &splice_items[0] {
                                if s == "unquote-splicing" {
                                    // Flush current list segment
                                    if current_list.len() > 1 {
                                        segments.push(LispVal::List(current_list.clone()));
                                    }
                                    current_list = vec![LispVal::Sym("list".to_string())];
                                    // Add spliced expression directly
                                    segments.push(splice_items[1].clone());
                                    continue;
                                }
                            }
                        }
                    }
                    current_list.push(expand_quasiquote(item)?);
                }
                // Flush remaining items
                if current_list.len() > 1 {
                    segments.push(LispVal::List(current_list));
                }

                if segments.is_empty() {
                    Ok(LispVal::List(vec![LispVal::Sym("list".to_string())]))
                } else if segments.len() == 1 {
                    Ok(segments.into_iter().next().unwrap())
                } else {
                    let mut append_form = vec![LispVal::Sym("append".to_string())];
                    append_form.extend(segments);
                    Ok(LispVal::List(append_form))
                }
            } else {
                // No splicing — simple list construction
                let mut result_items = vec![LispVal::Sym("list".to_string())];
                for item in items {
                    result_items.push(expand_quasiquote(item)?);
                }
                Ok(LispVal::List(result_items))
            }
        }
        LispVal::Sym(_) => Ok(LispVal::List(vec![
            LispVal::Sym("quote".to_string()),
            form.clone(),
        ])),
        _ => Ok(form.clone()),
    }
}
