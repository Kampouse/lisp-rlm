# lisp-rlm — common dev flows

# Full 3-layer gauntlet: build → mock → oracle (production VMLogic) → sandbox,
# cross-diffed for identical results. See scripts/verify.sh.
verify-erc20:
	cargo build
	./scripts/verify.sh deploy/erc20

verify-safe:
	cargo build
	./scripts/verify.sh deploy/safe

# Full verification board: battery (cargo test) + gauntlet + twins,
# serialized, each leg on an isolated mock state file.
board:
	./scripts/board.sh

.PHONY: verify-erc20 verify-safe board

# Regenerate the committed crypto artifact embedded by wasm_link
# (schnorr_verify_bip340 + schnorr_sign_bip340 + sha256_hash stitched into
# NEAR contracts). Sources: schnorr/src/lib.rs — rebuild when it changes.
SCHNORR_SRC := $(wildcard schnorr/src/*.rs)

schnorr-wasm: $(SCHNORR_SRC)
	cd schnorr && cargo build --release --target wasm32-unknown-unknown
	cp schnorr/target/wasm32-unknown-unknown/release/schnorr.wasm src/wasm_emit/schnorr.wasm
