# TASK: fix json_get_str input-size corruption (BLOCKER for protocol #16)

> **Status: DONE** — 1e86f26. Full history in `git log`.

## Symptom matrix (all reproduced 2026-09-02, mock + engine b6478f4+)
echo fn m(a,b,c,d: string) returns `${a.length}:${b.length}:${c.length}:${d.length}`:
- {"a":"a"*10, "b":"b"*192, "c":"c"*384, "d":"01"+"1"*64} → d = 0 (empty)
- {"a":"a"*384, "b":"b"*10} → b = 0
- {"a":"a"*96, "b":"b"*384, "c":"c"*192, "d":"0111"} → c = 0
- {"a":"a"*576, ...} → b,c = 0
- {"a":"a"*256, "b":"b"*256, "c":"c"*256, "d":"0111"} → ALL OK (830B total!)
- {"a":"a"*700} single key → OK
Real contract trap: setPoints(id, msgPoint 192-hex, g2gen 384-hex) traps wasm;
abort() then prints a corrupted pointer (LOG shows 31 bytes of the g2gen value).

## Where to look
- src/wasm_emit/json.rs:2557 — `scratch = heap_bump((8 * MAXELEM))` = **512B scratch**
- json_get_from_buf ~line 4438 comment: "Cursor bounds: 0..(8*MAXELEM)" — unrolled-scan era artifact
- ensure_json_get_func line ~4615 (631-line generated state machine)
- heap_bump in mod.rs documents the SAME class ("every json read after that
  returned fragment garbage" — U-fix 3b, 2026-08-29)
- INPUT_BUF = 16384 (mod.rs) — input copy itself is fine

## Hypotheses (ordered)
1. Scanner copies input into the 512B scratch → args beyond 512 invisible.
   (Doesn't explain 256×3@830B passing — unless the copy is chunked.)
2. Long-value unescape dst overruns the 512B scratch → clobbers the NEXT
   key's scan state (explains a384→b0 and the corrupted abort pointer).
3. Unrolled loop iterations = MAXELEM (64) — scan steps bounded regardless
   of buffer.

## Fix requirements
- json_get_str/json_get_int/jsonArr must handle ≥16KB args (INPUT_BUF size)
- Do NOT regress: 1572-test battery + playground gate + tsc all green
- Un-ignore the two tests in tests/test_ts_bls_msig.rs — they are the gate
- Add a regression test: multi-key echo with values 10/192/384/66 (the exact
  #16 shapes) + a 4KB single value
