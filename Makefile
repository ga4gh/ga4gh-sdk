# Common development commands; see docs/development.md. Run `make` or
# `make help` for the list. CI runs the same cargo commands.

# Fail a recipe line if any command of a pipe fails (`coverage` uses `tee`)
SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO ?= cargo
PRE_COMMIT ?= pre-commit
MKDOCS ?= mkdocs

# Generated OpenAPI models are left out of the coverage report (see AGENTS.md)
COVERAGE_IGNORE ?= lib/src/clients/[^/]+/(wire/[^/]+/)?models/
COVERAGE_DIR ?= coverage
# Python with maturin, pip and pytest, for the Python tests of `coverage`,
# which installs the bindings into its environment
PYTHON ?= python3
# The same interpreter from other directories: a path made absolute
COVERAGE_PYTHON = $(if $(findstring /,$(PYTHON)),$(abspath $(PYTHON)),$(PYTHON))
COVERAGE_WHEELS ?= target/coverage-wheels
# Sets cargo-llvm-cov's environment in a recipe line; a failure of `show-env`
# stops the line (a plain `eval "$$(...)"` would hide it)
COVERAGE_ENV = coverage_env="$$($(CARGO) llvm-cov show-env --sh)" && eval "$$coverage_env"

# Example crate with its own workspace (see docs/downstream.md)
DOWNSTREAM_DIR ?= examples/downstream-sdk
# Lists the packages of a lockfile as "name version source", sorted
LOCK_PACKAGES = awk '/^\[\[package\]\]/ {if (n != "") print n, v, s; n = v = s = ""} \
	/^name = / {n = $$3} /^version = / {v = $$3} /^source = / {s = $$3} \
	END {if (n != "") print n, v, s}' $(1) | tr -d '"' | sort
DOWNSTREAM_LISTS = $(DOWNSTREAM_DIR)/target/lockfile-check

.DEFAULT_GOAL := help

.PHONY: help build test fmt fmt-check clippy lint check coverage docs site \
	models pre-commit clean-coverage downstream

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
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

lint: fmt-check clippy ## Check formatting and run Clippy

check: lint build test ## Run the standard checks from AGENTS.md

coverage: ## Measure the coverage of the Rust and Python tests (see docs/development.md)
	mkdir -p $(COVERAGE_DIR) $(COVERAGE_WHEELS)
	rm -f $(COVERAGE_WHEELS)/ga4gh_sdk-*.whl
	@# The environment of `show-env` instruments every build in this line, the
	@# extension module that maturin builds included, and points `clean` and
	@# the report to their target directory and measurements
	$(COVERAGE_ENV) && \
	$(CARGO) llvm-cov clean --workspace && \
	$(CARGO) test --workspace --locked && \
	(cd python && $(COVERAGE_PYTHON) -m maturin build --locked --profile dev \
		--interpreter $(COVERAGE_PYTHON) --out $(abspath $(COVERAGE_WHEELS))) && \
	$(COVERAGE_PYTHON) -m pip install --force-reinstall --no-deps $(COVERAGE_WHEELS)/ga4gh_sdk-*.whl && \
	(cd python && $(COVERAGE_PYTHON) -m pytest) && \
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--lcov --output-path $(COVERAGE_DIR)/lcov.info && \
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--summary-only | tee $(COVERAGE_DIR)/summary.txt
	$(COVERAGE_PYTHON) utils/lcov_filter.py $(COVERAGE_DIR)/lcov.info | tee $(COVERAGE_DIR)/lines.txt

clean-coverage: ## Remove coverage data and reports
	$(COVERAGE_ENV) && $(CARGO) llvm-cov clean --workspace
	rm -rf $(COVERAGE_DIR)
	rm -f $(COVERAGE_WHEELS)/ga4gh_sdk-*.whl

downstream: ## Check, lint and test the downstream example crate
	cp Cargo.lock $(DOWNSTREAM_DIR)/Cargo.lock
	@# Adds the example to the copied lockfile; then every other package must
	@# have a version and source from the root lockfile
	$(CARGO) metadata --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml \
		--format-version 1 > /dev/null
	mkdir -p $(DOWNSTREAM_LISTS)
	$(call LOCK_PACKAGES,Cargo.lock) > $(DOWNSTREAM_LISTS)/root.txt
	$(call LOCK_PACKAGES,$(DOWNSTREAM_DIR)/Cargo.lock) > $(DOWNSTREAM_LISTS)/example.txt
	grep -q '^ga4gh-lib ' $(DOWNSTREAM_LISTS)/root.txt
	@extra=$$(comm -13 $(DOWNSTREAM_LISTS)/root.txt $(DOWNSTREAM_LISTS)/example.txt \
		| awk '$$1 != "downstream-sdk"'); \
	if [ -n "$$extra" ]; then \
		echo "Packages not in the root Cargo.lock:"; echo "$$extra"; exit 1; \
	fi
	$(CARGO) fmt --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --check
	$(CARGO) clippy --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --all-targets \
		--locked -- -D warnings
	$(CARGO) test --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml --locked

docs: ## Build the API documentation (target/doc)
	$(CARGO) doc --workspace --no-deps --locked

# As Read the Docs (`.readthedocs.yaml`) and the CI `docs` job build it.
# Any unsuppressed rustdoc warning fails it.
site: ## Build the documentation site in site/, strictly (needs MkDocs)
	RUSTDOCFLAGS="$${RUSTDOCFLAGS:-} -D warnings" \
		$(CARGO) doc --no-deps --locked -p ga4gh-lib -p ga4gh-cli
	$(MKDOCS) build --strict
	@# The hook must keep the theme's files (styles and scripts)
	@ls site/assets/javascripts/bundle.*.min.js > /dev/null

models: ## Regenerate the OpenAPI models (see docs/autogenerate_OAI_models.md)
	bash ./utils/build_models_wrapper.sh

pre-commit: ## Run all pre-commit hooks on all files
	$(PRE_COMMIT) run --all-files
