# Common development commands; see docs/development.md. Run `make` or
# `make help` for the list. CI runs the same cargo commands.

# Fail a recipe line if any command of a pipe fails (`coverage` uses `tee`)
SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO ?= cargo
PRE_COMMIT ?= pre-commit

# Generated OpenAPI models are left out of the coverage report (see AGENTS.md)
COVERAGE_IGNORE ?= lib/src/clients/[^/]+/models/
COVERAGE_DIR ?= coverage

# Example crate with its own workspace (see docs/downstream.md)
DOWNSTREAM_DIR ?= examples/downstream-sdk
# Lists the "name version" pairs of a lockfile, sorted
LOCK_PACKAGES = awk '/^name = /{name = $$3} /^version = /{print name, $$3}' $(1) | tr -d '"' | sort

.DEFAULT_GOAL := help

.PHONY: help build test fmt fmt-check clippy lint check coverage docs models \
	pre-commit clean-coverage downstream

help: ## Show this list of targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  %-15s %s\n", $$1, $$2}'

build: ## Build the workspace (library, CLI, Python bindings)
	$(CARGO) build --workspace --locked

test: ## Run the unit and documentation tests
	$(CARGO) test --workspace --locked

fmt: ## Format all code, including the generated models
	$(CARGO) fmt --all

fmt-check: ## Check the formatting without changing files
	$(CARGO) fmt --all --check

clippy: ## Run Clippy on all targets
	$(CARGO) clippy --workspace --all-targets --locked

lint: fmt-check clippy ## Check formatting and run Clippy

check: lint build test ## Run the standard checks from AGENTS.md

coverage: ## Measure test coverage (needs cargo-llvm-cov)
	mkdir -p $(COVERAGE_DIR)
	$(CARGO) llvm-cov clean --workspace
	$(CARGO) llvm-cov --workspace --locked --no-report
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--lcov --output-path $(COVERAGE_DIR)/lcov.info
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--summary-only | tee $(COVERAGE_DIR)/summary.txt

clean-coverage: ## Remove coverage data and reports
	$(CARGO) llvm-cov clean --workspace
	rm -rf $(COVERAGE_DIR)

downstream: ## Check, lint and test the downstream example crate
	cp Cargo.lock $(DOWNSTREAM_DIR)/Cargo.lock
	@# Adds the example to the copied lockfile; then every other package must
	@# have a version from the root lockfile
	$(CARGO) metadata --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml \
		--format-version 1 > /dev/null
	@extra=$$(comm -13 <($(call LOCK_PACKAGES,Cargo.lock)) \
		<($(call LOCK_PACKAGES,$(DOWNSTREAM_DIR)/Cargo.lock)) \
		| grep -v '^downstream-sdk ' || true); \
	if [ -n "$$extra" ]; then \
		echo "Packages not in the root Cargo.lock:"; echo "$$extra"; exit 1; \
	fi
	$(CARGO) fmt --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --check
	$(CARGO) clippy --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --all-targets \
		--locked -- -D warnings
	$(CARGO) test --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --locked

docs: ## Build the API documentation (target/doc)
	$(CARGO) doc --workspace --no-deps --locked

models: ## Regenerate the OpenAPI models (see docs/autogenerate_OAI_models.md)
	bash ./utils/build_models_wrapper.sh

pre-commit: ## Run all pre-commit hooks on all files
	$(PRE_COMMIT) run --all-files
