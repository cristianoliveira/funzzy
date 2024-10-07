{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    packages = with pkgs; [
        rustc
        cargo
        rustfmt
        libiconv

        gnused # for macos

        yq-go # jq for yaml

        # For development install latest version of funzzy
        # copkgs.funzzyNightly

        # if system contains "darwin" then darwin.apple_sdk.frameworks.CoreServices else null
        # Fix error: `ld: framework not found CoreServices`
        (if system == "x86_64-darwin" || 
        system == "aarch64-darwin" 
        then darwin.apple_sdk.frameworks.CoreServices
        else null)
    ];

    shellHook = ''
      cargo update
    '';
  }
