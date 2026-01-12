{
  description = "Funzzy (fzz) - the lightweight blazingly fast watcher";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, nixpkgs-unstable, ... }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [
            (final: prev: {
              unstable = import nixpkgs-unstable { inherit system; };
            })
          ];
        };
        srcpkgs = import ./default.nix { inherit pkgs; };
      in {
        packages = srcpkgs;

        devShells.default = pkgs.callPackage ./shell.nix { inherit pkgs srcpkgs; };
    });
}
