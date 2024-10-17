.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test --verbose

.PHONY: build
build: tests ## Execute all the tests and build funzzy binary
	@cargo build --release

.PHONY: prebuild
prebuild: ## Build the project in non-release mode
	@cargo build

.PHONY: integration ## Exectute integration tests
integration: integration-clean
	@cargo test --features test-integration

.PHONY: ci-enable-hook
ci-enable-hook: ## Enable the pre-push hook
	@ln -fs "${PWD}/scripts/git-hooks-checks" "${PWD}/.git/hooks/pre-push"
	@chmod +x "${PWD}/.git/hooks/pre-push"

.PHONY: ci-integration
ci-integration: integration-clean
	@cargo test --features test-integration

.PHONY: ci-run-on-push
ci-run-on-push: ## Run checks from .github/workflows/on-push.yml
	@cat .github/workflows/on-push.yml \
		| yq '.jobs | .[] | .steps | .[] | .run | select(. != null)' \
		| xargs -I {} bash -c {}

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
	echo "Creating a temp dir for integration tests in /tmp/fzz"
	rm -rf /tmp/fzz
	mkdir -p /tmp/fzz

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

.PHONY: nix-build-nightly
nix-build-nightly: ## Build the nix derivation with the nightly toolchain
	@nix build .# --verbose -L

.PHONY: nix-build
nix-build: ## Build the nix derivation with the nightly toolchain
	@nix build .# --verbose -L

.PHONY: nix-bump-default
nix-bump-default: ##  Bump the version in nix default package and generate a new revision
	@echo "Bumping the version in nix default"
	scripts/bump-nix-default

.PHONY: nix-bump-nightly
nix-bump-nightly: ##  Bump the version in nix nightly package and generate a new revision
	@echo "Bumping the version in nix packages"
	scripts/bump-nix-nightly

.PHONY: nix-bump-local
nix-bump-local: ##  Bump the version in nix local package and generate a new revision
	@echo "Bumping the version in nix packages"
	scripts/bump-nix-local

.PHONY: nix-bump-all
nix-bump-all: nix-bump-default nix-bump-nightly ## Bump all nix revisions
	@echo "Bumping all nix packages"
