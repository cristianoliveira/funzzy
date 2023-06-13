.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test

.PHONY: build
build: tests ## Execute all the tests and build funzzy binary
	@cargo test

.PHONY: integration-cleanup
integration-cleanup:
	rm -rf target && \
		rm -rf tests/integration/release && \
		rm -rf tests/integration/workdir && \
		rm -f tests/integration/funzzy

.PHONY: integration ## Exectute integration tests
integration: integration-cleanup
	@bash tests/integration/runner.sh

.PHONY: integration-batch-1
integration-batch-1: integration-cleanup
	@bash tests/integration/batches.sh 5

.PHONY: integration-batch-2
integration-batch-2: integration-cleanup
	@bash tests/integration/batches.sh 10

.PHONY: lint
lint:
	cargo clippy

.PHONY: linter
linter: lint

.PHONY: install
install: tests ## Install funzzy on your machine
	GITSHA="$(shell git rev-parse --short HEAD)" cargo install
