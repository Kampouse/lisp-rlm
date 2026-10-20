# P3: u128 op-inline + parse-kill — Implementation Plan (lisp-rlm, phase 3)

> **For Hermes:** Execute task-by-task; measurement gate M0 decides whether S1–S3 proceed as planned or get re-scoped.

**Goal:** Drive a-leg swap gas from 3.032 Tgas toward the ≤1.2 Tgas acceptance by removing u128 helper-call and string-marshalling overhead from hot op sites — measured before and after, honestly.

**Architecture:** The 2B binary-cell codec made storage reads limb-context, but every u128 *op* still routes through string-domain helpers (`__h_u128_add/sub/muldiv/divmod` take decimal strings, parse internally, render results). 98 helper call sites sit inside the swap/quote_buy closure of both pools (identical: 55 parse, 24 to_str, 7 muldiv, 5 add, 5 sub, 2 divmod; hot mass in `step`/`stepb`/`swap*`/`walk*`). P3 moves ops to the i64-limb domain at emit time and eliminates parses/renders that 2B already made unnecessary — governed by a fuel-attribution gate so we attack only what the numbers indict.

**Tech stack:** Rust emitter (`src/wasm_emit/*`, wasm_encoder), near-mock (exact fuel metering, 1 fuel = 1 gas unit), wasm-tools for WAT audit, existing test battery (2085-passing baseline, only `yield_oracle` pre-existing red).

---

## Measured starting point (2026-10-13)

| Metric | Value |
|---|---|
| a-leg live swap (2B tip) | 3.032 Tgas (acceptance ≤1.2) |
| b-leg live swap | 3.613 Tgas |
| `__h_u128_*` sites in swap+quote closure | 98 (pool-a AND pool-b, identical) |
| Twins firing in closure | 0 (2A design doesn't fire on these contracts) |
| Helper bodies | internal wasm (instruction gas, no host boundary) |
| Census baseline | 2085 pass / 1 fail (`yield_oracle`, pre-existing) |

**Hypothesis under test:** parse+render marshalling inside helpers dominates their cost, and helper bodies are a large fraction of swap fuel. M0 measures both before any emitter work.

---

## Task M0a — Fuel attribution table (measurement, no behavior change)

**Objective:** Per-helper fuel cost inside one seeded swap; % of total swap fuel.

**Method — stub-swap differential (no mock changes needed):**
1. Seed `/tmp/p3_seed.json` exactly like the 2B mock differential (`s:sqp=3000000000`, `s:y=6e22`, `s:out=0`, `paid:owner.test.near=2e32`; state lands under `escrow.test.near`). Baseline run: `near-mock pool-b.wasm swap '{...}' --deposit ... --dry-run`, record total fuel.
2. For each helper H ∈ {parse, to_str, add, sub, muldiv, divmod}: rebuild pool-b with H's body stubbed (drop args, return a fixed valid-shaped result — stack-shape identical, fuel near-zero), rerun the same swap, record fuel. Fuel(old) − Fuel(stubH) = H's total cost in that swap. **Results are garbage for stubbed runs — fuel arithmetic only. Stub rebuilds via the emitter behind a debug flag OR direct WAT surgery + `wasm-tools` rebuild; pick whichever is 20 minutes, not 2 hours.**
3. Also measure input-dependence: parse cost at 1-digit vs 39-digit inputs (two seeded swaps).

**Files:**
- Scratch: `examples/p3_stub_wasm.rs` (or `/tmp` WAT scripts — scratch, delete after)
- Numbers land in: this plan file + `docs/PLAN.md` P3 section

**Verification:** Stub-swaps still execute to completion (fuel measured); baseline fuel matches the known 2B mock number (~1.698 Tgas ± trace overhead).

**DECISION GATE M0 — MEASURED VERDICT (2026-10-13, seeded b-leg swap, dry-run, fuel-exact):**

Total 1.700T · **host calls 1.434T (84%)** · wasm instructions 0.266T (16%).

| Host call | Count | Gas | Share |
|---|---|---|---|
| storage_write | 8 | 865.4G | 51% |
| storage_read | 7 | 482.7G | 28% |
| read_register + register_len | 16 | 63.9G | 4% |
| log_utf8 | 1 | 13.9G | <1% |
| misc (input, predecessor, value_return) | 3 | 8.6G | <1% |

Per-swap cell diff: 5 distinct cells touched (`s:out`, `s:sqp`, `s:y`, `paid:`, `left:`) vs **8 write calls → ≥3 redundant writes**; 7 reads over ≤5 keys → **≥2 redundant reads**. Write cost ≈ base 64.9G + trie nodes (~108G avg/call) — **codec byte-size is irrelevant at this scale; op count is everything.**

**Gate outcome: "<10%/10-30%" wasm-only paths are dead — op-inline ceiling is ~−8% of total (266G wasm × ~50% helper share). P3 AS ORIGINALLY SCOPED IS REFUTED for acceptance.** The measured levers, ranked:

1. **S1′ Write coalescing (potential ~−19%+):** emit-level write-back cache per execution — route `storage_write` through a linear-memory pending map; subsequent same-key reads see pending values (correctness contract: read-your-own-writes); single flush at call end (last-write-wins). Kills the ≥3 redundant writes; any read-modify-write chains collapse too. Also kills the 865G's *trie* re-touched nodes when values repeat.
2. **S2′ Read memoization (~−8%):** per-execution read cache (key → value) so 7 reads → ≤5; pairs with S1′ (same table).
3. **S3′ Op-inline add/sub + limb muldiv (the original S1/S2, tertiary):** ceiling ~−8%, still worth it after S1′/S2′ shrink the denominator.

**Honest ceiling if all three land:** ~3.03T − (325G writes + 138G reads + ~200G wasm) ≈ **~2.3T** — still ABOVE 1.2T. **Acceptance ≤1.2T is not reachable by any compiler-side lever measured so far**: the remaining floor is ~5 unavoidable host ops (~550G) + base contract logic. Reaching 1.2T requires contract-level changes (fewer cells per swap, e.g. stop rewriting `s:y` when unchanged, merge paid/left into one cell) — that's pool-source territory, user's call.

S1′–S3′ remain worth building (−25% is −0.75T gas → real money at volume), but the acceptance promise is formally withdrawn with these numbers.

---

## Task S1 — Op-inline add/sub (only after gate passes)

**Objective:** `+`/`-` on two limb-context u128 values emit inline i64-limb chains instead of `call $__h_u128_add/sub`.

**Files:**
- Modify: `src/wasm_emit/mod.rs` (op emit arm), `src/wasm_emit/call_u128_str.rs` (shared MemArg/scratch — import `ma0/ma8/U128_*`, never redeclare)
- Test: `tests/op_inline_p3.rs` (new)

**Step 1: Failing pins first.** Pin (a) limb+limb add emits zero `__h_u128_add` calls in WAT; (b) semantics: overflow-wrap, carry propagation, max-u128 + 1 → 0, A − B underflow wrap — mirror helper behavior EXACTLY (pins assert against helper outputs, not intuition); (c) stringly operand → falls back to helper (tagged path untouched).
**Step 2:** Implement inline emission: two-limb i64 schoolbook (carry via `i64.add` + unsigned compare, or 3-limb split if helpers use 3×44-bit limbs — READ the helper's limb layout first, do not assume). Flat stack pushes for operands (no staging checkpoints). Result stays limb-borne; only tag at sinks via existing `emit_tag_str`.
**Step 3:** `cargo check --lib` clean; pins green; targeted battery (`NEAR_MOCK_STATE=/tmp/<fresh>.bin`, rm stale first): storage_codec_2b, test_u128_native_chain, twin_fire, layer_parity, storage_family, test_compiler_concat_m2, test_ts_pool_internal.
**Step 4:** Byte-diff pool-b rebuild — if byte-identical, ZERO adoption: root-cause before proceeding (the 2A lesson).
**Step 5:** Mock differential vs 2B tip (bit-identical outputs, fuel delta = S1 win). Commit separately.

---

## Task S2 — Parse census + muldiv limb routing

**Objective:** Kill redundant parses — especially inside `__h_u128_muldiv` (7 sites × 4 internal parses each could be the whale M0 exposes).

**Files:**
- Modify: `src/wasm_emit/mod.rs` / helper call sites; possibly a limb-domain muldiv twin helper taking limbs directly (kept as ONE shared helper — no duplicated schoolbook per site).
- Test: extend `tests/op_inline_p3.rs`.

**Steps:** (1) Runtime parse census — which of the 55 parse sites actually execute per swap (stub-count or WAT audit of step/walk loop bodies; static sites ≠ executions). (2) Rank executed parses by loop multiplicity. (3) Route: limb-borne operands skip parse entirely (provenance-driven, single shared classifier — `storage_prov`/limb scan, no new solver). (4) Mandatory parses stay: json arg entry (`amount_in`), charter-decimal cells. (5) Pins + battery + byte-diff + mock diff. Commit.

---

## Task S3 — to_str reduction

**Objective:** Of 24 render sites, keep only sink-mandated ones: the swap log line's single decimal render (charter), `paid:`/`left:` key strings (charter-decimal). Renders whose consumers are u128 ops become limb-passthrough.

**Steps:** audit each site's consumer in pool-b source; route limb-borne feeds; pins + battery + byte-diff + mock diff. Commit.

---

## Task S4 — Wrap-up

Full battery `--no-fail-fast` (expect 2085/1), final mock differential table, PLAN.md P3 verdict with honest ceiling, memory update. Live A/B on poolan/poolbn ONLY on user approval (writes cost gas; ~0.79 N signer balance available, 0.3 N per swap).

---

## Verification (every stage)

1. `cargo check --lib` clean (arbiter over rustfmt noise)
2. Pins + targeted battery green (fresh `NEAR_MOCK_STATE`, rm stale `/tmp/near-mock-state.bin` first)
3. Pool byte-diff ≠ empty (adoption proof — 2A lesson)
4. Mock differential: bit-identical outputs, fuel delta recorded
5. Full census before final report

## Risks / tradeoffs

- **Wrap/overflow semantics drift:** pins must be generated FROM helper outputs; a "cleaner" schoolbook that differs on edge inputs is a correctness bug, not an optimization.
- **Tag discipline in fast paths:** pack (ptr,len) then real `emit_tag_str`; raw payloads get corrupted by `emit_untag`'s arithmetic shift (known trap).
- **Stack-shape regressions:** flat pushes; no instruction-count checkpoints.
- **Zero-adoption trap:** byte-diff gates every stage.
- **Acceptance may stay out of reach:** M0's gate exists to say so with numbers instead of shipping a 3rd "~2%" surprise.
