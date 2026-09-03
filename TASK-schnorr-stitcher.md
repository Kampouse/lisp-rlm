# TASK: Fix schnorr stitcher regression — call site vanishes, lib never merged

## Summary
`(schnorr-verify pk sig msg)` through the CURRENT wasm_emit produces a wasm
where (a) the 768KB stitched schnorr library is NOT merged and (b) the call
site is MISSING from the user function's body entirely. Result: schnorrVerify
silently returns 0 for valid BIP-340 signatures. The interpreter path is
correct (official vectors pass in the battery); this is wasm_emit-only.

Regression window: the wasm_emit rewrite blitz Aug 30–31 (after commit
dab0602 "TS frontend M3 — nostr-gov 22/22" which was last known-green).

## Forensic evidence already gathered (2026-09-02, verified with debug prints)
1. Repro: compile this and run through near-mock — returns 0, must return 1:
   ```ts
   export function probe(pk: string, sig: string, msg: string): number {
     return schnorrVerify(hexDecode(pk), hexDecode(sig), hexDecode(msg));
   }
   ```
   Args (official BIP-340 vector 0):
   pk=D69C3509BB99E412E68B0FE8544E72837DFA30746D8BE2AA65975F29D22DC7B9
   sig=E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DBA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0
   msg=0000000000000000000000000000000000000000000000000000000000000000
   Run: `./target/release/near-mock /tmp/….wasm probe '<json>' --view`
2. The `schnorr-verify` arm in call_near_crypto.rs DOES fire (3 args).
3. `finish()` reports wasm_imports=["schnorr_verify_bip340"] — registration works.
4. But `wasm2wat` of the final module shows: only 41 funcs, NO function with
   type (i32,i32,i32,i32)->i32 anywhere, and the exported `probe` body calls
   only the input-parser + value_return — the schnorr call site is GONE.
5. The emitted body contains Call(WASM_IMPORT_BASE | 0) sentinel calls
   (WASM_IMPORT_BASE = 0xFF02_0000, mod.rs:175). Somewhere between emission
   and module finish/stitch, those call sites are dropped or the body is
   re-emitted without them. Suspects: the TS input-wrapper rewrite, tree
   shaking, or the sentinel→real-index remap in finish()/link path silently
   discarding them (hard-error policy violation: should never vanish
   silently).
6. link_schnorr_wat → merge_lib_wasm_multi should merge
   src/wasm_emit/schnorr.wat (768KB) into the module — final wasm is 66991
   bytes, so the merge either didn't run or dropped the lib.

## Your job
1. Failing test FIRST: a Rust test (tests/ dir) compiling the probe TS and
   running it through near-mock style instantiation asserting 1. Use the
   official vector above (machine-verify constants with python hashlib /
   BIP-340 reference — do NOT hand-type them).
2. Root-cause and fix the two failures: vanishing call site + missing merge.
   Likely files: src/wasm_emit/wasm_link.rs, src/wasm_emit/compile.rs
   (finish/stitch path), src/wasm_emit/call_near_crypto.rs (arm), possibly
   the WASM_IMPORT_BASE sentinel remap in helpers.rs / emit paths.
3. `cargo test --release` battery green (1576/0 as of bf02ea8).
4. Then rebuild + run the nostr-gov gauntlet:
   ```
   ./target/release/near-compile projects/nostr-gov-lisp/src/main.ts -o projects/nostr-gov-lisp/target/nostr-gov-lisp.wasm
   bash projects/nostr-gov-lisp/tests/run-gauntlet.sh
   ```
   (The script does NOT rebuild the wasm — always rebuild first.)
   Expect a big recovery from the current 9/34. Report the number. Note:
   gauntlet cases involving event-auth (ev paths) legitimately stub
   ERR_EVENT_AUTH_NOT_IN_TS_PORT — those can stay red if expected value
   matches the stub error; report any OTHER reds.

## Constraints — IMPORTANT (another agent is live in this crate)
- Another subagent is currently editing `src/wasm_emit/call_string.rs`
  (str-cat fix). DO NOT touch call_string.rs. Avoid helpers.rs unless
  strictly required; if you must, check `git diff` first and only append
  disjoint additions.
- Never `git add -A`. Explicit paths only.
- Do not push. Do not deploy to testnet.
- Commit message references this file.

---
## OUTCOME (2026-09-02, commit e2aa618)

**Forensics correction — the stitcher was never broken.** A fresh rebuild of
current HEAD merges the crypto lib and resolves the WASM_IMPORT_BASE sentinel
correctly. The original repro's "vector 0" was mangled: pk D69C3509… is
vector 4's NOT-ON-CURVE key and the sig had a CA→BA typo at byte 28 —
returning 0 for that input is *correct* BIP-340 behavior. (The 41-func wasm
in evidence #4 came from a stale binary.)

**Actual root cause of gauntlet 9/34:** commit 742aab9 (sha256-hash → hex
digests, justified by bug #12) silently changed the digest contract. nostr-gov
fed the 64-char hex string into schnorrVerify where the RAW 32-byte digest is
required → every owner sig failed with ERR_INVALID_OWNER_SIGNATURE.

**Fix (app-level, both twins):** `hexDecode(sha256Hash(msg))` at all 6 schnorr
sites (main.ts ×3, main.lisp ×6 sites incl. event-auth paths); plus ev-routing
hoisted above legacy arg validation in create_wallet so event vectors hit the
TS stub honestly (ERR_EVENT_AUTH_NOT_IN_TS_PORT, matching the Rust reference
order encoded in gen-vectors.py).

**Numbers:**
- tests/schnorr_stitcher_test.rs: 4/4 (L1 official vectors 0/1 through the
  stitcher — locks the merge pipeline; L2 nostr-gov owner-sig recovery)
- cargo test --release: **1585 / 0**
- gauntlet: **22 / 34** (from 9/34) — all 12 remaining reds are the
  pre-blessed event-auth stub class (+#29 is its storage cascade)
- twins: 22/22 trace-equivalent (diff-ts.sh)
