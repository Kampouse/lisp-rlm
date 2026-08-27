# lisp-rlm — common dev flows

# Full 3-layer gauntlet: build → mock → oracle (production VMLogic) → sandbox,
# cross-diffed for identical results. See scripts/verify.sh.
verify-erc20:
	cargo build
	./scripts/verify.sh deploy/erc20

verify-safe:
	cargo build
	./scripts/verify.sh deploy/safe

.PHONY: verify-erc20 verify-safe
