install:
	@cargo build --release && cp target/release/funzzy /usr/local/bin/
