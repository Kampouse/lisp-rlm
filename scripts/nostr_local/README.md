# Local Nostr-from-Contract pipeline (near-mock + inlayer + lisp-rlm)

End-to-end rehearsal of: **smart contract → per-caller Nostr identity → signed
event → on-chain verify → real relay**. Runs 100% locally except the final
relay publish (real ws, trustless — the sig self-verifies).

## Components

| Piece | File | What it is |
|---|---|---|
| Worker wasm | `../probe/bip340_nostr.lisp` → `nostr_worker.wasm` | bip340 lib + 2 dispatcher cases (spliced by `splice_nostr.py`): **N** = derive `sk=SHA256(root‖0x1f‖caller) mod n` → `pk\|flip\|sk`; **E** = NIP-01 serialize → id → BIP-340 sig → `id\|sig` |
| Registrar | `registrar_main.ts` (TS dialect) | `post()` attests predecessor, `yieldCreate(on_sign)`; `on_sign` re-computes id (sha256), verifies sig via host `schnorrVerify` (k256), stores `pk:<caller>` + `last:<caller>`; `resume(data_id,payload)` calls `near.yieldResume`; `get_pk`/`get_last` views |
| Driver | `driver.py` | plays the OutLayer worker: near-mock cross calls, inlayer runs, resume; 7 assertions incl. tamper + replay rejection |
| Relay hop | `relay_hop.py` | re-runs worker with real unix ts, publishes to `wss://nos.lol`, expects `["OK",…,true]`, queries back by id |

## Run

```bash
cd /tmp/nostr_local   # or recreate: registrar needs types/lisp-rlm.d.ts
~/dev/lisp-rlm/target/release/near-compile \
  ../nostr_probe/bip340_nostr.lisp --target=outlayer-p2 -o nostr_worker.wasm
cd registrar && ~/dev/lisp-rlm/target/release/near-compile src/main.ts target/registrar.wasm
cd .. && python3 driver.py && python3 relay_hop.py
```

## Semantics proven (2026-09-21, driver 7/7)

1. **Caller binding**: contract attests `predecessorAccountId()`; the yield
   spec freezes (caller, content, pk) at post() time — the resuming party
   cannot change them (tamper probe died at `ERR_ID_MISMATCH` for wrong
   content, `ERR_SIG_INVALID` for corrupted sig).
2. **sk isolation**: derivation+signing inside wasm (TEE role). Contract
   state holds pk + sig only.
3. **On-chain verification**: host `schnorrVerify(hexDecode(pk),
   hexDecode(sig), hexDecode(id))` — decoded bytes, nostr-gov convention.
4. **One-shot yield**: `yieldResume` returns 0 on consumed handle →
   `ERR_RESUME_FAILED`; replay cannot re-store.
5. **NotReady leg**: mock runs the callback once with nil payload at post
   time — must early-return (log NOT_READY) WITHOUT dying or storing.
6. **Relay acceptance**: worker-signed event with real created_at accepted
   by nos.lol (id+sig valid); identical content+key → deterministic id.

## Mock-fidelity notes (near-mock specifics)

- `promise_yield_create` persists `\x00yield:<idx>` = account␟method␟creator␟args
  (0x1f-separated) in STATE — resume works across CLI invocations.
- Host 83 ABI: `(data_id_str, payload_str)` as strings; data_id "yd:<idx>"
  or "<idx>"; the idx IS the promise index (mock simplification).
- Payload reaches the callback via `promise_result(0)`; callback args stay
  as created (NOT payload-appended).
- nil checks: use `strLength(x) === 0` (dialect nil ≠ null; runtime `=` on
  strings is not content-compare).
- `schnorrVerify` builtin → host `schnorr_verify_bip340(pk*, sig*, msg*,
  msg_len)` — REAL k256, bounds-checked, panic-safe.

## Mainnet mapping

| Local | Mainnet |
|---|---|
| near-mock cross | real tx via any wallet/CLI |
| driver.py (this process) | OutLayer TEE worker (exec on outlayer.near) |
| ts="0" deterministic | TEE supplies real unix time (relays reject future ts) |
| resume() call | worker → `yield_resume` on the contract (NEAR yield/resume) |
| nos.lol publish | same, or any relay; the sig makes it trustless |
