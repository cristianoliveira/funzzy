#!/usr/bin/env bash
#
# Used in:
#   nix/package.nix
#   .watch.yaml

set -e

# Try to remove and ignore errors
mkdir -p /tmp/fzz

touch "/tmp/fzz/accepts_full_or_relativepaths.txt"
touch "/tmp/fzz/accepts_full_or_relativepaths2.txt"
touch "/tmp/fzz/accepts_full_or_relativepaths3.txt"
