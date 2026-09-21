//! rust-twin-bench — near-sdk twin of lisp-rlm outlayer-oracle v2.
//! Identical storage keys, identical promise chain (100 Tgas fwd / 40 Tgas cb),
//! idiomatic near-sdk Rust. Purpose: A/B gas comparison vs the lisp-compiled wasm.

use near_sdk::env;
use near_sdk::{near, Gas, NearToken, Promise, PromiseOrValue, PromiseResult};

const ARGS: &str = "{\"source\":{\"WasmUrl\":{\"url\":\"https://raw.githubusercontent.com/Kampouse/lisp-rlm/main/examples/btc_avg_json.wasm\",\"hash\":\"0eeb1b8747d5080e1d69f26603c61576ddb51793323ca8f9b518eb28ebae116d\",\"build_target\":\"wasm32-wasip2\"}},\"resource_limits\":{\"max_instructions\":10000000000,\"max_memory_mb\":128,\"max_execution_seconds\":30},\"input_data\":\"{}\",\"response_format\":\"Text\"}";
const OUTLAYER: &str = "outlayer.testnet";
const DEPOSIT_YOCTO: u128 = 10_000_000_000_000_000_000_000; // 0.01 NEAR

fn get_str(key: &str) -> String {
    env::storage_read(key.as_bytes())
        .map(|v| String::from_utf8(v).unwrap_or_default())
        .unwrap_or_default()
}

fn get_count() -> u64 {
    get_str("count").parse().unwrap_or(0)
}

#[near(contract_state)]
pub struct Oracle;

impl Default for Oracle {
    fn default() -> Self {
        Oracle
    }
}

#[near]
impl Oracle {
    /// Bisect variants for harness gas accounting (bench-only).
    pub fn r0_empty(&mut self) {}

    pub fn r1_create(&mut self) {
        let _p = Promise::new(OUTLAYER.parse::<near_sdk::AccountId>().unwrap());
    }

    pub fn r2_one_action(&mut self) {
        let _p = Promise::new(OUTLAYER.parse::<near_sdk::AccountId>().unwrap()).function_call(
            "request_execution".parse::<String>().unwrap(),
            ARGS.as_bytes().to_vec(),
            NearToken::from_yoctonear(DEPOSIT_YOCTO),
            Gas::from_tgas(100),
        );
    }

    pub fn r3_two_actions(&mut self) {
        let _p = Promise::new(OUTLAYER.parse::<near_sdk::AccountId>().unwrap())
            .function_call(
                "request_execution".parse::<String>().unwrap(),
                ARGS.as_bytes().to_vec(),
                NearToken::from_yoctonear(DEPOSIT_YOCTO),
                Gas::from_tgas(100),
            )
            .function_call(
                "on_result".parse::<String>().unwrap(),
                vec![],
                NearToken::from_yoctonear(0),
                Gas::from_tgas(40),
            );
    }

    pub fn r4_then_no_return(&mut self) {
        let _cb = Promise::new(OUTLAYER.parse::<near_sdk::AccountId>().unwrap())
            .function_call(
                "request_execution".parse::<String>().unwrap(),
                ARGS.as_bytes().to_vec(),
                NearToken::from_yoctonear(DEPOSIT_YOCTO),
                Gas::from_tgas(100),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "on_result".parse::<String>().unwrap(),
                    vec![],
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            );
    }

    /// Promise chain → OutLayer request_execution → back to SELF.on_result.
    /// Mirrors lisp: fwd gas 100 Tgas, cb gas 40 Tgas, deposit 0.01 N.
    #[payable]
    pub fn refresh(&mut self) -> PromiseOrValue<()> {
        let cb = Promise::new(OUTLAYER.parse().unwrap())
            .function_call(
                "request_execution".parse::<String>().unwrap(),
                ARGS.as_bytes().to_vec(),
                NearToken::from_yoctonear(DEPOSIT_YOCTO),
                Gas::from_tgas(100),
            )
            .then(
                Promise::new(env::current_account_id()).function_call(
                    "on_result".parse::<String>().unwrap(),
                    vec![],
                    NearToken::from_yoctonear(0),
                    Gas::from_tgas(40),
                ),
            );
        PromiseOrValue::Promise(cb)
    }

    /// OutLayer's stdout lands here. Stores: oracle, ts, h:<i>, count (ring 0..49).
    pub fn on_result(&mut self) {
        match env::promise_result(0) {
            PromiseResult::Successful(bytes) => {
                let out = String::from_utf8_lossy(&bytes).to_string();
                let c = get_count();
                env::storage_write(b"oracle", out.as_bytes());
                env::storage_write(b"ts", env::block_timestamp().to_string().as_bytes());
                env::storage_write(format!("h:{c}").as_bytes(), out.as_bytes());
                env::storage_write(b"count", ((c + 1) % 50).to_string().as_bytes());
                env::log_str(&format!("outlayer-oracle v2: {out}"));
            }
            _ => env::panic_str("promise failed"),
        }
    }

    // ── views (mirror of lisp surface) ─────────────────────────────
    pub fn get_oracle(&self) -> String {
        get_str("oracle")
    }

    pub fn get_ts(&self) -> String {
        get_str("ts")
    }

    pub fn get_runs(&self) -> String {
        get_count().to_string()
    }

    /// {"ttl": "600"} — newest JSON if age <= ttl seconds, else {"error":"stale"}.
    pub fn get_fresh(&self, ttl: String) -> String {
        let out = get_str("oracle");
        let ts: u128 = match get_str("ts").parse() {
            Ok(v) => v,
            Err(_) => return "{\"error\":\"stale\"}".to_string(),
        };
        if out.is_empty() || ts == 0 {
            return "{\"error\":\"stale\"}".to_string();
        }
        let limit: u128 = ttl.parse().unwrap_or(0) * 1_000_000_000;
        let age = env::block_timestamp() as u128 - ts;
        if age > limit {
            "{\"error\":\"stale\"}".to_string()
        } else {
            out
        }
    }

    /// {"n": "5"} — newest first, "|" separated.
    pub fn get_history(&self, n: String) -> String {
        let n: u64 = n.parse().unwrap_or(0);
        let mut c = get_count() as i64 - 1;
        let mut parts: Vec<String> = vec![];
        while parts.len() < n as usize && c >= 0 {
            parts.push(get_str(&format!("h:{c}")));
            c -= 1;
        }
        parts.join("|")
    }
}
