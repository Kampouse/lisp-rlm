module VMUnfoldHelpers

// Placeholder — helper lemmas for VM recursive functions
// Currently empty; CallSelf/Recur admits are due to Z3's inability to
// unfold fill_slots inside record construction in the handler call chain.

#set-options "--z3rlimit 100"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
