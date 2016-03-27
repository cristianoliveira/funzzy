#!/usr/bin/env bash

sudo apt-get update

# Git
sudo apt-get -y install git curl

# Rust
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Clone and install
git clone https://github.com/cristianoliveira/funzzy.git
cd funzzy
cargo test
sudo make install
