# lisp-rlm OutLayer Integration Plan

## ✅ Progress (May 13, 2026)

### Done:
- **finish_outlayer()** implemented in `wasi_emit.rs` (~300 lines)
- WASI Preview 1 imports: fd_read, fd_write, proc_exit, random_get, environ_sizes_get, environ_get, fd_seek
- OutLayer host imports: view, call, transfer
- NEAR host function stubs imported from "env" (same module, no dispatch changes needed)
- `_start()` wrapper: stdin → iov → fd_read → call user func → untag → fd_write → proc_exit
- WasmEmitter fields made `pub(crate)` for cross-module access
- tree_shake, resolve_static, HOST_FUNCS made `pub(crate)`
- **Tests passing**: simple (square), counter (with storage ops)
- Lib builds clean (warnings only, no errors)

### Architecture
```
lisp source (.lisp)
       │
       ▼
   [parser + typecheck]  ← shared
       │
       ▼
   [WasmEmitter]          ← shared core (near/* calls unchanged)
       │
       ├── finish()           → NEAR contract (existing)
       └── finish_outlayer()  → WASI P1 binary for OutLayer
```

### What OutLayer WASI P1 provides:
- stdin/stdout for raw i64 I/O
- random_get() for randomness
- environ_get() for secrets + NEAR context
- Host functions: view(), call(), transfer() via imports

### API Mapping (NEAR → OutLayer) — NEXT STEP

| NEAR Function        | OutLayer Equivalent                          |
|----------------------|----------------------------------------------|
| near/storage_get     | Stub → needs mapping to view() or KV         |
| near/storage_set     | Stub → needs mapping to call() or KV         |
| near/storage_remove  | Stub → needs mapping to call()               |
| near/storage_has_key | Stub → needs mapping to view()               |
| near/input           | stdin (fd_read) ✅                           |
| near/return          | stdout (fd_write) ✅                         |
| near/log             | stderr (fd_write to fd 2) — TODO             |
| near/random_seed     | random_get() — TODO                         |
| near/block_index     | environ NEAR_BLOCK_HEIGHT — TODO            |
| near/signer_account_id | environ NEAR_SENDER_ID — TODO             |
| near/current_account_id | environ NEAR_CONTRACT_ID — TODO          |
| near/sha256          | Pure WASM implementation — TODO             |
| near/promise_*       | call() / transfer() — TODO                  |

### Files Created/Modified

1. **src/wasi_emit.rs** (REWRITTEN, ~300 lines)
   - finish_outlayer() — full WASI P1 binary emission
   - _start() entry point with fd_read/fd_write/proc_exit
   - WASI + OutLayer import descriptors
   - NEAR host stub type mapping
   - Tests: test_outlayer_simple, test_outlayer_counter

2. **src/wasm_emit.rs** (MINOR CHANGES)
   - WasmEmitter fields: pub(crate)
   - FuncDef struct: pub(crate)
   - HOST_FUNCS const: pub(crate)
   - tree_shake(): pub(crate)
   - resolve_static → resolve_static_pub: pub(crate)

3. **OUTLAYER_INTEGRATION.md** (UPDATED)
   - Progress tracking

### Next Steps

1. **Test with wasmtime** — load the WASM and verify _start works (mock stdin/stdout)
2. **Remap NEAR host stubs → OutLayer** — replace env imports with actual OutLayer host calls
   - near/storage_* → outlayer.view/call with KV contract
   - near/log → fd_write(fd=2) to stderr
   - near/random_seed → random_get()
   - near/block_index, signer, etc → environ_get()
3. **JSON string I/O** — for complex contracts, parse JSON from stdin, serialize result
4. **near-compile --target outlayer** CLI flag
5. **Integration test with real OutLayer API**
