> **Status: RESOLVED — misdiagnosed as compiler, was a protocol violation** (2026-09-05 17:20)
>
> **Actual root cause:** the nostr-gov FE signed gov envelope content with REAL
> quotes; the sentinel protocol (e62452e) requires `~` in place of `"` so the
> contract's naive NIP-01 concat equals canonical JSON.stringify output. With
> bare quotes the concat can never match — ser short by exactly the content's
> quote count (the 36/40-byte deltas measured below), SIG_INVALID, deterministic
> per event. The "data-dependence" was: content-with-quotes (every gov envelope)
> vs content-without (create_wallet — always passed).
>
> Fix: FE sentinel-encodes content (nostr-gov 6ae22b6), watcher unsentinels
> before its envelope JSON.parse (same commit), sentineled propose verified ✅
> e2e on the twin. Separately, JP's a136fb7 fixed a REAL compiler bug in the
> same neighborhood (string literals double-unescaped by the emitter — pinned
> by test_json_set round-trip) — keep that fix, it was masking/mimicking here.
>
> --- archived original task below ---

# TASK: json_get_str drops quote chars from long escaped string values (envelope verify SIG_INVALID)

## Symptom (live + local, 100% repro)
`nostr-gov` gasless envelope calls (`propose`) panic `ERR_EVENT_SIG_INVALID`
for SOME events, deterministically per input. Root measurement (instrumented
twin + lisp probes): the contract's canonical event serialization comes out
**exactly N bytes short, N = number of `"` chars inside the `ct` argument
value** (observed: 440→404 with N=36; 516→476 with N=40). I.e. the ct string
that reaches `event-serialize`/`str-cat` has its quote characters REMOVED, so
`sha256(serialized) ≠ event id` and the schnorr check dies.

Canonical python check proves every input file is internally consistent
(`sha256(json.dumps([0,pk,cat,kind,tags,ct])) == ev` — YES for all fixtures),
and `test_verify_nostr` (crypto core: jsonGetStr + sha256Hash + hexDecode +
schnorrVerify on the same canonical 516-char string) PASSES. So the bug is
strictly in the input-scan/concat path for these values.

## 100% repro (near-vm-run, contract-ts twin)
```
VM=/Users/asil/dev/lisp-rlm/near-vm-run/target/release/near-vm-run
W=<nostr-gov>/contract-ts/target/nostr-gov-ts.wasm   # current main build
FIX=/Users/asil/.openclaw/workspace/lisp-rlm/tasks/fixtures
$VM $W reset
$VM $W init '{"npub":"<pk from fixture create>"}'        # matrix create: see matrix-create in /tmp/nostr-gov-debug (regenerate if lost)
$VM $W create_wallet '<matrix-create args>' --deposit 1.002   # PASSES (short content, no quotes)
$VM $W propose "$(cat $FIX/matrix-propose.json)"          # PANICS ERR_EVENT_SIG_INVALID, deterministic
# passing reference event (passed via identical flow, 2026-09-05): $FIX/saved-ev-1.json
```
Fixture generator (mint fresh signed events with nostr-tools):
`/tmp/nostr-gov-debug/matrix-mint.mjs` (fail) / `save-pass.mjs` (pass reference).

## Evidence trail (2026-09-05 session, /tmp/nostr-gov-debug/)
- ser.len deltas = ct quote count exactly (404 vs canonical 440; 476 vs 516).
- ser.mid shows tags embedded with RAW quotes; digest mismatch confirmed by
  instrumented twin logging (DBG lines).
- **Allocation-order sensitivity**: injecting `near.log` + `strSlice` calls
  between the concat and the hash in verifyOwnerEvent made a PASSING event
  FAIL — consistent with heap/buffer reuse clobbering in emitted `str-cat`
  chains or `sha256-hash` reading a reused block. Transport (shell argv vs
  python subprocess) also flipped one saved event (shell pass → python fail,
  same bytes) — pointing at environment/address layout, not JSON semantics.
- Plain short-content events (create_wallet, "nostr-gov owner action")
  ALWAYS verify — no quotes in content.

## Two suspect sites (bisect which)
1. `src/wasm_emit/json.rs` — `json_get_str` value scan for quoted strings:
   the `\"` escape-parity handling (value extraction after the colon,
   "Value shape fix 2026-08-30" region). Quote-dropping under specific byte
   patterns (fixture bisect: mutate input bytes with probe-style length
   checks — sig validity NOT required to isolate the scanner).
2. Emitted `str-cat` chain / heap allocation reuse (wasm_emit call path for
   string concat + `sha256-hash` input read). The instrumentation-flip
   evidence points here if the scanner proves clean on the failing fixture.

## The usual 4 places (landmine checklist)
1. src/wasm_emit/json.rs — scanner fix OR wasm_emit str-cat/alloc fix
2. Interpreter twin (bytecode/) — keep input-read parity
3. tests/ — regression: run the failing fixture end-to-end (needs a
   schnorr-verify round-trip like nostr-gov test_verify_nostr), plus a
   scanner-level test: `{"ct":"gov:{\"a\":\"b\"}"}` must read back with all
   quotes intact, at 150/186/258/298-char value lengths
4. Gate: nostr-gov contract-ts twin e2e propose with the fixture event must
   verify (nostr-gov repo, fresh state, near-vm-run flow above)

## User impact (why this is hot)
- Live: JP's real propose on bro.kampy.testnet failed on-chain with this bug
  (watcher submitted fine, contract rejected). Every gasless envelope action
  is a coin flip until fixed. Also FE ships a stale wasm bundle
  (nostr-gov/public/nostr-gov.wasm, Sep 3) — refresh after the fix + redeploy
  treasuries.
