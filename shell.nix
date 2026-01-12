{
  pkgs ? import <nixpkgs> {},
  srcpkgs ? import ./packages.nix {}
}:
pkgs.mkShell {
  packages = with pkgs; [
    ## funzzy local
    # srcpkgs.local

    rustc
    cargo
    rustfmt
    libiconv

    gnused # for macos

    yq-go # jq for yaml

    fzf # Used in scripts

    # For development install latest version of funzzy
    # copkgs.funzzyNightly

    # if system contains "darwin" then darwin.apple_sdk.frameworks.CoreServices else null
    # Fix error: `ld: framework not found CoreServices`
    (if system == "x86_64-darwin" ||
    system == "aarch64-darwin"
    then darwin.apple_sdk.frameworks.CoreServices
    else null)

    unstable.beads
  ];

  shellHook = ''
    echo "$@"

    cargo update
    cargo build
  '';
}
