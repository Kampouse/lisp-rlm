# Typed Surface Design — lisp-rlm

*Design agreed JP ↔ Gork, 2026-10-08. Task spec: `TASK-TYPED.md` (T1–T4, queued behind M3).*

---

## The problem

The language is exact where it matters (limb-kernel u128 math, parity-proven) but *blind to kinds*: nothing stops mixing a yocto amount with an i64 counter, an Fp with an Int, or a 3-arg `mod`. The historical bug classes all have this shape — `str->num` 2^60 sign-flip (fixed `2daf02b3`), `==` alias divergence, json-get arg-order split (`9a034ad0`). Types exist to make these *unrepresentable*, not to catch them at review.

**Design rule zero (JP):** types live only where bindings are born — the `defn` line and typed literals. Bodies are inferred. No per-expression annotations, ever. The moment TS or types start shaping how the Lisp *looks*, the layer has failed. Code stays a Lisp; TS stays infrastructure.

## The type universe

| type | runtime payload | arithmetic | notes |
|---|---|---|---|
| `Int` | i64 number | `+ - * /` direct | real runtime tag |
| `Fp` | f64 | float ops | via `to-float` conversions |
| `Yocto` | **decimal string** | `li-add` family ONLY | nominal tag over Str |
| `Str` | string | none | unbranded text |
| `Bool`, `Value` | — | — | `Value` = unknown, flows dynamically |

Hard-error policy preserved: KNOWN conflicts reject at compile; UNKNOWN flows as `Value`. Wrong is fatal, unclear is permissive.

## Why Yocto is a string (the load-bearing decision)

```
json arg "125000000000000000000000"     ← boundary: string (NEAR's u128 ABI —
        │                                  JSON cannot carry u128)
        ▼  (y "...") / annotation
   Yocto                                 ← CHECK TIME: nominal kind —
        │                                  not Int, not Str, its own thing
        ▼
   li-add / li-sub / li-mul              ← only road through the limb kernel
        ▼                                  (exact u128/u256; wasm has no u128)
   "125000000000000000000000"           ← runtime & payout: string again
```

- NEAR's protocol boundary already speaks decimal-string for u128. A bigint-style
  runtime tag would be: parse string → compute → format string. A string with
  extra steps.
- wasm has no u128 — that's *why* the limb kernel exists. Yocto arithmetic IS
  the kernel; the type's job is to make it the only road.
- The brand is a **customs seal**: unit tracking + op gating, not parsing.
  `(+ yocto 1)` fails because *u128-quantity ≠ i64*, not because string≠number.
- Closed under ops: `(li-add Yocto Yocto) → Yocto`.

## The five inference rules

1. **`defn` sig is ground truth; body inferred.** `(defn f (x:Int m:Int -> Int) …)`.
   Recursion requires the declared return type.
2. **Literals self-type.** `100`→Int · `1.5`→Fp · `100y`→Yocto · `(y "...")`→Yocto.
3. **`Yocto` is nominal-over-Str at the checker only.** No new runtime tag, no
   bigint runtime. Sigs for limb ops read from the actual `lib/limb_math.lisp` surface.
4. **Union narrowing on the existing idiom:** `(if (= x false) A B)` prunes `false`
   from `x` in branch B, the rest in A. Only this one pattern — no general flow typing.
5. **One escape:** `(cast:Int e)` / `(cast:Yocto e)` — greppable, policy-in-the-name
   (same discipline as `wrap-*`). CI can count casts per PR.

`define` stays untyped (everything `Value`) — the existing corpus compiles
unchanged. `defn` is the only new form; annotations erase at emit.

## Effect tracking (the 80/20 upgrade — T5 candidate)

One lattice, inferred bottom-up, no new syntax:

```
pure  <  read  <  write  <  external/promise
```

| effect | meaning | enforced |
|---|---|---|
| `pure` | no storage, no near/* | kernel defns pinned pure — a `store-bytes` in the batch path is a BUILD ERROR |
| `read` | slot-get/storage reads only | |
| `write` | storage mutation | |
| `external` | promises / cross-contract | only sanctioned entry defns |

Pays three ways:
1. **Kernel purity from convention to compile error** (M2's zero-alloc discipline mechanized).
2. **Differential fuzzing throughput**: `pure` defns hammered interp-vs-wasm with zero near-mock/state setup; the fuzzer discovers its own surface.
3. **Agent gate**: RLM-generated trees constrained — synthesized math must be pure, writes only in sanctioned defns. Malformed generations die before emit.

## Phantom types (runner-up — T6 candidate)

`Amount<TokA>`, `Shares<pool>`, `Liquidity<point>` — compile-only parameters, zero
runtime, inferred from constructor. Kills cross-token/cross-pool mixing (the v4/v5
`\x04`/`\x05` storage-prefix discipline becomes a type error). Same machinery as
Yocto, one level up. Partially redundant with the invariant battery at test time —
take effects first.

## What the TS layer looks like after

**Generated, never hand-written:** `compile --ts sdk.ts lib/pool.lisp` — same
source, wasm + SDK, one truth.

**Effect determines return-type shape — three structural tiers:**

```ts
export const math  = { clampFee: (f: Int, max: Int) => Int };              // pure ⇒ SYNC
export const views = { poolState: () => Promise<State> };                  // read ⇒ view RPC
export const api   = { deposit: (a: DepositArgs,                          // write ⇒ TX:
  o: { gas: Gas; signer: Signer }) => Promise<Ret<Str>> };                 //   gas+signer REQUIRED
```

- Pure math callable in hot loops — the type PROVES it costs nothing on chain.
- "Forgot gas" is unrepresentable in the signature.
- Check-then-act as a typed combinator: guards must be pure/read tier; you
  can't smuggle a tx into your own precondition.

**Input shapes harvested from the body:** every `json-get` key the defn reads
becomes the generated args record (`{s, w, g, r}`) — the SDK mirrors what the
contract *does*, not what docs claim.

**Standalone by default:** `--standalone` emits one self-contained file
(brands + ~30-line runtime preamble + api). Zero imports, zero node_modules —
the right shape for bots/agents. Shared-library import is the opt-in for real
dapps (runtime constructors + version coherence). Brands are structural:
inline copies of `bigint & {__brand:'Int'}` across files are mutually assignable,
so inlining is safe.

**TS-side Yocto may be bigint-branded** (client display math exact either way;
decimal-string conversion exactly at `nearCall()`). Cosmetic choice in T4;
correctness identical.

## Deliberately refused

- **Full HM / let-polymorphism** — bad error messages, generalization edge cases, ~zero value for a pool (≈2 polymorphic fns). Monomorphic + `Value`-default is the ceiling.
- **SMT inside the compile path** — Z3 = nondeterminism = threat to byte-identical emit and reproducible builds. Refinements, if ever, live in a separate opt-in `verify` command (decidable fragments only: nonneg, nonzero, range).
- **A second authoring syntax in TS** — builder chains "look like Rust." If authoring in TS starts shaping the language's look, the layer is overstepping. The `.lisp` is the language; TS is tooling/ABI/agents.
- **Typed AST as human syntax** — nested `If(…)` constructors are just the Lisp with worse highlighting. Their value is *programmatic* generation (agents, fuzzers), not human authoring.

## The one-table architecture

`OPS_SPEC` (`src/ops_spec.rs`) is the single source of truth. One table feeds:

```
interp (bytecode) ──┐
wasm emitter ───────┤
source checker ─────┼── OPS_SPEC: name · arity · backends · signature · (lisp type, effect)
types.gen.ts ───────┤     + golden drift tests chaining every consumer
TS SDK emit ────────┤
LSP (checker-as-library: same rules in editor and build — they cannot disagree)
```

## Gates (non-negotiable)

- **Byte-identity:** compile the full existing corpus before/after — wasm byte-identical. Annotations must be erasable, or stop and root-cause.
- **Suite green** at each staged commit (56-binary baseline; pre-existing deep-nesting env failure exempt + identical).
- Golden regen+compare for every generated artifact (`types.gen.ts`, SDK).

---
*Status: TASK-TYPED.md T1–T4 specced & committed (`bb1435f1`), queued behind M3. Effects/phantoms (T5/T6) agreed in principle, not yet in the task file.*
