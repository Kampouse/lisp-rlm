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

## Phase 1: Workspace Setup
- [ ] Create workspace `Cargo.toml` at repo root
- [ ] Create `crates/lisp-core/` with empty lib
- [ ] Move `parser.rs`, `types.rs`, `helpers.rs` into `lisp-core`
- [ ] Verify `lisp-core` compiles standalone
- [ ] Verify `lisp-core` compiles to `wasm32-unknown-unknown`

## Phase 2: WASM Emitter Split
- [ ] Create `crates/lisp-wasm/` with empty lib
- [ ] Move `wasm_emit.rs` base (core emit, define, call, if/else, let, while) into `lisp-wasm/src/emit.rs`
- [ ] Split into modules:
  - [ ] `src/emit.rs` — core codegen (define, call, if, let, while, for, set!)
  - [ ] `src/hof.rs` — hof/map, hof/filter, hof/reduce, extract_lambda
  - [ ] `src/storage.rs` — near/storage_set/get/has/remove
  - [ ] `src/u128.rs` — all u128 ops, parse_u128
  - [ ] `src/logging.rs` — near/log, near/log_num
  - [ ] `src/json.rs` — json_get_int/str/u128, json_return_int/str (depth tracking, boundary checks)
  - [ ] `src/typing.rs` — lightweight type checker
  - [ ] `src/host.rs` — need_host registration, host call indices
  - [ ] `src/tree_shake.rs` — dead code elimination
  - [ ] `src/near_validate.rs` — WASM validation with function-name mapping
  - [ ] `src/lib.rs` — public API: compile_near, compile_near_named, resolve_modules
- [ ] All existing tests pass
- [ ] `near-compile` binary still works

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
