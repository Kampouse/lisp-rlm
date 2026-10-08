//! async v2 — the full Tier B async surface, end-to-end on near-mock.
//!
//! Supersedes the V1 shape (one await, first-statement-only, zero-deposit):
//!   · awaits ANYWHERE + MULTIPLE awaits — CPS split at each await point,
//!     per-resume frame restore (`__await:<fn>:<param>` storage, the
//!     V1-proven path), pre-await statements run in the entry
//!   · `near.all([callA, callB])` — promise_and fanout, ONE resume reading
//!     promise_result(0..n) in dependency order (portfolio-fanout shape)
//!   · payable awaits — the deposit argument flows through the manual
//!     promise DAG (V1's call-await was hardwired zero-deposit)
//!   · T4 await-frame rule (COMPILE-time): a local read after an await, or
//!     an assignment to a param/pre-await local after an await, is a
//!     compile error NAMING the variable — only params + await results
//!     cross await boundaries
//!
//! Negative tests live in ts_to_lisp_source-land (compile-time), positive
//! end-to-end tests drive ./target/release/near-mock cross mode like
//! test_ts_cross / test_ts_portfolio.

use lisp_rlm_wasm::ts_frontend::ts_to_lisp_source;
use lisp_rlm_wasm::{compile_near_from_exprs, parse_all};
use std::sync::{Mutex, OnceLock};

const TOKEN_SRC: &str = include_str!("../fixtures/token_view.ts");
const SEQ_SRC: &str = include_str!("../fixtures/async_v2_seq.ts");
const ALL_SRC: &str = include_str!("../fixtures/async_v2_all.ts");
const PAY_SRC: &str = include_str!("../fixtures/async_v2_pay.ts");
const PITCH_SRC: &str = include_str!("../fixtures/async_v2_pitch.ts");

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    match L.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn w(src: &str, tag: &str) -> String {
    let ir = ts_to_lisp_source(src).unwrap_or_else(|e| panic!("lower: {e}"));
    let exprs = parse_all(&ir).expect("parse");
    lisp_rlm_wasm::typing::type_check_program(&exprs, true).expect("typecheck");
    let wasm = compile_near_from_exprs(&exprs).unwrap_or_else(|e| panic!("compile: {e}"));
    let p = std::env::temp_dir().join(format!("av2_{}_{}.wasm", tag, std::process::id()));
    std::fs::write(&p, &wasm).unwrap();
    p.to_str().unwrap().into()
}

fn run(manifest: &str, acct: &str, method: &str, args: &str, attach: &str) -> String {
    // state path MUST match state_path()'s delete target exactly — a literal
    // "/tmp/..." here vs std::env::temp_dir() there silently diverges on
    // macOS (/tmp is a symlink; temp_dir() is /var/folders/...), leaving the
    // reset a no-op and balances ACCUMULATING across runs
    let state = std::env::temp_dir().join("av2-test-state.bin");
    let state = state.to_str().unwrap().to_string();
    let mut cmd = std::process::Command::new("./target/release/near-mock");
    cmd.arg("cross")
        .arg(&state)
        .arg(manifest)
        .arg(acct)
        .arg(method)
        .arg(args)
        .env("NEAR_MOCK_SIGNER", "alice.test.near")
        .env("NEAR_MOCK_BLOCK_TS", "1800000000000000000");
    if !attach.is_empty() {
        cmd.env("NEAR_MOCK_ATTACH", attach);
    }
    let out = cmd.output().expect("near-mock");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn state_path(name: &str) {
    let p = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&p);
}

#[test]
fn v2_seq_two_awaits_with_preawait_stmts() {
    let _l = lock();
    state_path("av2-test-state.bin");
    let u: u128 = 10u128.pow(18);

    let tok = w(TOKEN_SRC, "seq_tok");
    let seq = w(SEQ_SRC, "seq");
    let manifest = format!("tok2.v2.test.near={},caller.v2.test.near={}", tok, seq);

    // user balance 700K, contract balance 250K → join 950K
    assert!(run(
        &manifest,
        "tok2.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"alice.test.near","amount":"{}"}}"#, 700_000 * u),
        ""
    )
    .contains("supply:"));
    assert!(run(
        &manifest,
        "tok2.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"tok2.v2.test.near","amount":"{}"}}"#, 250_000 * u),
        ""
    )
    .contains("supply:"));

    let out = run(
        &manifest,
        "caller.v2.test.near",
        "depositTwice",
        r#"{"user":"alice.test.near"}"#,
        "",
    );
    // 700K + 250K joined ACROSS TWO sequential awaits, and the "pre:"
    // marker written BEFORE the first await survived to the last resume
    assert!(
        out.contains(&format!("twice:{}:pre:alice.test.near", 950_000 * u)),
        "two-await seq failed: {out}"
    );
}

#[test]
fn v2_near_all_fanout_parallel_join() {
    let _l = lock();
    state_path("av2-test-state.bin");
    let u: u128 = 10u128.pow(18);

    let tok = w(TOKEN_SRC, "all_tok");
    let all = w(ALL_SRC, "all");
    let manifest = format!(
        "toka.v2.test.near={},tokb.v2.test.near={},fanout.v2.test.near={}",
        tok, tok, all
    );

    assert!(run(
        &manifest,
        "toka.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"bob.test.near","amount":"{}"}}"#, 500_000 * u),
        ""
    )
    .contains("supply:"));
    assert!(run(
        &manifest,
        "tokb.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"bob.test.near","amount":"{}"}}"#, 450_000 * u),
        ""
    )
    .contains("supply:"));

    // THE FAN-OUT: two parallel sub-calls, ONE resume, joined in dep order
    let out = run(
        &manifest,
        "fanout.v2.test.near",
        "portfolioBoth",
        r#"{"user":"bob.test.near"}"#,
        "",
    );
    assert!(
        out.contains(&format!("both:{}", 950_000 * u)),
        "near.all fanout failed (wrong sum = promise_result aliasing): {out}"
    );
}

#[test]
fn v2_payable_await_deposit_flows_through() {
    let _l = lock();
    state_path("av2-test-state.bin");
    let u: u128 = 10u128.pow(18);

    let tok = w(TOKEN_SRC, "pay_tok");
    let pay = w(PAY_SRC, "pay");
    let manifest = format!("tok3.v2.test.near={},payer.v2.test.near={}", tok, pay);

    // attach 123 tokens: the deposit forwards to ftMint ON tok3, then the
    // resume reads back the mint receipt ("supply:<total>", fresh ledger)
    let out = run(
        &manifest,
        "payer.v2.test.near",
        "attach",
        r#"{"user":"carol.test.near"}"#,
        &format!("{}", 123 * u),
    );
    assert!(
        out.contains(&format!("attached:supply:{}", 123 * u)),
        "payable await failed (V1 rejected this at compile time): {out}"
    );
}

#[test]
fn v2_t4_local_across_await_is_compile_error() {
    let _l = lock(); // compile-only, but keep the family serialized anyway

    // CASE 1: let local read after the await — silent snapshot in V1-land,
    // hard error in v2, message NAMES the variable
    let src = r#"
export async function f(user: string): string {
  let scratch = "s:" + user;
  const res = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  return scratch + res;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("`scratch`") && err.contains("crosses an await boundary"),
        "expected the T4 error naming `scratch`, got: {err}"
    );

    // CASE 2: assignment to a param after the await
    let src = r#"
export async function g(user: string): string {
  const res = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  user = "moved";
  return res + user;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("`user`") && err.contains("cannot assign to parameter"),
        "expected the param-assign error naming `user`, got: {err}"
    );

    // CASE 3: assignment to a pre-await local after the await
    let src = r#"
export async function h(user: string): string {
  let acc = 0;
  acc = acc + 1;
  const res = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  acc = acc + 2;
  return res + acc;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("`acc`") && err.contains("cannot assign"),
        "expected the local-assign error naming `acc`, got: {err}"
    );

    // CASE 4 (the release valve): recompute the local INSIDE the resume
    // segment — same name, fresh binding, legal
    let src = r#"
export async function okfn(user: string): string {
  let scratch = "s:" + user;
  const res = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  let scratch2 = scratch + res;
  return scratch2;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("`scratch`"),
        "reading pre-await `scratch` after the await must still error, got: {err}"
    );

    // CASE 5: legal v2 shape — local confined to its region lowers fine
    let src = r#"
export async function fine(user: string): string {
  const res = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  let local = "L" + res;
  return local + user;
}
"#;
    let ir = ts_to_lisp_source(src).expect("post-await-only locals are legal");
    assert!(ir.contains("fine__resume"), "resume not generated: {ir}");
}

#[test]
fn v2_surface_contract() {
    let _l = lock();

    // the emitted lisp is the portfolio-fanout DAG, not call-await
    let ir = ts_to_lisp_source(ALL_SRC).expect("lower");
    assert!(ir.contains("near/promise_and"), "fanout must use promise_and: {ir}");
    assert!(ir.contains("portfolioBoth__resume"), "one named resume: {ir}");
    assert!(!ir.contains("call-await"), "v2 never emits call-await: {ir}");

    // single await keeps the V1-stable callback name (existing contracts)
    let ir = ts_to_lisp_source(PAY_SRC).expect("lower");
    assert!(ir.contains("attach__resume"), "single-await resume name: {ir}");

    // multiple awaits → indexed resumes, each exported (on-chain entries)
    let ir = ts_to_lisp_source(SEQ_SRC).expect("lower");
    assert!(ir.contains("depositTwice__resume0"), "indexed resume 0: {ir}");
    assert!(ir.contains("depositTwice__resume1"), "indexed resume 1: {ir}");
}

#[test]
fn v2_pitch_shape_destructuring_object_args() {
    let _l = lock();
    state_path("av2-test-state.bin");
    let u: u128 = 10u128.pow(18);

    let tok = w(TOKEN_SRC, "pitch_tok");
    let pitch = w(PITCH_SRC, "pitch");
    let manifest = format!(
        "toka.v2.test.near={},tokb.v2.test.near={},pitch.v2.test.near={}",
        tok, tok, pitch
    );

    assert!(run(
        &manifest,
        "toka.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"carol.test.near","amount":"{}"}}"#, 700_000 * u),
        ""
    )
    .contains("supply:"));
    assert!(run(
        &manifest,
        "tokb.v2.test.near",
        "ftMint",
        &format!(r#"{{"to":"carol.test.near","amount":"{}"}}"#, 300_000 * u),
        ""
    )
    .contains("supply:"));

    // THE BASE EXAMPLE (2026-10-07 pitch, verbatim shape): destructured
    // near.all + object args auto-quoted + Promise<Money> annotation.
    let out = run(
        &manifest,
        "pitch.v2.test.near",
        "portfolioTotal",
        r#"{"user":"carol.test.near"}"#,
        "",
    );
    assert!(
        out.contains(&format!("total:{}", 1_000_000 * u)),
        "pitch shape failed: {out}"
    );
}

#[test]
fn v2_money_source_only_seeding_getname_is_not_yocto() {
    let _l = lock();

    // An await of a NON-balance method is a plain string (Value) —
    // returning it as Promise<Yocto> is a contract violation. Money
    // comes from money SOURCES (typed-surface rule 2).
    let src = r#"
type Yocto = string;
export async function f(): Promise<Yocto> {
  const n = await near.call("tokx.v2.test.near", "getName", { who: "a" }, 20000000000000, "0");
  return n;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("promises a money type"),
        "expected the return-contract error, got: {err}"
    );

    // Control: a balance read IS a money source — same shape compiles.
    let ok = r#"
type Yocto = string;
export async function f(): Promise<Yocto> {
  const b = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  return b;
}
"#;
    ts_to_lisp_source(ok).unwrap();
}

#[test]
fn v2_transfer_u128_sink_demands_provable_u128() {
    let _l = lock();

    // The SINK: transferU128 moves real value — an unbranded string is
    // not checked money (the customs seal). Fix: `: Yocto` at birth.
    let src = r#"
export function f(): void {
  const s = "5";
  near.transferU128("carol.v2.test.near", s);
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("transferU128 amount must be a provable u128"),
        "expected the sink error, got: {err}"
    );

    // Control: the annotated const carries the seal — compiles.
    let ok = r#"
type Yocto = string;
export function f(): void {
  const s: Yocto = "5";
  near.transferU128("carol.v2.test.near", s);
}
"#;
    ts_to_lisp_source(ok).unwrap();
}

#[test]
fn v2_money_taint_raw_arithmetic_on_amounts_is_compile_error() {
    let _l = lock(); // compile-only, but keep the family serialized anyway

    // CASE 1: `+` on two await results — the original hazard. `+` lowers
    // to concat on decimal strings: balances MERGE instead of summing.
    let src = r#"
export async function f(user: string): string {
  const a = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  const b = await near.call("toky.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  return a + b;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("raw arithmetic on money values"),
        "expected the money-taint error, got: {err}"
    );

    // CASE 2: Yocto-annotated params — `+` corrupts the same way.
    let src = r#"
type Yocto = string;
export function g(a: Yocto, b: Yocto): Yocto {
  return a + b;
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("raw arithmetic on money values"),
        "expected the money-taint error for annotated params, got: {err}"
    );

    // CASE 3: deposit + balance — money-source calls count too.
    let src = r#"
export function h(): string {
  return near.attachedDepositU128() + near.accountBalance();
}
"#;
    let err = ts_to_lisp_source(src).unwrap_err();
    assert!(
        err.contains("raw arithmetic on money values"),
        "expected the money-taint error for deposit+balance, got: {err}"
    );

    // NEGATIVE 1: one-sided money `+` is concat by design — legal.
    let src = r#"
export async function ok1(user: string): string {
  const a = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  return "total:" + a;
}
"#;
    assert!(ts_to_lisp_source(src).is_ok(), "one-sided + must stay legal");

    // NEGATIVE 2: u128Add is the blessed op — legal.
    let src = r#"
export async function ok2(user: string): string {
  const a = await near.call("tokx.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  const b = await near.call("toky.v2.test.near", "ftBalanceRaw", "{}", 20000000000000, 0);
  return u128Add(a, b);
}
"#;
    assert!(ts_to_lisp_source(src).is_ok(), "u128Add must stay legal");
}

#[test]
fn v2_return_contract_money_annotation_is_enforced() {
    let _l = lock(); // compile-only, but keep test-state hygiene
    // OK: await result returned raw under Promise<Yocto>
    let ok = "export async function f(user: string): Promise<Yocto> { const a = await near.call(\"toka.v2.test.near\", \"ftBalanceRaw\", { who: user }, 20000000000000, \"0\"); return a; }";
    assert!(ts_to_lisp_source(ok).is_ok(), "raw await return must compile");

    // REJECT: labeled concat under a money annotation — display text is
    // not an amount. This is the exact dishonesty the contract check kills.
    let bad = "export async function f(user: string): Promise<Yocto> { const a = await near.call(\"toka.v2.test.near\", \"ftBalanceRaw\", { who: user }, 20000000000000, \"0\"); return \"total:\" + u128Add(a, a); }";
    let e = ts_to_lisp_source(bad).unwrap_err();
    assert!(e.contains("promises a money type"), "got: {e}");
    assert!(e.contains("concatenation"), "got: {e}");

    // REJECT: plain param returned under Yocto
    let bad2 = "export function g(user: string): Yocto { return user; }";
    let e2 = ts_to_lisp_source(bad2).unwrap_err();
    assert!(e2.contains("not a provable u128 value"), "got: {e2}");

    // OK: money-annotated const feeds the return (taint pre-pass)
    let ok2 = "export function h(): Yocto { const d: Yocto = \"5\"; return u128Add(d, d); }";
    assert!(ts_to_lisp_source(ok2).is_ok(), "money const return must compile");

    // REJECT: money const initialized from a non-u128 source
    let bad3 = "export function h(fn: () => string): string { const d: Yocto = fn(); return d; }";
    let e3 = ts_to_lisp_source(bad3).unwrap_err();
    assert!(e3.contains("annotated as a money type"), "got: {e3}");
}
