# TASK: Playground multi-contract sandbox — local promises & receipts

> **Status: DONE** — 4cdbc3f. Full history in `git log`.

## Goal
The browser playground can host MULTIPLE contracts (separate accounts), and
cross-contract promises (`near.callAwait`, `promiseCreate`+`promiseThen`,
async/await V1) execute LOCALLY between them — real receipt semantics, no
RPC. Deliverable: a two-contract example — NEP-141-style FT + a Lender that
locks collateral via a genuine `ft_transfer` promise call + callback.

## Current state (verified 2026-09-03)
- TS dialect surface EXISTS: `callAwait`, `promiseResult`, `promiseCreate/
  Then/And`, promise batches (transfer/actions), async/await V1 (entry +
  `__resume` continuation), yieldCreate/yieldResume. See ts/lisp-rlm.d.ts
  lines 174–250, ts_frontend.rs callAwait lowering (line ~3294).
- Browser mock (crates/browser-compiler/web-app/src/lib/compiler.ts):
  - `promise_create` → two-pass RPC VIEW call against real mainnet/testnet
    (nearPass 0 queues → results fetched → pass 1 re-runs whole function).
  - `promise_then` → logs "not supported in view-only mode", no-op.
  - `promise_and` → returns counter. `promise_result` reads nearPromiseResults.
  - Single-contract assumption throughout runNear (one wasm, one storage map,
    localStorage key 'near_mock_storage').

## Design
### Contract registry
- `MockContract { accountId, wasm, instance, storage: Map, registers,
  logs, returnValue }` — per-contract state, namespaced.
- Reserved accounts: contracts bind to `.<name>.pg` accounts; storage per
  contract. Default single-contract behavior unchanged (back-compat).

### Receipt engine (replaces two-pass hack for local targets)
- Host `promise_create/then` interception: if target accountId is a LOCAL
  contract → schedule a local receipt; else fall through to existing RPC
  view path (keep live-chain views working).
- Execution model (matches nearcore): current function runs to completion,
  scheduled receipts drain AFTER return, in order; callback receipts run
  the callback export on the ORIGINATING contract with promise results
  preloaded (nearPromiseResults), fresh input args. Loop until queue empty.
- Receipt trace surfaced in UI (target/method/args/ret/success chain).

### Runtime pieces
- promise_result(idx): reads from the promise-results register of the
  CURRENTLY EXECUTING receipt (per-contract, not global).
- promise_and: AND-combine — all inputs must succeed (AND promise succeeds
  iff all succeeded; result = last successful output like nearcore).

### UI (minimal Phase 1)
- examples.ts: new optional field `sidecar?: { name, source, account }[]`.
- App: collapsible "Sidecar contracts" panel (Monaco-lite textarea ok for
  P1), compiled alongside main, bound to accounts, shown in receipt trace.
- Run: unchanged UX — executes selected method on MAIN contract; receipts
  cascade into sidecars automatically.

### Phases
- P1: registry + local promise execution + callAwait callbacks + two-
  contract FT/Lender example. (async/await V1 & yield can follow.)
- P2: promise batches with local transfer actions; async/await `__resume`
  flows in sandbox; promise_and joins end-to-end.
- P3: examples/docs polish — NEP-141 ft_on_transfer pattern example.

## Verification (must pass before deploy)
- Gate: `cargo test --test test_playground_examples` green (main source).
- Node harness: compile FT + Lender, instantiate both, run Lender flow —
  assert ft_transfer promise executed on FT contract (balance moved on FT
  storage, NOT lender storage), callback read result via promiseResult(0),
  failure path (abort in callee) fails closed in callback (NIL → abort).
- Battery: cargo test full — 0 failures.
- Deploy JS-only; md5 wasm unchanged; live smoke via harness.

## Landmines
- memory.grow detaches views — always re-derive Uint8Array(memory.buffer).
- Register-based host convention (2026-09-03 fix) — new hosts MUST match.
- panic_utf8 import now required by abort path — provide in every env.
- No backticks in examples.ts template literals (build breaker).
- Two-pass RPC path must keep working for real-chain view calls.
