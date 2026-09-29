# Local Nostr-from-Contract pipeline — SUPERSEDED (2026-09-29)

**This Python-driven pipeline is obsolete.** Signing now lives at the Rust
layer: `(schnorr-sign sk msg aux)` works in NEAR contracts AND P2 components
(self-contained, no two-mult limit, no A/B run split). The worker no longer
needs the 640KB generated kernel or the splice scripts — both removed.
A rewrite of this e2e rehearsal against the new API is the natural follow-up:
the worker collapses to ~20 lines of Lisp around `(schnorr-sign)`.

Kept here: `registrar_main.ts` (the contract — still valid) and the
**semantics proven** below (they carry over to any rewrite).

## Semantics proven (2026-09-21, driver 7/7)

1. **Caller binding**: contract attests `predecessorAccountId()`; the yield
   spec freezes (caller, content, pk) at post() time — the resuming party
   cannot change them (tamper died at `ERR_ID_MISMATCH` / `ERR_SIG_INVALID`).
2. **sk isolation**: derivation+signing inside wasm (TEE role). Contract
   state holds pk + sig only.
3. **On-chain verification**: host `schnorrVerify(hexDecode(pk),
   hexDecode(sig), hexDecode(id))` — decoded bytes, nostr-gov convention.
4. **One-shot yield**: `yieldResume` returns 0 on consumed handle →
   `ERR_RESUME_FAILED`; replay cannot re-store.
5. **NotReady leg**: mock runs the callback once with nil payload at post
   time — must early-return WITHOUT dying or storing.
6. **Relay acceptance**: worker-signed event with real created_at accepted
   by nos.lol (id+sig valid); identical content+key → deterministic id.

## Mock-fidelity notes (near-mock specifics) — still current

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
| OutLayer TEE worker (driver role) | exec on outlayer.near — now `(schnorr-sign)` in a P2 component |
| ts="0" deterministic | TEE supplies real unix time (relays reject future ts) |
| resume() call | worker → `yield_resume` on the contract (NEAR yield/resume) |
| nos.lol publish | same, or any relay; the sig makes it trustless |
