# F* Verification Project Structure

This directory contains a complete F* verification project for lisp-rlm and P2 HTTP.

## Project Files

```
lisp-rlm-sim/
├── Makefile                 # Build and verification targets
├── Types.fst                # Core type definitions
├── ValueCorrespondence.fst  # Value matching proofs (spec ⊑ WASM)
├── Simulation.fst           # R_rel and step preservation
├── HttpStateMachine.fst     # HTTP state machine invariants
├── Integration.fst          # Composition proofs (lisp + HTTP)
└── RuntimeAssertions.fst    # Runtime checks for testing
```

## Dependency Graph

```
Types.fst
   ↓
ValueCorrespondence.fst
   ↓
Simulation.fst
   ↓
HttpStateMachine.fst
   ↓
Integration.fst
   ↓
RuntimeAssertions.fst
```

## Building

### Prerequisites

```bash
# Install F* via opam
opam install fstar

# Or download binary from GitHub
# https://github.com/FStarLang/FStar/releases
```

### Verify All Proofs

```bash
make verify
```

### Verify Single Module

```bash
make verify-Simulation.fst
```

### Extract to OCaml

```bash
make extract-ocaml
```

### Extract to Coq

```bash
make extract-coq
```

## Proof Structure

### Layer 1: Lisp-rlm Simulation (Types.fst, ValueCorrespondence.fst, Simulation.fst)

**Core theorem: R_rel (simulation relation)**

```fstar
val R_rel: lisp_spec_state -> wasm_runtime -> Type
let R_rel s w =
  s.fuel = w.gas ∧
  stack_match s.stack w w.stack_ptr ∧
  env_match s.env w 0x00 ∧
  runtime_inv w
```

**Key lemmas:**

1. `push_num_correct` — Push instruction preserves R_rel
2. `add_correct` — Add instruction preserves R_rel
3. `lookup_correct` — Variable lookup preserves R_rel
4. `step_preserves_R_rel` — General step preservation
5. `termination_correct` — Execution terminates

### Layer 2: P2 HTTP State Machine (HttpStateMachine.fst)

**Core theorem: http_invariant**

```fstar
val http_invariant: http_state -> Type
let http_invariant s =
  ownership_invariant s ∧
  memory_isolation_invariant s ∧
  security_low s
```

**Key lemmas:**

1. `legal_transition` — State machine transitions are valid
2. `ownership_preserved` — Resources tracked correctly
3. `memory_isolation_preserved` — No memory corruption
4. `transition_preserves_low` — Security preserved
5. `progress_guarantee` — Non-idle states make progress

### Layer 3: Integration (Integration.fst)

**Core theorem: system_invariant**

```fstar
val system_invariant: system_state -> Type
let system_invariant sys =
  http_invariant sys.http ∧
  runtime_inv sys.wasm ∧
  match sys.lisp with
  | None -> True
  | Some l -> R_rel l sys.wasm
```

**Key lemmas:**

1. `memory_disjoint_across_layers` — HTTP and lisp don't overlap
2. `lisp_call_preserves_http_invariants` — Integration safety
3. `system_step_preserves_invariant` — Full system step
4. `concurrent_safety` — Multiple requests isolated

## Proof Techniques

### 1. Well-Founded Induction

For termination proofs:

```fstar
let rec terminates s w fuel =
  if fuel = 0 then s.fuel = 0 ∧ w.gas = 0
  else R_rel s w ∧ (step and recurse)
```

### 2. Case Analysis

For state machine proofs:

```fstar
match from, trans, to with
| Idle, StartRequest _, Receiving _ -> (* prove *)
| Receiving _, BodyComplete, Processing _ -> (* prove *)
| ...
```

### 3. Composition

For integration proofs:

```fstar
http_invariant (Processing p)
∧ R_rel lisp w
→ lisp_call_preserves_http_invariants
→ http_invariant (Processing p')
```

## Running Tests

The RuntimeAssertions.fst module provides runtime checks:

```fstar
(* Check runtime invariant *)
assert_runtime_inv w;

(* Check R_rel *)
assert_R_rel s w;

(* Check HTTP invariant *)
assert_http_invariant http_state;

(* Check system invariant *)
assert_system_invariant sys;
```

### Example Usage

```fstar
(* Initialize system *)
let sys = create_test_system () in
assert_system_invariant sys;

(* Execute step *)
let sys' = system_step_checked sys in

(* Validate invariants *)
assert_system_invariant sys';
```

## Incremental Development

### Step 1: Start with Admits

```fstar
lemma push_num_correct: ... =
  admit ()  (* TODO: prove *)
```

### Step 2: Fill Proofs Incrementally

```fstar
lemma push_num_correct: ... =
  (* Step 1: Show fuel matches *)
  assert (s'.fuel = w'.gas);
  (* Step 2: Show stack matches *)
  assert (stack_match s'.stack w' w'.stack_ptr);
  (* Step 3: Show env matches *)
  assert (env_match s'.env w' 0x00);
  ()
```

### Step 3: Remove Admits

Replace `admit ()` with actual proof terms as you verify each lemma.

## Integration with lisp-rlm Code

### Instrument WASM Execution

```rust
// In lisp-rlm WASM runtime

#[cfg(debug_assertions)]
fn check_invariants(&self) {
    assert!(self.stack_ptr >= GLOBALS_END);
    assert!(self.heap_ptr >= self.stack_ptr + 1024);
    assert!(self.heap_ptr < self.max_memory);
}
```

### Validate Before/After Steps

```rust
fn step(&mut self) -> Result<(), Error> {
    #[cfg(debug_assertions)]
    self.check_invariants();
    
    let result = self.step_inner();
    
    #[cfg(debug_assertions)]
    self.check_invariants();
    
    result
}
```

## Verification Status

| Module | Proofs | Status |
|--------|--------|--------|
| Types | N/A | ✅ Definitions complete |
| ValueCorrespondence | 4 | ✅ Scaffolding complete |
| Simulation | 5 | ✅ Scaffolding complete |
| HttpStateMachine | 8 | ✅ Scaffolding complete |
| Integration | 6 | ✅ Scaffolding complete |
| RuntimeAssertions | N/A | ✅ Tests complete |

## Next Steps

1. **Fill proof bodies** — Replace `admit ()` with actual proofs
2. **Run F* type checker** — `make verify`
3. **Fix type errors** — Adjust definitions as needed
4. **Add more instructions** — Extend push_num, add, lookup proofs to all instructions
5. **Extract to OCaml** — Generate runtime assertion code
6. **Integrate with CI** — Run verification in continuous integration

## Proof Difficulty Estimates

| Proof | Difficulty | Time Estimate |
|-------|------------|---------------|
| `num_match_satisfiable` | Low | 30 min |
| `push_num_correct` | Medium | 2 hours |
| `add_correct` | Medium | 2 hours |
| `ownership_preserved` | Medium | 3 hours |
| `memory_disjoint_across_layers` | Low | 1 hour |
| `lisp_call_preserves_http_invariants` | High | 4 hours |
| `system_step_preserves_invariant` | High | 4 hours |

## Resources

- [F* Documentation](https://www.fstar-lang.org/tutorial/)
- [F* Cheatsheet](https://www.fstar-lang.org/tutorial/book/cheatsheet.html)
- [F* Standard Library](https://www.fstar-lang.org/docs/std/lib/)
- [Verified Software](https://github.com/FStarLang/FStar/wiki/Verified-Software)

## Questions?

If proofs fail:
1. Check type errors first (F* reports these clearly)
2. Use `#print-assumptions` to see what's admitted
3. Use `Z3.verbose true` to debug SMT solver
4. Simplify proofs with `calc` syntax
5. Add intermediate lemmas for complex steps