
# Full 3-layer gauntlet: build → mock → oracle (production VMLogic) → sandbox,
# cross-diffed for identical results. scripts/verify.sh for details.
verify-erc20: build
	./scripts/verify.sh deploy/erc20
verify-safe: build
	./scripts/verify.sh deploy/safe
.PHONY: verify-erc20 verify-safe
