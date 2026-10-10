# afp-clmm/port — dev entrypoints (layout: contracts/ scenarios/ proof/ generators/)
.PHONY: prove build-ts clean regenerate charts

prove:            ## run every proof stage, exit 0 only if ALL GREEN
	python3 proof/prove.py

build-ts:         ## compile all 9 TS twins
	python3 generators/build_ts.py

regenerate:       ## regenerate lisp contracts + TS twins from constants
	python3 generators/gen.py && python3 generators/gen3.py && python3 generators/gen_ts.py

clean:            ## build artifacts + scenario state (state.bin additive-ledger trap)
	rm -f contracts/*/*/target/*.wasm* contracts/*/*/ts/target/*.wasm*
	rm -f scenarios/lisp/*/state.bin scenarios/ts/*/state.bin

charts:           ## regenerate pool-shape SVGs in each contract README
	python3 generators/gen_charts.py
