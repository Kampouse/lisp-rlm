# Task Board

Specs live in `tasks/` (active) and `tasks/done/` (landed, stamped with
their landing commit). Each file is self-contained — feed one to a
subagent as-is.

## Queued
- **json-get-str-object-args** — input scanner can't decode raw `{...}`/`[...]`
  object literals in args; string-encoded form works. Balanced-substring
  extraction + interp parity + tests. (`tasks/TASK-json-get-str-object-args.md`)

## Parked
- **wasm-u128** — paused at to_str live-local clobber; fix sketch documented
  in-file. (`tasks/TASK-wasm-u128.md`)

## Done (7)
multi-contract-sandbox · json-bug · concat-bug · schnorr-stitcher ·
correctness-sweep · nil-miss-tests · storage-read-cache — see
`tasks/done/`, stamps cite landing commits.
