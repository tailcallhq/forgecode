# forge development Makefile
#
# All build commands run inside `nix develop` so protoc, cargo, and
# other dependencies are available automatically.
#
# Use `nix develop` first if you want an interactive shell, or prefix
# any target below with `nix develop --command` if running outside nix.
#
# Usage:
#   make <target>   — runs inside nix develop automatically

SHELL := bash
NIX_CMD := nix develop --command

# ── Build ──────────────────────────────────────────────────────────

.PHONY: build
build: ## Build the forge binary via nix
	nix build .#forge

.PHONY: check
check: ## Check all crates compile (fast)
	$(NIX_CMD) cargo check

.PHONY: check-app
check-app: ## Check only the forge_app crate
	$(NIX_CMD) cargo check -p forge_app

.PHONY: check-services
check-services: ## Check only the forge_services crate
	$(NIX_CMD) cargo check -p forge_services

.PHONY: clippy
clippy: ## Run clippy on all crates
	$(NIX_CMD) cargo clippy -- -D warnings

# ── Test ───────────────────────────────────────────────────────────

.PHONY: test
test: ## Run all tests
	$(NIX_CMD) cargo test

.PHONY: test-app
test-app: ## Run all forge_app tests
	$(NIX_CMD) cargo test -p forge_app

.PHONY: test-services
test-services: ## Run all forge_services tests
	$(NIX_CMD) cargo test -p forge_services

.PHONY: test-tool-registry
test-tool-registry: ## Run tool_registry unit tests
	$(NIX_CMD) cargo test -p forge_app --lib -- tool_registry

.PHONY: test-policy
test-policy: ## Run policy service tests
	$(NIX_CMD) cargo test -p forge_services --lib -- policy

.PHONY: test-instas
test-instas: ## Run and accept insta snapshot tests
	$(NIX_CMD) cargo insta test --accept

# ── Utility ────────────────────────────────────────────────────────

.PHONY: fmt
fmt: ## Format all Rust code
	$(NIX_CMD) cargo fmt

.PHONY: clean
clean: ## Clean build artifacts
	cargo clean

.PHONY: help
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'
