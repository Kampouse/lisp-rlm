# afp-clmm/port — dev entrypoints (layout: contracts/ scenarios/ proof/ generators/)
.PHONY: prove build-ts clean regenerate charts escrow

NC ?= $(shell if [ -x $(HOME)/dev/lisp-rlm/target/release/near-compile ]; then echo $(HOME)/dev/lisp-rlm/target/release/near-compile; else command -v near-compile; fi)
NM ?= $(shell if [ -x $(HOME)/dev/lisp-rlm/target/release/near-mock ]; then echo $(HOME)/dev/lisp-rlm/target/release/near-mock; else command -v near-mock; fi)

prove:            ## run every proof stage, exit 0 only if ALL GREEN
	python3 proof/prove.py

escrow:           ## escrow leg: regenerate twins, compile, scenarios + mutation gate
	python3 generators/gen_escrow.py
	cd contracts/escrow/escrow && $(NC) build .
	cd contracts/escrow/escrow/ts && $(NC) build .
	cd contracts/escrow/ft && $(NC) build .
	cd scenarios/lisp/escrow-ledger && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/lisp/ft-probe && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/lisp/ft-refund && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/lisp/lp-ledger && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/ts/escrow-probe && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/ts/lp-ts && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/lisp/lp-keyed && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/ts/lp-keyed && rm -f state.bin && $(NM) scenario s.json
	python3 proof/mutate_escrow.py

accrual:          ## accrual leg: regenerate twins, compile, fixture-wired scenarios
	python3 generators/gen_accrual.py
	python3 generators/gen_accrual_vectors.py
	cd contracts/accrual/accrual && $(NC) build .
	cd contracts/accrual/accrual/ts && $(NC) build .
	cd scenarios/lisp/accrual-smoke && rm -f state.bin && $(NM) scenario s.json
	cd scenarios/ts/accrual-ts && rm -f state.bin && $(NM) scenario s.json

build-ts:         ## compile all TS twins (13 pool twins + escrow-ts)
	python3 generators/build_ts.py

accrual-mount:    ## fee-ledger mount harness (PS accumulator, skim+solvency pins)
	cd contracts/accrual-mount && python3 gen_scenario.py
	cd contracts/accrual-mount && $(NC) build .
	cd scenarios/lisp/accrual-mount && rm -f state.bin && $(NM) scenario s.json

accrual-mount-v4: ## fee-ledger mount WIRED into clmm_pool_v4 (H1-H4, real FT legs)
	cd contracts/accrual-mount && python3 gen_accrual_v4.py
	cd contracts/accrual-mount && $(NC) build v4-pool
	cd contracts/escrow/ft && $(NC) build .
	cd scenarios/lisp/accrual-mount-v4 && rm -f state.bin && $(NM) scenario s.json

regenerate:       ## regenerate lisp contracts + TS twins from constants
	python3 generators/gen.py && python3 generators/gen3.py && python3 generators/gen_ts.py && python3 generators/gen_escrow.py

clean:            ## build artifacts + scenario state (state.bin additive-ledger trap)
	rm -f contracts/*/*/target/*.wasm* contracts/*/*/ts/target/*.wasm*
	rm -f scenarios/lisp/*/state.bin scenarios/ts/*/state.bin

charts:           ## regenerate pool-shape SVGs in each contract README
	python3 generators/gen_charts.py
