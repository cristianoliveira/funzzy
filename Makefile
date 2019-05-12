.PHONY: help
help: ## Lists the available commands. Add a comment with '##' to describe a command.
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'

.PHONY: tests
tests: ## Execute all the tests
	@cargo test

.PHONY: build
build: test ## Execute all the tests and build funzzy binary
	@cargo test

.PHONY: install
install: build ## Install funzzy on your machine
	cp target/release/funzzy /usr/local/bin/
