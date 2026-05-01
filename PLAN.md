# Refactor Plan — lisp-rlm Workspace

## Goal
Split monolithic `lisp-rlm-wasm` into focused crates. Each compiles independently. `lisp-core` compiles to `wasm32` enabling on-chain Lisp VM.

## Target Structure

```
lisp-rlm/
├── Cargo.toml              # workspace root
├── crates/
│   ├── lisp-core/          # Shared: parser, types, helpers
│   ├── lisp-vm/            # Bytecode VM + eval + stdlib
│   ├── lisp-wasm/          # WASM emitter (near-compile backend)
│   └── near-vm/            # On-chain Lisp VM contract
├── bin/
│   ├── near-compile/       # CLI: REPL, deploy, project system
│   ├── lisp-run/           # CLI: native REPL
│   └── lisp-vm-contract/   # Binary: on-chain contract
├── verification/           # F* proofs (unchanged)
├── tests/                  # Integration tests
├── GAPS.md
├── PLAN.md
└── README.md
```

## Phase 1: Workspace Setup ✅ DONE (commit 8f432a4)
- [x] Create workspace `Cargo.toml` at repo root
- [x] Create `crates/lisp-core/` with parser, types, helpers
- [x] Feature-gate eval/bytecode deps behind `#[cfg(feature = "full")]`
- [x] `lisp-core` compiles standalone
- [x] `lisp-core` compiles to `wasm32-unknown-unknown` ✅

## Phase 2: WASM Emitter Split ✅ DONE
- [x] Create `crates/lisp-wasm/` with Cargo.toml
- [x] Split into modules: emit, json, tree_shake, host, logging, hof, u128
- [x] Root `src/wasm_emit.rs` → thin re-export
- [x] All 14 WASM tests pass
- [x] near-compile works, test_json compiles (2762 bytes)

## Phase 3: Bytecode VM Split
- [ ] Create `crates/lisp-vm/` 
- [ ] Move `bytecode.rs`, `program.rs` into `lisp-vm`
- [ ] Move `eval/` directory into `lisp-vm`
- [ ] Feature-gate heavy deps:
  - [ ] `crypto` feature (sha256, keccak) — off by default
  - [ ] `http` feature (reqwest, fetch) — off by default
  - [ ] `llm` feature (provider, openai) — off by default
  - [ ] `stdlib-full` feature (all stdlib) — on by default
- [ ] `lisp-vm` with no extra features compiles to `wasm32`
- [ ] All existing tests pass

## Phase 4: On-chain VM
- [ ] Create `crates/near-vm/`
- [ ] Minimal eval: arithmetic, logic, let, define, if, while, strings, lists
- [ ] NEAR host function bindings (storage, log, input, value_return)
- [ ] JSON input/output (parse args, return results)
- [ ] Compile to `wasm32-unknown-unknown`
- [ ] Deploy to testnet
- [ ] Test: `near call contract eval '{"expr": "(+ 1 2)"}'` → 3

## Phase 5: CLI Binaries
- [ ] Create `bin/near-compile/` — existing near_compile.rs + REPL + deploy + project system
- [ ] Create `bin/lisp-run/` — existing rlm.rs
- [ ] Both depend on their respective crates
- [ ] All commands work: `near-compile init/build/deploy/test --repl`
- [ ] `lisp-run` works as before

## Phase 6: Cleanup
- [ ] Update GAPS.md for new structure
- [ ] Update README.md
- [ ] Update co-dev's F* verification paths if needed
- [ ] Remove old monolithic `src/` directory
- [ ] Git history preserved — each phase is a commit

## Phase 7: Memory Bounds Checking
- [ ] Add compile-time offset validation for constant stores
- [ ] Add runtime bounds checks for dynamic offsets (trap before out-of-bounds write)
- [ ] Statically prove: all known offsets (TEMP_MEM, LOG_BUF, etc.) are within 4 pages

## Phase 8: WASM Differential Fuzzing
- [ ] Extend `test_differential_fuzz.rs` to also test WASM emitter output
- [ ] Compare: Lisp source → WASM → wasmtime execution vs bytecode VM vs F* spec
- [ ] Triple equivalence: F* spec = bytecode VM = WASM emitter
- [ ] Run millions of random programs through all three, verify identical results

## Dependency Graph

```
lisp-core (no external deps except `im`)
    ↑
    ├── lisp-vm (+ optional crypto, http, llm)
    ├── lisp-wasm (+ wasm-encoder, wasmparser)
    └── near-vm  (+ no external deps, compiles to wasm32)
    
near-compile → lisp-wasm + lisp-vm + wasmtime
lisp-run     → lisp-vm
near-vm      → lisp-core only
```

## Key Principle
Each phase is a working commit. Never break the build. Co-dev's verification must pass at every step.

## Notes
- Start from current HEAD (`78fdc10`)
- Work on a `refactor/workspace` branch
- Merge to main after all phases complete
- Co-dev's `verification/` directory stays untouched
