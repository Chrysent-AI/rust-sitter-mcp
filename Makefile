SHELL := /bin/bash
.DEFAULT_GOAL := help

.PHONY: help check-tools install-hooks install-tools install fmt fmt-check lint test deny machete quality check precommit test-tooling build run clean version bump-version

help: ## Show developer commands
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

check-tools: ## Verify pinned Rust tools and compatible gitleaks 8.x
	@bash scripts/check-tools.sh

install-hooks: ## Activate tracked hooks; safe to repeat
	@bash scripts/install-hooks.sh

install-tools: ## Install pinned Cargo gates and gitleaks (brew/Go; see CONTRIBUTING.md)
	cargo install --locked cargo-deny --version 0.20.2
	cargo install --locked cargo-machete --version 0.9.2
	@if command -v brew >/dev/null; then brew install gitleaks; \
	elif command -v go >/dev/null; then go install -ldflags '-X github.com/zricethezav/gitleaks/v8/version.Version=8.30.1' github.com/zricethezav/gitleaks/v8@v8.30.1; \
	else printf '%s\n' 'Install gitleaks 8.x from https://github.com/gitleaks/gitleaks/releases and add it to PATH; see CONTRIBUTING.md (Ubuntu).' >&2; exit 1; fi

install: ## Install tools/hooks, verify, build and install the binary
	@for tool in git make bash awk rustup cargo; do command -v "$$tool" >/dev/null || exit 1; done
	rustup component add --toolchain 1.98.1 rustfmt clippy
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

precommit: ## Scan staged secrets first, then run parity and full quality gates
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
