# TASK: json-get-str — miss-over-span scanner fix ✅ RESOLVED 2026-10-09

## Status
**FIXED + PINNED** (same day as discovery). No remaining scope — this card is
the paper trail for the root cause and the fix.

## Symptom (found 2026-10-09 during json-object-args verification)
2-arg `(json-get-str KEY SPAN)` where the buffer came from a raw span
(`(near/json_get_str K)`) and KEY does **not** match returned **tail garbage**
instead of empty:

| input | key | old result | expected |
|---|---|---|---|
| `{"cfg":{"server":{"port":"XYZ"}}}` → span 21B | `nope` | `YZ"` (3B) | `""` |
| `{"cfg":"hello world 12345"}` → span 22B | `zzz` | `2345` (4B) | `""` |

Hits were never affected (deep matches, dot-paths, array indexes, string-encoded
buffers all correct on a fresh build).

## Root cause
`__json_get` (wasm_emit/json.rs, `ensure_json_get_func`): the `temp` local
doubled as byte-scratch (`ch` staging) inside the scan loop AND as the
match flag. When the final iteration before the bounds exit skipped the
key compare — `depth != 1` (nested objects, or a fully quoted string
span) — the bounds-exit branch left a **raw buffer byte** in `temp`. The
not-found gate (`temp == 0` → miss) misread that byte as "match found";
the value extractor then ran from end-of-buffer and copied whatever tail
bytes were there.

Miss over a *literal* buffer (`{"a":"b"}` in-source) was never affected:
its last scan iteration always ran a compare, leaving `temp=0`.

## Fix
Force `temp = 0` on the bounds-exit path in `ensure_json_get_func`
(json.rs, scan-loop bounds check) — bounds exit is a definitive miss;
only a match + colon survives to the extractor.

## Interpreter parity
No twin exists: bytecode `eval_near_builtin` stubs `near/json_get_str`
as `Str("")` (miss semantics) — the wasm fix moves toward the stub, not
away. No interpreter change required.

## Regression pins
`tests/test_json_object_args.rs` Tier 2b:
- `nm_miss_over_object_span_returns_empty`
- `nm_miss_over_string_span_returns_empty`
- `nm_deep_key_hit_still_works` (fix must not break deep matches)

## Investigation notes (avoid re-deriving)
- First repro matrix ran against a STALE near-compile binary → two false
  "failures" (deep-match, string-encoded) that polluted the initial
  analysis. Fresh `cargo build --release -p near-compile` first; only
  then trust probe output. (deep-hit r1 + string-encoded r3 were correct
  all along on the fresh binary)
- Confirmed-adjacent non-bugs (fresh build): TS obj-param lowering is
  root-keyed by design (name erased, examples.ts documents it); TS
  string-array params are NAME-keyed via `near/json_get_arr`
  (`firstItem({"items":["a","b"]})`); flat `p.a` obj-param reads work.
