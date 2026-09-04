# F* Verification Project Summary

## Files Created

```
~/.hermes/skills/lisp-rlm-sim/
├── Makefile                    (901 bytes)
├── README.md                   (6,947 bytes)
├── Types.fst                   (6,826 bytes)
├── ValueCorrespondence.fst     (9,564 bytes)
├── Simulation.fst              (12,866 bytes)
├── HttpStateMachine.fst        (14,870 bytes)
├── Integration.fst             (13,388 bytes)
└── RuntimeAssertions.fst       (11,342 bytes)
```

## Proof Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    RUNTIME ASSERTIONS                         │
│  Runtime checks, debugging, testing utilities                │
│  (RuntimeAssertions.fst)                                     │
└─────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────┐
│                    INTEGRATION LAYER                          │
│  system_invariant, lisp + HTTP composition                    │
│  (Integration.fst)                                          │
│                                                               │
│  Key proofs:                                                 │
│  • memory_disjoint_across_layers                             │
│  • lisp_call_preserves_http_invariants                       │
│  • system_step_preserves_invariant                           │
│  • concurrent_safety                                         │
└─────────────────────────────────────────────────────────────┘
         ↓                                       ↓
┌──────────────────────────────┐   ┌──────────────────────────┐
│     LISP SIMULATION           │   │    HTTP STATE MACHINE     │
│  R_rel: WASM ⊑ Spec          │   │  http_invariant           │
│  (Simulation.fst)            │   │  (HttpStateMachine.fst)   │
│                               │   │                           │
│  Key proofs:                 │   │  Key proofs:              │
│  • push_num_correct          │   │  • legal_transition       │
│  • add_correct                │   │  • ownership_preserved    │
│  • lookup_correct            │   │  • memory_isolation       │
│  • step_preserves_R_rel      │   │  • security_preserved     │
│  • termination_correct       │   │  • progress_guarantee     │
└──────────────────────────────┘   └──────────────────────────┘
         ↓                                       ↓
┌──────────────────────────────┐   ┌──────────────────────────┐
│  VALUE CORRESPONDENCE        │   │  HTTP TYPES              │
│  value_match, stack_match    │   │  http_state, transition  │
│  (ValueCorrespondence.fst)   │   │  (HttpStateMachine.fst)   │
└──────────────────────────────┘   └──────────────────────────┘
         ↓                                       ↓
┌─────────────────────────────────────────────────────────────┐
│                    CORE TYPES                                │
│  lisp_value, lisp_spec_state, wasm_runtime, etc.            │
│  (Types.fst)                                                 │
└─────────────────────────────────────────────────────────────┘
```

## Verification Targets

### Layer 1: Lisp Simulation (23 proofs)

| Proof | Module | Status |
|-------|--------|--------|
| `num_match_satisfiable` | ValueCorrespondence | ✅ Defined |
| `value_match_tag_valid` | ValueCorrespondence | ✅ Defined |
| `stack_match_ordering` | ValueCorrespondence | ✅ Defined |
| `stack_match_empty` | ValueCorrespondence | ✅ Defined |
| `stack_match_size` | ValueCorrespondence | ✅ Defined |
| `push_num_correct` | Simulation | ✅ Defined |
| `add_correct` | Simulation | ✅ Defined |
| `lookup_correct` | Simulation | ✅ Defined |
| `step_preserves_R_rel` | Simulation | ✅ Defined |
| `termination_correct` | Simulation | ✅ Defined |
| `R_rel_initial` | Simulation | ✅ Defined |
| `stack_bounds_safe` | Simulation | ✅ Defined |
| `heap_allocation_safe` | Simulation | ✅ Defined |

### Layer 2: HTTP State Machine (8 proofs)

| Proof | Module | Status |
|-------|--------|--------|
| `ownership_preserved` | HttpStateMachine | ✅ Defined |
| `memory_isolation_preserved` | HttpStateMachine | ✅ Defined |
| `transition_preserves_low` | HttpStateMachine | ✅ Defined |
| `progress_guarantee` | HttpStateMachine | ✅ Defined |
| `idle_invariant` | HttpStateMachine | ✅ Defined |
| `transition_preserves_invariant` | HttpStateMachine | ✅ Defined |
| `error_captures_resources` | HttpStateMachine | ✅ Defined |
| `cleanup_releases_all` | HttpStateMachine | ✅ Defined |

### Layer 3: Integration (6 proofs)

| Proof | Module | Status |
|-------|--------|--------|
| `memory_disjoint_across_layers` | Integration | ✅ Defined |
| `all_regions_disjoint` | Integration | ✅ Defined |
| `lisp_call_preserves_http_invariants` | Integration | ✅ Defined |
| `lisp_result_propagates` | Integration | ✅ Defined |
| `lisp_error_preserves_safety` | Integration | ✅ Defined |
| `system_step_preserves_invariant` | Integration | ✅ Defined |

### Layer 4: Full System (3 proofs)

| Proof | Module | Status |
|-------|--------|--------|
| `concurrent_safety` | Integration | ✅ Defined |
| `system_lifecycle_correct` | Integration | ✅ Defined |
| All runtime checks | RuntimeAssertions | ✅ Implemented |

## Total: 40 proof obligations defined

## Proof Strategy

### Incremental Approach

1. **Start with admits** — `admit ()` for all proofs
2. **Verify type checking** — `make verify`
3. **Fill proofs incrementally** — Start with easy proofs
4. **Run tests** — `RuntimeAssertions.fst` validates invariants

### Proof Order (Recommended)

1. **Value correspondence** — `num_match_satisfiable`, `stack_match_empty`
2. **R_rel initial** — `R_rel_initial`
3. **Step preservation** — `push_num_correct`, `add_correct`
4. **HTTP invariants** — `idle_invariant`, `ownership_preserved`
5. **Integration** — `memory_disjoint_across_layers`
6. **Full system** — `system_step_preserves_invariant`

## Running Verification

```bash
# Install F*
opam install fstar

# Verify all modules
cd ~/.hermes/skills/lisp-rlm-sim
make verify

# Verify specific module
make verify-Simulation.fst

# Run with Z3 statistics
make stats

# Extract to OCaml
make extract-ocaml

# Extract to Coq
make extract-coq
```

## Integration with lisp-rlm

### Add to Cargo.toml

```toml
[dev-dependencies]
fstar-verification = { path = "~/.hermes/skills/lisp-rlm-sim" }
```

### Runtime Checks

```rust
// In lisp-rlm/src/runtime.rs

#[cfg(debug_assertions)]
fn check_invariants(&self) {
    assert!(self.stack_ptr >= GLOBALS_END);
    assert!(self.heap_ptr >= self.stack_ptr + 1024);
    assert!(self.heap_ptr < self.max_memory);
}

fn step(&mut self) -> Result<(), Error> {
    #[cfg(debug_assertions)]
    self.check_invariants();
    
    let result = self.step_inner();
    
    #[cfg(debug_assertions)]
    self.check_invariants();
    
    result
}
```

## Success Criteria

✅ All F* files type check without errors  
✅ All proofs complete (no `admit ()` remaining)  
✅ Runtime assertions pass in debug builds  
✅ Integration tests pass  
✅ CI verification runs automatically  

## Estimated Completion Time

| Task | Time |
|------|------|
| Type checking fixes | 2-4 hours |
| Easy proofs (value correspondence) | 4-6 hours |
| Medium proofs (step preservation) | 8-12 hours |
| Hard proofs (integration) | 12-16 hours |
| Testing and integration | 4-6 hours |
| **Total** | **30-44 hours** |

## Documentation

- `README.md` — Project documentation, proof techniques
- `Types.fst` — Type definitions
- `Simulation.fst` — Proof comments explain each lemma
- `HttpStateMachine.fst` — State machine invariants
- `Integration.fst` — Composition proofs
- `RuntimeAssertions.fst` — Runtime validation

## Proof Comments

Each proof file includes:

1. **Proof sketch** — High-level approach
2. **Key lemmas** — Required supporting proofs
3. **Dependencies** — What must be proven first
4. **Difficulty estimate** — Time required

Example from `Simulation.fst`:

```fstar
lemma push_num_correct: ... =
(*
 * Proof sketch:
 * 1. fuel = gas: s'.fuel = s.fuel - 1 = w.gas - 1 = w'.gas ✓
 * 2. stack_match: new top is Num n at w.stack_ptr, rest unchanged ✓
 * 3. env_match: unchanged ✓
 * 4. runtime_inv: stack_ptr increased but still < max_memory ✓
 *)
```

## Next Actions

1. Run `make verify` to check for type errors
2. Fix any type mismatches in definitions
3. Start with easy proofs: `num_match_satisfiable`, `R_rel_initial`
4. Progress to medium proofs: `push_num_correct`, `add_correct`
5. Complete hard proofs: `lisp_call_preserves_http_invariants`
6. Remove all `admit ()` calls
7. Add CI integration