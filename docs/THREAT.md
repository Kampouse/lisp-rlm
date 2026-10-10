# THREAT MODEL — CLMM pool v4.x

_Who the pool trusts, what it defends against, and what it deliberately
does not do. Written 2026-10-06 after the external-checklist review
(NEAR protocol docs, rekt.news incident DB, Trail of Bits fuzzing
methodology). If you deploy this anywhere real, read this first._

## Trusted parties (assumptions, not guarantees)

1. **TOKA / TOKB token contracts are honest.** The pool acts on the
   `sender_id` / `amount` args the *token* passes to `ft_on_transfer`.
   A malicious token can attribute deposits to anyone or claim any
   amount. This is inherent to NEP-141 — the tokens ARE the collateral.
   Consequence: only deploy against tokens you control or have audited.
2. **The deployer account is not compromised at init.** `init` is
   one-shot and immutable afterwards; tokens are bound on first call.
3. **The runtime is NEAR.** Promise semantics (chaining, flattening,
   `promise_result` on failures) verified against testnet — the near-mock
   harness now reproduces live-NEAR flattening (slot-0 = final value).

## Deliberate design choices (non-goals)

- **No admin, no pause, no upgrade path.** No governance surface to
  seize (Term Labs class), but also **no circuit breaker** — if a live
  bug is found, the only tool is liquidity withdrawal. Accept or fork.
- **The ladder is a price snapshot, not an oracle.** Slot state is
  manipulable within one transaction (push, read, unwind). If anything
  ever consumes pool state as a price feed, it must build its own TWAP.
  (Nostra-class oracle rigs are the incident to fear here.)

## Known residuals (quantified, accepted)

- **leg2-fault withdrawal** — if the B-payout leg fails after the A leg
  committed: shares burned, A paid, B slice shorted, the B stays in the
  pool (orphaned, unclaimable). Fuzzer measures it exactly (I1 drift ==
  cumulative orphan). Triggering requires a failing/broken TOKB.
- **Payout to a deleted account** — if a withdrawer's account is deleted
  between legs, the refund bounces to the pool with PB already
  decremented: same orphan shape as leg2-fault. Recoverable by no one.
- **Storage economics** — LP entries (`SH:<acct>`) are deleted on
  full exit (v4.5), and the min deposit (10) plus that deletion bounds
  per-LP permanent storage. But each *active* LP still stakes pool NEAR
  (~key+value bytes); a sustained many-minimum-LPs campaign still costs
  the pool. A full storage-registration pass-through is future work if
  this ever faces hostile LPs.

## Defense layers (what tests enforce what)

| Layer | Tool | Classes |
|---|---|---|
| Input hardening | numOk 40-digit guard, 1e18 caps, dust/sybil/min-deposit guards, `no-shares` | Cetus-class OOB, parse traps, zero-share grief |
| Book invariants | `clmm_invariants.py` I1–I7 (balance==book, ΣSH==SHT, Σslots≤B, wealth conservation) | accounting drift, oversold ladder, value leaks |
| Rollback | I5 bit-identity on every refund/reject/fault path | failed-payout desync (v4.3 class) |
| Fault injection | toggle_fail tokens, 0–50% fault rates, edge battery | partial-failure withdrawal/swap legs |
| Parity | lisp vs TS byte-identical storage, 17-op battery + fuzz | implementation divergence |

## Testing methodology gaps (honest list)

- The fuzzer is random, not coverage-guided (no corpus replay / shrinking).
- No adversarial-profit *search* beyond the I7 conservation ceiling.
- No contract-internal runtime asserts in a debug build.
- Concurrency: delta-rollback is designed to compose, but no test
  interleaves receipts mid-flight.
- No post-deploy monitoring (design exists, not built).
