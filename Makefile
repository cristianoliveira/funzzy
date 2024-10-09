.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test $UNIT_TEST --verbose --lib

.PHONY: build
build: tests ## Execute all the tests and build funzzy binary
	@cargo build --release

.PHONY: integration ## Exectute integration tests
integration:
	@cargo test --features test-integration

.PHONY: ci-integration
ci-integration:
	@cargo test --features test-integration

.PHONY: lint
lint:
	@cargo fmt -- --check

.PHONY: linter
linter: lint

.PHONY: install
install: tests ## Install funzzy on your machine
	GITSHA="$(shell git rev-parse --short HEAD)" cargo install --path .

.PHONY: integration-clean
integration-clean:
	rm -rf /tmp/fzz || sudo rm -rf /tmp/fzz

.PHONY: nix-gen-patch
nix-gen-patch: ## Generate a patch for the nix derivation
	@git diff origin/master -r -u > nix/gitdiff.patch

.PHONY: nix-flake-check
nix-flake-check: ## Check the nix flake
	@nix flake check

.PHONY: nix-build-all
nix-build-all: nix-build nix-build-nightly ## Build the nix derivation with the nightly toolchain
	echo "Done"

.PHONY: nix-build-local
nix-build-local: ## Build the nix derivation with the local toolchain
	@nix build .#local --verbose -L

.PHONY: nix-build
nix-build: ## Build the nix derivation with the nightly toolchain
	@nix build .# --verbose -L

.PHONY: nix-bump
nix-bump:
	@sed -i 's/sha256-.*=//g' nix/package.nix
	@sed -i 's/sha256-.*=//g' nix/package-from-source.nix

.PHONY: ci-run-on-push
ci-run-on-push: ## Run checks from .github/workflows/on-push.yml
	@cat .github/workflows/on-push.yml \
		| yq '.jobs | .[] | .steps | .[] | .run | select(. != null)' \
		| xargs -I {} bash -c {}
