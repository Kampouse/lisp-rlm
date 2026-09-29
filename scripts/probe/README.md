# scripts/probe — BIP-340 pure-Lisp signer probe

Working proof that **BIP-340 Schnorr signing (and Nostr event signing) runs
entirely in Lisp** on the P2/outlayer runtime — no stitched Rust lib, no host
crypto. Big, unrolled, machine-generated on purpose (see "Why is it so big").

## Files

| File | What it is |
|---|---|
| `bip340.lisp` | Base generated lib: secp256k1 field/scalar arithmetic (Montgomery, nine 30-bit limbs), Lisp SHA-256, Jacobian point ops, phase-A/phase-B sign pipeline, debug-leg dispatcher |
| `bip340_nostr.lisp` | **Generated** = `bip340.lisp` + Nostr cases (`N`=78 derive per-caller key, `E`=69 NIP-01 event id + sig). Committed copy is the **v1** splice artifact — do not hand-edit |
| `bip340_verify.lisp` | Verify-only variant (pk/r/s/msg in → 1/0) |
| `shamod.lisp` | Shared SHA-256 module the generators started from |
| `bip340-runner.py` | Drives the compiled wasm via `inlayer run`: `sign` (2 runs: A then B), `verify`, `roundtrip`. Expects `/tmp/nostr_probe/bip340{,_verify}.wasm` |
| `interp_mul.py`, `iso_battery.py`, `probe_ptadd.py` | Re-runnable dev probes used to validate fe-mul / point-add against Python references while building the lib |
| `archive/` | One-off patch scripts (`fix_*.py`, already baked into `bip340.lisp`) and stale run logs (`bip_runQ*.log`) |

## Generation chain

```
scripts/probe/bip340.lisp  ──cp──▶  /tmp/nostr_probe/bip340.lisp
        │
        ▼  python3 scripts/nostr_local/splice_nostr.py      (v1: +N/+E cases)
/tmp/nostr_probe/bip340_nostr.lisp  ──cp──▶  scripts/probe/bip340_nostr.lisp   (committed)
        │
        ▼  python3 scripts/nostr_local/splice_nostr_v3.py   (v3: +sealed-root ops, in place)
/tmp/nostr_probe/bip340_nostr.lisp  ──near-compile --target=outlayer-p2──▶  nostr_worker.wasm
```

The v3 ops (`c`=99 claim-root, `i`=105 init, `d`=100 derive, `s`=115 sign —
root/sk never leave storage) exist **only in the /tmp variant** used to build
`nostr_worker.wasm` for the `scripts/nostr_local/` end-to-end pipeline. In the
committed v1 copy, csw 100/115 are the lowercase debug cases (sha-96 /
scalar-mul), not the sealed-root ops.

Regen the committed artifact:

```bash
mkdir -p /tmp/nostr_probe && cp scripts/probe/bip340.lisp /tmp/nostr_probe/
python3 scripts/nostr_local/splice_nostr.py
cp /tmp/nostr_probe/bip340_nostr.lisp scripts/probe/
```

## Why is it so big (~640 KB)?

Deliberate, load-bearing generation choices — do not "simplify":

- **Fully unrolled SHA-256 rounds** (`sha-h1`/`sha-h2`) for the fixed 96-byte
  tagged-hash inputs: loops with mults trip the runtime's two-mult limit, so
  the block compression is unrolled.
- **Two SHA-256 stacks** (streaming `blocks`/`hadd` + unrolled `sha-h1/h2`)
  and duplicated constant tables — each exists for a different call site shape.
- **9×30-bit limb math with everything inlined**: the dialect has no bignums,
  so every field op is emitted as straight-line `set!` code.

## Status (last verified 2026-09-27, logs in `archive/`)

> **2026-09-29: SIGNING NO LONGER NEEDS THIS KERNEL.** The Rust layer now
> does it all — `schnorr_sign` in `schnorr/src/lib.rs` was fixed (mod-P vs
> mod-n bug + odd-y negation) and exported as `schnorr_sign_bip340`, so Lisp
> contracts call `(schnorr-sign sk msg aux)` directly (71KB stitched lib,
> byte-identical to the official BIP-340 vectors; see
> `tests_p2/test_schnorr_sign.lisp`). This directory is now a historical
> probe — kept as evidence and as a pure-Lisp fallback. The `nostr_local`
> pipeline can migrate to `(schnorr-sign ...)` and drop the A/B two-run
> split entirely (native wasm has no two-mult limit).

- Full sign (case `S`, two-phase A→B) **matches** a Python reference
  byte-for-byte; `s·G == R + e·P` verifies True.
- Real Nostr event (case `E`): event id + sig accepted by `nos.lol`
  (`["OK",…,true]`), id deterministic for identical content+key.
- Known-benign debug-leg discrepancies (`kflip`, `KP.y`, `P.y` report 0 /
  pre-flip values; `ssum` low limbs in Q8) — intermediates only, final sigs
  correct. If a leg regresses beyond these, something actually broke.
- Single-run full sign (both EC mults in one execution) still TRAPs on the
  two-mult runtime limit — that is why signing is split A/B.

Related: `scripts/nostr_local/README.md` (contract → derive → sign → on-chain
verify → relay pipeline), `src/builtin_schnorr.rs` (host-side verify only),
`schnorr/` (stitched Rust lib — verify works, sign is broken there, see
GAPS.md).
