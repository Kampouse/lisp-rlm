/// CLI binary: run .lisp files through the lisp-rlm VM.
///
/// Usage:
///   cargo run --bin lisp-run <file.lisp>          Run file, print results
///   cargo run --bin lisp-run --eval "(+ 1 2)"     Eval single expression
///   cargo run --bin lisp-run --check <file>        Parse only (dry run)
///   cargo run --bin lisp-run --coverage <file>     per-form PASS/FAIL (no expected)
///   cargo run --bin lisp-run --verify <file>       Validate ;; => expected results
use std::env;
use std::fs;
use std::time::Instant;

use lisp_rlm_wasm::*;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("lisp-run: Run .lisp files through the lisp-rlm VM");
        eprintln!();
        eprintln!("Usage:");
        eprintln!("  lisp-run <file.lisp>            Run file");
        eprintln!("  lisp-run --eval \"expr\"          Eval expression");
        eprintln!("  lisp-run --check <file.lisp>    Compile only (dry run)");
        eprintln!("  lisp-run --coverage <file.lisp> Show per-form compile status");
        eprintln!("  lisp-run --verify <file.lisp>   Validate ;; => expected results");
        std::process::exit(1);
    }
    match args[1].as_str() {
        "--eval" => { if args.len() < 3 { eprintln!("--eval requires an expression"); std::process::exit(1); } eval_and_print(&args[2], false); }
        "--check" => { if args.len() < 3 { eprintln!("--check requires a file"); std::process::exit(1); } check_file(&args[2]); }
        "--coverage" => { if args.len() < 3 { eprintln!("--coverage requires a file"); std::process::exit(1); } coverage_file(&args[2]); }
        "--verify" => { if args.len() < 3 { eprintln!("--verify requires a file"); std::process::exit(1); } verify_file(&args[2]); }
        _ => { run_file(&args[1]); }
    }
}

// ── Helpers ─────────────────────────────────────────────────

fn strip_line_comments(code: &str) -> String {
    code.lines().map(|line| {
        if let Some(pos) = line.find(";;") {
            let before = &line[..pos];
            if before.matches('"').count() % 2 == 0 {
                return before.trim_end().to_string();
            }
        }
        line.to_string()
    }).collect::<Vec<_>>().join("\n")
}

/// Track paren depth across lines, yielding form boundaries.
/// Returns a vec of (start_line, end_line) for each top-level form.
fn find_form_spans(stripped: &str) -> Vec<(usize, usize)> {
    let lines: Vec<&str> = stripped.lines().collect();
    let mut spans = Vec::new();
    let mut depth = 0i32;
    let mut form_start = 0usize;
    let mut in_form = false;

    for (li, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with(";;;") { continue; }

        let mut saw_open = false;
        for ch in t.chars() {
            if ch == '(' { depth += 1; saw_open = true; }
            if ch == ')' { depth -= 1; }
        }

        if saw_open && !in_form {
            form_start = li;
            in_form = true;
        }

        if in_form && depth == 0 {
            spans.push((form_start, li));
            in_form = false;
        }

        // Atom forms: no parens at all, depth stayed 0, line is a bare value
        if !saw_open && !in_form && depth == 0 {
            // Must look like a Lisp atom/value
            let is_atom = t.starts_with('"')
                || t.starts_with('\'')
                || t.starts_with(':')
                || t.chars().next().map(|c| c.is_ascii_digit() || c == '-').unwrap_or(false)
                || t == "true" || t == "false" || t == "nil"
                || (!t.is_empty() && !t.starts_with('(') && !t.starts_with(';')
                    && !t.contains('(') && !t.contains(')')
                    // Symbol reference: a bare word like x, foo, my-var
                    && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '!' || c == '?' || c == '*'));
            if is_atom {
                spans.push((li, li));
            }
        }
    }
    spans
}

/// Walk ORIGINAL source, extract section headers for each form.
fn extract_sections(code: &str) -> Vec<String> {
    let stripped = strip_line_comments(code);
    let spans = find_form_spans(&stripped);
    let orig_lines: Vec<&str> = code.lines().collect();
    let mut cur = "(top)".to_string();
    let mut sections = Vec::new();

    // Build line->section map from original code
    let mut line_section: Vec<String> = Vec::new();
    for line in &orig_lines {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix(";;; ==") {
            let sec = rest.trim().trim_end_matches('=').trim();
            if !sec.is_empty() { cur = sec.to_string(); }
        }
        line_section.push(cur.clone());
    }

    for (start, _end) in &spans {
        sections.push(line_section.get(*start).cloned().unwrap_or_else(|| "(unknown)".to_string()));
    }
    sections
}

/// Walk ORIGINAL source, extract `;; => expected` for each form.
fn extract_expectations(code: &str) -> Vec<Option<String>> {
    let stripped = strip_line_comments(code);
    let spans = find_form_spans(&stripped);
    let orig_lines: Vec<&str> = code.lines().collect();

    // Build map: line_index -> expected value
    let mut line_exp: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    for (i, line) in orig_lines.iter().enumerate() {
        if let Some(pos) = line.find(";; =>") {
            let e = line[pos + 5..].trim();
            if !e.is_empty() { line_exp.insert(i, e.to_string()); }
        }
    }

    let mut expectations = Vec::new();
    for (start, end) in &spans {
        // Check if any line in [start..=end] has an expectation
        let exp = (*start..=*end).find_map(|l| line_exp.get(&l).cloned());
        expectations.push(exp);
    }
    expectations
}

fn display_val(v: &LispVal) -> String {
    match v {
        LispVal::Nil => "nil".into(),
        LispVal::Bool(b) => b.to_string(),
        LispVal::Num(n) => n.to_string(),
        LispVal::Float(f) => { if *f == f.floor() && f.abs() < 1e15 { format!("{:.1}", f) } else { format!("{}", f) } }
        LispVal::Str(s) => format!("\"{}\"", s),
        LispVal::Sym(s) => s.clone(),
        LispVal::List(items) => { let i: Vec<String> = items.iter().map(display_val).collect(); format!("({})", i.join(" ")) }
        LispVal::Map(m) => { let p: Vec<String> = m.iter().map(|(k, v)| format!("{} {}", k, display_val(v))).collect(); format!("(dict {})", p.join(" ")) }
        _ => format!("{:?}", v),
    }
}

fn truncate_display(v: &LispVal, max_len: usize) -> String { truncate_str(&format!("{:?}", v), max_len) }
fn truncate_str(s: &str, n: usize) -> String { if s.len() <= n { s.to_string() } else { format!("{}...", &s[..n]) } }

fn values_match(actual: &str, expected: &str) -> bool {
    if actual == expected { return true; }
    if let (Ok(a), Ok(e)) = (actual.parse::<f64>(), expected.parse::<f64>()) { return (a - e).abs() < 1e-10; }
    if actual.starts_with('(') && expected.starts_with('(') && actual.ends_with(')') && expected.ends_with(')') {
        let (a, e) = (parse_list_items(actual), parse_list_items(expected));
        if a.len() == e.len() { return a.iter().zip(e.iter()).all(|(a, e)| values_match(a, e)); }
    }
    false
}

fn parse_list_items(s: &str) -> Vec<String> {
    let inner = &s[1..s.len()-1];
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => { depth += 1; cur.push(ch); }
            ')' => { depth -= 1; cur.push(ch); }
            ' ' | '\t' if depth == 0 => { if !cur.is_empty() { items.push(cur.clone()); cur.clear(); } }
            _ => { cur.push(ch); }
        }
    }
    if !cur.is_empty() { items.push(cur); }
    items
}

// ── Commands ────────────────────────────────────────────────

fn eval_and_print(code: &str, verbose: bool) {
    let mut env = Env::new();
    let mut state = EvalState::new();
    let exprs = match parse_all(code) { Ok(e) => e, Err(e) => { eprintln!("PARSE ERROR: {}", e); std::process::exit(1); } };
    if verbose { eprintln!("Parsed {} top-level forms", exprs.len()); }
    let mut result = LispVal::Nil;
    let start = Instant::now();
    for (i, expr) in exprs.iter().enumerate() {
        if verbose { eprintln!("[{}] {:?}", i, truncate_display(expr, 80)); }
        match lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state) {
            Ok(v) => { if verbose { eprintln!("  => {:?}", truncate_display(&v, 60)); } result = v; }
            Err(e) => { eprintln!("ERROR at form {}: {}", i, e); std::process::exit(1); }
        }
    }
    println!("{}", display_val(&result));
    if verbose { eprintln!("Evaluated in {:?}", start.elapsed()); }
}

fn run_file(path: &str) {
    let code = fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("Cannot read {}: {}", path, e); std::process::exit(1); });
    eval_and_print(&strip_line_comments(&code), true);
}

fn check_file(path: &str) {
    let code = fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("Cannot read {}: {}", path, e); std::process::exit(1); });
    let exprs = match parse_all(&strip_line_comments(&code)) { Ok(e) => e, Err(e) => { eprintln!("PARSE ERROR: {}", e); std::process::exit(1); } };
    eprintln!("{} forms parsed OK", exprs.len());
    println!("OK");
}

fn coverage_file(path: &str) {
    let code = fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("Cannot read {}: {}", path, e); std::process::exit(1); });
    let stripped = strip_line_comments(&code);
    let exprs = match parse_all(&stripped) { Ok(e) => e, Err(e) => { eprintln!("PARSE ERROR: {}", e); std::process::exit(1); } };
    eprintln!("=== Syntax Coverage Report: {} ===", path);
    eprintln!("Total top-level forms: {}", exprs.len());
    eprintln!();
    let mut env = Env::new();
    let mut state = EvalState::new();
    let (mut ok, mut fail) = (0usize, 0usize);
    for (i, expr) in exprs.iter().enumerate() {
        let preview = truncate_display(expr, 100);
        match lisp_rlm_wasm::program::run_program(&[expr.clone()], &mut env, &mut state) {
            Ok(v) => { ok += 1; eprintln!("  [PASS] {:3}: {} => {}", i, preview, truncate_display(&v, 50)); }
            Err(e) => { fail += 1; eprintln!("  [FAIL] {:3}: {}", i, preview); eprintln!("          Error: {}", truncate_str(&e, 120)); }
        }
    }
    eprintln!();
    eprintln!("Results: {} passed, {} failed, {} total", ok, fail, exprs.len());
    eprintln!("Coverage: {:.1}%", ok as f64 / exprs.len() as f64 * 100.0);
    if fail > 0 { std::process::exit(1); }
}

fn verify_file(path: &str) {
    let code = fs::read_to_string(path).unwrap_or_else(|e| { eprintln!("Cannot read {}: {}", path, e); std::process::exit(1); });

    let expectations = extract_expectations(&code);
    let sections = extract_sections(&code);
    let expected_count = expectations.iter().filter(|e| e.is_some()).count();

    let stripped = strip_line_comments(&code);
    let exprs = match parse_all(&stripped) {
        Ok(e) => e,
        Err(e) => { eprintln!("PARSE ERROR: {}", e); std::process::exit(1); }
    };

    eprintln!("=== Verification: {} ===", path);
    eprintln!("{} forms parsed, {} with expectations", exprs.len(), expected_count);
    if expectations.len() != exprs.len() {
        eprintln!("WARNING: form tracker found {} forms, parser found {}", expectations.len(), exprs.len());
        eprintln!("  (verify will use min of both)");
    }
    eprintln!();

    let mut env = Env::new();
    let mut state = EvalState::new();
    let (mut pass, mut wrong, mut errors, mut skip) = (0usize, 0usize, 0usize, 0usize);
    let n = exprs.len().min(expectations.len());

    for i in 0..n {
        let section = sections.get(i).cloned().unwrap_or_else(|| "(unknown)".to_string());
        let preview = truncate_display(&exprs[i], 80);

        match lisp_rlm_wasm::program::run_program(&[exprs[i].clone()], &mut env, &mut state) {
            Ok(v) => {
                let actual = display_val(&v);
                if let Some(expected) = expectations.get(i).and_then(|e| e.as_ref()) {
                    if expected == "ERROR" {
                        wrong += 1;
                        eprintln!("  [WRONG] {:3} [{}]: {}", i, section, preview);
                        eprintln!("           Expected ERROR, got {}", truncate_str(&actual, 60));
                    } else if actual == *expected || values_match(&actual, expected) {
                        pass += 1;
                        eprintln!("  [PASS]  {:3} [{}]: {} => {}", i, section, truncate_str(&preview, 50), truncate_str(&actual, 50));
                    } else {
                        wrong += 1;
                        eprintln!("  [WRONG] {:3} [{}]: {}", i, section, truncate_str(&preview, 50));
                        eprintln!("           Expected: {}", expected);
                        eprintln!("           Actual:   {}", actual);
                    }
                } else {
                    skip += 1;
                    eprintln!("  [OK]    {:3} [{}]: {} => {}", i, section, truncate_str(&preview, 50), truncate_str(&actual, 50));
                }
            }
            Err(e) => {
                if let Some(expected) = expectations.get(i).and_then(|e| e.as_ref()) {
                    if expected == "ERROR" {
                        pass += 1;
                        eprintln!("  [PASS]  {:3} [{}]: {} => ERROR (expected)", i, section, truncate_str(&preview, 50));
                    } else {
                        errors += 1;
                        eprintln!("  [ERROR] {:3} [{}]: {}", i, section, truncate_str(&preview, 50));
                        eprintln!("           Error: {}", truncate_str(&e, 80));
                        eprintln!("           Expected: {}", expected);
                    }
                } else {
                    errors += 1;
                    eprintln!("  [ERROR] {:3} [{}]: {}", i, section, truncate_str(&preview, 50));
                    eprintln!("           Error: {}", truncate_str(&e, 80));
                }
            }
        }
    }

    let checked = pass + wrong + errors;
    eprintln!();
    eprintln!("=== Results ===");
    eprintln!("  PASS:   {}", pass);
    eprintln!("  WRONG:  {}", wrong);
    eprintln!("  ERROR:  {}", errors);
    eprintln!("  OK (unchecked): {}", skip);
    eprintln!("  Checked: {}/{}", checked, exprs.len());
    if checked > 0 { eprintln!("  Accuracy: {:.1}%", pass as f64 / checked as f64 * 100.0); }

    if wrong > 0 || errors > 0 { std::process::exit(1); }
}
