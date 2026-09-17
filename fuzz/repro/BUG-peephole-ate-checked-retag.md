# BUG-2026-09-17: peephole ate the checked-retag overflow guard → silent 61-bit payload wrap

**Status:** FIXED on branch `fuzz-campaign-0917` (pending commit + tests)
**Severity:** HIGH — silent money-corruption class, **ships in published 0.1.15 / near-compile 0.1.16**
**Introduced by:** `9384836` (2026-09-14, "perf(peephole): ShrS tag/untag cancellation + shorter emit_tag_num")
**Caught by:** `cargo test --release` at session start — `test_money_safety::shl_out_of_range_traps` (this is why the session protocol exists)

## Shape

`WasmEmitter::peephole` (src/wasm_emit/gas.rs) cancelled 4-instruction
adjacencies `(3,Shl)(3,ShrS)` and `(3,ShrS)(3,Shl)` unconditionally.
But `emit_tag_num_checked` — the money-safety guard that makes shl
TRAP instead of wrap past the 61-bit tagged payload — is *built from
those exact instructions*:

```
LocalSet(tmp)
LocalGet(tmp) LocalGet(tmp) Const(3) Shl Const(3) ShrS I64Ne   ; overflow?
If(Unreachable)                                                ; trap
LocalGet(tmp) Const(3) Shl                                     ; tag
```

After the peephole ran, the check collapsed to `t != t` (never fires)
and every shl result re-tagged with a bare `<<3` — silent two's-complement
wrap. `bor`/`bnot` retags inherit the same guard shape.

## Repro

```
export function main(x: number): number { return (x << 1); }
```
`main(2^59)` returned `-1152921504606846976` (wrapped 2^60) with exit 0.
Interpreter correctly errors: `shl: result out of range for tagged num`.

Const-folded variant (`shl 576460752303423488 1` on literals) wrapped the
same way via the same emitted code.

## Why the const-shaped wrap persisted past the "294 tests green" gate

`9384836` ran the suite excluding/without `test_money_safety` green —
`shl_out_of_range_traps` fails from that commit forward. The raw-locals
feature the arms served was reverted in the same commit; the arms stayed.

## Fix

Removed the four blind cancellation arms from `gas.rs::peephole`
(ShrU flavors from `5d0a1dd` were already dead at HEAD — `emit_untag`
is ShrS since 2026-09-13; the ShrS arms were the live ones).
`raw_locals` is never populated at HEAD, so the arms had no legitimate
effect — their only observable behavior was corrupting the guard.

## Follow-ups

- [ ] Release patch: lisp-rlm-wasm 0.1.17 / near-compile 0.1.17 (0.1.15/0.1.16 are affected)
- [ ] Audit any other consumers of `emit_tag_num_checked`-shaped code for peephole interactions
- [ ] Session-protocol lesson: run the FULL suite (`--no-fail-fast`) after peephole/perf commits, not a subset
