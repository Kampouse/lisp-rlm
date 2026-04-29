#!/bin/bash
cd "$(dirname "$0")"

FSTAR="/tmp/fstar-install/fstar/bin/fstar.exe"
OPTS="--odir build --cache_dir build/cache --include semantics/lisp --include semantics/lisp_ir --include tests"

mkdir -p build/cache
LOG=build/verify_results.txt
> "$LOG"

verify() {
    local file="$1"
    local label="$2"
    local start=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")
    if $FSTAR $OPTS "$file" >build/tmp_out.txt 2>&1; then
        local end=$(date +%s%N 2>/dev/null || python3 -c "import time; print(int(time.time()*1e9))")
        local elapsed=$(( (end - start) / 1000000 ))
        echo "✓ $label (${elapsed}ms)" >> "$LOG"
        echo "✓ $label (${elapsed}ms)"
    else
        echo "✗ $label" >> "$LOG"
        echo "✗ $label"
        # Show error summary
        grep -E "Error|error" build/tmp_out.txt | head -3 >> "$LOG"
        echo "" >> "$LOG"
    fi
}

echo "=== F* Verification — $(date) ===" >> "$LOG"

# Layer 0
verify semantics/lisp/Lisp.Types.fst "Lisp.Types"
# Layer 1
verify semantics/lisp/Lisp.Values.fst "Lisp.Values"
# Layer 2
verify semantics/lisp_ir/LispIR.Semantics.fst "LispIR.Semantics"
verify semantics/lisp/Lisp.Source.fst "Lisp.Source"
# Layer 3
verify semantics/lisp/Lisp.Closure.fst "Lisp.Closure"
verify semantics/lisp_ir/LispIR.ClosureVM.fst "LispIR.ClosureVM"
verify semantics/lisp/Lisp.Compiler.fst "Lisp.Compiler"
verify semantics/lisp_ir/LispIR.Correctness.fst "LispIR.Correctness"
verify semantics/lisp_ir/LispIR.Determinism.fst "LispIR.Determinism"
# Layer 4
verify semantics/lisp_ir/LispIR.CompilerSpec.fst "LispIR.CompilerSpec"
verify semantics/lisp_ir/LispIR.CompilerSpec3.fst "LispIR.CompilerSpec3"
verify semantics/lisp_ir/LispIR.CompilerSpec4.fst "LispIR.CompilerSpec4"
verify semantics/lisp_ir/LispIR.CompilerCorrectness.fst "LispIR.CompilerCorrectness"
verify semantics/lisp_ir/LispIR.PerExpr3.fst "LispIR.PerExpr3"
verify semantics/lisp_ir/LispIR.StackHeight.fst "LispIR.StackHeight"
# Layer 5: VM proofs
verify tests/VMUnfoldHelpers.fst "VMUnfoldHelpers"
verify tests/VmView.fst "VmView"
verify tests/VmIfTest.fst "VmIfTest"
verify tests/OpcodeProofs.fst "OpcodeProofs"
verify tests/BuiltinOpcodeProofs.fst "BuiltinOpcodeProofs"
verify tests/StepProofs.fst "StepProofs"
# Layer 6: Closure VM
verify tests/ClosureVMSteps.fst "ClosureVMSteps"
verify tests/ClosureOpcodeProofs.fst "ClosureOpcodeProofs"
verify tests/ClosureRoundtrip.fst "ClosureRoundtrip"
verify tests/CallSelfLoop.fst "CallSelfLoop"
verify tests/SelfCallVMTest.fst "SelfCallVMTest"
verify tests/HandlerProofs.fst "HandlerProofs"
verify tests/HardOpcodeProofs.fst "HardOpcodeProofs"
verify tests/HarnessProofs.fst "HarnessProofs"
verify tests/ExtendedClosureVM.fst "ExtendedClosureVM"
verify tests/NewFormRoundtrips.fst "NewFormRoundtrips"
verify tests/StackFuel.fst "StackFuel"
verify tests/MapFilterReduce.fst "MapFilterReduce"
verify tests/ExtendedOps.fst "ExtendedOps"
verify tests/NewOpcodes.fst "NewOpcodes"
# Layer 7: Compiler/source tests
verify tests/ClosureTest.fst "ClosureTest"
verify tests/LambdaBody.fst "LambdaBody"
verify tests/LambdaMap.fst "LambdaMap"
verify tests/DictCompilerSpec.fst "DictCompilerSpec"
verify tests/DictOps.fst "DictOps"
verify tests/DictSetTest.fst "DictSetTest"
verify tests/DispatchRouting.fst "DispatchRouting"
verify tests/NilQSpec.fst "NilQSpec"
verify tests/NilQVm.fst "NilQVm"
verify tests/PatchJumpTest.fst "PatchJumpTest"
verify tests/SelfCallTest.fst "SelfCallTest"
verify tests/ShadowingFix.fst "ShadowingFix"
verify tests/StackHeight.fst "StackHeight"
verify tests/ProgramDesugaring.fst "ProgramDesugaring"
verify tests/PureTypeSoundness.fst "PureTypeSoundness"

echo ""
echo "=== SUMMARY ===" >> "$LOG"
grep -c "✓" "$LOG" >> "$LOG"
grep -c "✗" "$LOG" >> "$LOG"
echo "" >> "$LOG"

echo ""
cat "$LOG"
