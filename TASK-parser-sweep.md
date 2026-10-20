# TASK: Parser sweep — 6 stale suites broken by human_num comma formatting

**Priority:** runs BEFORE the timelock vault task (TASK-timelock-vault-v1.md).
**Scope fence:** test parsers ONLY. Do NOT touch `lib/clmm_pool_v5.lisp`, its tests, or any contract logic. Do NOT change the mock's human_num display (it's intended UX).

## Background
The near-mock's `human_num` (landed Oct 15) comma-groups numbers in `📄`-prefixed output lines. Six test suites parse those lines as raw integers and broke. Two suites were already fixed in 72139e42 (`test_loop_gas` — strips comma grouping before parsing; `surface_parity` — documented json-get-str-esc as promise-runtime-only). Follow the SAME fix pattern: strip comma grouping at the parse site.

## The 6 suites (from /tmp/fulltest.log, 2026-10-16 11:12 run)
1. amm — `amm_constant_product_full_suite` FAILED
2. lending
3. flashloan
4. launchpad
5. u128 (chunked / limb_locals) — incl. `repeat_literal` (5 passed, 3 failed)
6. u128 second suite

Reproduce first: `CARGO_BUILD_JOBS=2 cargo test --release --test <name>` per suite. Confirm each failure is the comma-parse class (parse error or truncated number) BEFORE fixing. If any suite fails for a DIFFERENT reason, do not blind-fix — document it and leave it red with a note in your report.

## Fix pattern (from 72139e42's test_loop_gas)
Parse sites that read numeric fields from 📄 lines must strip `,` grouping before `.parse()`. Keep the strip local to the parser helper — no global format changes.

## Acceptance
- All 6 suites green individually
- One attended full-suite run (`CARGO_BUILD_JOBS=2 cargo test --release --no-fail-fast`) — zero failures expected, INCLUDING test_pool_v5_custody + test_pool_v5_c2b (already green at d11a5683; if either regresses, STOP and report — that's a real bug, not parser rot)
- Single commit: `test-infra: sweep stale parsers past human_num comma grouping (6 suites)`
- Do NOT push.

## Machine budget (HARD — this is a fanless MacBook Air)
- ONE cargo lane at a time. `pgrep -f 'cargo|rustc'` before any run; wait if busy.
- Always `CARGO_BUILD_JOBS=2`. No `--release` full-suite runs unattended/overnight.
- If swap is hot (`sysctl vm.swapusage`), stop and report.

## Report format
Commit hash, per-suite before/after status, full-suite final tally, anything you chose NOT to fix and why.
