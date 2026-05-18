# Lisp-RLM Browser Playground — Plan

A browser-based IDE where users write Lisp, compile to WASM client-side, and deploy/run on NEAR — supporting both on-chain smart contracts (P1) and off-chain wasi:http programs (P2), including hybrid programs that bridge both environments.

---

## Vision

**Write Lisp → Compile in Browser → Deploy to NEAR**

- **P1 (On-Chain):** Compile Lisp → NEAR smart contract WASM → deploy as a contract. Uses NEAR host functions (storage_read/write, context, crypto). Gas-metered, stateful on-chain.
- **P2 (Off-Chain):** Compile Lisp → wasi:http WASM → execute via OutLayer daemon. Can make HTTP requests, access NEAR storage through OutLayer API. Runs off-chain with on-chain settlement.
- **Hybrid Programs:** P2 fetches external data (APIs, prices, feeds) and feeds results into P1 contract storage. Oracle/bridge pattern — best of both worlds.

### Why Both Targets Matter

A real dApp needs both:
- On-chain contracts for trust-minimized state transitions and value transfers
- Off-chain workers for HTTP access, heavy computation, and cross-chain data

With hybrid programs, you write one Lisp program that declares which parts run where — the compiler handles the rest.

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│                  Browser App                     │
│                                                  │
│  ┌──────────┐   ┌────────────────────────────┐  │
│  │  Monaco   │   │  Lisp Compiler (WASM)      │  │
│  │  Editor   │──▶│  ┌──────────┐ ┌─────────┐  │  │
│  │          │   │  │ P1 Emit  │ │ P2 Emit │  │  │
│  │  .lisp   │   │  │ (NEAR)   │ │ (WASI)  │  │  │
│  │  files   │   │  └──────────┘ └─────────┘  │  │
│  └──────────┘   └────────────────────────────┘  │
│        │                   │                     │
│        │         ┌─────────┴─────────┐          │
│        │         ▼                   ▼          │
│        │   .wasm (P1)         .wasm (P2)        │
│        │         │                   │          │
└────────┼─────────┼───────────────────┼──────────┘
         │         │                   │
         │    ┌────▼────┐        ┌─────▼─────┐
         │    │  NEAR   │        │  OutLayer  │
         │    │  RPC    │        │  Daemon    │
         │    │         │        │  (wasi:http)│
         │    └─────────┘        └───────────┘
         │                            │
         │                       ┌────▼────┐
         │                       │  NEAR   │
         │                       │  RPC    │
         │                       │(settle) │
         │                       └─────────┘
         │
    ┌────▼──────────────────────────────┐
    │          NEAR Wallet              │
    │   (MyNearWallet / Meteor)         │
    └───────────────────────────────────┘
```

---

## Existing Code We Build On

### Already Working

| Component | Status | Location |
|-----------|--------|----------|
| Lisp parser & evaluator | ✅ 783 tests | `lisp-rlm/src/` |
| P1 WASM emitter (NEAR) | ✅ Working | `lisp-rlm/src/wasm_emit.rs` |
| P2 WASM emitter (WASI) | ✅ Working | `lisp-rlm/src/wasi_emit.rs` |
| P2 wasi:http emitter | ✅ 752 instr verified | `lisp-rlm/src/p2_direct.rs` |
| OutLayer adapter WIT | ✅ Working | `lisp-rlm/src/outlayer_adapter.rs` |
| P1 on-chain contract | ✅ Deployed | `near-lisp/` |
| InLayer CLI + daemon | ✅ Mainnet live | `near-inlayer/` |
| Multi-URL http-get | ✅ On-chain | N requests from Lisp source |

### Needs Building

| Component | Effort | Description |
|-----------|--------|-------------|
| Browser compiler (wasm-bindgen) | 2-3 days | Port `WasmEmitter` to compile in browser via WASM |
| Web frontend (IDE) | 3-4 days | Monaco editor, file tabs, output panel, deploy buttons |
| NEAR wallet integration | 1 day | MyNearWallet/Meteor for signing deployments |
| P1 contract factory | 2-3 days | Factory contract to deploy user-compiled WASM as new contracts |
| P2→P1 bridge (hybrid) | 1-2 days | P2 worker calls P1 contract after fetching data |
| Template gallery | 1 day | Pre-built examples: counter, oracle, cross-chain fetch |
| Hosting & CI | 1 day | Cloudflare Pages, auto-deploy |

---

## Build Plan (~10-12 Days)

### Phase 1: Browser Compiler (Days 1-3)

**Goal:** Lisp source → WASM binary, entirely in the browser.

**Why it works:** The `WasmEmitter` is pure Rust — no filesystem, no network, no OS deps. It takes a string and returns `Vec<u8>`. Perfect for `wasm-bindgen`.

**Tasks:**
1. Create `crates/browser-compiler/` with `wasm-bindgen` + `wasm-pack` setup
2. Expose two functions:
   - `compile_p1(source: &str) -> Result<Vec<u8>, String>` — NEAR contract WASM
   - `compile_p2(source: &str) -> Result<Vec<u8>, String>` — wasi:http WASM
3. Strip `wasmtime`, `tokio`, `reqwest`, `rustyline` deps from browser build (already behind `cfg(not(target_arch = "wasm32"))`)
4. Add `wasm-pack build --target web` to build pipeline
5. Test: compile a `(define (hello) "world")` program in browser, verify output WASM

**Pitfalls:**
- `wasm-encoder` and `wit-component` compile fine to `wasm32-unknown-unknown` — already verified
- `im::HashMap` (persistent data structures) works in WASM — already used in interpreter
- Must avoid `std::fs`, `std::net`, `std::time::Instant` in browser path — these are already gated behind native-only deps

### Phase 2: Web IDE (Days 3-6)

**Goal:** Functional editor with compile + output display.

**Tasks:**
1. Svelte/Vite app (lightweight, fast)
2. Monaco editor with Lisp syntax highlighting
3. File tabs (multi-file support)
4. Compile button → loads browser compiler WASM → runs `compile_p1` or `compile_p2`
5. Output panel: show WASM size, disassembly preview, any compiler errors
6. Download `.wasm` button
7. Template selector with 5-6 starter programs:
   - Counter (P1): storage-based increment
   - Greeter (P1): read/write greeting
   - HTTP Fetch (P2): single URL fetch
   - Price Oracle (P2): multi-source price comparison
   - Hybrid Oracle (P2→P1): fetch price → store on-chain
   - Cross-chain Reader (P2): fetch from multiple APIs, aggregate

### Phase 3: NEAR Integration (Days 6-8)

**Goal:** Deploy compiled WASM to NEAR from the browser.

**P1 Deploy Flow:**
1. Factory contract (`lisp-factory.testnet`) — pre-deployed, holds creation code
2. User compiles Lisp → gets WASM binary
3. `near-api-js` sends `deploy_contract` transaction with user's WASM
4. New contract lives at user's subaccount or a generated account

**P2 Execute Flow:**
1. User compiles Lisp → gets wasi:http WASM
2. Upload to IPFS or pass as base64 to OutLayer
3. OutLayer daemon executes, settles result on-chain
4. Frontend polls for result

**Tasks:**
1. Factory contract: stores WASM template, deploys user variants
2. `near-api-js` integration in frontend
3. NEAR wallet connection (MyNearWallet / Meteor)
4. P2 submit flow: upload WASM → execute → display result
5. Transaction history: show recent P1/P2 executions

### Phase 4: Hybrid Programs (Days 8-10)

**Goal:** P2 fetches data → writes to P1 contract, orchestrated from browser.

**Pattern:**
```lisp
;; hybrid-oracle.lisp
;; Runs as P2 (off-chain, can do HTTP)
(define (main)
  (let ((btc-price (http-get "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd"))
        (eth-price (http-get "https://api.coingecko.com/api/v3/simple/price?ids=ethereum&vs_currencies=usd")))
    ;; P2 can also call NEAR contracts through OutLayer API
    (storage-set "btc-usd" btc-price)
    (storage-set "eth-usd" eth-price)
    (string-append "Updated: BTC=" btc-price " ETH=" eth-price)))
```

**Tasks:**
1. Extend compiler to recognize `storage-*` calls in P2 mode → emit OutLayer adapter calls
2. Frontend: "Run Hybrid" button → compile as P2 → submit → show result
3. After P2 execution, read P1 contract state to verify storage was updated
4. Display both off-chain result and on-chain state change

### Phase 5: Polish & Deploy (Days 10-12)

**Tasks:**
1. Cloudflare Pages deployment
2. Custom domain (e.g., `lisp.near.dev` or `rlm.sh`)
3. CI: build compiler WASM + frontend on push
4. Mobile-responsive layout
5. Share button: encode Lisp source in URL hash for shareable links
6. Error UX: clear compiler errors with source location highlighting
7. Docs: README with architecture, how to add templates, API reference

---

## Technical Decisions

### Compiler in Browser (not server)

- **Zero infrastructure cost** — no backend needed
- **Privacy** — user code never leaves their browser
- **Offline capable** — works without network (compile only, not deploy)
- **Instant** — no round-trip to server for compilation
- **Possible because** the `WasmEmitter` is pure computation — no I/O

### Svelte over React

- Smaller bundle (~10KB vs ~40KB for React)
- Simpler state management for this scope
- Better DX for a focused tool (not a full platform)

### Factory Contract Pattern

Instead of deploying raw user WASM (which requires account creation), use a factory:
- Factory holds the account/subaccount logic
- Users get `username.lisp-factory.testnet` subaccounts
- Factory manages access control (only owner can update their contract)

### OutLayer for P2

Already working — `inlayer submit ./program.wasm` → daemon executes → settles on mainnet. Browser just needs to upload the compiled WASM (via API call to daemon or IPFS pin).

---

## File Structure (New)

```
lisp-rlm/
├── crates/
│   └── browser-compiler/        # NEW: wasm-pack compatible crate
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs           # expose compile_p1(), compile_p2()
├── web/                         # NEW: Svelte frontend
│   ├── package.json
│   ├── vite.config.ts
│   ├── src/
│   │   ├── App.svelte
│   │   ├── lib/
│   │   │   ├── compiler.ts      # wasm-pack glue
│   │   │   ├── near.ts          # near-api-js integration
│   │   │   └── templates.ts     # starter programs
│   │   └── components/
│   │       ├── Editor.svelte
│   │       ├── Output.svelte
│   │       └── Deploy.svelte
│   └── public/
├── contracts/
│   └── lisp-factory/            # NEW: factory contract
│       ├── Cargo.toml
│       └── src/
│           └── lib.rs
└── plan.md                      # This file
```

---

## Success Metrics

1. **Compile in browser:** Type Lisp → get WASM in <500ms
2. **Deploy P1:** One click → contract live on testnet in <10s
3. **Execute P2:** Submit → result displayed in <15s
4. **Hybrid:** Run oracle → see on-chain state update in <20s
5. **Shareable:** URL with encoded source opens exact same program
6. **Zero backend:** Only NEAR RPC + OutLayer daemon (already running)
