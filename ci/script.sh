# `script` phase: you usually build, test and generate docs in this phase

set -ex

VERSION="v$(cat Cargo.toml | grep version | awk -F\" '{print $2}')"

# Add macOS Rust target
rustup target add $TARGET

cargo build --target $TARGET --release

ARTIFACT="funzzy-${RELEASE_TAG:-"$VERSION"}-${TARGET}.tar.gz"

mkdir -p pkg
cp target/$TARGET/release/funzzy pkg
cp target/$TARGET/release/fzz pkg

tar czf "$ARTIFACT" pkg

# sanity check the file type
file pkg/funzzy
