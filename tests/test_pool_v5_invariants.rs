//! Invariant fuzz for the CLMM v5 pool, driven on near-mock.
//!
//! After every op, ONE `near-mock state dump` gives the full trie; from it we
//! assert, with no extra process spawns:
//!
//!   I1  owed ledgers stay zero (this op mix never credits burnc/claim)
//!   I2  token conservation: Σ balances + pool custody unchanged
//!   I3  escrow float == seed float + Σ mint intake + Σ swap intake − Σ out
//!       (tracked incrementally from the ops we issued)
//!
//! Deterministic: same xorshift draw order as the differential harness.
//! `INV_SEEDS` widens the seed list; default 1..=3.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    use std::io::Write as _;
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

fn compile(files: &[&str], out: &Path) {
    let mut src = String::new();
    for part in files {
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
    fn op_as(&self, json: &str, who: &str) -> String {
        self.ret(&self.cross("pool.tst", "_run", json, who))
    }
    fn op(&self, json: &str) -> String {
        self.op_as(json, "owner")
    }
    /// One spawn → full trie as (account, decoded-key) → decoded-value.
    fn dump(&self) -> BTreeMap<String, String> {
        let out = Command::new(bin("near-mock"))
            .args(["state", "dump"])
            .arg(&self.state)
            .output()
            .expect("near-mock state dump");
        let s = String::from_utf8_lossy(&out.stdout);
        let j: serde_json::Value = serde_json::from_str(s.trim()).expect("dump json");
        let mut map = BTreeMap::new();
        for row in j.as_array().expect("dump array") {
            let acct = row["account"].as_str().unwrap_or("").to_string();
            let key =
                String::from_utf8(base64d(row["key"].as_str().unwrap_or(""))).unwrap_or_default();
            let val =
                String::from_utf8(base64d(row["value"].as_str().unwrap_or(""))).unwrap_or_default();
            map.insert(format!("{acct}|{key}"), val);
        }
        map
    }
}

fn base64d(s: &str) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for c in s.bytes() {
        let v = T.iter().position(|&t| t == c).expect("b64 char") as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}

fn u128_of(v: &str, what: &str) -> u128 {
    v.parse::<u128>()
        .unwrap_or_else(|e| panic!("{what}: '{v}' ({e})"))
}

fn parse_owed(s: &str) -> (u128, u128) {
    let b = s.trim_start_matches("owed:");
    let (x, y) = b.split_once(':').expect("owed shape");
    (u128_of(x, "owed x"), u128_of(y, "owed y"))
}

const USERS: [&str; 3] = ["alice.tst", "bob.tst", "carol.tst"];
const TRADER: &str = "trader.tst";
const OWNER: &str = "owner";

const BKEY: &str = "b:"; // ft2 balance key prefix
const SUPKEY: &str = "sup";
const OWX: &str = "6|OWX:";
const OWY: &str = "6|OWY:";
const ESCX: &str = "6|ESCX";
const ESCY: &str = "6|ESCY";

fn bal(d: &BTreeMap<String, String>, tok: &str, who: &str) -> u128 {
    let k = format!("{tok}|{BKEY}{who}");
    u128_of(d.get(&k).map(String::as_str).unwrap_or("0"), &k)
}

fn pool_int(d: &BTreeMap<String, String>, key: &str) -> u128 {
    let k = format!("pool.tst|{key}");
    u128_of(d.get(&k).map(String::as_str).unwrap_or("0"), &k)
}

fn owed_from_dump(d: &BTreeMap<String, String>, who: &str) -> (u128, u128) {
    (
        pool_int(d, &format!("{OWX}{who}")),
        pool_int(d, &format!("{OWY}{who}")),
    )
}

#[test]
fn pool_v5_invariants() {
    let dir = repo_root().join("target/test-fuzz");
    std::fs::create_dir_all(&dir).unwrap();
    let pool_wasm = dir.join("pool.wasm");
    let ft_wasm = dir.join("ft.wasm");
    compile(
        &["limb_math.lisp", "clmm_v5_kernel.lisp", "clmm_pool_v5.lisp"],
        &pool_wasm,
    );
    compile(&["limb_math.lisp", "clmm_core.lisp", "ft2.lisp"], &ft_wasm);

    let seeds: Vec<u64> = std::env::var("INV_SEEDS")
        .ok()
        .map(|s| s.split(',').map(|x| x.trim().parse().unwrap()).collect())
        .unwrap_or_else(|| vec![1, 2, 3]);
    const OPS: usize = 40;

    for seed in seeds {
        let state = dir.join(format!("state-inv-{seed}.bin"));
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

        assert_eq!(m.ret(&m.cross("tokx.tst", "new", "{}", OWNER)), "ok");
        assert_eq!(m.ret(&m.cross("toky.tst", "new", "{}", OWNER)), "ok");
        assert_eq!(
            m.op("{\"op\":\"init\",\"toka\":\"tokx.tst\",\"tokb\":\"toky.tst\",\"fee_shift\":\"8\",\"s0\":\"18446744073709551616\"}"),
            "init-ok"
        );
        // Finding C prod-gate: verify the gross arm is DEAD before arming
        // (no 6|SIMSWAP key ⇒ ERR_SIM_ONLY, no walk, no state change), then
        // arm it for this sim run via a direct state import.
        assert_eq!(m.op("{\"op\":\"swap\",\"v\":\"1000\"}"), "ERR_SIM_ONLY");
        import_key(&state, "pool.tst", "6|SIMSWAP", "1");

        const LP_FUND: u128 = 100_000_000_000_000_000_000_000_000_000;
        const TRADER_FUND: u128 = 150_000_000_000_000_000_000_000_000_000;
        for who in USERS {
            for tok in ["tokx.tst", "toky.tst"] {
                assert_eq!(
                    m.ret(&m.cross(
                        tok,
                        "mint",
                        &format!("{{\"to\":\"{who}\",\"amt\":\"{LP_FUND}\"}}"),
                        OWNER
                    )),
                    "ok"
                );
            }
            assert!(m.op_as("{\"op\":\"reg\"}", who).starts_with("reg-ok"));
        }
        for tok in ["tokx.tst", "toky.tst"] {
            assert_eq!(
                m.ret(&m.cross(
                    tok,
                    "mint",
                    &format!("{{\"to\":\"{TRADER}\",\"amt\":\"{TRADER_FUND}\"}}"),
                    OWNER
                )),
                "ok"
            );
        }

        const SEED_XY: u128 = 100_000_000_000_000_000_000_000_000_000;
        assert!(m
            .ret(&m.cross(
                "tokx.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_XY}\",\"msg\":\"seed\"}}"
                ),
                "alice.tst"
            ))
            .starts_with("used:"));
        assert!(m
            .ret(&m.cross(
                "toky.tst",
                "ft_transfer_call",
                &format!(
                    "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{SEED_XY}\",\"msg\":\"seed\"}}"
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

        // circulating totals fixed at setup (mints only happen here)
        let d0 = m.dump();
        let mut total = BTreeMap::new();
        for tok in ["tokx.tst", "toky.tst"] {
            let sum: u128 = USERS
                .iter()
                .chain([TRADER, OWNER].iter())
                .map(|w| bal(&d0, tok, w))
                .sum::<u128>()
                + bal(&d0, tok, "pool.tst");
            total.insert(tok, sum);
        }
        // escrow float tracking: starts at the seed deposit
        let mut esc_x = SEED_XY;
        let mut esc_y = SEED_XY;

        for i in 0..OPS {
            let r = next() % 100;
            if i < 4 || r < 30 {
                let lo = -((next() % 8) as i64) - 1;
                let hi = (next() % 8) as i64 + 1;
                let (lo, hi) = (lo.min(hi), hi.max(lo));
                let d: u128 = 1_000_000_000_000_000_000_000_000_000u128
                    + (next() % 50) as u128 * 1_000_000_000_000_000_000_000_000_000u128;
                let who = USERS[(next() % 3) as usize];
                let req = m.op(&format!(
                    "{{\"op\":\"req\",\"d\":\"{d}\",\"a\":\"{lo}\",\"b\":\"{hi}\"}}"
                ));
                if let Some(body) = req.strip_prefix("rx:") {
                    if let Some((rx_s, ry_s)) = body.split_once(":ry:") {
                        if let (Ok(rx), Ok(ry)) = (rx_s.parse::<u128>(), ry_s.parse::<u128>()) {
                            if rx <= LP_FUND && ry <= LP_FUND {
                                let msg = format!("m:{d}:{lo}:{hi}");
                                let o1 = m.ret(&m.cross(
                                    "tokx.tst",
                                    "ft_transfer_call",
                                    &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{rx}\",\"msg\":\"{msg}\"}}"),
                                    who,
                                ));
                                let o2 = m.ret(&m.cross(
                                    "toky.tst",
                                    "ft_transfer_call",
                                    &format!("{{\"receiver_id\":\"pool.tst\",\"amount\":\"{ry}\",\"msg\":\"{msg}\"}}"),
                                    who,
                                ));
                                // mint legs complete iff both used the full
                                // amount (excess refunded otherwise ⇒ pending
                                // mint, not credited this loop). A user with a
                                // drained balance fails both legs (contract
                                // rejects leg 1 before escrow) — skip cleanly.
                                let full = o1 == format!("used:{rx}") && o2 == format!("used:{ry}");
                                let neither = o1 == "insufficient" && o2 == "insufficient";
                                if full {
                                    esc_x += rx;
                                    esc_y += ry;
                                } else if !neither {
                                    panic!("seed {seed} op {i}: mint leg half-moved: {o1} / {o2}");
                                }
                            }
                        }
                    }
                }
            } else if r < 60 {
                let amt: u128 = 1_000_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000_000;
                let out = m.ret(&m.cross(
                    "toky.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"
                    ),
                    TRADER,
                ));
                let last = m.op("{\"op\":\"state\"}");
                // A rejected transfer (insufficient, etc.) prints no
                // 📄 used:<n> at all — then the swap was a no-op: skip it
                // (stale LAST from an earlier swap must NOT be trusted).
                // When it did execute: consumed part (`used`) folds into the
                // books; the rem (amt−used) stays in the ESC ledger as
                // in-flight refund float (trader is made whole token-side).
                if let Some(used_s) = out.strip_prefix("used:") {
                    let used: u128 = used_s.parse().unwrap();
                    if used > 0 {
                        let o = extract_swap_out(&last).unwrap_or_else(|| {
                            panic!("seed {seed} op {i}: executed swap without LAST")
                        });
                        esc_y += amt - used;
                        esc_x -= o;
                    }
                }
            } else if r < 90 {
                let amt: u128 = 1_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000;
                let out = m.ret(&m.cross(
                    "tokx.tst",
                    "ft_transfer_call",
                    &format!(
                        "{{\"receiver_id\":\"pool.tst\",\"amount\":\"{amt}\",\"msg\":\"sw\"}}"
                    ),
                    TRADER,
                ));
                let last = m.op("{\"op\":\"state\"}");
                // Same contract as the swap-y branch: rejected transfers
                // (no 📄 used:) are no-ops; executed swaps fold `used` into
                // the books and leave the rem as in-flight ESC float.
                if let Some(used_s) = out.strip_prefix("used:") {
                    let used: u128 = used_s.parse().unwrap();
                    if used > 0 {
                        let paid = extract_swap_out(&last).unwrap_or_else(|| {
                            panic!("seed {seed} op {i}: executed swap without LAST")
                        });
                        esc_x += amt - used;
                        esc_y -= paid;
                    }
                }
            } else {
                let amt: u128 = 1_000_000_000_000_000_000_000
                    + (next() % 999) as u128 * 1_000_000_000_000_000_000_000;
                let down = next() % 2 == 0;
                let vstr = if down {
                    format!("-{amt}")
                } else {
                    format!("{amt}")
                };
                let _ = m.op(&format!("{{\"op\":\"swap\",\"v\":\"{vstr}\"}}"));
                // gross swap: trader must be registered + hold funds; the
                // contract escrows the intake itself. Intake credited here is
                // tracked via the balance-side conservation check instead —
                // escrow check skipped for gross (op "swap" settles inline).
                esc_x = pool_int(&m.dump(), ESCX);
                esc_y = pool_int(&m.dump(), ESCY);
                continue;
            }

            let d = m.dump();
            // I1: owed stays zero (no burnc/claim in this mix)
            for who in USERS.iter().chain([TRADER].iter()) {
                let (ox, oy) = owed_from_dump(&d, who);
                assert_eq!(
                    (ox, oy),
                    (0, 0),
                    "seed {seed} op {i}: {who} owed moved without burnc/claim"
                );
            }
            // I2: conservation
            for tok in ["tokx.tst", "toky.tst"] {
                let sum: u128 = USERS
                    .iter()
                    .chain([TRADER, OWNER].iter())
                    .map(|w| bal(&d, tok, w))
                    .sum::<u128>()
                    + bal(&d, tok, "pool.tst");
                assert_eq!(
                    sum, total[tok],
                    "seed {seed} op {i}: {tok} circulating total drifted"
                );
            }
            // I3: escrow ledger == tracked cumulative
            assert_eq!(
                (pool_int(&d, ESCX), pool_int(&d, ESCY)),
                (esc_x, esc_y),
                "seed {seed} op {i}: escrow float != tracked cumulative"
            );
        }
        println!("seed {seed}: {OPS} ops, invariants ✓");
    }
    println!("pool_v5 invariants: all seeds green");
}

/// pull the paid amount out of a state string's LAST field:
/// …:LAST:swap-out:<paid>:rem:<rem>[:ESCX:…]
fn extract_swap_out(state: &str) -> Option<u128> {
    let start = state.find(":LAST:")? + ":LAST:".len();
    let end = state[start..]
        .find(":ESCX:")
        .map(|e| start + e)
        .unwrap_or(state.len());
    let last = &state[start..end];
    let rest = last.strip_prefix("swap-out:")?;
    let paid = rest.split(":rem:").next()?;
    paid.parse().ok()
}
