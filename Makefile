.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test

.PHONY: build
build: tests ## Execute all the tests and build funzzy binary
	@cargo build --release

.PHONY: integration ## Exectute integration tests
integration:
	@cargo test --test '*' -- --nocapture

.PHONY: lint
lint:
	cargo clippy

.PHONY: linter
linter: lint

.PHONY: install
install: tests ## Install funzzy on your machine
	GITSHA="$(shell git rev-parse --short HEAD)" cargo install --path .
