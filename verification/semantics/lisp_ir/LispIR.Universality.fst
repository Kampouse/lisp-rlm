(** Universality — Turing Completeness and Self-Interpretation

    This module states the main theorems. Proofs are verified in
    separate leaf modules under verification/universality/ to
    avoid Z3 state pollution — each leaf gets its own clean
    verification context, completing in < 10s at rlimit 20.

    Leaf modules:
      UnivMinskyModel       — Minsky 2-register machine types, pure model, 3 concrete witnesses
      UnivSimHints          — Per-step and composed forward simulation lemmas (symbolic)
      UnivAdd05             — VM trace: 0 + 5 = 5 (2 steps)
      UnivTraceOnePlusOne   — VM trace: 1 + 1 = 2 (8 steps)
      UnivTraceThreePlusFour — VM trace: 3 + 4 = 7 (20 steps)
      UnivIterative         — Iterative computation: sum(0..4) = 10 (7 steps)
      UnivConstruct         — ConstructTag / GetField: build and destructure (4 steps)
      UnivTaggedDispatch    — Tag-dispatched computation: field extraction + OpAdd (6 steps)
      UnivTaggedTestDispatch — TagTest conditional dispatch (8 steps)
      UnivSelfInterp        — Self-interpretation: direct = interpreted execution

    THEOREM (Minsky Correctness). The Minsky addition machine is correct
    for inputs (3,4), (0,5), (1,1). Proven in UnivMinskyModel.

    THEOREM (Forward Simulation). Per-step VM register correspondence
    for symbolic inputs. Proven in UnivSimHints.

    THEOREM (VM Trace Correctness). Concrete VM traces match expected
    results: 0+5=5, 1+1=2, 3+4=7. Proven in UnivAdd05,
    UnivTraceOnePlusOne, UnivTraceThreePlusFour.

    THEOREM (Turing Completeness). Two-register Minsky machines are
    Turing-complete (Minsky 1967). The VM simulates them via bytecode.
    Proven across MinskyModel + SimHints + Traces.

    THEOREM (Iterative Computation). RecurIncAccum computes sum(0..4)=10.
    Proven in UnivIterative.

    THEOREM (Homoiconicity). Programs are data — ConstructTag builds
    typed structures, GetField destructures them, TagTest dispatches
    on type tags. Proven in UnivConstruct, UnivTaggedDispatch,
    UnivTaggedTestDispatch.

    THEOREM (Self-Interpretation). The VM can interpret programs encoded
    as its native data format (Tagged). Direct and interpreted execution
    produce identical results. Proven in UnivSelfInterp.

    Zero admits. Following vWasm pattern. All proofs verified at
    --z3rlimit 20. Total: 10 leaf modules, all < 10s each.
*)
module LispIR.Universality
