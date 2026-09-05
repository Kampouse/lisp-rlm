// deep-shadow: load EXACT test source from file, run, then bisect truncations
use lisp_rlm_wasm::compile_near;

fn run_src(src: &str, tag: &str) -> String {
    let wasm = match compile_near(src) {
        Ok(w) => w,
        Err(e) => return format!("COMPILE ERR: {e}"),
    };
    let p = format!("/tmp/nmdebug/bisect_{tag}.wasm");
    std::fs::write(&p, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&p)
        .arg("_run")
        .arg("{}")
        .env("NEAR_MOCK_STATE", format!("/tmp/nmdebug/bisect_state_{tag}.bin"))
        .output()
        .expect("run");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.starts_with("📄") || l.starts_with("❌"))
        .unwrap_or("?")
        .to_string()
}

fn main() {
    let full = std::fs::read_to_string("/tmp/nmdebug/deep.lisp").unwrap();
    println!("full (11): {}", run_src(&full, "full"));
    // Bisect: drop the OUTERMOST k let* wrappers, renaming keeps value = k+1
    for k in 0..=9 {
        let lines: Vec<&str> = full.lines().collect();
        // line 0 = (define (main)  ; lines 1..11 = let* wrappers ; body ; tail closes
        let mut s = String::from("(define (main)\n");
        for l in &lines[1 + k..12] {
            s.push_str(l);
            s.push('\n');
        }
        s.push_str(&format!("{}x\n", " ".repeat(11 - k + 1)));
        // tail: lines after body are the closing parens; keep the innermost (11-k) pairs
        for l in &lines[13..] {
            let trimmed = l.trim_end();
            s.push_str(trimmed);
            s.push('\n');
        }
        println!("drop outermost {k} (expect {}): {}", 12 + k, run_src(&s, &format!("k{k}")));
    }
}
