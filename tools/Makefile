# The gates of every crate in this workspace, as CI runs them.
# `make ci` fails when any of them fails; `make ci-<crate>` runs one crate's.

CRATES := $(patsubst %/Cargo.toml,%,$(wildcard */Cargo.toml))
COVERAGE_MIN := 80

.PHONY: ci fmt-check clippy test coverage build install vendor-words $(addprefix ci-,$(CRATES))

ci: fmt-check clippy test coverage vendor-words

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

# One gate per crate: a crate under 80 % of lines fails even if the workspace passes.
coverage:
	@for crate in $(CRATES); do \
	  echo "== coverage $$crate"; \
	  cargo llvm-cov --package $$(sed -n 's/^name = "\(.*\)"/\1/p' $$crate/Cargo.toml | head -1) \
	    --summary-only --fail-under-lines $(COVERAGE_MIN) || exit 1; \
	done

# Nothing of the installation these tools were first written for may remain.
vendor-words:
	@! grep -rniE 'sisu|inditex|1424293|3991990|\bsku\b|picking' --exclude-dir=target --include='*.rs' --include='*.toml' --include='*.md' . \
	  || { echo "vendor words found above"; exit 1; }

build:
	cargo build --workspace --release

# Copies every binary of the workspace to ~/.local/bin: the MCP servers and the mirror daemon.
install: build
	@for bin in target/release/*-mcp target/release/*-mirror; do install -m755 "$$bin" "$$HOME/.local/bin/"; echo "installed $$bin"; done

$(addprefix ci-,$(CRATES)): ci-%:
	cargo fmt -p $$(sed -n 's/^name = "\(.*\)"/\1/p' $*/Cargo.toml | head -1) -- --check
	cargo clippy -p $$(sed -n 's/^name = "\(.*\)"/\1/p' $*/Cargo.toml | head -1) --all-targets -- -D warnings
	cargo llvm-cov -p $$(sed -n 's/^name = "\(.*\)"/\1/p' $*/Cargo.toml | head -1) --summary-only --fail-under-lines $(COVERAGE_MIN)
