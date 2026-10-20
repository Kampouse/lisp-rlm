# TASK: u128 Provenance Phase 2 — Param Inference + Storage Codec

**Repo:** lisp-rlm · **Risk:** medium-high (typing inference + emit + interpreter + storage edge) · **Verification:** live A/B both legs
**Machine budget:** single build lane, `CARGO_BUILD_JOBS=2`, no overnight suites, verify procs dead when parking.

## Ground truth (measured live A/B, 2026-10-13)
Phase 1 is landed (main = `f8b857c9`) and SEMANTICALLY PERFECT — outputs bit-identical
on both legs (`out=199999` a-leg, `swapb out=1.5e32` b-leg, refunds+ledgers equal) —
but gas barely moved: a-leg 3.079T → **3.021T** (-2%), b-leg 3.654T → **3.503T** (-4.1%).
Acceptance was ≤1.2T. Disasm census says why:
- Wrappers inlined fine (`$add/$sub/$mdiv/$ult` gone from NEW), but parse SITES went 19 → 59:
  the work moved into callers. Every param-borne value is **Stringly** (`FnProvenance.params`
  defaults Stringly by design), so `swap → walk → step` re-parses at every use (~24
  parses/swap at runtime, unchanged from old).
- Storage edges stringify: `sput/sget` per walk iteration (~6 codec round-trips/swap)
  are untouched by phase 1 (intra-procedural only).
- Deployed evidence: poolan = tip build (8884ef16…), poolbn created+deployed this leg;
  raw wasms + .wats in `~/dev/pool{a,b}_{old,new}.{wasm,wat}`; census scripts
  `~/dev/disasm_*.py`; gas pullers `~/dev/ab_gas{,_b}.py`.

## Scope (both levers, this order)
### 2A — Param provenance inference (big lever)
1. **typing/checker.rs**: after whole-program typecheck, compute per-fn param provenance
   by fixed-point over DIRECT calls only: param P of fn F is `Native` iff **every** call
   site passes an arg whose checker type/provenance is Native (ph1 births: u128 ops,
   u128 literals, `json-get :u128`, `attached_deposit_u128`, let-locals via ph1 rules).
   Any indirect/first-class call touching F, or recursion through non-direct paths →
   all params Stringly (conservative, taint=fallback — never error, ph1 rule unchanged).
   Entry-point (export) params stay Stringly — host ABI hands strings.
2. **wasm_emit**: Native params hold (i64,i64) through the body (calling convention
   already passes pairs since ph1); ops on Native params skip parse. FnProvenance is the
   single source — reuse the checker predicate (no emit-side re-derivation).
3. **bytecode interpreter**: mirror param handling; parity pins red→green
   (param-threaded fixture: wrapper fn feeding another wrapper, loop like walk/step).

### 2B — Storage binary codec (edge lever, second)
4. Native-provenance values at `sput`/`sget` use 16-byte LE; stringly keep decimal.
   **Migration hazard (design before code):** a deployed contract reading OLD decimal
   state with NEW code must not misread — version the encoding (e.g. key prefix or
   length sniff with strict decimal fallback) and pin cross-version reads in tests.
   Human-audit keys (paid:/left:) stay decimal by default.

## Fences (unchanged from phase 1)
- No parser changes, no new sugar, no clmm logic edits. Semantics bit-for-bit:
  same outputs, same refunds, same logs. Taint = fallback, never error.
- Do not touch afp-clmm accrual WIP (uncommitted files under port/).
- `near-compile` is a separate build — after wasm_emit changes RELINK it
  (memory gotcha) or tests compile with the stale compiler.

## Verification ladder
1. `cargo test -j2` green; new differential pins for param inference (direct-call
   native → native; indirect call → taint fallback; export boundary stays stringly).
2. layer_parity + test_u128_native_chain green (existing 3 pins must stay green).
3. Rebuild pool-a + pool-b at tip, disasm census (scripts exist): param-borne parse
   sites in `$step/$walk/$swap` families must COLLAPSE toward 0; storage sites only
   at sput/sget edges after 2B.
4. Live A/B both legs, same protocol as 2026-10-13 (0.3N, 4T cap, receipts via
   EXPERIMENTAL_tx_status — script `ab_gas.py` reusable):
   **acceptance: a-leg ≤ 1.2 Tgas** (expect big margin from census math), b-leg
   proportional (~≤1.4T), outputs bit-identical to the 10-13 runs
   (199999 / 1.5e32, same refunds).

## Deliverables
- Commits on `u128-native-chain` (already = main tip): checker inference →
  interpreter → emit → storage codec, one layer per commit.
- A/B logs + receipt hashes pasted in PR; updated census tables.
