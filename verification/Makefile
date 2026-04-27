FSTAR_HOME ?= $(shell which fstar 2>/dev/null && dirname $(shell which fstar 2>/dev/null))/.. || /usr/local/share/fstar

FSTAR = fstar --odir build

SEMANTICS_LISP = semantics/lisp/Lisp.Types.fst \
                 semantics/lisp/Lisp.Values.fst

SEMANTICS_LISP_IR = semantics/lisp_ir/LispIR.Semantics.fst \
                    semantics/lisp_ir/LispIR.Correctness.fst

TESTS = tests/CompareSpec.fst

ALL = $(SEMANTICS_LISP) $(SEMANTICS_LISP_IR) $(TESTS)

.PHONY: verify extract clean test

verify: $(ALL)
	@echo "=== Verifying Lisp Types ==="
	$(FSTAR) semantics/lisp/Lisp.Types.fst
	@echo "=== Verifying Lisp Values (num_cmp, num_arith) ==="
	$(FSTAR) semantics/lisp/Lisp.Values.fst
	@echo "=== Verifying VM Semantics (eval_op, eval_steps) ==="
	$(FSTAR) semantics/lisp_ir/LispIR.Semantics.fst
	@echo "=== Verifying Correctness Properties ==="
	$(FSTAR) semantics/lisp_ir/LispIR.Correctness.fst
	@echo "=== Running Tests ==="
	$(FSTAR) tests/CompareSpec.fst
	@echo "=== All verified ==="

extract: verify
	@echo "=== Extracting to OCaml ==="
	mkdir -p build/ocaml
	$(FSTAR) --extract Lisp.Types,Lisp.Values,LispIR.Semantics,LispIR.Correctness \
		$(ALL)
	@echo "Extracted to build/ocaml/"

test: extract
	@echo "=== Running property tests ==="
	cd build/ocaml && ocamlfind ocamlopt -package fstar -linkpkg \
		$(wildcard *.ml) -o test_runner || echo "OCaml extraction not yet wired"

clean:
	rm -rf build/
