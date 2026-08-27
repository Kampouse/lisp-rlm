# Corpus Coverage Matrix

Last audited: 2026-08-27 · pyramid: trace-equiv (interp↔wasm) → near-mock →
near-vm-run oracle → sandbox (verify.sh)

## Legend
✅ covered & green · ⚠️ covered, known divergence · ❌ hole · 🔬 wasm-only (no interp differential)

## A. Pure language surface (tests/equiv/ — interpreter ↔ wasm differential)

| Feature            | Probe                     | Status |
|--------------------|---------------------------|--------|
| arithmetic int     | e01, e12                  | ✅ / ⚠️(e12 uses try) |
| arithmetic edges   | e18                       | ⚠️ try — interp-only form |
| strings            | e02, e17                  | ✅ |
| string builtins deep | e24, e32 (NEW: upcase/downcase/trim/starts/ends/replace) | ✅ |
| bool logic         | e03, e25 (NEW)            | ✅ |
| truthiness         | e04                       | ✅ |
| let / shadowing    | e05, e26 (NEW, deep)      | ✅ |
| closures           | e06                       | ⚠️ inner (define (f)) unsupported in wasm |
| while              | e07, e28 (NEW, composite) | ✅ |
| dotimes            | e27 (NEW, nested)         | ✅ |
| u128 string math   | e08, e16 (edge)           | ✅ / ⚠️(try) · e29 (NEW) |
| lists              | e09, e15, e30 (NEW)       | ✅ |
| recursion          | e10, e31 (NEW, mutual)    | ✅ |
| arity errors       | e11                       | ⚠️ uses try |
| typed arith        | e12                       | ⚠️ uses try |
| equality           | e13                       | ⚠️ wasm stricter (heterogeneous lists) |
| value defines      | e14, e20                  | ✅ |
| control flow       | e19                       | ✅ |
| neq                | e23                       | ✅ |
| `try` form         | e11/e12/e16/e18           | ❌ wasm checker rejects — form unimplemented in wasm_emit |

### A.1 String builtin surface (3-way: checker ∩ interp ∩ wasm_emit)
Round 2 (2026-08-27): str-upcase/downcase/trim/starts-with/ends-with/replace
now wasm-emitted (e32). Divergences:
- ASCII-bounded: wasm case/trim is ASCII-only; interp is Rust Unicode —
  non-ASCII input diverges (probes stay ASCII).
- str-replace: literal from/to only in wasm; empty pattern = wasm compile
  error (interp inserts between chars).
- println renders raw control bytes differently across surfaces (display
  divergence only — assert via str-length for control-byte strings).
- Parser unescapes now UNIFIED: n t r 0 bs quote xHH single-pass in
  parser.rs, matching wasm_emit/helpers.rs (e32 finding: backslash-r was
  2 chars interp / 1 wasm). v f are literal (no escape) on BOTH surfaces.
Still interp-only (need emission): str-split*, str-chunk, str-join,
string->list, list->string.
| quasiquote         | —                         | ❌ no probe |
| set! mutation      | —                         | ❌ no dedicated probe |
| return in loops    | —                         | ❌ (torture t10 partial) |

## B. Compiler torture (tests/compiler-torture/ — 33 files, wasm-side)
shadow matrix, error-in-loops, recursion depth, coercion, string edges,
builtin shadow, ... (run: `cargo test` — see tests harness)

## C. NEAR surface (corpus/ contracts → deploy → 3-layer verify)

| Builtin               | erc20 | safe | voting (NEW) | battery |
|-----------------------|-------|------|--------------|---------|
| storage_set/store     | ✅    | ✅   | ✅           | ✅      |
| storage_get/load      | ✅    | ✅   | ✅           | ✅      |
| has_key               | —     | ✅   | ✅           | —       |
| remove                | —     | ✅   | ✅ (cleanup) | —       |
| signer_account_id     | —     | ✅   | ✅           | —       |
| transfer_u128         | —     | ✅   | —            | —       |
| json_get_str          | ✅(shim) | ✅ | ✅(shim)     | ✅(shim)|
| json_return_str       | ✅(shim) | ✅ | ✅(shim)     | ✅(shim)|
| predecessor           | ✅    | —   | —            | —       |
| view/write split      | ✅    | ✅   | ✅           | —       |

## D. Known divergences (bugs filed by this corpus)
1. `try` form: interpreter-only — wasm checker hard-errors (e11/e12/e16/e18)
2. Inner function defines (closures) unsupported in wasm (e06) — T4-adjacent
3. Heterogeneous lists: wasm checker stricter than interpreter (e13)
4. 4 closure fuzz reds (see memory 2026-08-25)
