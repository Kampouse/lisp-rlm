//! BIP-340 schnorr stitcher regression battery (TASK-schnorr-stitcher.md).
//!
//! Layer 1 — stitcher e2e: official BIP-340 vectors through the TS probe →
//! `compile_near` → near-mock-style instantiation. Guards the whole stitch
//! pipeline: WASM_IMPORT_BASE sentinel call → env import registration →
//! `link_schnorr_wat` merge of the crypto lib into the final module.
//!
//! Layer 2 — nostr-gov digest chain: the REAL contract source
//! (projects/nostr-gov-lisp/src/main.ts) owner-signature path. Regression for
//! the sha256-hash hex-digest change (742aab9): the contract must feed
//! schnorrVerify the RAW 32-byte digest, not the 64-char hex rendering.
//!
//! Vector provenance (all machine-verified before embedding):
//!   - BIP-340 vectors 0/1: bitcoin/bips bip-0340/test-vectors.csv,
//!     cross-checked with projects/nostr-gov-lisp/tests/bip340.py.
//!   - nostr-gov chain vector: sign(SK=0xAA*32, sha256(msg)) over the exact
//!     owner_msg format from tests/gen-vectors.py (escrow.test.near,
//!     TS=1787000000000000000), digest cross-checked against python hashlib.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::*;

const TS: u64 = 1_787_000_000_000_000_000;
const EXPIRES: &str = "1787003600000000000"; // TS + 1h, matches gen-vectors.py
const ACCOUNT: &str = "escrow.test.near";
const PREDECESSOR: &str = "caller.test.near";

// BIP-340 official test vector 0 (msg = 32 zero bytes, sig verifies TRUE)
const V0_PK: &str = "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9";
const V0_SIG: &str = "E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0";
const V0_MSG: &str = "0000000000000000000000000000000000000000000000000000000000000000";

// nostr-gov owner sig over sha256("expires {EXPIRES}.000000000:
// create_wallet:satoshi | nonce: 7 | contract: escrow.test.near"), SK=0xAA*32
const OWNER_PK: &str = "6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3";
const OWNER_SIG: &str = "07174f9c163d20656bb4731758a00dcbd54cec3deac4bfbb537262ae7581093b5e29c02e43564e5b5a77ac8e655f697a771622709475a27ba9a68d063144c226";

// ── minimal near-mock-style host state ─────────────────────────────────

#[derive(Default, Clone)]
struct Mock {
    registers: HashMap<u64, Vec<u8>>,
    storage: HashMap<Vec<u8>, Vec<u8>>,
    logs: Vec<String>,
    ret: Option<Vec<u8>>,
    trapped: bool,
    panic_msg: Option<String>,
    input: Vec<u8>,
}

struct DriveResult {
    logs: Vec<String>,
    #[allow(dead_code)]
    trapped: bool,
    ret: Option<Vec<u8>>,
}

impl DriveResult {
    fn ret_i64(&self) -> Option<i64> {
        self.ret.as_ref().and_then(|d| d[..].try_into().ok()).map(i64::from_le_bytes)
    }
}

fn tamper(sig: &str) -> String {
    let mut b: Vec<u8> = hex(sig);
    let last = b.pop().unwrap();
    b.push(if last == 0 { 1 } else { last ^ 1 });
    b.iter().map(|x| format!("{:02X}", x)).collect()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect()
}

/// Compile TS source → wasm (TS frontend lowering + compile_near).
fn compile_ts(ts: &str) -> Vec<u8> {
    let lisp = lisp_rlm_wasm::ts_frontend::ts_to_lisp_source(ts)
        .unwrap_or_else(|e| panic!("TS lowering: {}", e));
    lisp_rlm_wasm::wasm_emit::compile_near(&lisp)
        .unwrap_or_else(|e| panic!("compile_near: {}", e))
}

fn read_mem(caller: &mut Caller<'_, Arc<Mutex<Mock>>>, ptr: usize, len: usize) -> Vec<u8> {
    caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .and_then(|m| Some(m.data(caller)[ptr..ptr + len].to_vec()))
        .unwrap_or_default()
}

fn write_reg(caller: &mut Caller<'_, Arc<Mutex<Mock>>>, rid: u64, data: Vec<u8>) {
    caller.data_mut().lock().unwrap().registers.insert(rid, data);
}

/// A contract driven with the full env host surface the compiler emits.
/// Storage persists across `call`s within one Contract.
struct Contract {
    wasm: Vec<u8>,
    mock: Arc<Mutex<Mock>>,
}

impl Contract {
    fn new(wasm: Vec<u8>) -> Contract {
        Contract { wasm, mock: Arc::new(Mutex::new(Mock::default())) }
    }

    fn call(&self, method: &str, args_json: &str) -> DriveResult {
        let engine = Engine::default();
        let module = Module::new(&engine, &self.wasm).expect("module compiles");
        let mock = self.mock.clone();
        let mut store = Store::new(&engine, mock.clone());
        let mut linker = Linker::new(&engine);

        linker
            .func_wrap(
                "env",
                "input",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, rid: i64| {
                    let bytes = caller.data().lock().unwrap().input.clone();
                    caller.data_mut().lock().unwrap().registers.insert(rid as u64, bytes);
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "read_register",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, rid: i64, ptr: i64| {
                    let data =
                        caller.data().lock().unwrap().registers.get(&(rid as u64)).cloned();
                    if let Some(d) = data {
                        let mem = caller.get_export("memory").unwrap().into_memory().unwrap();
                        let md = mem.data_mut(&mut caller);
                        let p = ptr as usize;
                        md[p..p + d.len()].copy_from_slice(&d);
                    }
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "register_len",
                move |caller: Caller<'_, Arc<Mutex<Mock>>>, rid: i64| -> i64 {
                    caller
                        .data()
                        .lock()
                        .unwrap()
                        .registers
                        .get(&(rid as u64))
                        .map(|d| d.len() as i64)
                        .unwrap_or(-1)
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "current_account_id",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, rid: i64| {
                    write_reg(&mut caller, rid as u64, ACCOUNT.as_bytes().to_vec());
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "predecessor_account_id",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, rid: i64| {
                    write_reg(&mut caller, rid as u64, PREDECESSOR.as_bytes().to_vec());
                },
            )
            .unwrap();
        linker
            .func_wrap("env", "block_timestamp", move || -> i64 { TS as i64 })
            .unwrap();
        linker
            .func_wrap(
                "env",
                "attached_deposit",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, ptr: i64| {
                    // 16 LE bytes of a zero deposit, written DIRECTLY to
                    // memory at ptr (real host shape; see deposit-gte emit)
                    let mem = caller.get_export("memory").unwrap().into_memory().unwrap();
                    let md = mem.data_mut(&mut caller);
                    md[ptr as usize..ptr as usize + 16].copy_from_slice(&[0u8; 16]);
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "storage_write",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, kl: i64, kp: i64, vl: i64,
                      vp: i64, _rid: i64|
                      -> i64 {
                    let k = read_mem(&mut caller, kp as usize, kl as usize);
                    let v = read_mem(&mut caller, vp as usize, vl as usize);
                    let existed =
                        caller.data_mut().lock().unwrap().storage.insert(k, v).is_some();
                    existed as i64
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "storage_read",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, kl: i64, kp: i64, rid: i64| -> i64 {
                    let k = read_mem(&mut caller, kp as usize, kl as usize);
                    let v = caller.data().lock().unwrap().storage.get(&k).cloned();
                    match v {
                        Some(d) => {
                            write_reg(&mut caller, rid as u64, d);
                            1
                        }
                        None => 0,
                    }
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "storage_remove",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, kl: i64, kp: i64, _rid: i64| -> i64 {
                    let k = read_mem(&mut caller, kp as usize, kl as usize);
                    caller.data_mut().lock().unwrap().storage.remove(&k).is_some() as i64
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "value_return",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, len: i64, ptr: i64| {
                    let d = read_mem(&mut caller, ptr as usize, len as usize);
                    let mut st = caller.data_mut().lock().unwrap();
                    if st.ret.is_none() {
                        st.ret = Some(d);
                    }
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "panic",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>| -> Result<(), wasmtime::Error> {
                    caller.data_mut().lock().unwrap().trapped = true;
                    Err(wasmtime::Error::msg("panic"))
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "panic_utf8",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>,
                      len: i64,
                      ptr: i64|
                      -> Result<(), wasmtime::Error> {
                    let d = read_mem(&mut caller, ptr as usize, len as usize);
                    let msg = String::from_utf8_lossy(&d).to_string();
                    let mut st = caller.data_mut().lock().unwrap();
                    st.trapped = true;
                    st.panic_msg = Some(msg.clone());
                    st.logs.push(msg);
                    Err(wasmtime::Error::msg("panic_utf8"))
                },
            )
            .unwrap();
        linker
            .func_wrap(
                "env",
                "log_utf8",
                move |mut caller: Caller<'_, Arc<Mutex<Mock>>>, len: i64, ptr: i64| {
                    let d = read_mem(&mut caller, ptr as usize, len as usize);
                    let msg = String::from_utf8_lossy(&d).to_string();
                    caller.data_mut().lock().unwrap().logs.push(msg);
                },
            )
            .unwrap();
        // promise stubs (not exercised in these tests)
        linker
            .func_wrap("env", "promise_batch_create", |_a: i64, _b: i64| -> i64 { 0 })
            .unwrap();
        linker
            .func_wrap("env", "promise_batch_action_transfer", |_a: i64, _b: i64| {})
            .unwrap();

        let instance = linker.instantiate(&mut store, &module).expect("instantiate");
        let f = instance
            .get_func(&mut store, method)
            .unwrap_or_else(|| panic!("export {} missing", method));

        {
            let mut st = mock.lock().unwrap();
            st.input = args_json.as_bytes().to_vec();
            st.trapped = false;
            st.ret = None;
            st.logs.clear();
        }
        let _ = f.call(&mut store, &[], &mut []);
        let st = mock.lock().unwrap();
        DriveResult { logs: st.logs.clone(), trapped: st.trapped, ret: st.ret.clone() }
    }
}

// ── Layer 1: stitcher e2e with official vectors ────────────────────────

const PROBE_TS: &str = r#"
export function probe(pk: string, sig: string, msg: string): number {
  return schnorrVerify(hexDecode(pk), hexDecode(sig), hexDecode(msg));
}
"#;

#[test]
fn stitcher_bip340_vector0_valid() {
    let wasm = compile_ts(PROBE_TS);
    // sanity: the crypto lib must actually be merged in
    assert!(
        wasm.len() > 60_000,
        "stitched lib missing: wasm only {} bytes",
        wasm.len()
    );
    let c = Contract::new(wasm);
    let r = c.call(
        "probe",
        &format!(r#"{{"pk":"{}","sig":"{}","msg":"{}"}}"#, V0_PK, V0_SIG, V0_MSG),
    );
    assert_eq!(r.ret_i64(), Some(1), "vector 0 must verify: logs={:?}", r.logs);
}

#[test]
fn stitcher_bip340_vector0_tampered_rejected() {
    let wasm = compile_ts(PROBE_TS);
    let c = Contract::new(wasm);
    let r = c.call(
        "probe",
        &format!(
            r#"{{"pk":"{}","sig":"{}","msg":"{}"}}"#,
            V0_PK,
            tamper(V0_SIG),
            V0_MSG
        ),
    );
    assert_eq!(r.ret_i64(), Some(0), "tampered sig must fail: logs={:?}", r.logs);
}

#[test]
fn stitcher_bip340_vector1_valid() {
    let wasm = compile_ts(PROBE_TS);
    let c = Contract::new(wasm);
    let r = c.call(
        "probe",
        r#"{"pk":"DFF1D77F2A671C5F36183726DB2341BE58FEAE1DA2DECED843240F7B502BA659","sig":"6896BD60EEAE296DB48A229FF71DFE071BDE413E6D43F917DC8DCF8C78DE33418906D11AC976ABCCB20B091292BFF4EA897EFCB639EA871CFA95F6DE339E4B0A","msg":"243F6A8885A308D313198A2E03707344A4093822299F31D0082EFA98EC4E6C89"}"#,
    );
    assert_eq!(r.ret_i64(), Some(1), "vector 1 must verify: logs={:?}", r.logs);
}

// ── Layer 2: nostr-gov digest chain (the real regression) ──────────────

#[test]
fn nostr_gov_owner_signature_recovers() {
    let ts_src =
        std::fs::read_to_string("projects/nostr-gov-lisp/src/main.ts").expect("main.ts readable");
    let wasm = compile_ts(&ts_src);

    let c = Contract::new(wasm);

    // init with the owner pubkey
    let r = c.call("init", &format!(r#"{{"npub":"{}"}}"#, OWNER_PK));
    assert!(
        !r.logs.iter().any(|l| l.starts_with("ERR_")),
        "init failed: {:?}",
        r.logs
    );

    // create_wallet with a VALID owner signature: must get past signature
    // verification and abort at the (zero) deposit gate, NOT at sig check.
    let r = c.call(
        "create_wallet",
        &format!(
            r#"{{"name":"satoshi","signature":"{}","expires_at":"{}","nonce":"7"}}"#,
            OWNER_SIG, EXPIRES
        ),
    );
    assert!(
        !r.logs.iter().any(|l| l == "ERR_INVALID_OWNER_SIGNATURE"),
        "valid owner sig rejected — digest chain broken: {:?}",
        r.logs
    );
    assert!(
        r.logs.iter().any(|l| l == "ERR_STORAGE_DEPOSIT"),
        "expected to reach deposit gate, logs: {:?}",
        r.logs
    );
}
