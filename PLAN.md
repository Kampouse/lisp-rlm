# lisp-rlm — Master Plan

## Current State

**Commit:** `f26a062` on `main`
**Tests:** 557 pass / 248 fail / 805 total (69.2%)
**Fuzz harness:** 22/22 pass (test_wasm_fuzz.rs)
**F* Verification:** 54 files, 12,391 lines, 767 lemmas, 3 admits (0.39%)
**WASM build:** 796KB release, 625KB wasm-opt -Oz, 0 errors
**Lib tests:** 42/42 pass (0 regressions)

---

## Completed Work

### VM Correctness — All panics eliminated ✅
- Checked arithmetic at all sites (both VMs) — overflow returns `Err` instead of panicking
- `safe_slot` helper — OOB slot access returns `&LispVal::Nil` instead of panicking
- Stack underflow protection — `MakeList` and `TypedBinOp` in both VMs
- `num_arith_checked` Nil fallback fixed — was short-circuiting the overflow path
- Slot*MImm ops do NOT write back to slot — `Recur`/`RecurDirect` pop from stack
- `binop_name` helper for consistent error messages across all arithmetic paths

### Differential fuzz — overflow fuzz passes ✅
- `test_differential_fuzz_overflow`: 0 mismatches (was 22 failures)
- Root cause analysis: 211 failures → 5 categories (60% VM panics, 25% unsupported opcodes, 3 float coercion, div-by-zero, 1 SlotAddImm writeback)
- All panic categories fixed; remaining ~50 "slot index 0 out of bounds" in closure VM fixed with safe_slot

### F* Sync — Fixes 1-5 complete ✅
- Full diff analysis: `verification/FSTAR_VS_RUST_DIFF.md`
- Root cause #2 (`pure`/type annotations): all 8 sub-items done
- `pure_type` stored on `CompiledLambda`, propagated via `pending_pure_type` on `LoopCompiler`
- `desugar_define_to_pairs` returns `Vec<((String, LispVal), Option<String>)>`

### F* ClosureVM F64 TypedBinOp fix ✅
- Bug: F64 branch matched `Num na` (int) but called `typed_add_f64` (needs `ffloat`) — type error
- Fix: `to_ffloat a` / `to_ffloat b` for coercion, matching Rust's `Num(n) => *n as f64`
- Added missing F64 binops: `Mod`, `Eq`, `Lt`, `Le`, `Gt`, `Ge`
- Added `ff_rem` assume to `Lisp.Types.fst`
- `OpMod` uses `typed_mod_i64` helper to avoid F* `nonzero` proof obligation

### F* Soundness audit ✅
- 54 files, 12,391 lines, 767 lemmas
- 3 admits total (0.39%) — all in test files, zero in core semantics
- 1 pre-existing timeout in Soundness.fst `sound_add` (Z3 trigger issue, trivially true `x=x`)

### WASM compilation ✅
- Entire crate compiles to `wasm32-unknown-unknown` — 0 errors, 40 cosmetic warnings only
- Wasmtime behind `cfg(not(target_arch = "wasm32"))` gate
- Release WASM = 796KB, `wasm-opt -Oz` = 625KB

### Fuel metering + `.wasm` bench support ✅ (pushed)
- Commit `d0ea8a2`
- `Config::new().consume_fuel(true)` + `store.set_fuel(initial)` + `initial - store.get_fuel()?` for consumed
- `run_bench` auto-detects `.wasm` vs `.lisp` extension

### Build/infra fixes ✅
- `autobins = false` in Cargo.toml (Cargo auto-discovers `src/bin/*.rs`)
- `near-compile` as explicit `[[bin]]`
- Test helpers outside `#[cfg(test)]` (integration tests are separate crates)

---

## Phase 1: Emitter Correctness — Fuzz Harness ✅ (complete)

**Goal:** Differential fuzz between ClosureVM and WASM emitter. Same compiler front-end, two execution engines, compare outputs.

**Result:** 22/22 tests pass. 5 real emitter bugs found and fixed.

### 3-bit Tag Scheme (implemented)
- Bottom 3 bits = type tag, upper 61 bits = payload
- `TAG_NUM=0, TAG_BOOL=1, TAG_FNREF=2, TAG_CLOSURE=3, TAG_NIL=4, TAG_STR=5`
- Falsy set: `{Num(0)=0, Bool(false)=1, Nil=4}` — 0 is truthy (Lisp convention)
- `emit_tag(val, tag)` = `(val << 3) | tag`, `emit_untag()` = `val >> 3` (arithmetic shift)
- `finish_fuzz()` stores tagged value at `TEMP_MEM=64` (no untag, no value_return)

### Fuzz Harness (`tests/test_wasm_fuzz.rs`)
- `compile_fuzz(source)` → WASM with `fuzz_mode=true` (reuse `finish()` with gated untag)
- Dynamic wasmtime host stubs: iterate `module.imports()`, match exact signatures
- Handles both internal and imported memory
- Auto-calls `(run)` after defining it in ClosureVM for result comparison

### Bugs Found (all fixed)
1. **`emit_dynamic_call` untag used `>> 2` instead of `>> TAG_BITS`** — closures read from wrong heap offset
2. **`emit_is_truthy` stack corruption** — `I64Eq` consumed tagged value, breaking `if`/`and`/`or`
3. **Falsy set included `Num(0)`** — in Lisp, 0 is truthy
4. **`or` returned `Bool(true)` instead of actual truthy value**
5. **`not` missing `I64ExtendI32U`** — `I64Eqz` produces i32, `emit_tag_bool` expects i64

### Coverage
- Arithmetic: +, -, *, /, mod, abs, chained, deep nesting
- Comparisons: >, <, >=, <=, =, !=
- Logic: and, or, not (all edge cases)
- Control flow: if/then/else, begin, cond
- Functions: define, call, recursion (fibonacci), nested calls
- Closures: capture, higher-order (make-adder pattern)
- State: let bindings, set! mutation
- Zero-distinguishing: Num(0) ≠ Bool(false) ≠ Nil (the whole point)

### Skipped (pre-existing ClosureVM limitations)
- `for` loop — ClosureVM doesn't compile `for` the same way
- `while` loop — not a builtin in the eval environment

---

## Phase 2: Emitter Correctness — F* WASM Model

**Goal:** Formal proof that the WASM emitter preserves bytecode semantics.

**Why after fuzz:** Fuzz finds the bugs. F* proves there are no more. The fuzz corpus from Phase 1 guides what to model and what lemmas are needed. Each fuzz crash that gets fixed becomes a regression lemma.

### Proof Chain

```
Source → CompilerSpec → Bytecode (DONE: 767 lemmas)
                                    ↓
                         WASM Emitter Spec (NEW)
                                    ↓
                         WASM Instructions (NEW)
                                    ↓
                         WASM Eval (NEW)
                                    ↓
              ≈ Bytecode VM (simulation proof, NEW)
```

### Steps

- [ ] 2.1 `semantics/wasm/Wasm.Types.fst` — WASM instruction enum, memory model
  - ~30 instruction types used by the emitter (i64.const, i64.add, i64.load, local.set, br, if, etc.)
  - Linear memory model (i32 address space, little-endian)
  - Stack machine state: `(instrs: list instr, stack: list i64, locals: list i64, memory: memory, pc: nat)`

- [ ] 2.2 `semantics/wasm/Wasm.Eval.fst` — WASM step function
  - One step per instruction type
  - Determinism proof
  - Fuel/budget parameter for termination

- [ ] 2.3 `semantics/wasm/Wasm.Emitter.fst` — bytecode → WASM translation spec
  - For each bytecode opcode: what WASM instruction sequence the emitter produces
  - Memory layout spec (tagged value encoding, slot layout, code table)
  - Extract from `wasm_emit.rs` patterns

- [ ] 2.4 `semantics/wasm/Wasm.EmitterCorrectness.fst` — simulation proof
  - **Main theorem:** For each bytecode step, the emitted WASM sequence has equivalent observable effect
  - Step-indexed simulation relation: `rel(vm_state, wasm_state)`
  - Key invariants: stack correspondence, memory correspondence, pc advancement
  - ~200-300 lemmas estimated

- [ ] 2.5 Verify against fuzz corpus
  - Each program in the fuzz corpus becomes a concrete test lemma
  - If all pass, high confidence the simulation is complete

### What F* Already Provides (no rework needed)
- Source → Bytecode compiler correctness (LispIR.CompilerCorrectness)
- Bytecode VM determinism (LispIR.Determinism)
- Stack height preservation (LispIR.StackHeight)
- Universality (LispIR.Universality)

### What F* Cannot Prove (fuzz covers these)
- Actual wasmtime behavior (wasmtime IS the ground truth for WASM sem)
- Host function interaction (storage, logging, promises)
- Real memory layout bugs (off-by-one, alignment)
- Integer overflow in WASM (WASM i64 wrapping vs checked arithmetic)

---

## Phase 3: Structural Refactor (in progress, paused)

### 3a: Extract wasmtime mock runtime (analysis done, no code yet)
- 4 duplication sites in `src/bin/near_compile.rs` (run_test_fn, run_bench, eval_wasm, run_wasmtime)
- ~40 `linker.define()` calls for NEAR host function stubs
- Extract to `src/near_mock_runtime.rs` or `src/wasm_test_harness.rs`
- Differences: fuel metering (bench only), shared state Arcs (REPL only), return type (i64 vs String+logs)

### 3b: Split bytecode.rs (not started)
- 4,342 lines — unmaintainable
- Target: `loop_vm.rs` + `closure_vm.rs` + `shared.rs`
- Must not break any of the 557 passing tests

### 3c: Crate split (deferred — PLAN.md original phases 1-5)
- Original plan: split into lisp-core, lisp-vm, lisp-wasm, near-vm
- Blocked on: structural stability (bytecode split needs to land first)
- See original PLAN.md phases for detailed task breakdown

---

## Phase 4: Remaining Verification Gaps

### 4.1 F* Model — Semantic Mismatches (~1172 fuzz failures)
- NaN propagation differences
- Float→int coercion edge cases
- Dict op error conditions
- Mod overflow edge cases
- Priority: LOW — fuzz finds these faster than formal proofs

### 4.2 F* Model — Missing Opcodes
- ~25% of original 211 fuzz failures from unsupported opcodes
- SpecVm handles them, Rust VM has catch-all error
- Fix: add opcode cases to F* ClosureVM model

### 4.3 F* Model — Soundness.fst timeout
- `sound_add` lemma (line 141): Z3 can't prove `val_eq_num (op_int_add a b) (op_int_add a b)`
- Pre-existing trigger issue, not a soundness gap
- Fix: add explicit unfold hints or reformulate

---

## Phase 5: Test Suite Health

### Fully green test files (17 files, 170 tests) ✅
- lib unit tests, fuzz_test, bytecode_shadow, compiler_extensions, compiler_v3, compose, deep, hof_fastpaths, lambda_hof, let_debug, let_loop, proven_programs, repeat, repeat2, repeat3, shadow_minimal, trace_overflow

### Mixed test files (15 files, 363 tests) ~
- core_language (141/19), test_syntax_coverage (86/13), test_types_extended (9/32), test_runtime_features (15/25), test_macros (10/21), test_pure_probe_arrow (11/11), test_stdlib_tier1 (22/4), test_types (3/15), test_loop_recur (3/8), deftype_tests (13/2), test_pure_types (11/4), test_fast_path (12/2), test_budget (11/1), test_bytecode_coverage (7/4), test_closure_mutation (1/4), test_compiler_v2 (28/1)

### Fully red test files (14 files, 116 tests) ✗
- test_harness_extended (0/36), test_harness_full (0/29), test_compiler_bugs (0/4), test_edge (0/3), test_harness (0/3), norvig_tests (0/1), + 8 single-test failures

### Priority for fixing
1. Fully red files — likely API mismatches or missing features, high bug/effort ratio
2. Mixed files with low pass rate — test_types (3/15), test_loop_recur (3/8)
3. High-value single failures in mostly-green files

---

## Verification Status Summary

### F* Core Semantics (18 files)
| Module | Status | Notes |
|--------|--------|-------|
| Lisp.Types | ✅ verified | ffloat assume vals, added ff_rem |
| Lisp.Values | ✅ verified | |
| Lisp.Source | ✅ verified | |
| Lisp.Closure | ✅ verified | |
| Lisp.Compiler | ✅ verified | |
| LispIR.Semantics | ✅ verified | |
| LispIR.CompilerSpec | ✅ verified | |
| LispIR.CompilerSpec3 | ✅ verified | |
| LispIR.CompilerSpec4 | ✅ verified | |
| LispIR.ClosureVM | ✅ verified | Fixed F64 TypedBinOp branch |
| LispIR.CompilerCorrectness | ✅ verified | |
| LispIR.Correctness | ✅ verified | |
| LispIR.Determinism | ✅ verified | |
| LispIR.PerExpr3 | ✅ verified | |
| LispIR.Soundness | ⚠️ 1 timeout | sound_add trigger issue |
| LispIR.StackHeight | ✅ verified | |
| LispIR.Universality | ✅ verified | |

### F* Test Proofs (37 files)
- 35 files: ✅ all VCs discharged
- 1 file (HardOpcodeProofs): ✅ verified, 2 admits (F64 TypedBinOp — Z3 can't unfold through to_ffloat)
- 1 file (MapFilterReduce): ✅ verified, 1 admit (dict set — Z3 quantifier trigger)

### Total Admits: 3 / 767 lemmas (0.39%)
- All in test files, none in core semantics
- All are Z3 automation limits, not specification gaps

---

## Dependency Graph (current priority order)

```
Phase 1 (Fuzz harness) ✅ ──────────────────────────┐
    ↓                                              │
Phase 2 (F* WASM model) ← uses fuzz corpus ───────┘
    ↓
Phase 3 (Structural refactor) ← stable verification
    ↓
Phase 4 (Verification gaps) ← incrementally
    ↓
Phase 5 (Test suite health) ← ongoing
```

## Key Principle
Phase 1 (fuzz) and Phase 2 (F*) are complementary. Fuzz finds bugs immediately — Phase 1 found 5 real emitter bugs. F* proves there are no more of that class. The fuzz corpus guides what to model in F*. When fuzz stops finding bugs, the F* proof tells you why.
