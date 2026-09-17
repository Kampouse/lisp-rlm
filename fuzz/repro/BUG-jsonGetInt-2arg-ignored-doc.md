# BUG: 2-arg jsonGetInt silently ignored the doc argument (fixed 41b1887)

**Severity:** HIGH (silent wrong answer in money code)
**Status:** FIXED + pinned (4 regression tests, test_dynamic_json.rs, 18/18 green)
**Fixed:** commit `41b1887` on `fuzz-campaign-0917` (2026-09-17)

## Symptom

`jsonGetInt(key, doc)` never read `doc`. With tx input `{"n":99}` and doc
`{"n":7}`, `jsonGetInt("n", doc)` returned **99** — the input's value, not
the doc's. With empty input it returned −1 (miss) even when the doc had the
key. The str twin `jsonGetStr(key, doc)` behaved correctly throughout.

```ts
// doc = "{\"n\":7}", tx input = {"n":99}
near.jsonGetInt("n", doc)  // → 99  (WRONG: read tx input)
near.jsonGetInt("n", doc)  // → 7   (after fix)
```

## Root cause

`src/wasm_emit/call_json.rs`, `"near/json_get_int"` arm: both sub-branches
(literal key → `json_lit_lookup_str`, dynamic key → `json_dyn_lookup_str`)
read the **tx input buffer only**. There was no `a.len() >= 2` branch — the
second argument was compiled away. This is the exact footgun jsonGetStr had
before its 2026-09-14 fix (`json-get-str` IR handler + buffer copy); the fix
was never ported to the int twin.

Found by the overnight fuzz campaign (2026-09-17): primitive probes showed
2-arg int lookups hitting only when the tx input contained the key — which
also explains the JSON phase's 1,020 "hits" in a corpus whose probes were
supposed to miss on input.

## Fix

Mirror of the str twin, no new logic:

- `"near/json_get_int"` arm gained the `a.len() >= 2` guard (literal key
  required, else compile error — never silent).
- 2-arg emission = `self.call_json("json-get-str?", &args2)` (the nil-on-miss
  buffer scan, dot-paths supported) composed with the existing shared tail
  `emit_int_parse_gated` (miss → TAG_NIL so `??` fires; hit → first-byte
  digit/minus gate + `__str_to_num`; found-but-non-numeric → nil).
- `ts/lisp-rlm.d.ts` now declares `jsonGetInt(key, json): number | null`.

Semantics: identical to the 1-arg unified path, except the scan target is
the given string. Miss renders nil (`??` fires), NOT "" or −1 silently.

## Regression pins (tests/test_dynamic_json.rs)

- `int2arg_scans_doc_not_input` — THE discriminator: doc 7 vs input 99 → 7.
- `int2arg_hit_miss_dot_spaced` — 42 / `??`-fires / `p.q`→7 / spaced `: 42`.
- `int2arg_str_twin_unaffected` — str 2-arg still works.
