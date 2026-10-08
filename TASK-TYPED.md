# TASK: TYPED — Typed surface on the Lisp, TS SDK codegen (src/ fence)

**Status:** QUEUED — do NOT start until M3 (`clmm-v5-m3`) has landed its pool or been killed. `lib.rs` registration lines can collide; coordinate.
**You own:** `src/typing/` (checker), `src/parser` annotations, `src/ops_spec.rs`, new `src/ts_emit.rs`, `tests/typed_*.rs` + fixtures, this file's checkboxes.
**You do NOT touch:** `lib/`, `scripts/`, `data/`, `candidates/`, `types.gen.ts` semantics (it stays generated — regen via REGEN=1 only), `src/wasm_emit/` behavior (byte-identical gate below), `src/bytecode.rs` (annotations are checker-only).

## Context (the design, agreed with JP 2026-10-08)
- OPS_SPEC (`src/ops_spec.rs`, commit a472c136) is the single op table: name, arity, backend flags, TS sig. Today it feeds the drift test + `types.gen.ts` golden. This task makes it ALSO feed the source-level type checker and a per-contract TS SDK emitter.
- Design rule (JP's words): types only where bindings are born — `defn` signature + typed literals. Bodies are inferred. No per-expression annotations ever. Code stays a Lisp; TS is infrastructure, never a second authoring syntax.
- Hard-error policy preserved: KNOWN conflicts reject at compile; UNKNOWN types flow as `Value` (dynamic). Wrong is fatal, unclear is permissive.
- Strict-subset rule preserved: interp/wasm availability comes from OPS_SPEC flags — a form accepted by the checker must be emittable.

## The five inference rules (implement exactly, no more)
1. `defn` sig is ground truth; body inferred. Recursion requires declared return type (reject otherwise).
2. Literals self-type: integer → `Int`, `1.5` → `Fp`, `100y` → `Yocto` (reader suffix), `(y "...")` → `Yocto`.
3. `Yocto` is a NOMINAL TAG OVER `Str` at the checker level. Runtime = decimal strings through limb-math, exactly as today. `li-add` etc. get sigs `(Yocto Yocto -> Yocto)`; mixing `Yocto` with `Int`/`Str` in arithmetic = checker error. NO new runtime tag, NO bigint runtime.
4. Union narrowing on the existing idiom: `(if (= x false) A B)` prunes `false` from `x`'s type in `B`, prunes the non-false arm in `A`. Only this one pattern — no general flow typing.
5. Escape hatch: `(cast:Int e)` / `(cast:Str e)` etc. — one form, greppable, policy-in-the-name like `wrap-*`. Casts to `Yocto` allowed from `Str`.

`define` stays UNtyped (everything `Value`) — the existing corpus compiles unchanged. `defn` is the only new form. Check the parser lands it in every layer it needs (parser/sugar path, AST, checker; NOT bytecode/wasm — it compiles to the same thing as define, with sigs erased).

## Milestones (staged commits, each independently green)

### T1 — checker env from OPS_SPEC  [ ] 
- `src/typing/checker.rs` builtin env derived from OPS_SPEC instead of hardcoded. Add a `lisp: &str` type column (or map in ops_spec.rs) — `Int`/`Fp`/`Str`/`Bool`/`Value` for now; `Yocto` lands in T3.
- Special forms need a `special: bool` flag (if/not) — they're parsed, not dispatched; exclude from BUILTIN_NAMES existence check the right way (the test exemption is already there, make the table honest instead).
- Gate: `cargo test` green; no fixture behavior change; checker rejects at least: arity overflow on a fixed-arity op, `Int` arg where `Str` sig, Fp literal into Int-typed binding (write these as `tests/typed_t1.rs`).

### T2 — `defn` annotations + body inference  [ ]
- Reader/parser: `(defn name (x:Int max-f:Int -> Int) body…)`; param list is positional, `->` return optional ONLY for non-recursive fns.
- Checker: inference per the five rules. Errors cite defn name + form.
- Gate: new fixtures — clamp-fee, handle-transfer (transfer shapes from the 10-08 session), a recursive fn WITH declared return, reject: recursive without return, `(+ 1y 1)` (once T3 in), Fp into Int param.
- **The killer gate:** compile the full existing corpus before/after with `compile` — wasm BYTE-IDENTICAL (extend the `cbbea9c3` determinism-sweep pattern; annotations must be erasable). Any emit diff = stop, root-cause.

### T3 — `Yocto` branded-Str + literals  [ ]
- Reader: `100y` suffix (decimal digits + underscores, trailing `y`), `(y "...")` constructor form.
- Checker: `Yocto` nominal over `Str`; sigs for `li-add/li-sub/…` from `lib/limb_math.lisp` surface (READ the lib for the real names — do not invent).
- Gate: `(+ yocto int)` rejects; `(li-add (y "1") (y "2"))` infers `Yocto`; corpus still byte-identical.

### T4 — TS SDK emit + CLI  [ ]
- New `src/ts_emit.rs`: for each annotated defn, emit a TS binding (op-level brands from the regenerated `types.gen.ts`; defn sigs from checker). Golden test `tests/typed_ts_golden.rs` — regen + byte-compare, same pattern as ops_spec_test.
- Emit: branded types (`Int`=bigint brand, `Str`, `Yocto`=string brand), boundary constructors, `Promise<Ret<…>>` for promise-returning entries, exported `api` object.
- CLI: `compile --ts out.ts <file.lisp>` (extend the compile bin; usage on no args).
- STRETCH (own checkbox, do last): harvest `json-get` keys from defn bodies → infer input record types (`TransferArgs`) for `msg:json` params. Skip cleanly if it exceeds one sitting — mark STRETCH-DONE/STRETCH-SKIPPED with one line why.

## Discipline (non-negotiable — same as DX/M3)
- Staged commits, `feat(typed-tN): …` style, specific-file `git add`, NEVER `git add -A`.
- Full suite green before each commit (56 binaries baseline; pre-existing deep-nesting env failure exempt, must remain IDENTICAL failure).
- Machine-verify: every claim in the report has a command + output pasted. Checker rejections demonstrated with actual compiler output.
- First-divergence protocol; 45-min rabbit-hole cap → BLOCKED note + move on.
- `popcnt`/`bit_get` BUILTIN_NAMES gap and `__json_get` span audit are OUT OF SCOPE (separate queue).
- If the checker's current shape conflicts with T1 unification in a way that needs semantic changes beyond "env source swap," STOP and report — do not rewrite the checker.

## Definition of done
T1–T4 checkboxes ticked, corpus byte-identity proven, TS golden locked, report with: commits, rejection demos, regen instructions, and the honest list of what stayed `Value`.
