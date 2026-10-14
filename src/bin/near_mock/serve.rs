//! # `near-mock serve` — the mock chain as a NEAR-RPC + neardata + coordinator server
//!
//! Runs the production outlayer worker loop UNMODIFIED against a local mock
//! chain. One process, three protocols on one TCP port:
//!
//! 1. **NEAR JSON-RPC** (`POST /`): `block`, `query` (call_function /
//!    view_access_key / view_access_key_list / view_account),
//!    `broadcast_tx_commit` (borsh-decodes the real signed tx, executes each
//!    FunctionCall action against the mock, returns a wire-exact
//!    FinalExecutionOutcomeView).
//! 2. **neardata block feed** (`GET /v0/block/{height}`): replays the
//!    EVENT_JSON logs recorded from executed txs as
//!    `shards[].receipt_execution_outcomes[]` — exactly what the worker's
//!    EventMonitor scans on a real chain.
//! 3. **Coordinator stub** (`POST/GET /executions/*`, `/tasks/*`,
//!    `/workers/*`, heartbeats): `/executions/poll` serves the most recent
//!    `execution_requested` event; create/complete/heartbeat/cursor get 200.
//!
//! Execution model: the promise engine is one-chain-per-thread (TLS), so ALL
//! contract execution happens inline on the server's main thread. Requests are
//! serialized — fine for a local mock (the worker polls serially; the poll
//! endpoint answers immediately with 204/200, no long-poll).
//!
//! State persists to the state file after every committed tx, same as the CLI.
//!
//! Usage:
//! ```text
//! near-mock serve <state.bin> <acct=wasm,...> [--port 3030]
//! ```
//!
//! Worker env pointing at it:
//! ```text
//! NEAR_RPC_URL=http://localhost:3030
//! NEARDATA_API_URL=http://localhost:3030/v0/block
//! API_BASE_URL=http://localhost:3030
//! USE_TEE_REGISTRATION=false
//! OPERATOR_PRIVATE_KEY=ed25519:...   (any key; the mock decodes, doesn't verify)
//! ENABLE_EVENT_MONITOR=true
//! ```

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use super::chain::MockChain;

/// Recorded event feed: every committed tx appends (block_height, tx_hash,
/// executor_account, EVENT_JSON log lines). The neardata endpoint replays
/// them; /executions/poll serves the newest execution_requested.
struct EventRecord {
    block_height: u64,
    tx_hash: String,
    /// Receipt executor the events belong to (the outlayer contract account).
    receiver_id: String,
    logs: Vec<String>,
}

pub(crate) fn run_serve(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let usage = "usage: near-mock serve <state.bin> <acct=wasm,...> [--port 3030]";
    let mut pos: Vec<String> = Vec::new();
    let mut port: u16 = 3030;
    let mut it = args[2..].iter();
    while let Some(t) = it.next() {
        match t.as_str() {
            "--port" => {
                port = it
                    .next()
                    .ok_or("--port requires a number")?
                    .parse()
                    .map_err(|_| "--port must be a number")?;
            }
            other if other.starts_with("--") => {
                eprintln!("  ⚠ serve: ignoring flag {other}");
            }
            other => pos.push(other.to_string()),
        }
    }
    if pos.len() < 2 {
        eprintln!("{usage}");
        std::process::exit(1);
    }
    let state_path = pos[0].clone();
    let manifest = pos[1].clone();

    // Load each manifest contract via the ChainBuilder (the builder reads
    // + compiles the wasm itself). Accounts are leaked to satisfy
    // CallBuilder's &'static str contract — bounded set, server-lifetime.
    let mut builder = MockChain::builder().state_path(&state_path);
    let mut receivers: Vec<String> = Vec::new();
    for pair in manifest.split(',') {
        let Some((acct, path)) = pair.split_once('=') else {
            return Err(format!("manifest entries must be acct=/path, got '{pair}'").into());
        };
        builder = builder.contract(acct, path)?;
        receivers.push(acct.to_string());
    }
    let chain = builder.build()?;

    let records: Arc<Mutex<Vec<EventRecord>>> = Arc::new(Mutex::new(Vec::new()));
    let next_height = Arc::new(Mutex::new(mock_block_height()));

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!(
        "🛰  near-mock serve: http://127.0.0.1:{port}  (state: {state_path})"
    );
    eprintln!("   contracts: {}", receivers.join(", "));
    eprintln!("   NEAR_RPC_URL=http://127.0.0.1:{port}");
    eprintln!("   NEARDATA_API_URL=http://127.0.0.1:{port}/v0/block");
    eprintln!("   API_BASE_URL=http://127.0.0.1:{port}");

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = handle_conn(
            &mut stream,
            &chain,
            &records,
            &next_height,
            &receivers,
        );
    }
    Ok(())
}

fn handle_conn(
    stream: &mut TcpStream,
    chain: &MockChain,
    records: &Arc<Mutex<Vec<EventRecord>>>,
    next_height: &Arc<Mutex<u64>>,
    receivers: &[String],
) -> std::io::Result<()> {
    // Read the full request (headers + body). Local requests are small.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end;
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            // Parse Content-Length; keep reading until we have the body.
            let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let content_length = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let have = buf.len() - pos - 4;
            if have >= content_length {
                header_end = pos + 4;
                // keep exactly header_end + content_length bytes
                buf.truncate(header_end + content_length);
                break;
            }
        } else {
            header_end = 0; // unreachable; pacifies the borrow checker
            unreachable!();
        }
    }
    let head_end = find_subslice(&buf, b"\r\n\r\n").unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let body = buf
        .get(head_end + 4..)
        .map(|b| b.to_vec())
        .unwrap_or_default();

    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("").to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let response = route(&method, &path, &body, chain, records, next_height, receivers);
    write_response(stream, response.0, &response.1)
}

struct HttpResp(u16, String);

fn write_response(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

fn route(
    method: &str,
    path: &str,
    body: &[u8],
    chain: &MockChain,
    records: &Arc<Mutex<Vec<EventRecord>>>,
    next_height: &Arc<Mutex<u64>>,
    receivers: &[String],
) -> HttpResp {
    // ── neardata block feed ──
    if method == "GET" && path.starts_with("/v0/block/") {
        let height: u64 = path.rsplit('/').next().and_then(|h| h.parse().ok()).unwrap_or(0);
        return neardata_block(height, records, receivers);
    }
    // ── coordinator stub ──
    if path == "/executions/poll" {
        // Serve the newest recorded execution_requested request, or 204.
        let recs = records.lock().unwrap();
        for rec in recs.iter().rev() {
            for log in &rec.logs {
                if let Some(req) = extract_execution_request(log) {
                    eprintln!("📡 poll → request served (block {})", rec.block_height);
                    drop(recs);
                    return HttpResp(200, req);
                }
            }
        }
        return HttpResp(204, String::new());
    }
    if path == "/executions/create"
        || path == "/tasks/complete"
        || path.starts_with("/workers/")
        || path.contains("heartbeat")
    {
        // create → {"request_id":N,"created":false} (false = duplicate, no retry)
        if path == "/executions/create" {
            return HttpResp(
                200,
                r#"{"request_id":0,"created":false}"#.to_string(),
            );
        }
        if path == "/workers/block-cursor" {
            let h = *next_height.lock().unwrap();
            return HttpResp(200, format!(r#"{{"block_height":{h}}}"#));
        }
        return HttpResp(200, "{}".to_string());
    }
    if method == "GET" && path == "/" {
        return HttpResp(
            200,
            r#"{"service":"near-mock-serve","rpc":"/","neardata":"/v0/block/{h}"}"#
                .to_string(),
        );
    }
    // ── NEAR JSON-RPC ──
    if method == "POST" {
        let parsed: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(_) => return HttpResp(400, r#"{"error":"bad json"}"#.into()),
        };
        let rpc_method = parsed
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let params = parsed.get("params").cloned().unwrap_or(serde_json::Value::Null);
        let id = parsed.get("id").cloned().unwrap_or(serde_json::json!(0));
        let result = match rpc_method.as_str() {
            "block" => Some(rpc_block(next_height)),
            "query" => Some(rpc_query(&params, chain)),
            "broadcast_tx_commit" | "broadcast_tx_async" => {
                return rpc_broadcast_tx_commit(&params, chain, records, next_height, id);
            }
            "tx" | "EXPERIMENTAL_tx_status" => Some(rpc_tx_status(&params, chain)),
            "status" => Some(serde_json::json!({
                "chain_id": "near-mock",
                "latest_block_height": *next_height.lock().unwrap(),
            })),
            "health" => Some(serde_json::json!({})),
            other => {
                return HttpResp(
                    200,
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {"name": "HANDLER_ERROR", "code": -32601,
                                  "message": format!("near-mock serve: method {other} not implemented")},
                        "id": id,
                    })
                    .to_string(),
                )
            }
        };
        let j = serde_json::json!({"jsonrpc": "2.0", "result": result, "id": id});
        return HttpResp(200, j.to_string());
    }
    HttpResp(404, r#"{"error":"not found"}"#.into())
}

/// `block` — the mock's monotonically increasing height (bumps per tx).
fn rpc_block(next_height: &Arc<Mutex<u64>>) -> serde_json::Value {
    let h = *next_height.lock().unwrap();
    serde_json::json!({
        "header": {
            "height": h,
            "hash": bs58_hash(&format!("mock-block-{h}")),
            "prev_hash": bs58_hash(&format!("mock-block-{}", h.saturating_sub(1))),
        }
    })
}

/// Mock chain height: NEAR_MOCK_BLOCK_HEIGHT env pin, else a process-local
/// counter that starts at 1000 (the mock's genesis height) and bumps.
fn mock_block_height() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static HEIGHT: AtomicU64 = AtomicU64::new(1000);
    if let Ok(pin) = std::env::var("NEAR_MOCK_BLOCK_HEIGHT") {
        if let Ok(h) = pin.parse::<u64>() {
            return h;
        }
    }
    HEIGHT.load(Ordering::Relaxed)
}

fn bump_block_height() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static HEIGHT: AtomicU64 = AtomicU64::new(1000);
    HEIGHT.fetch_add(1, Ordering::Relaxed) + 1
}

/// Deterministic fake base58 hash (bs58 of 32 bytes derived from a label).
fn bs58_hash(label: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(label.as_bytes());
    bs58::encode(digest).into_string()
}

/// `query` — call_function (view) / view_access_key / view_access_key_list /
/// view_account.
fn rpc_query(params: &serde_json::Value, chain: &MockChain) -> serde_json::Value {
    use base64::Engine;
    let request_type = params
        .get("request_type")
        .and_then(|r| r.as_str())
        .unwrap_or("");
    let block_hash = bs58_hash("mock-view-block");
    match request_type {
        "call_function" => {
            let account = params
                .get("account_id")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string();
            let method = params
                .get("method_name")
                .and_then(|m| m.as_str())
                .unwrap_or("")
                .to_string();
            let args_b64 = params
                .get("args_base64")
                .and_then(|a| a.as_str())
                .unwrap_or("e30=");
            let args = base64::engine::general_purpose::STANDARD
                .decode(args_b64)
                .unwrap_or_default();
            let height = mock_block_height();
            let acct: &'static str = Box::leak(account.clone().into_boxed_str());
            let meth: &'static str = Box::leak(method.clone().into_boxed_str());
            match chain.view(acct, meth).args_bytes(args).fire() {
                Ok(outcome) => {
                    let result_bytes = outcome.return_data.unwrap_or_default();
                    serde_json::json!({
                        "block_hash": block_hash,
                        "block_height": height,
                        "logs": outcome.logs,
                        "result": result_bytes,
                    })
                }
                Err(e) => serde_json::json!({
                    "block_hash": block_hash,
                    "block_height": height,
                    "logs": [],
                    "error": format!("{e}"),
                }),
            }
        }
        "view_access_key" => {
            // Fixed nonce — the worker only needs it to build a signed tx;
            // the mock ignores nonces.
            serde_json::json!({
                "block_hash": block_hash,
                "block_height": mock_block_height(),
                "nonce": 1,
                "permission": "FullAccess",
            })
        }
        "view_access_key_list" => {
            serde_json::json!({
                "block_hash": block_hash,
                "block_height": mock_block_height(),
                "keys": [{
                    "access_key": {"nonce": 1, "permission": "FullAccess"},
                    "public_key": "ed25519:1111111111111111111111111111111111111111111111111111111111111111",
                }],
            })
        }
        "view_account" => {
            // Real state: balances live under `\0near-bal`; existence under
            // `\0acct-exists`. Accounts the server has seen (signers,
            // transfer targets, manifest contracts) report their actual
            // balance; never-seen accounts report the mock genesis grant so
            // worker startup checks don't fail on throwaway accounts.
            let account = params
                .get("account_id")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string();
            let bal = chain
                .storage_get(&account, b"\x00near-bal")
                .and_then(|v| String::from_utf8(v).ok())
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let exists = chain.storage_get(&account, b"\x00acct-exists").is_some();
            let amount = if bal > 0 {
                bal
            } else if exists {
                0
            } else {
                // unknown account → pretend genesis (lenient mock view)
                100_000_000_000_000_000_000_000_000_000_000
            };
            serde_json::json!({
                "block_hash": block_hash,
                "block_height": mock_block_height(),
                "amount": amount.to_string(),
                "locked": "0",
                "code_hash": bs58_hash("mock-code"),
                "storage_usage": 1000,
                "storage_paid_at": "0",
            })
        }
        _ => serde_json::json!({"error": format!("query type {request_type} not supported")}),
    }
}

/// `broadcast_tx_commit` — borsh-decode the signed tx, execute each
/// FunctionCall action inline, return a wire-exact FinalExecutionOutcomeView.
fn rpc_broadcast_tx_commit(
    params: &serde_json::Value,
    chain: &MockChain,
    records: &Arc<Mutex<Vec<EventRecord>>>,
    next_height: &Arc<Mutex<u64>>,
    id: serde_json::Value,
) -> HttpResp {
    use base64::Engine;
    let bytes_b64 = match params
        .get("signed_tx_base64")
        .and_then(|s| s.as_str())
    {
        Some(s) => s,
        None => {
            return HttpResp(
                200,
                serde_json::json!({"jsonrpc": "2.0", "id": id,
                    "error": {"name": "HANDLER_ERROR", "code": -32602,
                              "message": "missing signed_tx_base64"}})
                .to_string(),
            )
        }
    };
    let raw = match base64::engine::general_purpose::STANDARD.decode(bytes_b64) {
        Ok(b) => b,
        Err(e) => {
            return HttpResp(
                200,
                serde_json::json!({"jsonrpc": "2.0", "id": id,
                    "error": {"name": "HANDLER_ERROR", "code": -32602,
                              "message": format!("bad base64: {e}")}})
                .to_string(),
            )
        }
    };
    let Some(decoded) = decode_signed_tx(&raw) else {
        return HttpResp(
            200,
            serde_json::json!({"jsonrpc": "2.0", "id": id,
                      "error": {"name": "HANDLER_ERROR", "code": -32602,
                                "message": "signed tx borsh decode failed"}})
                .to_string(),
        );
    };

    // ── account semantics BEFORE execution (NEAR validates actions first) ──
    let acct = decoded.account_actions.clone();
    if let Err(e) = apply_account_semantics(chain, &decoded, &acct) {
        return HttpResp(
            200,
            serde_json::json!({"jsonrpc": "2.0", "id": id,
                "result": {"final_execution_status": "FINAL",
                    "status": {"Failure": {
                        "error_message": e,
                        "error_type": "ActionError",
                    }},
                    "transaction": null,
                    "transaction_outcome": null,
                    "receipts_outcome": []}})
                .to_string(),
        );
    }

    let signer_id = decoded.signer_id.clone();
    let mut status = serde_json::json!({"SuccessValue": ""});
    let mut all_logs: Vec<String> = Vec::new();
    let mut receipt_outcomes: Vec<serde_json::Value> = Vec::new();

    for (i, action) in decoded.actions.iter().enumerate() {
        let recv: &'static str = Box::leak(action.receiver_id.clone().into_boxed_str());
        let meth: &'static str = Box::leak(action.method_name.clone().into_boxed_str());
        let signer: &'static str = Box::leak(signer_id.clone().into_boxed_str());
        let outcome = chain
            .call(recv, meth)
            .args_bytes(action.args.clone())
            .from(signer)
            .attach(action.deposit)
            .fire();
        let outcome = match outcome {
            Ok(o) => o,
            Err(e) => {
                status = serde_json::json!({"Failure": {
                    "error_message": format!("{e}"),
                    "error_type": "ActionError",
                }});
                break;
            }
        };
        let kind = if outcome.ok {
            let ret = outcome.return_data.unwrap_or_default();
            // NEAR SuccessValue = base64 of the raw return bytes
            serde_json::json!({"SuccessValue": base64::engine::general_purpose::STANDARD.encode(&ret)})
        } else {
            serde_json::json!({"Failure": {
                "error_message": outcome.error.clone().unwrap_or_default(),
                "error_type": "GuestPanic",
            }})
        };
        all_logs.extend(outcome.logs.iter().cloned());
        receipt_outcomes.push(serde_json::json!({
            "block_hash": bs58_hash("mock-receipt-block"),
            "id": bs58_hash(&format!("mock-receipt-{}-{i}", action.method_name)),
            "proof": [],
            "outcome": {
                "proof": [],
                "block_hash": bs58_hash("mock-receipt-block"),
                "id": bs58_hash(&format!("mock-receipt-{}-{i}", action.method_name)),
                "receipts": [],
                "receipts_root": bs58_hash("mock-root"),
                "executor_id": action.receiver_id,
                "gas_burnt": outcome.gas_burned,
                "logs": outcome.logs,
                "metadata": {"gas_profile": null, "version": 3},
                "status": kind,
                "tokens_burnt": "0",
            },
        }));
        if outcome.ok {
            record_events(chain, records, next_height, &action.receiver_id, &outcome.logs);
        } else {
            break;
        }
    }

    let tx_hash = bs58_hash(&format!(
        "mock-tx-{}-{}",
        decoded.signer_id, decoded.nonce
    ));
    let height = mock_block_height();
    // Save state after every submitted tx (chain.call persists via MockChain
    // only if save() is called; the CLI contract = persist committed txs).
    let _ = chain.save();

    let view = serde_json::json!({
        "final_execution_status": "FINAL",
        "status": status,
        "transaction": {
            "signer_id": decoded.signer_id,
            "public_key": format!("ed25519:{}", bs58::encode(decoded.public_key).into_string()),
            "nonce": decoded.nonce,
            "receiver_id": decoded.receiver_id,
            "block_hash": bs58_hash("mock-block"),
            "actions": decoded.actions.iter().map(|a| serde_json::json!({
                "FunctionCall": {
                    "method_name": a.method_name,
                    "args": base64::engine::general_purpose::STANDARD.encode(&a.args),
                    "gas": a.gas,
                    "deposit": a.deposit.to_string(),
                }
            })).collect::<Vec<_>>(),
            "signature": format!("ed25519:{}", bs58::encode(decoded.signature).into_string()),
            "hash": tx_hash,
            "priority_fee": 0,
        },
        "transaction_outcome": {
            "block_hash": bs58_hash("mock-block"),
            "id": tx_hash,
            "proof": [],
            "outcome": {
                "executor_id": decoded.signer_id,
                "gas_burnt": 0,
                "logs": [],
                "metadata": {"gas_profile": null, "version": 3},
                "receipt_ids": receipt_outcomes.iter()
                    .filter_map(|r| r.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                    .collect::<Vec<_>>(),
                "status": {"SuccessReceiptId": receipt_outcomes.first()
                    .and_then(|r| r.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                    .unwrap_or_else(|| bs58_hash("mock-receipt-0"))},
                "tokens_burnt": "0",
            },
        },
        "receipts_outcome": receipt_outcomes,
    });
    let _ = height;
    let resp = serde_json::json!({"jsonrpc": "2.0", "result": view, "id": id});
    HttpResp(200, resp.to_string())
}

/// `tx` — return the recorded outcome of a submitted tx (worker resolves
/// receipts only right after broadcasting, so a synthetic success is fine).
fn rpc_tx_status(params: &serde_json::Value, chain: &MockChain) -> serde_json::Value {
    let hash = params
        .get(0)
        .and_then(|h| h.as_str())
        .unwrap_or("unknown")
        .to_string();
    let signer = params
        .get(1)
        .and_then(|s| s.as_str())
        .unwrap_or("system")
        .to_string();
    let _ = chain;
    serde_json::json!({
        "final_execution_status": "FINAL",
        "status": {"SuccessValue": ""},
        "transaction": {
            "signer_id": signer,
            "public_key": "ed25519:1111111111111111111111111111111111111111111111111111111111111111",
            "nonce": 1,
            "receiver_id": signer,
            "block_hash": bs58_hash("mock-block"),
            "actions": [],
            "signature": format!("ed25519:{}", bs58::encode([0u8; 64]).into_string()),
            "hash": hash,
            "priority_fee": 0,
        },
        "transaction_outcome": {
            "block_hash": bs58_hash("mock-block"),
            "id": hash,
            "proof": [],
            "outcome": {
                "executor_id": signer,
                "gas_burnt": 0,
                "logs": [],
                "metadata": {"gas_profile": null, "version": 3},
                "receipt_ids": [],
                "status": {"SuccessValue": ""},
                "tokens_burnt": "0",
            },
        },
        "receipts_outcome": [],
    })
}

/// Append a record for the neardata feed: only EVENT_JSON logs matter.
fn record_events(
    chain: &MockChain,
    records: &Arc<Mutex<Vec<EventRecord>>>,
    next_height: &Arc<Mutex<u64>>,
    receiver_id: &str,
    logs: &[String],
) {
    let event_logs: Vec<String> = logs
        .iter()
        .filter(|l| l.contains("EVENT_JSON:"))
        .cloned()
        .collect();
    if event_logs.is_empty() {
        return;
    }
    let mut h = next_height.lock().unwrap();
    *h = h.wrapping_add(1);
    let height = *h;
    drop(h);
    let _ = chain;
    let mut recs = records.lock().unwrap();
    recs.push(EventRecord {
        block_height: height,
        tx_hash: bs58_hash(&format!("mock-event-tx-{height}")),
        receiver_id: receiver_id.to_string(),
        logs: event_logs,
    });
}

/// neardata `/v0/block/{height}` — build the BlockData JSON from records at
/// that height. Unknown height → `null` body (neardata semantics).
fn neardata_block(
    height: u64,
    records: &Arc<Mutex<Vec<EventRecord>>>,
    receivers: &[String],
) -> HttpResp {
    let recs = records.lock().unwrap();
    let matching: Vec<&EventRecord> =
        recs.iter().filter(|r| r.block_height == height).collect();
    if matching.is_empty() {
        // Prior blocks "exist" but are empty (already consumed); the future
        // is null (block not indexed yet).
        let known_max = recs.iter().map(|r| r.block_height).max().unwrap_or(0);
        drop(recs);
        if height <= known_max {
            return HttpResp(200, r#"{"shards":[]}"#.to_string());
        }
        return HttpResp(200, "null".to_string());
    }
    let mut receipts = Vec::new();
    for rec in matching {
        for receiver in receivers {
            let _ = receiver; // all manifest accounts are valid executors
            receipts.push(serde_json::json!({
                "receipt": {
                    "receiver_id": rec.receiver_id,
                    "receipt_id": bs58_hash(&format!("mock-rcpt-{}", rec.tx_hash)),
                    "predecessor_id": "system",
                    "receipt": {"Action": {"signer_public_key": null, "signature": null}},
                },
                "execution_outcome": {
                    "id": bs58_hash(&format!("mock-outcome-{}", rec.tx_hash)),
                    "outcome": {
                        "logs": rec.logs,
                        "receipt_ids": [],
                        "gas_burnt": 300_000_000_000_000u64,
                    },
                },
                "tx_hash": rec.tx_hash,
            }));
            break; // one shard entry per record
        }
    }
    let body = serde_json::json!({
        "shards": [{"receipt_execution_outcomes": receipts}],
    });
    HttpResp(200, body.to_string())
}

/// Pull the inner request_data JSON out of an execution_requested EVENT_JSON
/// log, wrapped as the coordinator's ExecutionRequest the worker's poll
/// endpoint returns.
fn extract_execution_request(log: &str) -> Option<String> {
    let idx = log.find("EVENT_JSON:")?;
    let event: serde_json::Value =
        serde_json::from_str(&log[idx + "EVENT_JSON:".len()..]).ok()?;
    if event.get("event")?.as_str()? != "execution_requested" {
        return None;
    }
    let data0 = event.get("data")?.as_array()?.first()?.clone();
    let data_id: Vec<u8> = serde_json::from_value(data0.get("data_id")?.clone()).ok()?;
    let rd: serde_json::Value =
        serde_json::from_str(data0.get("request_data")?.as_str()?).ok()?;
    let request_id = rd.get("request_id")?.as_u64()?;
    // Coordinator ExecutionRequest shape (api_client.rs:63): the worker needs
    // request_id, data_id (hex), code_source, resource_limits, input_data,
    // response_format, context. Everything else has serde defaults.
    let req = serde_json::json!({
        "request_id": request_id,
        "data_id": hex_encode(&data_id),
        "code_source": rd.get("code_source")?,
        "resource_limits": rd.get("resource_limits")?,
        "input_data": rd.get("input_data").cloned().unwrap_or(serde_json::json!("")),
        "input_data_in_state": rd.get("input_data_in_state").and_then(|v| v.as_bool()).unwrap_or(false),
        "response_format": rd.get("response_format").cloned().unwrap_or(serde_json::json!("Text")),
        "context": {
            "sender_id": rd.get("sender_id"),
            "contract_id": null,
            "transaction_hash": null,
            "receipt_id": null,
            "predecessor_id": rd.get("predecessor_id"),
            "signer_public_key": null,
            "gas_burnt": null,
            "block_height": null,
            "block_timestamp": rd.get("timestamp"),
        },
        "near_payment_yocto": rd.get("payment"),
        "attached_usd": rd.get("attached_usd"),
        "compile_only": rd.get("compile_only").and_then(|v| v.as_bool()).unwrap_or(false),
        "force_rebuild": rd.get("force_rebuild").and_then(|v| v.as_bool()).unwrap_or(false),
        "store_on_fastfs": rd.get("store_on_fastfs").and_then(|v| v.as_bool()).unwrap_or(false),
    });
    Some(req.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ── borsh decoding of NEAR SignedTransaction ──

struct DecodedAction {
    receiver_id: String,
    method_name: String,
    args: Vec<u8>,
    gas: u64,
    deposit: u128,
}

/// Non-FunctionCall actions the mock actually executes (account semantics).
#[derive(Clone, Debug)]
enum AccountAction {
    CreateAccount,
    Transfer(u128),
    AddKey([u8; 32]),
    DeleteKey([u8; 32]),
    DeleteAccount(String),
}

struct DecodedTx {
    signer_id: String,
    public_key: [u8; 32],
    nonce: u64,
    receiver_id: String,
    signature: [u8; 64],
    actions: Vec<DecodedAction>,
    account_actions: Vec<AccountAction>,
}

struct BorshReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BorshReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return None;
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?.first().copied()?)
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes(b.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        let b = self.take(8)?;
        Some(u64::from_le_bytes(b.try_into().ok()?))
    }
    fn u128(&mut self) -> Option<u128> {
        let b = self.take(16)?;
        Some(u128::from_le_bytes(b.try_into().ok()?))
    }
    fn string(&mut self) -> Option<String> {
        let len = self.u32()? as usize;
        let b = self.take(len)?;
        Some(String::from_utf8_lossy(b).into_owned())
    }
    fn bytes32(&mut self) -> Option<[u8; 32]> {
        Some(self.take(32)?.try_into().ok()?)
    }
    fn bytes64(&mut self) -> Option<[u8; 64]> {
        Some(self.take(64)?.try_into().ok()?)
    }
    /// A borsh enum discriminant (u8 tag).
    fn tag(&mut self) -> Option<u8> {
        self.u8()
    }
}

/// Decode a borsh `SignedTransaction`: { transaction, signature } where
/// Transaction = { signer_id: String, public_key: (u8, [u8;32]), nonce: u64,
/// receiver_id: String, block_hash: [u8;32], actions: Vec<Action> } and
/// Signature = (u8 tag, [u8;64]).
fn decode_signed_tx(raw: &[u8]) -> Option<DecodedTx> {
    let mut r = BorshReader::new(raw);
    // transaction (first field)
    let signer_id = r.string()?;
    let _pk_tag = r.tag()?; // 0 = ED25519
    let public_key = r.bytes32()?;
    let nonce = r.u64()?;
    let receiver_id = r.string()?;
    let _block_hash = r.bytes32()?;
    let action_count = r.u32()? as usize;
    let mut actions = Vec::new();
    let mut account_actions: Vec<AccountAction> = Vec::new();
    for _ in 0..action_count {
        let tag = r.tag()?;
        match tag {
            2 => {
                // FunctionCall { method_name: String, args: Vec<u8>, gas: u64, deposit: u128 }
                let method_name = r.string()?;
                let args_len = r.u32()? as usize;
                let args = r.take(args_len)?.to_vec();
                let gas = r.u64()?;
                let deposit = r.u128()?;
                actions.push(DecodedAction {
                    receiver_id: receiver_id.clone(),
                    method_name,
                    args,
                    gas,
                    deposit,
                });
            }
            3 => {
                // Transfer { deposit: u128 }
                let deposit = r.u128()?;
                account_actions.push(AccountAction::Transfer(deposit));
            }
            0 => {
                // CreateAccount — no payload
                account_actions.push(AccountAction::CreateAccount);
            }
            1 => {
                // DeployContract { code: Vec<u8> }
                let len = r.u32()? as usize;
                r.take(len)?;
            }
            4 => {
                r.u128()?; // Stake { public_key(33? no—) , stake }
                // StakeAction { public_key: PublicKey, stake: u128 }
            }
            5 => {
                // AddKey { public_key, access_key } — pk = tag(1) + data(32)
                let _pk_tag = r.tag()?;
                let pk = r.bytes32()?;
                r.u64()?;
                let perm = r.tag()?;
                match perm {
                    0 => {} // FullAccess
                    1 => {
                        // FunctionCall permission: allowance: Option<u128>,
                        // receiver_id: String, method_names: Vec<String>
                        let has = r.tag()?;
                        if has == 1 {
                            r.u128()?;
                        }
                        r.string()?;
                        let names = r.u32()? as usize;
                        for _ in 0..names {
                            r.string()?;
                        }
                    }
                    _ => return None,
                }
                account_actions.push(AccountAction::AddKey(pk));
            }
            6 => {
                // DeleteKey { public_key } — pk = tag(1) + data(32)
                let _pk_tag = r.tag()?;
                let pk = r.bytes32()?;
                account_actions.push(AccountAction::DeleteKey(pk));
            }
            7 => {
                // DeleteAccount { beneficiary_id: String }
                let beneficiary = r.string()?;
                account_actions.push(AccountAction::DeleteAccount(beneficiary));
            }
            _ => return None, // Delegate(8) and beyond: unsupported
        }
    }
    // signature (second field)
    let sig_tag = r.tag()?;
    if sig_tag != 0 {
        return None; // only ED25519
    }
    let signature = r.bytes64()?;
    Some(DecodedTx {
        signer_id,
        public_key,
        nonce,
        receiver_id,
        signature,
        actions,
        account_actions,
    })
}

/// Access-key nonce record: `\0key:<raw pubkey>` → nonce (decimal string).
/// RAW key — `chain.storage_*` adds the account partition prefix itself.
fn key_key(_acct: &str, pk: &[u8; 32]) -> Vec<u8> {
    let mut k = b"\x00key:".to_vec();
    k.extend_from_slice(pk);
    k
}

const ACCT_EXISTS: &[u8] = b"\x00acct-exists";
const GENESIS_BAL: u128 = 100_000_000_000_000_000_000_000_000_000_000; // 100M NEAR

/// Account semantics for `broadcast_tx_commit` — real NEAR rules:
/// 1. signer account must exist
/// 2. the signing key must be registered on the signer (AddKey'd)
/// 3. nonce must exceed the key's current nonce
/// 4. CreateAccount must target a NON-existent account
/// 5. balances move: signer - (deposit + transfer amounts), receiver +
///    (implicit account creation on first transfer pays storage)
fn apply_account_semantics(
    chain: &MockChain,
    tx: &DecodedTx,
    account_actions: &[AccountAction],
) -> Result<(), String> {
    // ── signer existence ──
    let signer_bal = chain
        .storage_get(&tx.signer_id, b"\x00near-bal")
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| s.parse::<u128>().ok());
    let mut signer_bal = match signer_bal {
        Some(b) => b,
        None => {
            // Auto-fund KNOWN test accounts on first touch (the mock's
            // genesis grants every account it learns about); an unknown
            // signer with no record is still an error for DeleteAccount
            // flows only. Mirror lenient mock behavior: grant genesis.
            chain.storage_insert(&tx.signer_id, b"\x00near-bal", GENESIS_BAL.to_string().as_bytes());
            chain.storage_insert(&tx.signer_id, ACCT_EXISTS, b"1");
            GENESIS_BAL
        }
    };

    // ── key + nonce checks ──
    let kk = key_key(&tx.signer_id, &tx.public_key);
    let cur_nonce = chain
        .storage_get(&tx.signer_id, &kk)
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| s.parse::<u64>().ok());
    match cur_nonce {
        None => {
            // Key never registered — auto-register (mock leniency: the
            // InMemorySigner generates throwaway keys; requiring a prior
            // AddKey would break the worker out of the box).
            chain.storage_insert(&tx.signer_id, &kk, tx.nonce.to_string().as_bytes());
        }
        Some(n) => {
            if tx.nonce <= n {
                return Err(format!(
                    "nonce {} too low for key (current {n}) on {}",
                    tx.nonce, tx.signer_id
                ));
            }
            chain.storage_insert(&tx.signer_id, &kk, tx.nonce.to_string().as_bytes());
        }
    }

    // ── apply account actions in order ──
    for act in account_actions {
        match act {
            AccountAction::CreateAccount => {
                if chain.storage_get(&tx.receiver_id, ACCT_EXISTS).is_some() {
                    return Err(format!("account {} already exists", tx.receiver_id));
                }
                chain.storage_insert(&tx.receiver_id, ACCT_EXISTS, b"1");
            }
            AccountAction::Transfer(amt) => {
                if *amt > signer_bal {
                    return Err(format!(
                        "transfer {amt} exceeds signer balance {signer_bal} ({})",
                        tx.signer_id
                    ));
                }
                signer_bal -= *amt;
                let recv_bal = chain
                    .storage_get(&tx.receiver_id, b"\x00near-bal")
                    .and_then(|v| String::from_utf8(v).ok())
                    .and_then(|s| s.parse::<u128>().ok())
                    .unwrap_or(0);
                chain.storage_insert(
                    &tx.receiver_id,
                    b"\x00near-bal",
                    (recv_bal + *amt).to_string().as_bytes(),
                );
                chain.storage_insert(&tx.receiver_id, ACCT_EXISTS, b"1");
            }
            AccountAction::AddKey(pk) => {
                let kk = key_key(&tx.signer_id, pk);
                chain.storage_insert(&tx.signer_id, &kk, b"0");
            }
            AccountAction::DeleteKey(pk) => {
                let kk = key_key(&tx.signer_id, pk);
                chain.storage_remove(&tx.signer_id, &kk);
            }
            AccountAction::DeleteAccount(beneficiary) => {
                let bal = signer_bal;
                chain.storage_remove(&tx.signer_id, b"\x00near-bal");
                chain.storage_remove(&tx.signer_id, ACCT_EXISTS);
                let b_bal = chain
                    .storage_get(beneficiary, b"\x00near-bal")
                    .and_then(|v| String::from_utf8(v).ok())
                    .and_then(|s| s.parse::<u128>().ok())
                    .unwrap_or(0);
                chain.storage_insert(
                    beneficiary,
                    b"\x00near-bal",
                    (b_bal + bal).to_string().as_bytes(),
                );
            }
        }
    }

    // ── attached deposits (FunctionCall + Transfer): debit the signer ──
    for a in &tx.actions {
        if a.deposit > 0 {
            if a.deposit > signer_bal {
                return Err(format!(
                    "attached deposit {} exceeds balance {signer_bal}",
                    a.deposit
                ));
            }
            signer_bal -= a.deposit;
        }
    }
    chain.storage_insert(
        &tx.signer_id,
        b"\x00near-bal",
        signer_bal.to_string().as_bytes(),
    );
    Ok(())
}
