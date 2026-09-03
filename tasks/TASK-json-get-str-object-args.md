> **Status: QUEUED** — found in 2026-09-03 example sweep; engine gap (devtree territory)

# TASK: json_get_str — extract object/array-valued input args as raw JSON substrings

## Symptom (found 2026-09-03 via the playground Objects example)
Args containing a raw nested object literal decode to empty:
- `{ "cfg": {"server": {"port": "80"}} }` → `get_port` returns ""
- `{ "b": {"title":"prez","votes":5} }` → `cast` returns ": 1" (title "",
  votes 0 — object value never reaches the contract)
String-encoded form works perfectly today:
- `{ "cfg": "{\"server\":{\"port\":\"80\"}}" }` → "80" (dotted json-get-str path OK)
- `{ "b": "{\"title\":\"prez\",\"votes\":5}" }` → "prez: 6" (numeric
  auto-decode OK)

## Root cause
`Wasm::json_get_str` (src/wasm_emit/json.rs, value extraction after the
colon, ~lines 4340-4450, "Value shape fix 2026-08-30") supports exactly
two shapes: quoted strings (scan to closing quote, unquote) and bare
tokens (numbers/true/false/null — scan to next , } ]). Object/array
values hit the bare-token path and stop at the first inner , } ] →
garbage/empty.

## Fix
Extend the value extractor with a third shape: when the first value byte
is `{` or `[`, copy the BALANCED substring (track depth; strings inside
respect backslash-escaped quotes — the string scanner already handles
escape parity, reuse that pattern). Return the raw slice (documented
object semantics: raw JSON string, same as the string-encoded form).

## The usual 4 places (landmine checklist)
1. src/wasm_emit/json.rs — json_get_str scanner (this bug)
2. Interpreter twin — bytecode/ (verify the interp path reads input the
   same way; keep parity)
3. tests/test_json_wasm.rs / test_api_sweep.rs — add cases: object arg,
   array arg (jsonArr path), nested-object arg, object arg containing
   escaped quotes and nested commas
4. Gate: playground Objects example currently documents the string form
   (shipped 2026-09-03). Once fixed, restore the raw-literal form in the
   example comments (examples.ts get_port/cast headers) — the gate test
   will keep it honest.

## Verification bar
- New tests pass; full battery 0 failures (397+ baseline).
- Node harness vs the playground compiler wasm: raw-object args decode
  to the same values as string-encoded args for get_port/cast/tally.
