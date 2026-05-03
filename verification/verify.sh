#!/bin/bash
# Verify all F* modules for lisp-rlm in dependency order
set -e
FSTAR=/tmp/fstar-install/fstar/bin/fstar.exe
FSTAR_FLAGS="-c --include semantics/lisp --include semantics/lisp_ir --include tests"

PASS=0
FAIL=0
ERRORS=""

verify() {
    local f=$1
    local name=$(basename $f .fst)
    printf "  [%02d] %-40s ... " "$((PASS+FAIL+1))" "$name"
    OUTPUT=$($FSTAR $FSTAR_FLAGS $f 2>&1)
    if echo "$OUTPUT" | grep -q "All verification conditions discharged successfully"; then
        echo "OK"
        PASS=$((PASS+1))
    else
        echo "FAILED"
        echo "$OUTPUT" | grep -E "^\\* (Error|Warning)" | head -5
        FAIL=$((FAIL+1))
        ERRORS="$ERRORS\n  $name"
    fi
}

echo "=== Verifying F* modules ==="
echo "F*: $FSTAR"

# Layer 1: Base types
echo "--- Layer 1: Base types ---"
verify semantics/lisp/Lisp.Types.fst
verify semantics/lisp/Lisp.Values.fst

# Layer 2: Source semantics
echo "--- Layer 2: Source semantics ---"
verify semantics/lisp/Lisp.Source.fst
verify semantics/lisp/Lisp.Compiler.fst
verify semantics/lisp/Lisp.Closure.fst

# Layer 3: VM semantics
echo "--- Layer 3: VM semantics ---"
verify semantics/lisp_ir/LispIR.Semantics.fst
verify semantics/lisp_ir/LispIR.Correctness.fst
verify semantics/lisp_ir/LispIR.Determinism.fst
verify semantics/lisp_ir/LispIR.ClosureVM.fst
verify semantics/lisp_ir/LispIR.Soundness.fst
verify semantics/lisp_ir/LispIR.StackHeight.fst
verify semantics/lisp_ir/LispIR.Universality.fst

# Layer 4: Compiler specs
echo "--- Layer 4: Compiler specs ---"
verify semantics/lisp_ir/LispIR.CompilerSpec.fst
verify semantics/lisp_ir/LispIR.CompilerSpec3.fst
verify semantics/lisp_ir/LispIR.CompilerSpec4.fst
verify semantics/lisp_ir/LispIR.CompilerCorrectness.fst
verify semantics/lisp_ir/LispIR.PerExpr3.fst

# Layer 5: Tests
echo "--- Layer 5: Tests ---"
for f in tests/*.fst; do
    verify $f
done

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
if [ $FAIL -gt 0 ]; then
    echo "Failed modules:"
    echo -e "$ERRORS"
fi
