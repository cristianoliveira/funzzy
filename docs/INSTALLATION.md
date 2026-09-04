# Install Funzzy

Funzzy installs two equivalent binaries: `funzzy` and `fzz`.

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

The installer detects `x86_64` or `aarch64`, verifies the published SHA-256 checksum, and installs both binaries into `/usr/local/bin`:

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

Ensure Cargo's binary directory is on `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

## From source

Funzzy requires Rust and Cargo. The minimum supported Rust version is declared by `rust-version` in `Cargo.toml`.

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

Continue with the [one-minute start](../README.md#start-in-one-minute) or the complete [usage guide](USAGE.md).
