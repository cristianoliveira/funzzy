.PHONY install
install:
	cargo build --release && cp target/release/funzzy /urs/local/bin/funzzy
