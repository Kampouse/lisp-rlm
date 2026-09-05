// run the paren-balanced 11-deep shadow
use lisp_rlm_wasm::compile_near;

fn run_src(src: &str, tag: &str) -> String {
    let wasm = match compile_near(src) {
        Ok(w) => w,
        Err(e) => return format!("COMPILE ERR: {e}"),
    };
    let p = format!("/tmp/nmdebug/bal_{tag}.wasm");
    std::fs::write(&p, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&p)
        .arg("_run")
        .arg("{}")
        .env("NEAR_MOCK_STATE", format!("/tmp/nmdebug/bal_state_{tag}.bin"))
        .output()
        .expect("run");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.starts_with("📄") || l.starts_with("❌"))
        .unwrap_or("?")
        .to_string()
}

fn main() {
    // fixed full form
    let mut src = String::from("(define (main)\n");
    for i in 1..=11 {
        src.push_str(&format!("{}(let* ((x {}))\n", " ".repeat(i), i));
    }
    src.push_str(&format!("{}x{}\n", " ".repeat(12), ")".repeat(12)));
    println!("balanced full: {}", run_src(&src, "full"));
    // bisect: truncate outermost wrappers, value must equal innermost binding
    for k in 0..=10 {
        let depth = 11 - k;
        let mut s = String::from("(define (main)\n");
        for i in (k + 1)..=11 {
            s.push_str(&format!("{}(let* ((x {}))\n", " ".repeat(i - k), i));
        }
        s.push_str(&format!(
            "{}x{}\n",
            " ".repeat(depth + 1),
            ")".repeat(depth + 1)
        ));
        println!("depth {depth} (expect 11): {}", run_src(&s, &format!("d{depth}")));
    }
}
