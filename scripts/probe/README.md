# scripts/probe — BIP-340 pure-Lisp signer probe (DEPRECATED KERNEL)

> **2026-09-29: signing lives at the Rust layer now.** `(schnorr-sign sk msg aux)`
> in any target (NEAR + P2 components) produces byte-exact BIP-340 signatures
> from the 71KB stitched lib — no kernel, no splicers, no two-run A/B split.
> See `tests_p2/test_schnorr_sign_p2.lisp` and GAPS.md (2026-09-29 entries).
> This directory is kept as historical evidence of the pure-Lisp approach.

## Files

| File | What it is |
|---|---|
| `bip340.lisp` | Base generated lib: secp256k1 field/scalar arithmetic (Montgomery, nine 30-bit limbs), Lisp SHA-256, Jacobian point ops, phase-A/phase-B sign pipeline, debug-leg dispatcher |
| `bip340_nostr.lisp` | Generated = `bip340.lisp` + Nostr cases (`N`=78 derive, `E`=69 NIP-01 event id + sig). Committed copy is the v1 splice artifact — do not hand-edit |
| `bip340_verify.lisp` | Verify-only variant (pk/r/s/msg in → 1/0) |
| `shamod.lisp` | Shared SHA-256 module the generators started from |
| `bip340_ref.py` | Pure-stdlib BIP-340 reference signer — ground-truth checker for any future crypto debugging (`bip340_ref.py <sk_hex> <msg_hex> [aux_hex]`) |
| `archive/` | Historical run logs (`bip_runQ*.log`) |

The generator/patcher scripts (`gen_bip340*.py`, `fix_*.py`, probes) were
removed 2026-09-29 — recoverable from git history (commit before `cleanup`)
if the kernel ever needs regeneration.

## Why the kernel is so big (~640 KB) — deliberate, load-bearing

- **Fully unrolled SHA-256 rounds** — loops with mults trip the emitted-Lisp
  runtime's two-mult limit, so block compression is unrolled
- **Two SHA-256 stacks** (streaming + unrolled) for different call-site shapes
- **9×30-bit limb math fully inlined** — the dialect has no bignums

None of this applies to the Rust layer (native wasm has no such limit),
which is why the kernel is obsolete.

## Status (last verified 2026-09-27, logs in `archive/`)

- Full sign (case `S`, two-phase A→B) matched a Python reference byte-for-byte;
  `s·G == R + e·P` verified True; real Nostr event accepted by `nos.lol`
- Known-benign debug-leg discrepancies (`kflip`, `KP.y`, `P.y`, `ssum`) —
  intermediates only, final sigs were correct

Related: `src/builtin_schnorr.rs` (host-side verify), `schnorr/` (the
Rust lib — verify AND sign, official vectors green).
