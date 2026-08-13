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

    # NOTE: legacy darwin.apple_sdk.frameworks references were removed in
    # nixpkgs; the default SDK (via SDKROOT) provides CoreServices now.
  ];

  shellHook = ''
    echo "$@"

    cargo update
    cargo build
  '';
}
