# `script` phase: you usually build, test and generate docs in this phase

set -ex

# Add macOS Rust target
rustup target add $TARGET

cargo build --target $TARGET --release

ARTIFACT="funzzy-${RELEASE_TAG:?"Missing release tag"}-${TARGET}.tar.gz"

cp target/$TARGET/release/funzzy funzzy

tar czf "$ARTIFACT" funzzy

# sanity check the file type
file funzzy
