//! Fuzz L5: random-walk differential gauntlet.
//!
//! Drives N seeds × M ops of mint/swap-y/swap-x on near-mock. After every
//! op, the contract's S/P (op "view") is compared to an OCaml oracle generated
//! by Isabelle `export_code` from the formal v5 kernel.
//!
//! Deterministic: seeds 1..=3 run in CI (gauntlet L5); `FUZZ_SEEDS` widens.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn bin(name: &str) -> PathBuf {
    let p = repo_root().join("target/release").join(name);
    assert!(p.exists(), "{name} missing — cargo build --release first");
    p
}

fn b64e(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |a, &x| (a << 8) | x as u32) << (8 * (3 - chunk.len()) as u32);
        for k in (0..4).rev() {
            if k < 3 - chunk.len() as u32 {
                out.push('=');
            } else {
                out.push(T[((n >> (6 * k)) & 63) as usize] as char);
            }
        }
    }
    out
}

/// Seed one storage key on `acct` (Finding C prod-gate arming for sim runs).
fn import_key(state: &Path, acct: &str, key: &str, val: &str) {
    let row = format!(
        "[{{\"account\":\"{acct}\",\"key\":\"{}\",\"value\":\"{}\"}}]",
        b64e(key),
        b64e(val)
    );
    let mut child = Command::new(bin("near-mock"))
        .args(["state", "import", state.to_str().unwrap(), "-"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn near-mock state import");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(row.as_bytes())
        .unwrap();
    let st = child.wait().expect("state import wait");
    assert!(st.success(), "state import failed: {row}");
}

fn compile(concat: &[&str], out: &Path) {
    let mut src = String::new();
    for part in concat {
        src.push_str(&std::fs::read_to_string(repo_root().join("lib").join(part)).unwrap());
        src.push('\n');
    }
    let tmp = repo_root().join("target/test-fuzz-src.lisp");
    std::fs::write(&tmp, src).unwrap();
    let o = Command::new(bin("near-compile"))
        .arg(&tmp)
        .arg(out)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        o.status.success() && all.contains('✅'),
        "compile failed: {all}"
    );
}

struct Oracle {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

#[derive(Debug, PartialEq, Eq)]
struct Quote {
    out: u128,
    s: u128,
    p: i64,
}

impl Oracle {
    fn new(exe: &Path) -> Self {
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn oracle");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn ask(
        &mut self,
        op: &str,
        s: u128,
        p: i64,
        imin: i64,
        imax: i64,
        v: u128,
        shift: u8,
        lq: &[(i64, u128)],
        fee: bool,
    ) -> Quote {
        let op_code = if fee { op } else { &format!("{}g", op) };
        let mut json = format!(
            "{{\"op\":\"{op_code}\",\"s\":\"{s}\",\"p\":\"{p}\",\"imin\":\"{imin}\",\"imax\":\"{imax}\",\"v\":\"{v}\",\"shift\":\"{shift}\"",
        );
        for (i, l) in lq {
            json.push_str(&format!(",\"lq_{i}\":\"{l}\""));
        }
        json.push_str("}\n");
        self.stdin
            .write_all(json.as_bytes())
            .expect("write oracle stdin");
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let mut out = None;
        let mut s1 = None;
        let mut p1 = None;
        for part in line.trim().split(',') {
            let part = part.trim().trim_start_matches('{').trim_end_matches('}');
            if let Some((k, v)) = part.split_once(':') {
                let v = v.trim().trim_matches('"').to_string();
                match k.trim().trim_matches('"') {
                    "out" => out = Some(v.parse().unwrap()),
                    "s" => s1 = Some(v.parse().unwrap()),
                    "p" => p1 = Some(v.parse().unwrap()),
                    _ => {}
                }
            }
        }
        Quote {
            out: out.unwrap(),
            s: s1.unwrap(),
            p: p1.unwrap(),
        }
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

struct Mock {
    state: PathBuf,
    manifest: String,
}

impl Mock {
    fn cross(&self, c: &str, m: &str, a: &str, s: &str) -> String {
        let out = Command::new(bin("near-mock"))
            .args(["cross"])
            .arg(&self.state)
            .arg(&self.manifest)
            .args([c, m, a])
            .env("NEAR_MOCK_SIGNER", s)
            .output()
            .expect("near-mock cross");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
    fn ret(&self, out: &str) -> String {
        out.lines()
            .rev()
            .find(|l| l.contains('📄'))
            .and_then(|l| l.split('📄').nth(1))
            .map(|l| {
                l.trim()
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .replace(',', "")
            })
            .unwrap_or_default()
    }
    fn op(&self, json: &str) -> String {
        self.ret(&self.cross("pool.tst", "exec", json, "owner"))
    }
    fn view(&self) -> String {
        self.op("{\"op\":\"view\"}")
    }
    fn state(&self) -> String {
        self.op("{\"op\":\"state\"}")
    }
}

/// Extract S,P,IMIN,IMAX and per-cell LQ from a p5-view string.
fn parse_view(v: &str) -> (u128, i64, i64, i64, Vec<(i64, u128)>) {
    let mut s = 0u128;
    let mut p = 0i64;
    let mut imin = 0i64;
    let mut imax = 0i64;
    let mut lq = Vec::new();
    let parts: Vec<&str> = v.split(':').collect();
    for chunk in parts.chunks(2) {
        if chunk.len() != 2 {
            continue;
        }
        let key = chunk[0].trim();
        let val = chunk[1].trim();
        if key == "LQ" {
            if let Some((idx, amt)) = val.split_once('|') {
                if let (Ok(i), Ok(a)) = (idx.parse(), amt.parse()) {
                    lq.push((i, a));
                }
            }
        } else {
            match key {
                "S" => {
                    if let Ok(x) = val.parse() {
                        s = x;
                    }
                }
                "P" => {
                    if let Ok(x) = val.parse() {
                        p = x;
                    }
                }
                "IMIN" => {
                    if let Ok(x) = val.parse() {
                        imin = x;
                    }
                }
                "IMAX" => {
                    if let Ok(x) = val.parse() {
                        imax = x;
                    }
                }
                _ => {}
            }
        }
    }
    (s, p, imin, imax, lq)
}

fn parse_last(v: &str) -> Option<&str> {
    // LAST value itself contains ':' (swap-out:<amt>:rem:<rm>); it is framed
    // by :LAST: ... :ESCX: in the p5-state string.
    let start = v.split(":LAST:").nth(1)?;
    let end = start.split(":ESCX:").next()?;
    Some(end)
}

fn sp_head(v: &str) -> String {
    v.split(":X:").next().unwrap_or("?").to_string()
}

#[test]
fn pool_v5_fuzz_differential() {
    let oracle_exe = repo_root().join("afp-clmm/v5-kernel/clmm_v5_quote_opt");
    assert!(
        oracle_exe.exists(),
        "OCaml oracle missing: build afp-clmm/v5-kernel first"
    );
    let mut oracle = Oracle::new(&oracle_exe);

    let dir = repo_root().join("target/test-fuzz");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let seeds: Vec<u64> = std::env::var("FUZZ_SEEDS")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().parse().unwrap()).collect())
        .unwrap_or_else(|| vec![1, 2, 3]);
    const OPS: usize = 40;

    for seed in seeds {
        let state = dir.join(format!("state-fuzz-{seed}.bin"));
        let _ = std::fs::remove_file(&state);
        let _ = std::fs::remove_file(state.with_extension("bin.manifest.json"));
        let m = Mock {
            manifest: format!(
                "pool.tst={},tokx.tst={},toky.tst={}",
                pool_wasm.display(),
                ft_wasm.display(),
                ft_wasm.display()
            ),
            state: state.clone(),
        };


        assert!(m.ret(&m.cross("tokx.tst", "new", "{}", "owner")) == "ok");
        assert!(m.ret(&m.cross("toky.tst", "new", "{}", "owner")) == "ok");
        // Finding C prod-gate: verify the gross arm is DEAD before arming
        // (no 6|SIMSWAP key ⇒ ERR_SIM_ONLY, no walk, no state change).
        assert_eq!(m.op("{\"op\":\"swap\",\"v\":\"1000\"}"), "sim-only");
        import_key(&state, "pool.tst", "6|SIMSWAP", "1");
        assert_eq!(m.op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\",\"fee_shift\":\"8\",\"s0\":\"18446744073709551616\"}"), "init-ok");

        const LP_FUND: u128 = 100_000_000_000_000_000_000_000_000_000; // 1e29
        const TRADER_FUND: u128 = 150_000_000_000_000_000_000_000_000_000; // 1.5e29
        for who in ["alice.tst", "bob.tst", "carol.tst"] {
            for tok in ["tokx.tst", "toky.tst"] {
                assert_eq!(
                    m.ret(&m.cross(
                        tok,
                        "mint",
                        &format!("{{\"to\":\"{who}\",\"amt\":\"{LP_FUND}\"}}"),
                        "owner"
                    )),
                    "ok"
                );
            }
            assert!(m
                .ret(&m.cross("pool.tst", "exec", "{\"op\":\"reg\"}", who))
                .starts_with("reg-ok"));
        }
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(
                m.ret(&m.cross(
                    tok,
                    "mint",
                    &format!("{{\"to\":\"trader.tst\",\"amt\":\"{TRADER_FUND}\"}}"),
                    "owner"
                )),
                "ok"
            );
        }

        const SEED_X: u128 = 100_000_000_000_000_000_000_000_000_000;
        const SEED_Y: u128 = 100_000_000_000_000_000_000_000_000_000;
        assert!(m
            .ret(&m.cross(
                "tokx.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_X}\",\"msg\":\"{{\\\"a\\\":\\\"seed\\\"}}\"}}"
                ),
                "alice.tst"
            ))
            .starts_with("used:"));
        assert!(m
            .ret(&m.cross(
                "toky.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_Y}\",\"msg\":\"{{\\\"a\\\":\\\"seed\\\"}}\"}}"
                ),
                "alice.tst"
            ))
            .starts_with("used:"));

        let mut rng = seed;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };

        // Track S/P before each op so the oracle can predict the swap result.
        let mut prev_s = 18446744073709551616u128;
        let mut prev_p: i64 = 0;

        for i in 0..OPS {
            let r = next() % 100;
            let mut swap_amt: Option<(u128, &str, bool)> = None;
            if i < 4 || r < 30 {
                let lo = -((next() % 8) as i64) - 1;
                let hi = (next() % 8) as i64 + 1;
                let (lo, hi) = (lo.min(hi), hi.max(lo));
                let d: u128 = 1_000_000_000_000_000_000_000_000_000u128
                    + (next() % 50) as u128 * 1_000_000_000_000_000_000_000_000_000u128;
                let who = ["alice.tst", "bob.tst", "carol.tst"][(next() % 3) as usize];
                let req = m.op(&format!(
                    "{{\"op\":\"req\",\"d\":\"{d}\",\"a\":\"{lo}\",\"b\":\"{hi}\"}}"
                ));
                let body = req.trim_start_matches("rx:");
                let (rx_s, ry_s) = body.split_once(":ry:").expect("req shape");
                let rx: u128 = rx_s.parse().unwrap();
                let ry: u128 = ry_s.parse().unwrap();
                if rx <= LP_FUND && ry <= LP_FUND {
                    let msg = format!("{{\"a\":\"mint\",\"d\":\"{d}\",\"lo\":\"{lo}\",\"hi\":\"{hi}\"}}");
                    let _lx = m.ret(&m.cross(
                        "tokx.tst",
                        "ft_transfer_call",
                        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{rx}\",\"msg\":\"{}\"}}", msg.replace('"', "\\\"")),
                        who
                    ));
                    let _ly = m.ret(&m.cross(
                        "toky.tst",
                        "ft_transfer_call",
                        &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{ry}\",\"msg\":\"{}\"}}", msg.replace('"', "\\\"")),
                        who
                    ));
                }
            } else if r < 60 {
                let amt: u128 = 1_000_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000_000;
                swap_amt = Some((amt, "y", true));
                let _ = m.ret(&m.cross(
                    "toky.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{{\\\"a\\\":\\\"swap\\\"}}\"}}"
                    ),
                    "trader.tst",
                ));
            } else if r < 90 {
                let amt: u128 = 1_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000;
                swap_amt = Some((amt, "x", true));
                let _ = m.ret(&m.cross(
                    "tokx.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"{{\\\"a\\\":\\\"swap\\\"}}\"}}"
                    ),
                    "trader.tst",
                ));
            } else {
                // gross parity swap (op "swap", feep?=0): sign picks direction
                let amt: u128 = 1_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000;
                let down = next() % 2 == 0;
                let vstr = if down {
                    format!("-{amt}")
                } else {
                    format!("{amt}")
                };
                // sell Y walks down (oracle op "y"), sell X walks up ("x")
                let side = if down { "y" } else { "x" };
                swap_amt = Some((amt, side, false));
                let _ = m.op(&format!("{{\"op\":\"swap\",\"v\":\"{vstr}\"}}"));
            }

            let state = m.state();
            let last = parse_last(&state);
            let (s, p, imin, imax, lq) = parse_view(&state);

            // A swap happened iff LAST carries a swap-out message; the side
            // and fee flag come from the op we just issued (LAST itself does
            // not encode the side).
            let swapped = last.map(|l| l.starts_with("swap-out:")).unwrap_or(false);
            if swapped {
                let (amt, side, fee) = swap_amt
                    .map(|(a, sd, f)| (a, sd, f))
                    .unwrap_or((0, "y", true));
                let want = oracle.ask(side, prev_s, prev_p, imin, imax, amt, 8, &lq, fee);
                if std::env::var("FUZZ_TRACE").is_ok() {
                    eprintln!(
                        "[trace] op {i} swap-{side} fee {fee} amt {amt}\n  prev {prev_s}:{prev_p}\n  view  {s}:{p} imin {imin} imax {imax}\n  lq {lq:?}\n  want  {}:{}",
                        want.s, want.p
                    );
                }
                assert_eq!(
                    (s, p),
                    (want.s, want.p),
                    "seed {seed} op {i}: swap-{side} S/P mismatch\n  contract: {s}:{p}\n  oracle:   {want_s}:{want_p}",
                    want_s = want.s,
                    want_p = want.p
                );
            }

            // Backstop: consecutive views must agree.
            let view2 = m.view();
            let again = sp_head(&view2);
            assert_eq!(
                sp_head(&state),
                again,
                "seed {seed} op {i}: view not stable"
            );

            prev_s = s;
            prev_p = p;
        }
        println!("seed {seed}: {OPS} ops, differential ✓");
    }
    println!("fuzz L5 differential: all seeds green");
}
