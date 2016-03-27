# `script` phase: you usually build, test and generate docs in this phase

set -ex

cargo build --target $TARGET --verbose
cargo test --target $TARGET

cargo build --target $TARGET --release

# sanity check the file type
file target/$TARGET/release/funzzy
