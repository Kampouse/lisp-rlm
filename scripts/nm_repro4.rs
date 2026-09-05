// deep-shadow debug: 11 nested lets, same name
use lisp_rlm_wasm::compile_near;

fn run(src: &str, tag: &str) -> String {
    let wasm = compile_near(src).expect("compile failed");
    let p = format!("/tmp/nmdebug/ds_{tag}.wasm");
    std::fs::write(&p, &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg(&p)
        .arg("_run")
        .arg("{}")
        .env("NEAR_MOCK_STATE", format!("/tmp/nmdebug/ds_state_{tag}.bin"))
        .output()
        .expect("run");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.starts_with("📄") || l.starts_with("❌"))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn main() {
    let mk = |n: usize| {
        let mut s = String::from("(define (main)\n");
        for i in 1..=n {
            s.push_str(&format!("  {}(let* ((x {}))\n", " ".repeat(i), i));
        }
        s.push_str(&format!("  {}x\n", " ".repeat(n + 1)));
        for i in (1..=n).rev() {
            let _ = i;
            s.push_str(&format!("{}))\n", " ".repeat(n + 1)));
        }
        // simpler: close all with n parens
        s = String::from("(define (main)\n");
        for i in 1..=n {
            s.push_str(&format!("{}(let* ((x {})) ", " ".repeat(i), i));
        }
        s.push_str("x");
        for _ in 0..n {
            s.push(')');
        }
        s.push_str(")\n");
        s
    };
    for n in [2, 3, 5, 8, 9, 10, 11] {
        println!("depth {n}: {}", run(&mk(n), &format!("d{n}")));
    }
}
