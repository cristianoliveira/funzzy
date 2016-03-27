.PHONY: tests
tests:
	@cargo test

.PHONY: install
install:
	@cargo build --release && cp target/release/funzzy /usr/local/bin/
