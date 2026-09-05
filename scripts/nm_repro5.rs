// exact test source for nm_let_deep_shadow
use lisp_rlm_wasm::compile_near;

fn main() {
    let src = r#"(define (main)
  (let* ((x 1))
    (let* ((x 2))
      (let* ((x 3))
        (let* ((x 4))
          (let* ((x 5))
            (let* ((x 6))
              (let* ((x 7))
                (let* ((x 8))
                  (let* ((x 9))
                    (let* ((x 10))
                      (let* ((x 11))
                        x)))))))))))
"#;
    let wasm = compile_near(src).expect("compile failed");
    std::fs::write("/tmp/nmdebug/dsx.wasm", &wasm).unwrap();
    let out = std::process::Command::new("./target/release/near-mock")
        .arg("/tmp/nmdebug/dsx.wasm")
        .arg("_run")
        .arg("{}")
        .env("NEAR_MOCK_STATE", "/tmp/nmdebug/dsx_state.bin")
        .output()
        .expect("run");
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        if l.starts_with("📄") || l.starts_with("❌") {
            println!("{l}");
        }
    }
    // binary-search: which depth breaks in the exact indented form?
    for n in 2..=11 {
        let mut s = String::from("(define (main)\n");
        for i in 1..=n {
            s.push_str(&format!("{}(let* ((x {}))\n", " ".repeat(i), i));
        }
        s.push_str(&format!("{}x\n", " ".repeat(n + 1)));
        for i in (1..=n).rev() {
            s.push_str(&format!("{}))\n", " ".repeat(i + 1)));
        }
        let w = compile_near(&s).expect("compile failed");
        let p = format!("/tmp/nmdebug/dsy{n}.wasm");
        std::fs::write(&p, &w).unwrap();
        let o = std::process::Command::new("./target/release/near-mock")
            .arg(&p)
            .arg("_run")
            .arg("{}")
            .env("NEAR_MOCK_STATE", format!("/tmp/nmdebug/dsy_state_{n}.bin"))
            .output()
            .expect("run");
        let line = String::from_utf8_lossy(&o.stdout)
            .lines()
            .find(|l| l.starts_with("📄") || l.starts_with("❌"))
            .unwrap_or("?")
            .to_string();
        println!("indent-depth {n}: {line}");
    }
}
