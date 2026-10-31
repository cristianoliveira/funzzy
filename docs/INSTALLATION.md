# Install Funzzy

Choose one installation method below, then run `fzz --version` to verify it. The `funzzy` and `fzz` binaries run the same application.

## macOS

Stable Homebrew package:

```bash
brew install funzzy
```

Latest project release:

```bash
brew install cristianoliveira/tap/funzzy
```

## Linux

The installer supports `x86_64` and `aarch64`. It checks the published SHA-256 checksum before it installs both binaries in `/usr/local/bin`:

```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/main/linux-install.sh | sh
```

Pin a release when reproducibility matters:

```bash
curl -s https://raw.githubusercontent.com/cristianoliveira/funzzy/main/linux-install.sh | bash - 2.0.0
```

## Nix

From nixpkgs:

```bash
nix-env -iA nixpkgs.funzzy
```

Latest project release or package fork:

```bash
nix profile install 'github:cristianoliveira/funzzy'
nix profile install 'github:cristianoliveira/nixpkgs#funzzy'
```

Nightly:

```bash
nix profile install 'github:cristianoliveira/funzzy#nightly'
```

In `shell.nix`:

```nix
{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  buildInputs = [pkgs.funzzy];
}
```

## Cargo

```bash
cargo install funzzy
```

If `fzz` is not on `PATH`, add Cargo's binary directory:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## From source

Install Rust and Cargo before you build from source. `Cargo.toml` lists the minimum Rust version in `rust-version`.

Install from GitHub:

```bash
cargo install --git https://github.com/cristianoliveira/funzzy.git
```

Or from a clone:

```bash
git clone https://github.com/cristianoliveira/funzzy.git
cd funzzy
make install
```

## Verify

```bash
fzz --version
fzz --help
```

A build from a Git checkout includes its commit in `--version` (for example,
`funzzy 2.0.1 (e6f069be6c42)`). A build from crates.io or a source archive
without Git metadata prints only the package version.

Continue with the [one-minute start](../README.md#start-in-one-minute) or the complete [usage guide](USAGE.md).
