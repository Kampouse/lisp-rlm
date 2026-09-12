//! # `MockChain` — the library face of near-mock
//!
//! In-process NEAR contract execution for tests and tooling:
//!
//!
//! ```no_run
//! use near_mock::chain::MockChain;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let chain = MockChain::builder()
//!     .contract("guestbook.test.near", "fixtures/guestbook.wasm")?
//!     .state_path("/tmp/guestbook.bin")      // omit → in-memory only
//!     .signer("alice.test.near")
//!     .now(1_788_000_000)                    // deterministic clock (unix secs)
//!     .build()?;
//!
//! let tx = chain
//!     .call("guestbook.test.near", "sign")
//!     .args("{}")
//!     .fire()?;
//! assert!(tx.ok);
//!
//! let view = chain
//!     .view("guestbook.test.near", "get_signature_count")
//!     .fire()?;
//! assert!(view.ok);
//!
//! chain.advance(3_600);                   // time-travel one hour (pinned clock)
//! chain.save()?;                          // explicit persist (needs state_path)
//! # Ok(())
//! # }
//! ```
//!
//! Design notes:
//! - The engine (promise DAG, modules, state) lives in thread-locals:
//!   **one chain per thread**. Tests: run each `MockChain` in its own `#[test]`.
//! - `call` = a NEAR transaction: atomic (entry trap or failed receipt chain
//!   rolls back everything; attached deposit refunds), while fire-and-forget
//!   orphan receipts commit independently — matching mainnet semantics.
//! - `view` = read-only: write hosts refuse, nothing persists.
//! - `fail_receipt(n)` mirrors the CLI's `--fail-receipt N` for testing
//!   rollback paths.

use std::sync::{Arc, Mutex};

use crate::{
    advance_time, engine_tls, execute_tx, host_trace_reset, install_sandbox, mock_state_view_get,
    mock_state_view_set, set_time_base, state_arc_tls, TxOutcome,
};

/// Result of one call — the CLI's outcome, minus the printing.
#[derive(Debug, Clone)]
pub struct CallOutcome {
    /// `true` = the entry (or final callback receipt) committed.
    pub ok: bool,
    /// Entry return-data when no promise was returned; otherwise the last
    /// receipt's result (NEAR transaction semantics).
    pub return_data: Option<Vec<u8>>,
    /// Receipt results in completion order (`None` = that receipt failed).
    pub receipt_results: Vec<Option<Vec<u8>>>,
    /// `true` when the ENTRY trapped (vs a receipt-chain failure).
    pub entry_trapped: bool,
    /// Error text (trap message / chain failure) when `!ok`.
    pub error: Option<String>,
    /// Normalized guest panic ("Smart contract panicked: <msg>", nearcore
    /// ExecutionError format) when the failure is a guest panic — comparable
    /// against mainnet receipt failures. `error` keeps the raw blob.
    pub panic: Option<String>,
    /// Normalized guest panics from PROMISE receipts of this tx (execution
    /// order). Compare against mainnet's same-tx receipt-subtree failures.
    pub receipt_failures: Vec<String>,
    /// Fire-and-forget receipts that failed (parent tx still commits).
    pub orphan_failures: usize,
    /// Gas burned by the entry call (PV155 units; 1e12 = 1 TGas). Receipt
    /// gas burns in per-receipt stores and is not included.
    pub gas_burned: u64,
    /// Raw log lines emitted during this tx (byte-exact, no debug suffixes).
    /// NEP-297 events are the `EVENT_JSON:`-prefixed entries.
    pub logs: Vec<String>,
}

impl From<TxOutcome> for CallOutcome {
    fn from(o: TxOutcome) -> Self {
        CallOutcome {
            ok: o.ok,
            return_data: o.return_data,
            receipt_results: o.receipt_results,
            entry_trapped: o.entry_trapped,
            error: o.error,
            panic: o.panic,
            receipt_failures: o.receipt_failures,
            orphan_failures: o.orphan_failures,
            gas_burned: o.entry_gas_burned,
            logs: o.logs,
        }
    }
}

impl CallOutcome {
    /// Return-data as UTF-8 (lossy) — what the CLI prints as `📄`.
    pub fn return_string(&self) -> Option<String> {
        self.return_data
            .as_ref()
            .map(|d| String::from_utf8_lossy(d).into_owned())
    }
}

/// Builder for [`MockChain`].
pub struct ChainBuilder {
    contracts: Vec<(String, Vec<u8>)>, // (account, wasm bytes)
    storage: std::collections::HashMap<Vec<u8>, Vec<u8>>,
    state_path: Option<String>,
    signer: String,
    now: Option<i64>,
    /// Fork-mode: page code + state lazily from an archival RPC at a pinned
    /// block ("anvil --fork-url" style). Contracts may be empty — code and
    /// storage arrive on demand; local writes shadow the fork.
    fork: Option<(String, Option<u64>)>,
    /// Unpinned fork: `finality: "final"` on every fetch (opt-in drift).
    fork_final: Option<String>,
}

impl Default for ChainBuilder {
    fn default() -> Self {
        Self {
            contracts: Vec::new(),
            storage: Default::default(),
            state_path: None,
            signer: "caller.test.near".into(),
            now: None,
            fork: None,
            fork_final: None,
        }
    }
}

impl ChainBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a contract from a wasm file.
    pub fn contract(
        mut self,
        account: &str,
        wasm_path: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("wasm for {account} ({wasm_path}): {e}"))?;
        self.contracts.push((account.into(), bytes));
        Ok(self)
    }

    /// Register a contract from raw wasm bytes.
    pub fn contract_bytes(mut self, account: &str, wasm: Vec<u8>) -> Self {
        self.contracts.push((account.into(), wasm));
        self
    }

    /// Start from a previously saved state file (`MockChain::save`,
    /// `near-mock state dump`, or `near-mock snapshot`). Missing file = fresh.
    pub fn state_path(mut self, path: &str) -> Self {
        self.state_path = Some(path.into());
        self
    }

    /// Default signer for `call`s (predecessor = signer, like a direct tx).
    pub fn signer(mut self, account: &str) -> Self {
        self.signer = account.into();
        self
    }

    /// Pin the block-timestamp base (unix seconds). Enables `advance`.
    pub fn now(mut self, unix_secs: i64) -> Self {
        self.now = Some(unix_secs);
        self
    }

    /// Fork mainnet (or any chain) at a block: contract code and storage are
    /// paged in lazily from `rpc` (archival) at `block` (None = latest,
    /// resolved once and pinned — deterministic per session).
    /// Local calls execute against that state; writes shadow it.
    pub fn fork(mut self, rpc: &str, block: Option<u64>) -> Self {
        self.fork = Some((rpc.to_string(), block));
        self
    }

    /// Fork with always-fresh state: every fetch uses `finality: "final"`
    /// (state may drift mid-session; reproducibility is traded for liveness).
    pub fn fork_final(mut self, rpc: &str) -> Self {
        self.fork_final = Some(rpc.to_string());
        self
    }

    pub fn build(self) -> Result<MockChain, Box<dyn std::error::Error>> {
        if let Some(rpc) = &self.fork_final {
            crate::set_fork_cfg(rpc.clone(), None);
        } else if let Some((rpc, block)) = &self.fork {
            let block = match block {
                Some(b) => Some(*b),
                None => Some(crate::fork_latest_block(rpc)?),
            };
            crate::set_fork_cfg(rpc.clone(), block);
        }
        let engine = std::rc::Rc::new(wasmtime::Engine::new(&crate::base_engine_config())?);

        let mut modules = std::collections::HashMap::new();
        for (acct, bytes) in &self.contracts {
            modules.insert(acct.clone(), crate::compile_module(&engine, bytes)?);
        }

        let storage: std::collections::HashMap<Vec<u8>, Vec<u8>> = match &self.state_path {
            Some(p) => std::fs::read(p)
                .ok()
                .and_then(|d| bincode::deserialize(&d).ok())
                .unwrap_or_default(),
            None => self.storage,
        };

        // TLS install — the same machinery the CLI uses.
        let state = install_sandbox(engine, modules, storage, false)?;

        if let Some(now) = self.now {
            set_time_base(now);
        }

        Ok(MockChain {
            state,
            state_path: self.state_path,
            signer: self.signer,
        })
    }
}

/// An in-process NEAR chain. **One per thread** (see module docs).
pub struct MockChain {
    state: Arc<Mutex<crate::state::MockState>>,
    state_path: Option<String>,
    signer: String,
}

impl MockChain {
    pub fn builder() -> ChainBuilder {
        ChainBuilder::new()
    }

    /// The chain this thread currently has installed (CLI-parity shortcut).
    pub fn current() -> Result<Self, Box<dyn std::error::Error>> {
        let state = state_arc_tls().ok_or("no MockChain installed on this thread")?;
        Ok(MockChain {
            state,
            state_path: None,
            signer: "caller.test.near".into(),
        })
    }

    /// Start building a transaction call.
    pub fn call(&self, contract: &'static str, method: &'static str) -> CallBuilder {
        CallBuilder {
            contract,
            method,
            args: "{}".into(),
            args_raw: None,
            signer: self.signer.clone(),
            attach: 0,
            view: false,
            fail_receipts: Vec::new(),
        }
    }

    /// Read-only view call: write hosts refuse, nothing persists.
    pub fn view(&self, contract: &'static str, method: &'static str) -> CallBuilder {
        let mut b = self.call(contract, method);
        b.view = true;
        b
    }

    /// Advance the deterministic clock. No-op unless `now` was pinned.
    pub fn advance(&self, secs: i64) {
        advance_time(secs);
    }

    /// Deliver queued receipts (see `fire_deferred`) in causal order, each
    /// atomically, against CURRENT state. Returns per-receipt results.
    pub fn settle(&self) -> Result<crate::SettleReport, Box<dyn std::error::Error>> {
        crate::settle_receipts()
    }

    /// Receipts queued by fire_deferred() and not yet settled.
    pub fn pending_receipts(&self) -> usize {
        crate::pending_receipt_count()
    }

    /// Persist current storage to `state_path`. Returns the key count.
    pub fn save(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let path = self
            .state_path
            .as_ref()
            .ok_or("MockChain::save: no state_path configured (builder .state_path())")?;
        let st = self.state.lock().unwrap();
        let n = st.storage.len();
        let encoded = bincode::serialize(&st.storage)?;
        std::fs::write(path, encoded)?;
        Ok(n)
    }

    /// Number of storage keys across all account partitions.
    pub fn storage_len(&self) -> usize {
        self.state.lock().unwrap().storage.len()
    }

    /// Read one storage key under an account's partition (NEAR trie model).
    pub fn storage_get(&self, account: &str, key: &[u8]) -> Option<Vec<u8>> {
        let full = crate::state::prefixed_key(account, key);
        self.state.lock().unwrap().storage.get(&full).cloned()
    }
}

/// Fluent call configuration (`MockChain::call` / `::view`).
pub struct CallBuilder {
    contract: &'static str,
    method: &'static str,
    args: String,
    /// Verbatim args (borsh/binary contracts like the aurora engine).
    /// Takes precedence over `args` when set.
    args_raw: Option<Vec<u8>>,
    signer: String,
    attach: u128,
    view: bool,
    fail_receipts: Vec<usize>,
}

impl CallBuilder {
    /// JSON method args (default `{}`).
    pub fn args(mut self, json: impl Into<String>) -> Self {
        self.args = json.into();
        self
    }

    /// Raw method args — verbatim bytes (binary/borsh contracts). Overrides
    /// any `.args()` JSON. Replay fidelity: byte-exact mainnet args.
    pub fn args_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.args_raw = Some(bytes);
        self
    }

    /// Override the signer for this call.
    pub fn from(mut self, account: impl Into<String>) -> Self {
        self.signer = account.into();
        self
    }

    /// Attach a deposit (decimal yocto), credited before the entry runs;
    /// refunded automatically if the transaction fails.
    pub fn attach(mut self, yocto: u128) -> Self {
        self.attach = yocto;
        self
    }

    /// Force receipt `idx` to fail (test rollback paths; CLI `--fail-receipt`).
    pub fn fail_receipt(mut self, idx: usize) -> Self {
        self.fail_receipts.push(idx);
        self
    }

    /// Execute the call but DEFER cross-contract receipts: the entry
    /// commits and its receipts queue for later `settle()` delivery —
    /// the chain's async-receipt model. Between fire and settle the
    /// intermediate state is observable (incident forensics: "round
    /// stuck in Rolling while MPC is silent").
    pub fn fire_deferred(self) -> Result<CallOutcome, Box<dyn std::error::Error>> {
        crate::set_defer_receipts(true);
        self.fire()
    }

    /// Execute the call.
    pub fn fire(self) -> Result<CallOutcome, Box<dyn std::error::Error>> {
        let state = state_arc_tls().ok_or("no MockChain on this thread (build one first)")?;
        let engine = engine_tls().ok_or("no MockChain engine on this thread")?;

        // View mode is a flag on the shared MockState; flip it for the
        // duration of a view call, restore whatever it was before.
        let saved_view = mock_state_view_get(&state);
        if self.view != saved_view {
            mock_state_view_set(&state, self.view);
        }
        host_trace_reset();
        let outcome = execute_tx(
            &engine,
            &state,
            self.contract,
            self.method,
            self.args_raw.as_deref().unwrap_or(self.args.as_bytes()),
            &self.signer,
            self.attach,
            &self.fail_receipts,
            self.view,
        );
        if self.view != saved_view {
            mock_state_view_set(&state, saved_view);
        }
        Ok(outcome?.into())
    }
}
