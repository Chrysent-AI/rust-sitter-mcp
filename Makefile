SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help check-tools install-hooks install-tools install fmt fmt-check lint test deny machete quality check precommit test-tooling build run clean version bump-version

help: ## Show developer commands
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

check-tools: ## Verify pinned toolchain, components and gate tools
	@bash scripts/check-tools.sh

install-hooks: ## Activate tracked hooks; safe to repeat
	@bash scripts/install-hooks.sh

install-tools: ## Install the pinned developer gate tools
	cargo install --locked cargo-deny --version 0.20.2
	cargo install --locked cargo-machete --version 0.9.2

install: ## Install tools/hooks, verify, build and install the binary
	@for tool in git make bash awk rustup cargo; do command -v "$$tool" >/dev/null || exit 1; done
	rustup component add --toolchain 1.97.1 rustfmt clippy
	$(MAKE) install-tools
	$(MAKE) check-tools
	$(MAKE) install-hooks
	cargo fetch --locked
	$(MAKE) quality
	cargo build --locked
	cargo install --path . --locked --force
	rust-sitter-mcp --version
	@printf '%s\n' 'See CONTRIBUTING.md for setup and dependency policy.'

fmt: ## Deliberately format Rust source
	cargo fmt --all

fmt-check: ## Check Rust formatting without rewriting
	cargo fmt --all -- --check

lint: ## Lint all targets; warnings fail
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

test: ## Run the complete Rust test suite
	cargo test --workspace --all-features --locked --quiet

deny: ## Audit advisories, licenses and sources
	cargo deny check

machete: ## Reject unused direct dependencies
	cargo machete

quality: ## Run all required gates, including tooling regression tests
	@bash scripts/quality.sh

check: ## Alias for the full quality gate
	@$(MAKE) quality

precommit: ## Run the full gate with staged-tree and dependency-log checks
	@bash scripts/quality.sh --precommit

test-tooling: ## Run disposable tooling regression fixtures
	@bash scripts/test-tooling.sh

build: ## Build the package with its committed dependency resolution
	cargo build --locked

run: ## Run the package skeleton (ARGS may contain --help or --version)
	cargo run --locked -- $(ARGS)

clean: ## Explicitly remove Cargo build artifacts
	cargo clean

version: ## Display the embedded package version and Git revision
	cargo run --locked -- --version

bump-version: ## Update only [package] version and lock entry (BUMP or VERSION)
	@bash scripts/bump-version.sh "$(BUMP)" "$(VERSION)"
