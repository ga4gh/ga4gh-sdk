# Common development commands; see docs/development.md. Run `make` or
# `make help` for the list. CI runs the same cargo commands.

# Fail a recipe line if any command of a pipe fails (`coverage` uses `tee`)
SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c

CARGO ?= cargo
PRE_COMMIT ?= pre-commit
MKDOCS ?= mkdocs

# Generated OpenAPI models (see AGENTS.md) and the test support module
# `lib/src/utils/test_utils.rs` are left out of the coverage report
COVERAGE_IGNORE ?= lib/src/clients/[^/]+/(wire/[^/]+/)?models/|lib/src/utils/test_utils\.rs$$
COVERAGE_DIR ?= coverage
# Python with maturin, pip and pytest, for the Python tests of `coverage`;
# the bindings go to COVERAGE_SITE, not into its environment
PYTHON ?= python3
# The same interpreter from other directories: a path made absolute
COVERAGE_PYTHON = $(if $(findstring /,$(PYTHON)),$(abspath $(PYTHON)),$(PYTHON))
COVERAGE_WHEELS ?= target/coverage-wheels
# The instrumented bindings, on PYTHONPATH for pytest only
COVERAGE_SITE ?= target/coverage-site
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

.PHONY: help build test fmt fmt-check clippy lint check coverage docs site-rustdoc site \
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

.PHONY: workflow-extension
workflow-extension: ## Package and test the workflow provider independently of the CLI
	python3 -B -m unittest discover -s extensions/workflow -p 'test_package.py'
	$(CARGO) fmt --manifest-path extensions/workflow/Cargo.toml --all --check
	CARGO_TARGET_DIR=$(abspath target) $(CARGO) clippy --locked --manifest-path extensions/workflow/Cargo.toml --workspace --all-targets -- -D warnings
	CARGO_TARGET_DIR=$(abspath target) $(CARGO) build --locked --manifest-path extensions/workflow/Cargo.toml
	GA4GH_WORKFLOW_LIB_DIR=$(abspath target/debug) CARGO_TARGET_DIR=$(abspath target) $(CARGO) test --locked --manifest-path extensions/workflow/Cargo.toml --test native_prepare
	CARGO_TARGET_DIR=$(abspath target) $(CARGO) test --locked --manifest-path extensions/workflow/Cargo.toml --lib --test prepare_contract
	CARGO_TARGET_DIR=$(abspath target) python3 extensions/workflow/package.py --debug target/workflow-package
	cmp extensions/workflow/README.md target/workflow-package/README.md
	$(MAKE) workflow-rust-binding
	$(CARGO) clippy --locked -p ga4gh-cli --features workflow-extension-tests --test workflow -- -D warnings
	GA4GH_WORKFLOW_MANIFEST=$(abspath target/workflow-package/manifest.json) $(CARGO) test --locked -p ga4gh-cli --features workflow-extension-tests --test workflow
	$(CARGO) test --locked -p ga4gh-lib --features workflow-contracts --lib --test result_reuse --test workflow_contracts

.PHONY: workflow-rust-binding workflow-bindings
workflow-rust-binding:
	@features=$$($(CARGO) tree --locked --manifest-path extensions/workflow/Cargo.toml -p ga4gh-workflow -e normal,build -i ga4gh-lib --depth 0 --format '{f}') && case ",$$features," in *,workflow-contracts,*) echo 'Typed binding enables workflow implementation' >&2; exit 1;; esac
	GA4GH_WORKFLOW_MANIFEST=$(abspath target/workflow-package/manifest.json) GA4GH_WORKFLOW_PARITY=$(abspath target/workflow-parity.json) CARGO_TARGET_DIR=$(abspath target) $(CARGO) test --locked --manifest-path extensions/workflow/Cargo.toml -p ga4gh-workflow

workflow-bindings: ## Compare installed Rust and Python clients against the same native artifact
	CARGO_TARGET_DIR=$(abspath target) python3 extensions/workflow/package.py --debug target/workflow-package
	$(MAKE) workflow-rust-binding
	GA4GH_WORKFLOW_MANIFEST=$(abspath target/workflow-package/manifest.json) GA4GH_WORKFLOW_PARITY=$(abspath target/workflow-parity.json) $(PYTHON) -m pytest -q extensions/workflow/bindings/python/tests

coverage: ## Measure the coverage of the Rust and Python tests (see docs/development.md)
	mkdir -p $(COVERAGE_DIR) $(COVERAGE_WHEELS)
	rm -f $(COVERAGE_WHEELS)/ga4gh_sdk-*.whl
	rm -rf $(COVERAGE_SITE)
	@# The environment of `show-env` instruments every build in this line, the
	@# extension module that maturin builds included, and points `clean` and
	@# the report to their target directory and measurements
	$(COVERAGE_ENV) && \
	$(CARGO) llvm-cov clean --workspace && \
	$(CARGO) test --workspace --locked && \
	(cd python && $(COVERAGE_PYTHON) -m maturin build --locked --profile dev \
		--interpreter $(COVERAGE_PYTHON) --out $(abspath $(COVERAGE_WHEELS))) && \
	$(COVERAGE_PYTHON) -m pip install --no-deps --target $(COVERAGE_SITE) \
		$(COVERAGE_WHEELS)/ga4gh_sdk-*.whl && \
	(cd python && PYTHONPATH=$(abspath $(COVERAGE_SITE)) $(COVERAGE_PYTHON) -m pytest) && \
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--lcov --output-path $(COVERAGE_DIR)/lcov.info && \
	$(CARGO) llvm-cov report --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--summary-only | tee $(COVERAGE_DIR)/summary.txt
	$(COVERAGE_PYTHON) utils/lcov_filter.py $(COVERAGE_DIR)/lcov.info | tee $(COVERAGE_DIR)/lines.txt

clean-coverage: ## Remove coverage data and reports
	$(COVERAGE_ENV) && $(CARGO) llvm-cov clean --workspace
	rm -rf $(COVERAGE_DIR)
	rm -f $(COVERAGE_WHEELS)/ga4gh_sdk-*.whl
	rm -rf $(COVERAGE_SITE)

downstream: ## Check, lint and test the downstream example crate
	cp Cargo.lock $(DOWNSTREAM_DIR)/Cargo.lock
	@# Adds the example to the copied lockfile; then every other package must
	@# have a version and source from the root lockfile
	$(CARGO) metadata --manifest-path $(DOWNSTREAM_DIR)/Cargo.toml \
		--format-version 1 > /dev/null
	mkdir -p $(DOWNSTREAM_LISTS)
	{ $(call LOCK_PACKAGES,Cargo.lock); $(call LOCK_PACKAGES,extensions/workflow/Cargo.lock) | awk '$$1 == "ga4gh-workflow" && NF == 2'; } | sort -u > $(DOWNSTREAM_LISTS)/root.txt
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
	$(CARGO) doc --workspace --no-deps --locked --features ga4gh-lib/workflow-contracts

# As Read the Docs (`.readthedocs.yaml`) and the CI `docs` job build it.
# Any unsuppressed rustdoc warning fails it.
site-rustdoc: ## Build the Rust API reference for the documentation site
	RUSTDOCFLAGS="$${RUSTDOCFLAGS:-} -D warnings" \
		$(CARGO) doc --no-deps --locked -p ga4gh-lib -p ga4gh-cli --features ga4gh-lib/workflow-contracts

site: site-rustdoc ## Build the documentation site in site/, strictly (needs MkDocs)
	$(MKDOCS) build --strict
	@# The hook must keep the theme's files (styles and scripts)
	@ls site/assets/javascripts/bundle.*.min.js > /dev/null

models: ## Regenerate the OpenAPI models (see docs/autogenerate_OAI_models.md)
	bash ./utils/build_models_wrapper.sh

pre-commit: ## Run all pre-commit hooks on all files
	$(PRE_COMMIT) run --all-files
