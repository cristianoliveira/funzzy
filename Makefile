.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test $UNIT_TEST --verbose --lib

.PHONY: build
build: tests ## Execute all the tests and build funzzy binary
	@cargo build --release

.PHONY: integration 
integration: ## Exectute integration tests
	@cargo test --test '*'

.PHONY: lint 
lint: ## Run the linter
	@cargo fmt -- --check

.PHONY: linter 
linter: lint

.PHONY: install
install: tests ## Install funzzy on your machine
	GITSHA="$(shell git rev-parse --short HEAD)" cargo install --path .

.PHONY: integration-clean
integration-clean: ## Clean up integration env
	rm -rf /tmp/fzz || sudo rm -rf /tmp/fzz

.PHONY: build-nix
build-nix: ## Build the project with nix
	@nix build .#funzzy
	@nix build .#fzzNightly

.PHONY: new-hash
new-hash: ## Clean current hash and generate a new one with nix
	@sed -i '' 's/sha256-.*=//g' pkgs/funzzy.nix
	@sed -i '' 's/sha256-.*=//g' pkgs/funzzy-nightly.nix
	make build-fzz

.PHONY: ci-integration
ci-integration:
	@cargo test --test '*'

