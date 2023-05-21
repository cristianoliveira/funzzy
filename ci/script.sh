# `script` phase: you usually build, test and generate docs in this phase

set -ex

# Add macOS Rust target
rustup target add $TARGET

cargo build --target $TARGET --verbose
cargo test --target $TARGET

cargo build --target $TARGET --release

cd target/release
ls -l

ARTIFACT="funzzy-${RELEASE_TAG:?"Missing release tag"}-${TARGET}.tar.gz"

tar czf "../$ARTIFACT" *

cp "../$ARTIFACT" ../../artifacts/

# sanity check the file type
file funzzy
